//! Bounded clipboard history with optional disk persistence.
//!
//! POC: text only. Images, files, and rich content are out of scope until the
//! tray UX for them is designed.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub text: String,
    /// Unix seconds — tray UI uses this for "10 min ago" labels later.
    pub captured_at: i64,
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

    /// Push a clipboard text. Dedupes against the most-recent entry, moves
    /// duplicates further back to the front, and trims to `cap`.
    pub fn push(&mut self, text: String, cap: usize) {
        if text.is_empty() {
            return;
        }
        if let Some(pos) = self.entries.iter().position(|e| e.text == text) {
            // Already present — bump to front instead of inserting a dup.
            let entry = self.entries.remove(pos).expect("position was valid");
            self.entries.push_front(entry);
            return;
        }
        self.entries.push_front(Entry {
            text,
            captured_at: now_secs(),
        });
        while self.entries.len() > cap {
            self.entries.pop_back();
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into());
        std::fs::write(path, json)
    }
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
