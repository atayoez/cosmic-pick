//! Persistent settings, stored via `cosmic_config`.
//!
//! Lives at `~/.config/cosmic/io.github.atayozcan.CosmicPick/v1/<field>`,
//! one RON-encoded file per field. cosmic_config is the COSMIC-native
//! config story (used by all upstream applets), gives us cross-process
//! live reload via inotify for free, and lets a future
//! `cosmic-settings` integration discover the schema without changes
//! here.

use cosmic_config::{Config, CosmicConfigEntry};
// Derive macro lives in the sibling `cosmic-config-derive` crate.
// Macros and traits inhabit different namespaces, so this second
// `use` doesn't shadow the trait import.
use cosmic_config::cosmic_config_derive::CosmicConfigEntry;
use serde::{Deserialize, Serialize};

use crate::APP_ID;

pub const CONFIG_VERSION: u64 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, CosmicConfigEntry)]
#[version = 1]
pub struct PickConfig {
    /// Max number of distinct clipboard entries to keep.
    pub history_size: u32,
    /// Clipboard poll interval. Lower = snappier, higher = less CPU.
    pub poll_interval_ms: u64,
    /// Persist clipboard history across applet restarts.
    pub persist_history: bool,
    /// Reject clipboard entries longer than this; avoids logging
    /// huge pastes.
    pub max_entry_chars: u32,
}

impl Default for PickConfig {
    fn default() -> Self {
        Self {
            history_size: 50,
            poll_interval_ms: 500,
            persist_history: true,
            max_entry_chars: 10_000,
        }
    }
}

pub fn handler() -> Result<Config, cosmic_config::Error> {
    Config::new(APP_ID, CONFIG_VERSION)
}

pub fn load() -> PickConfig {
    let Ok(h) = handler() else {
        return PickConfig::default();
    };
    match PickConfig::get_entry(&h) {
        Ok(cfg) => cfg,
        Err((errs, cfg)) => {
            for e in errs {
                eprintln!("cosmic-pick: config: {e}");
            }
            cfg
        }
    }
}

pub fn save(cfg: &PickConfig) -> Result<(), cosmic_config::Error> {
    let h = handler()?;
    cfg.write_entry(&h)
}
