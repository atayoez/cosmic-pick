#!/usr/bin/env bash
# cosmic-pick installer — places the panel-applet binary and the
# settings binary under `$XDG_BIN_HOME` (or ~/.local/bin). Per-user,
# no root required.
#
# cosmic-pick is a cosmic-panel applet, NOT a daemon. The panel
# spawns the binary as needed; this installer only deposits files.
#
# Cleans up artifacts from previous installs (cosmic-clip SNI tray,
# cosmic-clip-applet prototype, cosmic-emoji standalone, the old
# .desktop launcher, autostart entries) so the upgrade is hands-off.
#
# Usage:
#   ./install.sh             # build + install
#   ./install.sh --uninstall # remove everything this script wrote

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
cd "$SCRIPT_DIR"

BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
ICON_DIR="$DATA_DIR/icons/hicolor/scalable/apps"
APPS_DIR="$DATA_DIR/applications"
AUTOSTART_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"

# Files this installer (current or previous-name versions) has
# written. Used by both install cleanup and --uninstall.
OWNED_FILES=(
    # current cosmic-pick
    "$BIN_DIR/cosmic-pick"
    "$BIN_DIR/cosmic-pick-settings"
    "$APPS_DIR/io.github.atayozcan.CosmicPick.desktop"
    # pre-rename cosmic-clip SNI + applet prototype + cosmic-emoji
    "$BIN_DIR/cosmic-clip"
    "$BIN_DIR/cosmic-clip-applet"
    "$BIN_DIR/cosmic-emoji"
    "$ICON_DIR/cosmic-clip-symbolic.svg"
    "$APPS_DIR/cosmic-clip.desktop"
    "$APPS_DIR/cosmic-clip-settings.desktop"
    "$APPS_DIR/cosmic-emoji.desktop"
    "$APPS_DIR/io.github.atayozcan.CosmicClipApplet.desktop"
    "$AUTOSTART_DIR/cosmic-clip.desktop"
)

clean_old_artifacts() {
    local removed=0
    for f in "${OWNED_FILES[@]}"; do
        if [[ -e "$f" ]]; then
            rm -f "$f" && removed=$((removed + 1))
        fi
    done
    if (( removed > 0 )); then
        echo "cosmic-pick: cleaned up $removed stale file(s)."
    fi
}

refresh_caches() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$APPS_DIR" 2>/dev/null || true
    fi
}

# Stop any running pre-rename binaries so the panel can pick up the
# new applet on its next scan.
stop_old_processes() {
    for name in cosmic-clip cosmic-clip-applet cosmic-emoji cosmic-pick; do
        if pgrep -x "$name" >/dev/null 2>&1; then
            pkill -x "$name" 2>/dev/null || true
        fi
    done
    sleep 0.2
}

uninstall() {
    echo "cosmic-pick: uninstalling..."
    stop_old_processes
    clean_old_artifacts
    refresh_caches
    echo "cosmic-pick: uninstalled. (Remove the applet from your panel via cosmic-settings.)"
}

if [[ "${1:-}" == "--uninstall" ]]; then
    uninstall
    exit 0
fi

echo "cosmic-pick: building (cargo build --release)..."
cargo build --release

stop_old_processes

echo "cosmic-pick: cleaning previous install..."
clean_old_artifacts

mkdir -p "$BIN_DIR" "$APPS_DIR"

install -m 0755 target/release/cosmic-pick "$BIN_DIR/cosmic-pick"
install -m 0755 target/release/cosmic-pick-settings "$BIN_DIR/cosmic-pick-settings"

# cosmic-panel discovers applets by their APP_ID-named .desktop file
# in $XDG_DATA_HOME/applications. The Name= is what shows up in the
# panel's "Add Applet" picker.
sed "s|@BIN@|$BIN_DIR/cosmic-pick|g" resources/cosmic-pick.desktop \
    > "$APPS_DIR/io.github.atayozcan.CosmicPick.desktop"
chmod 0644 "$APPS_DIR/io.github.atayozcan.CosmicPick.desktop"

refresh_caches

cat <<EOF
cosmic-pick: installed.

  Applet:   $BIN_DIR/cosmic-pick
  Settings: $BIN_DIR/cosmic-pick-settings
  Manifest: $APPS_DIR/io.github.atayozcan.CosmicPick.desktop

To attach to the panel:
  cosmic-settings → Panel → <Top|Bottom|Dock> → Add Applet → Pick

The applet IS the long-running process — cosmic-panel spawns it
when you add it to the panel and keeps it alive. There's no
separate daemon, no autostart entry to manage.

To uninstall: ./install.sh --uninstall
EOF
