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

/// `$XDG_DATA_HOME/cosmic-pick/blobs/` — binary clipboard payloads
/// (images, HTML). One file per content hash; entries in
/// `clipboard.json` reference these by `<hash>.<ext>`.
pub fn blobs_dir() -> std::path::PathBuf {
    data_root().join("blobs")
}

/// Compute the content hash for a blob filename. blake3 is fast and
/// stable across runs, and using the hash as the filename gives us
/// dedup for free: copying the same image twice is one file on disk.
pub fn blob_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Write `bytes` to `blobs/<hash>.<extension>` if it isn't already
/// there. Returns the hash (the on-disk filename stem). Idempotent.
pub fn store_blob(bytes: &[u8], extension: &str) -> std::io::Result<String> {
    let hash = blob_hash(bytes);
    let dir = blobs_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{hash}.{extension}"));
    if !path.exists() {
        std::fs::write(&path, bytes)?;
    }
    Ok(hash)
}

/// Path to the file that backs a blob hash with a given extension.
/// Does not check existence.
pub fn blob_path(hash: &str, extension: &str) -> std::path::PathBuf {
    blobs_dir().join(format!("{hash}.{extension}"))
}

/// Delete any blob files whose hash isn't in `keep`. Called after a
/// history mutation that removes entries (Clear, cap eviction,
/// unpinning then trim). Best-effort — IO errors are ignored.
pub fn gc_blobs(keep: &std::collections::HashSet<String>) {
    let Ok(dir) = std::fs::read_dir(blobs_dir()) else { return };
    for entry in dir.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if !keep.contains(stem) {
            let _ = std::fs::remove_file(path);
        }
    }
}
