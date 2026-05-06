#!/usr/bin/env bash
# cosmic-clip installer — places binaries, icons, and desktop entries under
# $XDG_DATA_HOME (or ~/.local/share). Per-user, no root required.
#
# Usage:
#   ./install.sh             # builds release + installs both binaries
#   ./install.sh --uninstall # removes everything this script wrote

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
cd "$SCRIPT_DIR"

BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
ICON_DIR="$DATA_DIR/icons/hicolor/scalable/apps"
APPS_DIR="$DATA_DIR/applications"
AUTOSTART_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"

uninstall() {
    echo "cosmic-clip: uninstalling..."
    rm -f "$BIN_DIR/cosmic-clip" "$BIN_DIR/cosmic-clip-settings"
    rm -f "$ICON_DIR/cosmic-clip-symbolic.svg"
    rm -f "$APPS_DIR/cosmic-clip.desktop" "$APPS_DIR/cosmic-clip-settings.desktop"
    rm -f "$AUTOSTART_DIR/cosmic-clip.desktop"
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -f -t "$DATA_DIR/icons/hicolor" 2>/dev/null || true
    fi
    echo "cosmic-clip: uninstalled."
}

if [[ "${1:-}" == "--uninstall" ]]; then
    uninstall
    exit 0
fi

echo "cosmic-clip: building (cargo build --release)..."
cargo build --release

mkdir -p "$BIN_DIR" "$ICON_DIR" "$APPS_DIR"

install -m 0755 target/release/cosmic-clip "$BIN_DIR/cosmic-clip"
install -m 0755 target/release/cosmic-clip-settings "$BIN_DIR/cosmic-clip-settings"
install -m 0644 resources/icons/cosmic-clip-symbolic.svg "$ICON_DIR/cosmic-clip-symbolic.svg"

# Render desktop files with the actual binary path
sed "s|@BIN@|$BIN_DIR/cosmic-clip|" resources/cosmic-clip.desktop \
    > "$APPS_DIR/cosmic-clip.desktop"
sed "s|@BIN@|$BIN_DIR/cosmic-clip-settings|" resources/cosmic-clip-settings.desktop \
    > "$APPS_DIR/cosmic-clip-settings.desktop"

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$DATA_DIR/icons/hicolor" 2>/dev/null || true
fi

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "cosmic-clip: warning: $BIN_DIR is not in PATH; add it to your shell rc." ;;
esac

cat <<EOF
cosmic-clip: installed.

  Daemon:   $BIN_DIR/cosmic-clip
  Settings: $BIN_DIR/cosmic-clip-settings
  Icon:     $ICON_DIR/cosmic-clip-symbolic.svg
  Launchers: $APPS_DIR/cosmic-clip{,-settings}.desktop

Next steps:
  - Run 'cosmic-clip' to start the tray daemon.
  - Run 'cosmic-clip-settings' to tune history size, poll cadence, and autostart.
EOF
