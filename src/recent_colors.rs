//! Bounded "recently picked colors" list with JSON persistence.
//!
//! Colors are stored as 7-character lowercase `#rrggbb` strings —
//! that's the format the popover hands back to the clipboard.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;

const MAX: usize = 16;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RecentColors {
    entries: VecDeque<String>,
}

impl RecentColors {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Push a color (case-insensitively normalized to `#rrggbb`),
    /// dedupe + trim.
    pub fn push(&mut self, color: &str) {
        let Some(normalized) = normalize_hex(color) else {
            return;
        };
        if let Some(pos) = self.entries.iter().position(|e| *e == normalized) {
            let v = self.entries.remove(pos).expect("position was valid");
            self.entries.push_front(v);
            return;
        }
        self.entries.push_front(normalized);
        while self.entries.len() > MAX {
            self.entries.pop_back();
        }
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

/// Accept `#rrggbb`, `#rgb`, or bare-hex variants; emit
/// `#rrggbb` lowercase. Returns `None` if it can't be parsed.
pub fn normalize_hex(input: &str) -> Option<String> {
    let s = input.trim().trim_start_matches('#');
    let bytes = s.as_bytes();
    let (r, g, b) = match bytes.len() {
        3 => {
            let r = u8::from_str_radix(&s[0..1], 16).ok()?;
            let g = u8::from_str_radix(&s[1..2], 16).ok()?;
            let b = u8::from_str_radix(&s[2..3], 16).ok()?;
            (r * 17, g * 17, b * 17)
        }
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            (r, g, b)
        }
        _ => return None,
    };
    Some(format!("#{:02x}{:02x}{:02x}", r, g, b))
}
