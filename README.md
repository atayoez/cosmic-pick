# cosmic-clip

> Status: **proof of concept**. Daemon polls clipboard, keeps history,
> tray menu re-copies entries. Image / file payloads not handled yet.

A Wayland-native clipboard history manager with a `StatusNotifierItem`
tray icon and a libcosmic settings GUI. Built to fill a gap in the
COSMIC desktop — works under any DE that consumes
`org.kde.StatusNotifierItem` (KDE, Sway+waybar, Hyprland, COSMIC).

## What it does

- Polls the clipboard every N ms (default 500) and stores the last N
  distinct text entries (default 50)
- Tray icon → menu of recent entries; click one to copy it back
- "Clear History" wipes both memory and the on-disk cache
- "Settings…" launches `cosmic-clip-settings` (libcosmic GUI)
- History persists at `~/.local/share/cosmic-clip/history.json`
  (toggleable in settings)

## What it does not (yet) do

- No image/file clipboard support — text only
- No global hotkey to open the menu
- No fuzzy search across entries
- No password / sensitive-content masking heuristics
- Polling-based (no `wlr-data-control` event subscription)

These are POC limits, not by-design. PRs welcome.

## Install

```sh
git clone https://github.com/atayozcan/cosmic-clip
cd cosmic-clip
./install.sh
```

That installs:

| Path | What |
| --- | --- |
| `~/.local/bin/cosmic-clip` | tray daemon |
| `~/.local/bin/cosmic-clip-settings` | libcosmic settings GUI |
| `~/.local/share/icons/hicolor/scalable/apps/cosmic-clip-symbolic.svg` | tray icon |
| `~/.local/share/applications/cosmic-clip{,-settings}.desktop` | app-menu launchers |

`./install.sh --uninstall` removes everything the installer wrote.

### Build deps (Arch)

```sh
pkexec pacman -S --needed rust pkgconf libxkbcommon wayland mesa \
    vulkan-icd-loader fontconfig freetype2
```

## Run

```sh
cosmic-clip &
```

Or enable the *Start cosmic-clip on login* toggle in settings.

## Config

`~/.config/cosmic-clip/config.toml`:

```toml
history_size      = 50
poll_interval_ms  = 500
persist_history   = true
max_entry_chars   = 10000
```

## License

MIT — see `LICENSE`.
