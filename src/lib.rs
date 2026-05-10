//! Shared types for cosmic-pick: a unified panel applet that lets
//! you pick from clipboard history, emoji, or a color palette and
//! drop the result into the clipboard.

pub mod colors;
pub mod config;
pub mod emoji_recents;
pub mod history;
pub mod localize;
pub mod recent_colors;

pub const APP_ID: &str = "io.github.atayozcan.CosmicPick";
pub const BIN_NAME: &str = "cosmic-pick";

fn data_root() -> std::path::PathBuf {
    dirs::data_dir()
        .expect("no XDG_DATA_HOME and no $HOME")
        .join(BIN_NAME)
}

/// `$XDG_DATA_HOME/cosmic-pick/clipboard.json` — clipboard history.
pub fn history_path() -> std::path::PathBuf {
    data_root().join("clipboard.json")
}

/// `$XDG_DATA_HOME/cosmic-pick/emoji-recents.json` — recently picked emoji.
pub fn emoji_recents_path() -> std::path::PathBuf {
    data_root().join("emoji-recents.json")
}

/// `$XDG_DATA_HOME/cosmic-pick/color-recents.json` — recently picked colors.
pub fn color_recents_path() -> std::path::PathBuf {
    data_root().join("color-recents.json")
}
