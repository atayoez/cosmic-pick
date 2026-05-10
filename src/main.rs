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
use std::time::Duration;

use cosmic_pick::colors::{hex_to_rgb, PALETTE};
use cosmic_pick::config;
use cosmic_pick::emoji_recents::EmojiRecents;
use cosmic_pick::history::{Entry, History};
use cosmic_pick::recent_colors::{normalize_hex, RecentColors};
use cosmic_pick::{
    color_recents_path, emoji_recents_path, fl, history_path, localize, APP_ID, BIN_NAME,
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
    ClearClipboard,
    OpenSettings,
    /// New text observed by the clipboard watcher.
    Captured(String),
    /// Fire-and-forget terminator.
    Noop,
}

pub struct App {
    core: Core,
    popup: Option<window::Id>,
    tab: Tab,
    search: String,
    /// Hex color the user is composing in the color tab. May be a
    /// partial entry (e.g. "#ff" while typing).
    hex_input: String,
    history: History,
    emoji_recents: EmojiRecents,
    color_recents: RecentColors,
}

impl App {
    fn copy_to_clipboards(text: &str) {
        use arboard::{Clipboard, LinuxClipboardKind, SetExtLinux};
        if let Ok(mut cb) = Clipboard::new() {
            let _ = cb
                .set()
                .clipboard(LinuxClipboardKind::Clipboard)
                .text(text.to_string());
            let _ = cb
                .set()
                .clipboard(LinuxClipboardKind::Primary)
                .text(text.to_string());
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
                let text = self.history.get(idx).map(|e: &Entry| e.text.clone());
                if let Some(text) = text {
                    Self::copy_to_clipboards(&text);
                }
                self.close_popup_task()
            }
            Message::PickEmoji(emoji) => {
                self.emoji_recents.push(&emoji);
                self.save_emoji_recents_async();
                Self::copy_to_clipboards(&emoji);
                self.close_popup_task()
            }
            Message::PickColor(hex) => {
                if let Some(normalized) = normalize_hex(&hex) {
                    self.color_recents.push(&normalized);
                    self.save_color_recents_async();
                    Self::copy_to_clipboards(&normalized);
                }
                self.close_popup_task()
            }
            Message::ClearClipboard => {
                self.history.clear();
                self.save_history_async();
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
            Message::Captured(text) => {
                let cap = (config::load().history_size as usize).max(1);
                self.history.push(text, cap);
                self.save_history_async();
                Task::none()
            }
            Message::Noop => Task::none(),
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
        Subscription::batch([clipboard_subscription(), escape_subscription()])
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
                col = col.push(
                    widget::button::standard(preview_label(&entry.text))
                        .on_press(Message::PickClipboard(idx))
                        .width(Length::Fill),
                );
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

/// Subscription that drives the dual-clipboard watcher: ticks every
/// `poll_interval_ms`, mirrors writes between the regular clipboard
/// and the primary selection, emits `Captured(text)` on each
/// user-driven change.
fn clipboard_subscription() -> Subscription<Message> {
    Subscription::run(|| {
        stream::channel(64, |mut output: mpsc::Sender<Message>| async move {
            use arboard::{Clipboard, GetExtLinux, LinuxClipboardKind, SetExtLinux};
            use cosmic::iced::futures::SinkExt;

            let mut last_clipboard: Option<String> = None;
            let mut last_primary: Option<String> = None;

            loop {
                let cfg = config::load();
                tokio::time::sleep(Duration::from_millis(cfg.poll_interval_ms.max(100))).await;

                let (clip_now, primary_now) = tokio::task::spawn_blocking(|| {
                    let Ok(mut cb) = Clipboard::new() else {
                        return (None, None);
                    };
                    let c = cb.get().clipboard(LinuxClipboardKind::Clipboard).text().ok();
                    let p = cb.get().clipboard(LinuxClipboardKind::Primary).text().ok();
                    (c, p)
                })
                .await
                .unwrap_or((None, None));

                let new_text = match (&clip_now, &primary_now) {
                    (Some(c), _) if last_clipboard.as_deref() != Some(c.as_str()) => {
                        Some(c.clone())
                    }
                    (_, Some(p)) if last_primary.as_deref() != Some(p.as_str()) => Some(p.clone()),
                    _ => None,
                };

                last_clipboard = clip_now;
                last_primary = primary_now;

                let Some(text) = new_text else { continue };
                if text.chars().count() > cfg.max_entry_chars as usize {
                    continue;
                }

                let target = text.clone();
                let mirror_clip = last_clipboard.as_deref() != Some(target.as_str());
                let mirror_prim = last_primary.as_deref() != Some(target.as_str());
                if mirror_clip {
                    last_clipboard = Some(target.clone());
                }
                if mirror_prim {
                    last_primary = Some(target.clone());
                }
                if mirror_clip || mirror_prim {
                    let m = target;
                    tokio::task::spawn_blocking(move || {
                        let Ok(mut cb) = Clipboard::new() else { return };
                        if mirror_clip {
                            let _ = cb
                                .set()
                                .clipboard(LinuxClipboardKind::Clipboard)
                                .text(m.clone());
                        }
                        if mirror_prim {
                            let _ = cb
                                .set()
                                .clipboard(LinuxClipboardKind::Primary)
                                .text(m);
                        }
                    });
                }

                let _ = output.send(Message::Captured(text)).await;
            }
        })
    })
}

fn escape_subscription() -> Subscription<Message> {
    event::listen_with(|evt, _status, _id| match evt {
        Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
            if matches!(key, keyboard::Key::Named(keyboard::key::Named::Escape)) {
                Some(Message::Noop)
            } else {
                None
            }
        }
        _ => None,
    })
}

#[allow(dead_code)]
fn _bin_name_anchor() -> &'static str {
    BIN_NAME
}
