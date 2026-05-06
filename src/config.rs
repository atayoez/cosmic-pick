use serde::{Deserialize, Serialize};
use std::path::Path;

pub fn default_history_size() -> usize {
    50
}
pub fn default_poll_ms() -> u64 {
    500
}
pub fn default_persist() -> bool {
    true
}
pub fn default_max_entry_chars() -> usize {
    10_000
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Config {
    #[serde(default = "default_history_size")]
    pub history_size: usize,
    #[serde(default = "default_poll_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_persist")]
    pub persist_history: bool,
    #[serde(default = "default_max_entry_chars")]
    pub max_entry_chars: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            history_size: default_history_size(),
            poll_interval_ms: default_poll_ms(),
            persist_history: default_persist(),
            max_entry_chars: default_max_entry_chars(),
        }
    }
}

pub fn read(path: &Path) -> Result<Config, String> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let s = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    toml::from_str(&s).map_err(|e| format!("parse {}: {e}", path.display()))
}

pub fn write(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = String::new();
    out.push_str("# cosmic-clip config\n\n");
    out.push_str(&format!("history_size      = {}\n", cfg.history_size));
    out.push_str(&format!("poll_interval_ms  = {}\n", cfg.poll_interval_ms));
    out.push_str(&format!("persist_history   = {}\n", cfg.persist_history));
    out.push_str(&format!("max_entry_chars   = {}\n", cfg.max_entry_chars));
    std::fs::write(path, out)
}
