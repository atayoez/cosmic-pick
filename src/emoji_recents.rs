//! Bounded "recently picked emoji" list with JSON persistence.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;

/// Cap on stored recents — one full popover row plus some overflow.
const MAX: usize = 20;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EmojiRecents {
    entries: VecDeque<String>,
}

impl EmojiRecents {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn push(&mut self, emoji: &str) {
        if emoji.is_empty() {
            return;
        }
        if let Some(pos) = self.entries.iter().position(|e| e == emoji) {
            let v = self.entries.remove(pos).expect("position was valid");
            self.entries.push_front(v);
            return;
        }
        self.entries.push_front(emoji.to_string());
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
