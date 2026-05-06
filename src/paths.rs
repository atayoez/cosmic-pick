use std::path::PathBuf;

pub const APP_ID: &str = "io.github.atayozcan.CosmicClip";

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .expect("no XDG_CONFIG_HOME")
        .join("cosmic-clip/config.toml")
}

pub fn history_path() -> PathBuf {
    dirs::data_dir()
        .expect("no XDG_DATA_HOME")
        .join("cosmic-clip/history.json")
}

pub fn autostart_path() -> PathBuf {
    dirs::config_dir()
        .expect("no XDG_CONFIG_HOME")
        .join("autostart/cosmic-clip.desktop")
}

pub fn self_exec() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "cosmic-clip".to_string())
}

/// Locate the settings GUI binary. Prefer the one next to the daemon, then
/// fall back to PATH.
pub fn settings_exec() -> Option<PathBuf> {
    if let Ok(self_path) = std::env::current_exe() {
        if let Some(parent) = self_path.parent() {
            let candidate = parent.join("cosmic-clip-settings");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    which("cosmic-clip-settings")
}

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|p| p.join(bin))
            .find(|p| p.exists())
    })
}
