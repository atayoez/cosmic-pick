//! Bounded clipboard history with optional disk persistence.
//!
//! Stores text, images (referenced by blob hash), file URI lists,
//! and rich HTML (text fallback inline + HTML body in a blob).
//!
//! Pinned entries float to the top, are not evicted by the size cap,
//! and survive `clear()`.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;

/// Type of payload an entry carries. Text is the default so older
/// `clipboard.json` files (pre-multi-mime) deserialize without
/// migration: missing `payload` collapses to `Payload::Text`.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Payload {
    #[default]
    Text,
    /// PNG / JPEG / WebP image stored as a side-file in
    /// `blobs/<blob>.<ext>`.
    Image {
        /// Content hash → filename stem in `blobs/`.
        blob: String,
        /// Mime type, e.g. "image/png".
        mime: String,
        /// File extension (without the dot), e.g. "png".
        ext: String,
        /// Byte length, for the popup label.
        bytes: u64,
    },
    /// `text/uri-list` payload — typically file URIs from a file
    /// manager. Short enough to inline.
    Files {
        uris: Vec<String>,
    },
    /// Rich text. The plain-text fallback lives on `Entry.text`;
    /// the HTML body is in `blobs/<blob>.html`.
    Html {
        blob: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    /// Searchable, displayable label. For Text variants this is the
    /// content; for Image / Files / Html it's a synthesized label
    /// like "[image 800×600 · 124 KiB]" or "Documents/foo.txt"
    /// (matching what the popup shows).
    pub text: String,
    /// Unix seconds.
    pub captured_at: i64,
    /// Pinned entries float to the top, survive Clear History, and
    /// are never evicted by the cap. Serde defaults to `false` so
    /// older on-disk histories deserialize cleanly.
    #[serde(default)]
    pub pinned: bool,
    /// Detected by `looks_sensitive` on capture (Private keys,
    /// JWTs, common API-key prefixes, high-entropy alphanumeric
    /// blobs). Sensitive entries render as `••••` in the popup
    /// and are skipped on save — they live only in the running
    /// applet's memory, never on disk.
    #[serde(default)]
    pub sensitive: bool,
    /// What kind of payload this is. Defaults to Text for back-
    /// compat with pre-multi-mime history.json files.
    #[serde(default)]
    pub payload: Payload,
}

impl Entry {
    /// Whether two entries refer to the same content, used for
    /// dedup on push. Text dedups by text body; Image / Html dedup
    /// by blob hash; Files dedup by URI list.
    fn same_content(&self, other: &Entry) -> bool {
        match (&self.payload, &other.payload) {
            (Payload::Text, Payload::Text) => self.text == other.text,
            (Payload::Image { blob: a, .. }, Payload::Image { blob: b, .. }) => a == b,
            (Payload::Files { uris: a }, Payload::Files { uris: b }) => a == b,
            (Payload::Html { blob: a }, Payload::Html { blob: b }) => a == b,
            _ => false,
        }
    }

    /// All blob hashes this entry references. Used by the GC to
    /// figure out which files in `blobs/` are still live.
    pub fn blob_hashes(&self) -> impl Iterator<Item = &str> {
        let mut hashes: Vec<&str> = Vec::new();
        match &self.payload {
            Payload::Image { blob, .. } | Payload::Html { blob } => hashes.push(blob.as_str()),
            Payload::Text | Payload::Files { .. } => {}
        }
        hashes.into_iter()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct History {
    entries: VecDeque<Entry>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    pub fn get(&self, idx: usize) -> Option<&Entry> {
        self.entries.get(idx)
    }

    /// Push an entry. Dedupes against existing entries (matching
    /// content), moves the duplicate to the top of its section
    /// (pinned vs unpinned), and trims to `cap` by evicting the
    /// oldest unpinned entry. If every entry is pinned, the cap is
    /// exceeded — pinning always wins over the size limit.
    pub fn push(&mut self, entry: Entry, cap: usize) {
        if entry.text.is_empty() && matches!(entry.payload, Payload::Text) {
            return;
        }
        if let Some(pos) = self.entries.iter().position(|e| e.same_content(&entry)) {
            // Already present — bump to top of its section, preserve pin.
            let mut existing = self.entries.remove(pos).expect("position was valid");
            existing.captured_at = entry.captured_at;
            self.insert_in_section(existing);
            return;
        }
        self.insert_in_section(entry);
        while self.entries.len() > cap {
            let last_unpinned = self.entries.iter().rposition(|e| !e.pinned);
            match last_unpinned {
                Some(pos) => {
                    self.entries.remove(pos);
                }
                None => break,
            }
        }
    }

    /// Toggle the pin state of the entry at `idx`. Re-sorts so
    /// pinned entries always come first. Returns the new pinned
    /// state, or None if the index was out of range.
    pub fn toggle_pin(&mut self, idx: usize) -> Option<bool> {
        let entry = self.entries.get_mut(idx)?;
        entry.pinned = !entry.pinned;
        let new_state = entry.pinned;
        let (pinned, unpinned): (VecDeque<Entry>, VecDeque<Entry>) =
            std::mem::take(&mut self.entries)
                .into_iter()
                .partition(|e| e.pinned);
        self.entries = pinned;
        self.entries.extend(unpinned);
        Some(new_state)
    }

    /// Clear unpinned entries. Pinned entries persist.
    pub fn clear(&mut self) {
        self.entries.retain(|e| e.pinned);
    }

    /// Hashes of all blobs referenced by any current entry. Pass
    /// this to [`crate::gc_blobs`] after clearing or trimming.
    pub fn live_blob_hashes(&self) -> std::collections::HashSet<String> {
        self.entries
            .iter()
            .flat_map(|e| e.blob_hashes().map(String::from))
            .collect()
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist the history to disk, omitting any entry flagged
    /// `sensitive`. Pinned-but-sensitive entries are still
    /// dropped: pinning is for stickiness across the size cap and
    /// Clear, not for opting in to disk persistence.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let persistable = History {
            entries: self.entries.iter().filter(|e| !e.sensitive).cloned().collect(),
        };
        let json = serde_json::to_string_pretty(&persistable).unwrap_or_else(|_| "{}".into());
        std::fs::write(path, json)
    }

    /// Insert `entry` at the top of its section: pinned entries go
    /// to the very front; unpinned entries go just after the last
    /// pinned entry (top of the unpinned section).
    fn insert_in_section(&mut self, entry: Entry) {
        if entry.pinned {
            self.entries.push_front(entry);
        } else {
            let insert_at = self
                .entries
                .iter()
                .position(|e| !e.pinned)
                .unwrap_or(self.entries.len());
            self.entries.insert(insert_at, entry);
        }
    }
}

pub fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
