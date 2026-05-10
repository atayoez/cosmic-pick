# cosmic-clip

> Status: **proof of concept**. Daemon polls clipboard, keeps history,
> tray menu re-copies entries. Image / file payloads not handled yet.

A Wayland-native clipboard history manager with a `StatusNotifierItem`
tray icon and a libcosmic settings GUI. Built to fill a gap in the
COSMIC desktop — works under any DE that consumes
`org.kde.StatusNotifierItem` (KDE, Sway+waybar, Hyprland, COSMIC).

Sibling project to [`tb-tray`](https://github.com/atayozcan/tb-tray)
and [`cosmic-caffeine`](https://github.com/atayozcan/cosmic-caffeine);
shares the
[`cosmic-tray-app`](https://github.com/atayozcan/cosmic-tray-app)
helper crate for paths, autostart, and the single-binary
`--settings`-re-exec pattern.

## What it does

- Polls the clipboard every N ms (default 500) and stores the last N
  distinct text entries (default 50)
- Tray icon → menu of recent entries; click one to copy it back
- "Clear History" wipes both memory and the on-disk cache
- "Settings…" opens the libcosmic GUI (in a child process)
- History persists at `~/.local/share/cosmic-clip/history.json`
  (toggleable in settings)

## What it does not (yet) do

- No image/file clipboard support — text only
- No password / sensitive-content masking heuristics
- Polling-based (no `wlr-data-control` event subscription)
- Window can't anchor at the cursor position on Wayland (compositor
  decides; cosmic-comp typically centers the popup on the focused
  monitor)

These are POC limits, not by-design. PRs welcome.

## Install

```sh
git clone https://github.com/atayozcan/cosmic-clip
git clone https://github.com/atayozcan/cosmic-tray-app  # sibling lib (path dep)
cd cosmic-clip
./install.sh
```

That installs:

| Path | What |
| --- | --- |
| `~/.local/bin/cosmic-clip` | the binary (daemon + settings GUI in one) |
| `~/.local/share/icons/hicolor/scalable/apps/cosmic-clip-symbolic.svg` | tray icon |
| `~/.local/share/applications/cosmic-clip.desktop` | app-menu launcher |

Per-user, no root needed. The launcher's `Exec=` is templated with
the absolute binary path at install time so it keeps working even
when your desktop session's PATH doesn't include `~/.local/bin`. The
script cleans up artifacts from earlier installs (the obsolete
second `cosmic-clip-settings` binary, its launcher, etc.) before
laying down the new files, and (re)starts the daemon so the new
version is live immediately.

To uninstall:

```sh
./uninstall.sh
```

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

### Quick-pick popup (Super+V)

`cosmic-clip --popup` opens a small libcosmic window with a search
box and the recent history; clicking an entry copies it back to
**both** clipboards (regular + primary) and closes the window. Bind
this to a chord in cosmic-settings → Keyboard Shortcuts → Custom →
add a shortcut whose command is:

```
cosmic-clip --popup
```

A common chord is **Super+V**. Esc closes the popup without copying.

### Both clipboards in sync

Whatever you copy via Ctrl+C lands in the primary selection too, and
text you highlight (primary selection) lands in the regular
clipboard — they're mirrored on every poll. The history records
either flavor of "new content" once.

## Config

Stored via `cosmic_config` at
`~/.config/cosmic/io.github.atayozcan.CosmicClip/v1/`, one
RON-encoded file per field. Fields:

| Field | Type | Default | What |
| --- | --- | --- | --- |
| `history_size` | u32 | `50` | Max number of distinct entries kept |
| `poll_interval_ms` | u64 | `500` | Clipboard poll cadence |
| `persist_history` | bool | `true` | Save history to disk across restarts |
| `max_entry_chars` | u32 | `10000` | Reject entries longer than this |

Changes from the settings GUI propagate to the running daemon
without a restart — config is re-read each time the daemon's watch
or save loop iterates.

## License

MIT — see `LICENSE`.
