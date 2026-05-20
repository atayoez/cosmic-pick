// cosmic-pick: unified panel applet for picking from clipboard
// history, emoji, and a color palette. Single panel button →
// tabbed popover. Click any item → copy to clipboard + close.
//
// Hosted by cosmic-panel; no SNI tray, no daemon, no autostart.
// The clipboard watcher runs as an iced Subscription for the
// lifetime of the applet.

use cosmic::app::{Core, Task};
use cosmic::iced::futures::channel::mpsc;
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::{event, keyboard, stream, window, Background, Color, Event, Length, Subscription};
use cosmic::prelude::*;
use cosmic::widget;
use std::process::Command;

use cosmic_pick::colors::{hex_to_rgb, PALETTE};
use cosmic_pick::config;
use cosmic_pick::emoji_recents::EmojiRecents;
use cosmic_pick::history::{now_secs, Entry, History, Payload};
use cosmic_pick::recent_colors::{normalize_hex, RecentColors};
use cosmic_pick::{
    blob_hash, blob_path, color_recents_path, emoji_recents_path, fl, gc_blobs, history_path,
    localize, store_blob, APP_ID, BIN_NAME,
};

/// Emoji-grid columns when filling the emoji tab.
const EMOJI_COLUMNS: usize = 10;
/// Color-swatch columns.
const COLOR_COLUMNS: usize = 6;
/// Cap on the no-search emoji view so we don't render thousands of buttons.
const EMOJI_LIMIT: usize = 240;
/// Fixed height of the scrollable popover body — without this, the
/// emoji tab in particular makes the popup grow to fill the screen.
const BODY_HEIGHT: f32 = 360.0;

fn main() -> cosmic::iced::Result {
    localize::localize();
    cosmic::applet::run::<App>(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Clipboard,
    Emoji,
    Color,
}

#[derive(Clone, Debug)]
pub enum Message {
    TogglePopup,
    PopupClosed(window::Id),
    Tab(Tab),
    Search(String),
    HexInput(String),
    PickClipboard(usize),
    PickEmoji(String),
    PickColor(String),
    /// Toggle pin on the clipboard entry at this index. Pinned
    /// entries float to the top, survive Clear History, and never get
    /// evicted by the size cap.
    TogglePin(usize),
    /// Keyboard shortcut: pick the Nth (0-based) entry from the
    /// currently filtered clipboard list. Bound to keys 1-9.
    PickByNumber(usize),
    /// Keyboard shortcut: focus the search input. Bound to '/'.
    FocusSearch,
    ClearClipboard,
    OpenSettings,
    /// New clipboard content observed by the watcher (any payload).
    Captured(Entry),
    /// Primary selection has drifted from the regular clipboard
    /// (the user highlighted text). Re-snap primary back to the
    /// regular clipboard's text content. Only applies when the
    /// regular clipboard is text — for image / files / html the
    /// watcher just clears primary to avoid a type mismatch.
    MirrorPrimary(String),
    /// Esc key pressed — close the popup if open, otherwise no-op.
    CloseEscape,
}

pub struct App {
    core: Core,
    popup: Option<window::Id>,
    tab: Tab,
    search: String,
    /// Stable id of the search input so the `/` keyboard shortcut
    /// can target it via `widget::text_input::focus`. Per-applet-
    /// instance unique — generated in `init`.
    search_id: widget::Id,
    /// Hex color the user is composing in the color tab. May be a
    /// partial entry (e.g. "#ff" while typing).
    hex_input: String,
    history: History,
    emoji_recents: EmojiRecents,
    color_recents: RecentColors,
}

impl App {
    /// Write `text` to both the regular clipboard and the primary
    /// selection. Both go through `wl-copy` rather than the in-process
    /// iced/smithay-clipboard path: smithay-clipboard refuses to set
    /// a selection unless our applet has Wayland keyboard focus
    /// (`state.rs:146` — `if !seat.has_focus { return None; }`), so
    /// every write from the background watcher silently no-ops. A
    /// short-lived `wl-copy` subprocess is its own focus-independent
    /// Wayland client and the writes actually stick.
    fn copy_to_clipboards_side_effect(text: String) {
        wl_copy_write(text.clone(), /* primary */ false);
        wl_copy_write(text, /* primary */ true);
    }

    /// Re-publish a history entry to the live clipboard. Text goes
    /// to both clipboard + primary; non-text goes only to the
    /// regular clipboard with the right `--type` (primary is text-
    /// only by convention, so we clear it instead of writing a
    /// mismatched mime there).
    fn write_entry_to_clipboards(entry: &Entry) {
        match &entry.payload {
            Payload::Text => Self::copy_to_clipboards_side_effect(entry.text.clone()),
            Payload::Image { blob, mime, ext, .. } => {
                wl_copy_file(blob_path(blob, ext), mime.clone(), /* primary */ false);
                wl_copy_clear(/* primary */ true);
            }
            Payload::Files { uris } => {
                let body = uris.join("\n");
                wl_copy_write_typed(body, "text/uri-list".into(), /* primary */ false);
                wl_copy_clear(/* primary */ true);
            }
            Payload::Html { blob } => {
                wl_copy_file(blob_path(blob, "html"), "text/html".into(), /* primary */ false);
                // Also publish the plain-text fallback so paste into
                // a non-rich field still gets sensible text.
                wl_copy_write(entry.text.clone(), /* primary */ false);
                wl_copy_write(entry.text.clone(), /* primary */ true);
            }
        }
    }

    fn save_history_async(&self) {
        let path = history_path();
        let snapshot = self.history.clone();
        std::thread::spawn(move || {
            let _ = snapshot.save(&path);
        });
    }

    fn save_emoji_recents_async(&self) {
        let path = emoji_recents_path();
        let snapshot = self.emoji_recents.clone();
        std::thread::spawn(move || {
            let _ = snapshot.save(&path);
        });
    }

    fn save_color_recents_async(&self) {
        let path = color_recents_path();
        let snapshot = self.color_recents.clone();
        std::thread::spawn(move || {
            let _ = snapshot.save(&path);
        });
    }

    fn close_popup_task(&mut self) -> Task<Message> {
        if let Some(p) = self.popup.take() {
            destroy_popup(p)
        } else {
            Task::none()
        }
    }

    /// Filter the clipboard history by the current search needle.
    fn filtered_clipboard(&self) -> Vec<usize> {
        if self.search.is_empty() {
            return (0..self.history.len()).collect();
        }
        let needle = self.search.to_lowercase();
        self.history
            .iter()
            .enumerate()
            .filter(|(_, e)| e.text.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }

    fn filtered_emoji(&self) -> Vec<&'static emojis::Emoji> {
        if self.search.is_empty() {
            return emojis::iter().take(EMOJI_LIMIT).collect();
        }
        let needle = self.search.to_lowercase();
        emojis::iter()
            .filter(|e| e.name().to_lowercase().contains(&needle))
            .take(EMOJI_LIMIT)
            .collect()
    }

    fn filtered_palette(&self) -> Vec<(&'static str, &'static str)> {
        if self.search.is_empty() {
            return PALETTE.iter().copied().collect();
        }
        let needle = self.search.to_lowercase();
        PALETTE
            .iter()
            .copied()
            .filter(|(name, _)| name.to_lowercase().contains(&needle))
            .collect()
    }
}

fn preview_label(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    let max = 80;
    if trimmed.chars().count() > max {
        let cut: String = trimmed.chars().take(max).collect();
        format!("{cut}…")
    } else if trimmed.is_empty() {
        fl!("preview-whitespace-only")
    } else {
        trimmed.to_string()
    }
}

impl cosmic::Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _: ()) -> (Self, Task<Message>) {
        let history = History::load(&history_path());
        let emoji_recents = EmojiRecents::load(&emoji_recents_path());
        let color_recents = RecentColors::load(&color_recents_path());
        (
            App {
                core,
                popup: None,
                tab: Tab::Clipboard,
                search: String::new(),
                search_id: widget::Id::unique(),
                hex_input: String::new(),
                history,
                emoji_recents,
                color_recents,
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TogglePopup => {
                if let Some(p) = self.popup.take() {
                    return destroy_popup(p);
                }
                self.search.clear();
                self.hex_input.clear();
                let new_id = window::Id::unique();
                self.popup = Some(new_id);
                let popup_settings = self.core.applet.get_popup_settings(
                    self.core.main_window_id().expect("applet has main window"),
                    new_id,
                    None,
                    None,
                    None,
                );
                get_popup(popup_settings)
            }
            Message::PopupClosed(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                }
                Task::none()
            }
            Message::Tab(t) => {
                self.tab = t;
                self.search.clear();
                Task::none()
            }
            Message::Search(s) => {
                self.search = s;
                Task::none()
            }
            Message::HexInput(s) => {
                self.hex_input = s.chars().take(7).collect();
                Task::none()
            }
            Message::PickClipboard(idx) => {
                if let Some(entry) = self.history.get(idx).cloned() {
                    Self::write_entry_to_clipboards(&entry);
                }
                self.close_popup_task()
            }
            Message::PickEmoji(emoji) => {
                self.emoji_recents.push(&emoji);
                self.save_emoji_recents_async();
                Self::copy_to_clipboards_side_effect(emoji);
                self.close_popup_task()
            }
            Message::PickColor(hex) => {
                if let Some(normalized) = normalize_hex(&hex) {
                    self.color_recents.push(&normalized);
                    self.save_color_recents_async();
                    Self::copy_to_clipboards_side_effect(normalized);
                }
                self.close_popup_task()
            }
            Message::TogglePin(idx) => {
                if self.history.toggle_pin(idx).is_some() {
                    self.save_history_async();
                }
                Task::none()
            }
            Message::PickByNumber(n) => {
                let indices = self.filtered_clipboard();
                let Some(&idx) = indices.get(n) else {
                    return Task::none();
                };
                if let Some(entry) = self.history.get(idx).cloned() {
                    Self::write_entry_to_clipboards(&entry);
                }
                self.close_popup_task()
            }
            Message::FocusSearch => {
                self.tab = Tab::Clipboard;
                widget::text_input::focus::<cosmic::Action<Message>>(self.search_id.clone())
            }
            Message::ClearClipboard => {
                self.history.clear();
                self.save_history_async();
                // Drop blob side-files that no pinned entry still
                // references. Done on a thread to keep the iced
                // event loop responsive even when blobs/ is large.
                let keep = self.history.live_blob_hashes();
                std::thread::spawn(move || gc_blobs(&keep));
                // Also wipe the live OS clipboards so paste-after-clear
                // doesn't surface what was just cleared from history.
                wl_copy_clear(false);
                wl_copy_clear(true);
                Task::none()
            }
            Message::OpenSettings => {
                if let Ok(exe) = std::env::current_exe() {
                    let settings_bin = exe
                        .parent()
                        .map(|p| p.join("cosmic-pick-settings"))
                        .unwrap_or_else(|| std::path::PathBuf::from("cosmic-pick-settings"));
                    let _ = Command::new(settings_bin).spawn();
                }
                self.close_popup_task()
            }
            Message::Captured(entry) => {
                let cap = (config::load().history_size as usize).max(1);
                match &entry.payload {
                    // Re-publish the trimmed text to both selections
                    // so Ctrl+V matches what's in history (the watcher
                    // trims at capture). Idempotent when the source
                    // already had clean text.
                    Payload::Text => {
                        wl_copy_write(entry.text.clone(), /* primary */ false);
                        wl_copy_write(entry.text.clone(), /* primary */ true);
                    }
                    // Files: mirror the URI list to primary as text
                    // so middle-click paste gives you the paths.
                    // Don't rewrite the regular clipboard — the file
                    // manager set it with text/uri-list + extra mimes
                    // we shouldn't clobber.
                    Payload::Files { uris } => {
                        wl_copy_write(uris.join("\n"), /* primary */ true);
                    }
                    // HTML: mirror plain-text fallback to primary so
                    // middle-click paste into a terminal works.
                    // Don't touch the regular clipboard — the source
                    // already published the rich-text payload there.
                    Payload::Html { .. } => {
                        wl_copy_write(entry.text.clone(), /* primary */ true);
                    }
                    // Image: nothing meaningful to put in primary
                    // (it's text-only by convention). Wipe it so a
                    // stale highlight doesn't linger over an image
                    // copy.
                    Payload::Image { .. } => {
                        wl_copy_clear(/* primary */ true);
                    }
                }
                self.history.push(entry, cap);
                self.save_history_async();
                let keep = self.history.live_blob_hashes();
                std::thread::spawn(move || gc_blobs(&keep));
                Task::none()
            }
            Message::MirrorPrimary(text) => {
                wl_copy_write(text, /* primary */ true);
                Task::none()
            }
            Message::CloseEscape => self.close_popup_task(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        self.core
            .applet
            .icon_button("edit-paste-symbolic")
            .on_press(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let tabs = tab_bar(self.tab);
        let search = widget::text_input(
            match self.tab {
                Tab::Clipboard => fl!("search-clipboard"),
                Tab::Emoji => fl!("search-emoji"),
                Tab::Color => fl!("search-color"),
            },
            &self.search,
        )
        .id(self.search_id.clone())
        .on_input(Message::Search)
        .width(Length::Fill);

        let body: Element<Message> = match self.tab {
            Tab::Clipboard => self.view_clipboard(),
            Tab::Emoji => self.view_emoji(),
            Tab::Color => self.view_color(),
        };

        let footer = widget::row::with_children(vec![
            widget::button::standard(fl!("settings"))
                .on_press(Message::OpenSettings)
                .into(),
            widget::space::horizontal().into(),
            if self.tab == Tab::Clipboard {
                widget::button::destructive(fl!("clear"))
                    .on_press(Message::ClearClipboard)
                    .into()
            } else {
                widget::Space::new().into()
            },
        ])
        .spacing(8)
        .align_y(cosmic::iced::Alignment::Center);

        let content = widget::column::with_children(vec![
            tabs,
            search.into(),
            body,
            widget::space::vertical().height(Length::Fixed(4.0)).into(),
            footer.into(),
        ])
        .spacing(8)
        .padding(8);

        self.core.applet.popup_container(content).into()
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([clipboard_subscription(), keyboard_subscription()])
    }
}

impl App {
    fn view_clipboard(&self) -> Element<'_, Message> {
        let indices = self.filtered_clipboard();
        if indices.is_empty() {
            return widget::container(widget::text(if self.history.is_empty() {
                fl!("clipboard-empty")
            } else {
                fl!("no-matches")
            }))
            .padding(16)
            .width(Length::Fill)
            .into();
        }
        let mut col = widget::column::with_capacity(indices.len()).spacing(2);
        for idx in indices {
            if let Some(entry) = self.history.get(idx) {
                col = col.push(clipboard_row(idx, entry));
            }
        }
        widget::scrollable(col)
            .height(Length::Fixed(BODY_HEIGHT))
            .into()
    }

    fn view_emoji(&self) -> Element<'_, Message> {
        let mut sections: Vec<Element<Message>> = Vec::new();
        if self.search.is_empty() && !self.emoji_recents.is_empty() {
            sections.push(widget::text::heading(fl!("recents")).into());
            sections.push(emoji_grid(
                self.emoji_recents.iter().map(String::from).collect(),
            ));
            sections.push(widget::space::vertical().height(Length::Fixed(4.0)).into());
        }
        let matches = self.filtered_emoji();
        if matches.is_empty() {
            sections.push(
                widget::container(widget::text(fl!("no-matches")))
                    .padding(16)
                    .width(Length::Fill)
                    .into(),
            );
        } else {
            sections.push(emoji_grid(
                matches.iter().map(|e| e.as_str().to_string()).collect(),
            ));
        }
        let col = widget::column::with_children(sections).spacing(4);
        widget::scrollable(col)
            .height(Length::Fixed(BODY_HEIGHT))
            .into()
    }

    fn view_color(&self) -> Element<'_, Message> {
        let hex_for_submit = self.hex_input.clone();
        let hex_for_press = self.hex_input.clone();
        let hex_row = widget::row::with_children(vec![
            widget::text_input(fl!("hex-placeholder"), &self.hex_input)
                .on_input(Message::HexInput)
                .on_submit(move |_| Message::PickColor(hex_for_submit.clone()))
                .width(Length::Fill)
                .into(),
            widget::button::suggested(fl!("use"))
                .on_press(Message::PickColor(hex_for_press))
                .into(),
        ])
        .spacing(6);

        let mut sections: Vec<Element<Message>> = vec![hex_row.into()];
        if self.search.is_empty() && !self.color_recents.is_empty() {
            sections.push(widget::text::heading(fl!("recents")).into());
            sections.push(color_grid(
                self.color_recents.iter().map(String::from).collect(),
            ));
        }

        let palette = self.filtered_palette();
        if !palette.is_empty() {
            sections.push(widget::text::heading(fl!("palette")).into());
            sections.push(color_grid(
                palette.iter().map(|(_, hex)| hex.to_string()).collect(),
            ));
        } else {
            sections.push(
                widget::container(widget::text(fl!("no-matches")))
                    .padding(8)
                    .width(Length::Fill)
                    .into(),
            );
        }

        let col = widget::column::with_children(sections).spacing(8);
        widget::scrollable(col)
            .height(Length::Fixed(BODY_HEIGHT))
            .into()
    }
}

/// Tab selector. The active button renders as suggested, the rest as standard.
fn tab_bar(active: Tab) -> Element<'static, Message> {
    let mk = |t: Tab, label: String| -> Element<'static, Message> {
        if t == active {
            widget::button::suggested(label)
                .on_press(Message::Tab(t))
                .into()
        } else {
            widget::button::standard(label)
                .on_press(Message::Tab(t))
                .into()
        }
    };
    widget::row::with_children(vec![
        mk(Tab::Clipboard, fl!("tab-clipboard")),
        mk(Tab::Emoji, fl!("tab-emoji")),
        mk(Tab::Color, fl!("tab-color")),
    ])
    .spacing(4)
    .into()
}

fn emoji_grid(emojis: Vec<String>) -> Element<'static, Message> {
    let rows: Vec<Element<Message>> = emojis
        .chunks(EMOJI_COLUMNS)
        .map(|row| {
            let mut r = widget::row::with_capacity(row.len()).spacing(4);
            for e in row {
                let label = e.clone();
                let pick = e.clone();
                r = r.push(widget::button::standard(label).on_press(Message::PickEmoji(pick)));
            }
            r.into()
        })
        .collect();
    widget::column::with_children(rows).spacing(4).into()
}

fn color_grid(colors: Vec<String>) -> Element<'static, Message> {
    let rows: Vec<Element<Message>> = colors
        .chunks(COLOR_COLUMNS)
        .map(|row| {
            let mut r = widget::row::with_capacity(row.len()).spacing(4);
            for hex in row {
                let (red, green, blue) = hex_to_rgb(hex);
                let swatch_color = Color::from_rgb8(red, green, blue);
                let hex_owned = hex.clone();
                let label_owned = hex.clone();
                let swatch = widget::container(widget::text(label_owned))
                    .padding(8)
                    .width(Length::Fixed(72.0))
                    .height(Length::Fixed(40.0))
                    .style(move |_| widget::container::Style {
                        background: Some(Background::Color(swatch_color)),
                        text_color: Some(text_color_for(swatch_color)),
                        ..Default::default()
                    });
                let btn = widget::mouse_area(swatch).on_press(Message::PickColor(hex_owned));
                r = r.push(btn);
            }
            r.into()
        })
        .collect();
    widget::column::with_children(rows).spacing(4).into()
}

/// Pick black or white text depending on perceived luminance — readable
/// labels on light AND dark colors without per-color tuning.
fn text_color_for(c: Color) -> Color {
    let lum = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    if lum > 0.5 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

/// Subscription that drives the clipboard watcher. Two `wl-paste
/// --watch` children — one for the regular clipboard and one for
/// the primary selection — are spawned and their stdout lines are
/// awaited via `tokio::select!`. wl-paste fires once at startup
/// with the current selection state and once on every subsequent
/// change, so the applet picks up Ctrl+C, highlight events, and
/// external `wl-copy --clear` immediately — no poll interval.
///
/// On a clipboard tick: pick the highest-priority mime
/// (image > files > html > text), build an `Entry`, and emit
/// `Captured` if the content hash changed. Cache the canonical
/// text representation so the primary watcher knows what to
/// re-snap to.
///
/// On a primary tick: compare current primary text to the cached
/// canonical text. Emit `MirrorPrimary(canonical)` if they differ,
/// or `MirrorPrimary("")` to wipe primary when the clipboard
/// payload isn't textual.
fn clipboard_subscription() -> Subscription<Message> {
    Subscription::run(|| {
        stream::channel(8, |mut output: mpsc::Sender<Message>| async move {
            use cosmic::iced::futures::SinkExt;
            use std::process::Stdio;
            use tokio::io::{AsyncBufReadExt, BufReader};
            use tokio::process::Command;

            // `sh -c 'echo'` is a no-op marker — wl-paste pipes the
            // clipboard payload into our CMD's stdin but we don't
            // care about its bytes here, only that an event fired.
            // The echoed newline arrives on our captured stdout and
            // wakes `next_line().await`.
            let spawn_watch = |primary: bool| -> Option<tokio::process::Child> {
                let mut cmd = Command::new("wl-paste");
                if primary {
                    cmd.arg("--primary");
                }
                cmd.args(["--watch", "sh", "-c", "echo"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .ok()
            };

            let Some(mut clip_child) = spawn_watch(false) else { return };
            let Some(mut prim_child) = spawn_watch(true) else { return };

            let clip_stdout = match clip_child.stdout.take() {
                Some(s) => s,
                None => return,
            };
            let prim_stdout = match prim_child.stdout.take() {
                Some(s) => s,
                None => return,
            };
            let mut clip_lines = BufReader::new(clip_stdout).lines();
            let mut prim_lines = BufReader::new(prim_stdout).lines();

            let mut last_hash: Option<String> = None;
            // Cached canonical text for the current clipboard, used
            // by the primary watcher to know what to re-snap to.
            // `None` means the clipboard isn't textual (image/files
            // /empty), so primary should be wiped on drift.
            let mut canonical_text: Option<String> = None;

            loop {
                enum Tick {
                    Clipboard,
                    Primary,
                    Done,
                }
                let tick = tokio::select! {
                    line = clip_lines.next_line() => match line {
                        Ok(Some(_)) => Tick::Clipboard,
                        _ => Tick::Done,
                    },
                    line = prim_lines.next_line() => match line {
                        Ok(Some(_)) => Tick::Primary,
                        _ => Tick::Done,
                    },
                };

                match tick {
                    Tick::Done => break,
                    Tick::Clipboard => {
                        let cap_chars = config::load().max_entry_chars as usize;
                        let capture =
                            tokio::task::spawn_blocking(move || capture_clipboard(cap_chars))
                                .await
                                .unwrap_or(None);
                        match capture {
                            Some(cap) => {
                                let is_new = last_hash.as_deref() != Some(cap.hash.as_str());
                                canonical_text = match &cap.entry.payload {
                                    Payload::Text => Some(cap.entry.text.clone()),
                                    Payload::Files { uris } => Some(uris.join("\n")),
                                    Payload::Html { .. } => Some(cap.entry.text.clone()),
                                    Payload::Image { .. } => None,
                                };
                                if is_new {
                                    last_hash = Some(cap.hash.clone());
                                    let _ = output.send(Message::Captured(cap.entry)).await;
                                }
                            }
                            None => {
                                // Clipboard is empty or all-unsupported. Drop
                                // cached state and wipe primary too so a
                                // highlight doesn't linger.
                                last_hash = None;
                                canonical_text = None;
                                let _ = output.send(Message::MirrorPrimary(String::new())).await;
                            }
                        }
                    }
                    Tick::Primary => {
                        let primary_text =
                            tokio::task::spawn_blocking(|| wl_paste_text(/* primary */ true))
                                .await
                                .ok()
                                .flatten()
                                .unwrap_or_default();
                        match &canonical_text {
                            Some(target) if &primary_text != target => {
                                let _ = output
                                    .send(Message::MirrorPrimary(target.clone()))
                                    .await;
                            }
                            None if !primary_text.is_empty() => {
                                let _ = output.send(Message::MirrorPrimary(String::new())).await;
                            }
                            _ => {}
                        }
                    }
                }
            }
        })
    })
}

struct Capture {
    entry: Entry,
    /// blake3 of the underlying content — used by the watcher to
    /// detect "this is the same clipboard as last tick" without
    /// having to re-compare the full payload each time.
    hash: String,
}

/// Pick the highest-priority mime from the regular clipboard and
/// build an Entry. Priority: image (png/jpeg/webp) > file URI
/// list > rich text > plain text. Returns `None` when the
/// clipboard is empty, non-textual content we don't handle, or
/// the read fails.
fn capture_clipboard(cap_chars: usize) -> Option<Capture> {
    let types = wl_paste_types(/* primary */ false)?;
    if let Some(mime) = types
        .iter()
        .find(|t| matches!(t.as_str(), "image/png" | "image/jpeg" | "image/webp"))
        .cloned()
    {
        return capture_image(&mime);
    }
    if types.iter().any(|t| t == "text/uri-list") {
        return capture_files();
    }
    if types.iter().any(|t| t == "text/html") {
        return capture_html(cap_chars);
    }
    if types.iter().any(|t| t.starts_with("text/")) {
        return capture_text(cap_chars);
    }
    None
}

fn capture_image(mime: &str) -> Option<Capture> {
    let bytes = wl_paste_typed(mime, /* primary */ false)?;
    if bytes.is_empty() {
        return None;
    }
    let hash = blob_hash(&bytes);
    let ext = match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => "png",
    };
    // store_blob is idempotent — no-ops if the hash already exists.
    let _ = store_blob(&bytes, ext);
    let label = format!("[image · {} · {}]", mime, human_size(bytes.len()));
    Some(Capture {
        entry: Entry {
            text: label,
            captured_at: now_secs(),
            sensitive: false,
            pinned: false,
            payload: Payload::Image {
                blob: hash.clone(),
                mime: mime.to_string(),
                ext: ext.to_string(),
                bytes: bytes.len() as u64,
            },
        },
        hash,
    })
}

fn capture_files() -> Option<Capture> {
    let body = wl_paste_text_typed("text/uri-list", /* primary */ false)?;
    let uris: Vec<String> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect();
    if uris.is_empty() {
        return None;
    }
    let hash = blob_hash(uris.join("\n").as_bytes());
    let label = format_files_label(&uris);
    Some(Capture {
        entry: Entry {
            text: label,
            captured_at: now_secs(),
            sensitive: false,
            pinned: false,
            payload: Payload::Files { uris },
        },
        hash,
    })
}

fn capture_html(cap_chars: usize) -> Option<Capture> {
    let html_bytes = wl_paste_typed("text/html", /* primary */ false)?;
    if html_bytes.is_empty() {
        return None;
    }
    let hash = blob_hash(&html_bytes);
    let _ = store_blob(&html_bytes, "html");
    // Plain-text fallback drives the popup label + the post-paste
    // text rewrite. Best-effort: if no text/plain is offered we
    // synthesize a placeholder.
    let text = wl_paste_text(/* primary */ false)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.chars().count() > cap_chars {
                s.chars().take(cap_chars).collect()
            } else {
                s
            }
        })
        .unwrap_or_else(|| "[rich text]".into());
    Some(Capture {
        entry: Entry {
            text,
            captured_at: now_secs(),
            sensitive: false,
            pinned: false,
            payload: Payload::Html { blob: hash.clone() },
        },
        hash,
    })
}

fn capture_text(cap_chars: usize) -> Option<Capture> {
    let raw = wl_paste_text(/* primary */ false)?;
    let text = cleanup_text(raw);
    if text.is_empty() || text.chars().count() > cap_chars {
        return None;
    }
    let sensitive = looks_sensitive(&text);
    let hash = blob_hash(text.as_bytes());
    Some(Capture {
        entry: Entry {
            text: text.clone(),
            captured_at: now_secs(),
            pinned: false,
            sensitive,
            payload: Payload::Text,
        },
        hash,
    })
}

/// Run conservative cleanup on a captured text payload:
/// - strip a leading UTF-8 BOM
/// - strip ANSI CSI / OSC escape sequences (terminal copy carries them)
/// - strip a leading shell-prompt marker on each line (`$ `, `# `,
///   `> `, `❯ `) so copying multiple lines out of a terminal yields
///   just the commands
/// - strip well-known tracking query params (`utm_*`, `fbclid`,
///   `gclid`, `mc_eid`, etc.) from URLs in the text
/// - trim surrounding whitespace
///
/// Internal whitespace is NOT collapsed — that would mangle code,
/// poetry, and intentionally-formatted paragraphs.
fn cleanup_text(raw: String) -> String {
    let s = raw.strip_prefix('\u{feff}').unwrap_or(&raw).to_string();
    let s = strip_ansi(&s);
    let s = strip_prompt_prefix(&s);
    let s = strip_url_tracking(&s);
    s.trim().to_string()
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        // Recognise CSI ("ESC [ ... final"), OSC ("ESC ] ... BEL or
        // ESC \\"), and the simple two-char sequences ("ESC X").
        match chars.next() {
            Some('[') => {
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() || next == '~' {
                        break;
                    }
                }
            }
            Some(']') => loop {
                let Some(next) = chars.next() else { break };
                if next == '\x07' {
                    break;
                }
                if next == '\x1b' {
                    let _ = chars.next(); // consume the trailing '\\'
                    break;
                }
            },
            Some(_) | None => {}
        }
    }
    out
}

fn strip_prompt_prefix(s: &str) -> String {
    const PROMPTS: &[&str] = &["$ ", "# ", "> ", "❯ ", "PS> ", "% "];
    s.split_inclusive('\n')
        .map(|line| {
            let (lead_ws, body) = split_leading_whitespace(line);
            for prompt in PROMPTS {
                if let Some(rest) = body.strip_prefix(prompt) {
                    return format!("{lead_ws}{rest}");
                }
            }
            line.to_string()
        })
        .collect()
}

fn split_leading_whitespace(s: &str) -> (&str, &str) {
    let lead_end = s
        .char_indices()
        .find(|(_, c)| !c.is_whitespace() || *c == '\n')
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    (&s[..lead_end], &s[lead_end..])
}

fn strip_url_tracking(s: &str) -> String {
    s.split(char::is_whitespace)
        .map(|tok| {
            if tok.starts_with("http://") || tok.starts_with("https://") {
                strip_url_tracking_token(tok)
            } else {
                tok.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_url_tracking_token(token: &str) -> String {
    let Some(q_idx) = token.find('?') else { return token.to_string() };
    let (base, rest) = token.split_at(q_idx);
    // `rest` starts with `?`. Some URLs end in `#fragment`; preserve.
    let rest = &rest[1..];
    let (query, fragment) = match rest.find('#') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| {
            let key = kv.split_once('=').map(|(k, _)| k).unwrap_or(kv);
            !is_tracking_param(key)
        })
        .collect();
    let mut out = String::from(base);
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    out.push_str(fragment);
    out
}

fn is_tracking_param(key: &str) -> bool {
    matches!(
        key,
        "utm_source"
            | "utm_medium"
            | "utm_campaign"
            | "utm_term"
            | "utm_content"
            | "utm_id"
            | "utm_name"
            | "fbclid"
            | "gclid"
            | "yclid"
            | "dclid"
            | "msclkid"
            | "twclid"
            | "igshid"
            | "mc_eid"
            | "mc_cid"
            | "_hsenc"
            | "_hsmi"
            | "hsCtaTracking"
            | "ref"
            | "ref_src"
            | "ref_url"
            | "referer"
            | "vero_id"
            | "vero_conv"
            | "trk"
            | "trkCampaign"
            | "_ga"
            | "_gl"
            | "spm"
    )
}

/// Heuristic: does this text look like a secret? Used to mask the
/// preview in the popup and to skip on-disk persistence. False
/// positives are mostly harmless (entry still works, just shows as
/// `••••`); false negatives are the real cost so we keep the
/// rules conservative-and-broad.
fn looks_sensitive(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.contains("-----BEGIN ") {
        return true;
    }
    let first_line = trimmed.lines().next().unwrap_or("");
    let prefixed = [
        "sk-", "sk_", "pk-", "pk_", "rk_", "ghp_", "gho_", "ghs_", "ghu_", "ghr_", "github_pat_",
        "xoxb-", "xoxa-", "xoxp-", "xoxs-", "xoxr-", "ya29.", "AIza", "EAACEdEose0cBA", "AKIA",
        "ASIA",
    ];
    if prefixed.iter().any(|p| first_line.starts_with(p)) {
        return true;
    }
    // JWT: three URL-safe base64 segments joined by dots.
    if first_line.matches('.').count() == 2 {
        let parts: Vec<&str> = first_line.split('.').collect();
        if parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'=')
        }) && parts[0].len() > 10
        {
            return true;
        }
    }
    // High-entropy alphanumeric blob ≥ 32 chars — classic API token.
    if trimmed.len() >= 32
        && trimmed.lines().count() == 1
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && shannon_entropy(trimmed.as_bytes()) >= 4.0
    {
        return true;
    }
    false
}

fn shannon_entropy(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for &b in bytes {
        counts[b as usize] += 1;
    }
    let len = bytes.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

fn human_size(bytes: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = KIB * 1024;
    if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{} KiB", bytes / KIB)
    } else {
        format!("{} B", bytes)
    }
}

fn format_files_label(uris: &[String]) -> String {
    let name = uris
        .first()
        .map(|u| uri_basename(u))
        .unwrap_or_default();
    if uris.len() > 1 {
        format!("{name} + {} more", uris.len() - 1)
    } else {
        name
    }
}

/// Does this URI point at a directory? Trailing slash, or a local
/// `file://` path that resolves to a real directory on disk.
fn uri_is_dir(uri: &str) -> bool {
    let raw = uri.trim_matches(|c: char| c == '\'' || c == '"');
    if raw.ends_with('/') {
        return true;
    }
    let path = raw.strip_prefix("file://").unwrap_or(raw);
    let decoded = path
        .replace("%2F", "/")
        .replace("%25", "%")
        .replace("%20", " ");
    std::path::Path::new(&decoded).is_dir()
}

/// Extract the displayable basename from a URI or path. Strips a
/// `scheme://` prefix if present, trims any surrounding single or
/// double quotes (some terminal "copy file path" actions emit
/// shell-quoted paths instead of proper URIs), takes the last
/// `/`-separated segment, and decodes the few percent-escapes a
/// file manager reliably emits (%20, %2F, %25).
fn uri_basename(uri: &str) -> String {
    let uri = uri.trim_matches(|c: char| c == '\'' || c == '"');
    let path = uri.split_once("://").map(|(_, p)| p).unwrap_or(uri);
    let raw = path.rsplit('/').find(|s| !s.is_empty()).unwrap_or(path);
    raw.trim_matches(|c: char| c == '\'' || c == '"')
        .replace("%2F", "/")
        .replace("%25", "%")
        .replace("%20", " ")
}

fn wl_paste_run(args: &[&str]) -> Option<Vec<u8>> {
    use std::process::Command;
    let out = Command::new("wl-paste").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(out.stdout)
}

fn wl_paste_types(primary: bool) -> Option<Vec<String>> {
    let mut args: Vec<&str> = Vec::new();
    if primary {
        args.push("--primary");
    }
    args.push("--list-types");
    let bytes = wl_paste_run(&args)?;
    Some(
        String::from_utf8_lossy(&bytes)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

fn wl_paste_typed(mime: &str, primary: bool) -> Option<Vec<u8>> {
    let mut args: Vec<&str> = Vec::new();
    if primary {
        args.push("--primary");
    }
    args.push("--type");
    args.push(mime);
    wl_paste_run(&args)
}

fn wl_paste_text_typed(mime: &str, primary: bool) -> Option<String> {
    String::from_utf8(wl_paste_typed(mime, primary)?).ok()
}

fn wl_paste_text(primary: bool) -> Option<String> {
    let mut args: Vec<&str> = Vec::new();
    if primary {
        args.push("--primary");
    }
    args.push("--no-newline");
    String::from_utf8(wl_paste_run(&args)?).ok()
}

/// Popup keyboard shortcuts. `event::Status::Captured` is the iced
/// signal that some focused widget already consumed the keypress
/// (e.g. the user is typing in the search input) — we ignore those
/// so digit keys remain typeable in search.
///
/// - `Esc` → close popup
/// - `1`–`9` → pick the Nth filtered clipboard entry
/// - `/` → focus the search input
fn keyboard_subscription() -> Subscription<Message> {
    event::listen_with(|evt, status, _id| {
        if matches!(status, event::Status::Captured) {
            // Esc still closes even if the search input is the
            // active widget — text_input handles Esc as blur-only,
            // not consume. But err on the side of letting widgets
            // consume their own keys.
            if let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = &evt {
                if matches!(key, keyboard::Key::Named(keyboard::key::Named::Escape)) {
                    return Some(Message::CloseEscape);
                }
            }
            return None;
        }
        let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = evt else {
            return None;
        };
        match &key {
            keyboard::Key::Named(keyboard::key::Named::Escape) => Some(Message::CloseEscape),
            keyboard::Key::Character(c) => {
                let s: &str = c.as_ref();
                if s == "/" {
                    Some(Message::FocusSearch)
                } else if let Some(digit) =
                    s.chars().next().and_then(|ch| ch.to_digit(10))
                {
                    if (1..=9).contains(&digit) {
                        Some(Message::PickByNumber(digit as usize - 1))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    })
}

/// Set a Wayland selection by piping `text` into `wl-copy`. Runs on a
/// detached OS thread so we never block the iced runtime. We use the
/// `wl-clipboard` CLI rather than an in-process iced/smithay write
/// because the smithay-clipboard worker refuses to set a selection
/// unless our applet has Wayland keyboard focus, which it doesn't
/// when the popup is closed. `wl-copy` runs as its own short-lived
/// Wayland client and daemonizes by default so the selection
/// persists past the child exiting.
fn wl_copy_write(text: String, primary: bool) {
    wl_copy_pipe(text.into_bytes(), None, primary);
}

/// Write `text` to a selection under a specific mime (e.g.
/// `text/uri-list` so a file manager paste yields actual files,
/// not the URI list as plain text).
fn wl_copy_write_typed(text: String, mime: String, primary: bool) {
    wl_copy_pipe(text.into_bytes(), Some(mime), primary);
}

/// Read a blob from disk and pipe it into wl-copy under `mime`.
/// Used to re-publish images and HTML from history without ever
/// holding the bytes in main-thread RAM.
fn wl_copy_file(path: std::path::PathBuf, mime: String, primary: bool) {
    std::thread::spawn(move || {
        let Ok(bytes) = std::fs::read(&path) else { return };
        wl_copy_pipe_blocking(&bytes, Some(&mime), primary);
    });
}

/// Empty a Wayland selection via `wl-copy --clear` (`--primary` flag
/// targets the primary selection). Same focus-independence rationale
/// as [`wl_copy_write`].
fn wl_copy_clear(primary: bool) {
    std::thread::spawn(move || {
        use std::process::{Command, Stdio};
        let mut cmd = Command::new("wl-copy");
        if primary {
            cmd.arg("--primary");
        }
        cmd.arg("--clear")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let _ = cmd.spawn().map(|mut c| c.wait());
    });
}

fn wl_copy_pipe(bytes: Vec<u8>, mime: Option<String>, primary: bool) {
    std::thread::spawn(move || {
        wl_copy_pipe_blocking(&bytes, mime.as_deref(), primary);
    });
}

fn wl_copy_pipe_blocking(bytes: &[u8], mime: Option<&str>, primary: bool) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut cmd = Command::new("wl-copy");
    if primary {
        cmd.arg("--primary");
    }
    if let Some(m) = mime {
        cmd.args(["--type", m]);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(mut child) = cmd.spawn() else { return };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(bytes);
    }
    let _ = child.wait();
}

/// One row in the clipboard tab. Image payloads (clipboard-image or
/// a file:// URI pointing to an image on disk) get a thumbnail
/// rendered from the underlying path. Everything else falls back
/// to a plain text preview. All rows share a pin toggle on the
/// left.
fn clipboard_row(idx: usize, entry: &Entry) -> Element<'_, Message> {
    let pin_glyph = if entry.pinned { "★" } else { "☆" };
    let pin = widget::button::standard(pin_glyph).on_press(Message::TogglePin(idx));
    let main: Element<Message> = if entry.sensitive {
        // Mask sensitive entries — the user can still click to
        // paste the real value, but the popup never reveals it.
        text_button("•••••• (sensitive)", idx)
    } else {
        match &entry.payload {
            Payload::Image { blob, ext, .. } => image_button(blob_path(blob, ext), idx),
            Payload::Files { uris } => match first_local_image_path(uris) {
                Some(p) => image_button(p, idx),
                None => {
                    let is_dir = uris.first().map(|u| uri_is_dir(u)).unwrap_or(false);
                    file_button(is_dir, &entry.text, idx)
                }
            },
            Payload::Text => match normalize_hex(entry.text.trim()) {
                Some(hex) => color_swatch_button(hex, idx),
                None => text_button(&entry.text, idx),
            },
            Payload::Html { .. } => text_button(&entry.text, idx),
        }
    };
    widget::row::with_children(vec![pin.into(), main])
        .spacing(4)
        .align_y(cosmic::iced::Alignment::Center)
        .into()
}

fn image_button(path: std::path::PathBuf, idx: usize) -> Element<'static, Message> {
    let handle = cosmic::widget::image::Handle::from_path(path);
    widget::button::image(handle)
        .on_press(Message::PickClipboard(idx))
        .width(Length::Fill)
        .height(Length::Fixed(60.0))
        .into()
}

fn text_button<'a>(text: &'a str, idx: usize) -> Element<'a, Message> {
    widget::button::standard(preview_label(text))
        .on_press(Message::PickClipboard(idx))
        .width(Length::Fill)
        .into()
}

/// File / folder entry: a themed `folder` or `text-x-generic` icon
/// from the active icon theme, plus the name. Only these two icons
/// are used — no per-extension variety.
fn file_button<'a>(is_dir: bool, label: &'a str, idx: usize) -> Element<'a, Message> {
    let icon_name = if is_dir { "folder" } else { "text-x-generic" };
    let row = widget::row::with_children(vec![
        widget::icon::from_name(icon_name).size(16).into(),
        widget::text(preview_label(label)).into(),
    ])
    .spacing(8)
    .align_y(cosmic::iced::Alignment::Center);
    widget::button::custom(row)
        .on_press(Message::PickClipboard(idx))
        .width(Length::Fill)
        .into()
}

/// Render a Text entry whose body parses as a hex color as a
/// filled swatch — same visual language the Color tab uses, so
/// pasting `#3b82f6` from chat or a CSS file shows up as the
/// actual blue rather than a generic text row.
fn color_swatch_button(hex: String, idx: usize) -> Element<'static, Message> {
    let (r, g, b) = hex_to_rgb(&hex);
    let swatch_color = Color::from_rgb8(r, g, b);
    let label = hex.clone();
    let swatch = widget::container(
        widget::text(label).align_x(cosmic::iced::Alignment::Center),
    )
    .padding(8)
    .width(Length::Fill)
    .height(Length::Fixed(40.0))
    .style(move |_| widget::container::Style {
        background: Some(Background::Color(swatch_color)),
        text_color: Some(text_color_for(swatch_color)),
        ..Default::default()
    });
    widget::mouse_area(swatch)
        .on_press(Message::PickClipboard(idx))
        .into()
}

/// If the first URI in `uris` resolves to a local image file on
/// disk, return its path so the popup can render an inline
/// thumbnail. Recognised extensions: png, jpg, jpeg, webp, gif,
/// bmp, ico (case-insensitive). Other URI schemes
/// (http://, ftp://) are ignored — we only thumbnail bytes we can
/// actually read off the local filesystem.
fn first_local_image_path(uris: &[String]) -> Option<std::path::PathBuf> {
    let uri = uris.first()?;
    let raw = uri.trim_matches(|c: char| c == '\'' || c == '"');
    let path_part = raw
        .strip_prefix("file://")
        .or_else(|| {
            if raw.starts_with('/') {
                Some(raw)
            } else {
                None
            }
        })?;
    let decoded = path_part
        .replace("%2F", "/")
        .replace("%25", "%")
        .replace("%20", " ");
    let path = std::path::PathBuf::from(decoded);
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico"
    ) && path.exists()
    {
        Some(path)
    } else {
        None
    }
}

#[allow(dead_code)]
fn _bin_name_anchor() -> &'static str {
    BIN_NAME
}
