// Standalone settings GUI for cosmic-pick. Launched as a child
// process by the applet's "Settings…" button. The applet runs as a
// cosmic::applet, this is a regular cosmic::app — they can't share
// one binary's main loop, so they're split.
//
// Same cosmic_config-backed schema as the applet uses (`PickConfig`).

use cosmic::app::{Core, Settings, Task};
use cosmic::iced::{Alignment, Length, Size};
use cosmic::prelude::*;
use cosmic::widget::{self, space};

use cosmic_pick::config::{self, PickConfig};
use cosmic_pick::emoji_recents::EmojiRecents;
use cosmic_pick::history::History;
use cosmic_pick::recent_colors::RecentColors;
use cosmic_pick::{
    color_recents_path, emoji_recents_path, fl, history_path, localize, APP_ID,
};

#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // Saving is matched in the view but only async saves
                    // construct it; pick's saves are synchronous.
enum SaveStatus {
    #[default]
    Idle,
    Saving,
    Saved,
    Error(String),
}

fn main() -> cosmic::iced::Result {
    localize::localize();
    let settings = Settings::default()
        .size(Size::new(640.0, 520.0))
        .exit_on_close(true);
    cosmic::app::run::<App>(settings, ())
}

#[derive(Clone, Debug)]
pub enum Message {
    HistorySizeText(String),
    MaxCharsText(String),
    PersistHistory(bool),
    ClearHistory,
    ClearEmojiRecents,
    ClearColorRecents,
    Save,
}

pub struct App {
    core: Core,
    history_size_text: String,
    max_chars_text: String,
    persist_history: bool,
    status: SaveStatus,
}

impl App {
    fn build_config(&self) -> PickConfig {
        PickConfig {
            history_size: self.history_size_text.parse().unwrap_or(50).max(1),
            persist_history: self.persist_history,
            max_entry_chars: self.max_chars_text.parse().unwrap_or(10_000).max(1),
        }
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
        let cfg = config::load();
        let app = App {
            core,
            history_size_text: cfg.history_size.to_string(),
            max_chars_text: cfg.max_entry_chars.to_string(),
            persist_history: cfg.persist_history,
            status: SaveStatus::Idle,
        };
        // set_window_title's API now requires a window::Id we can't
        // produce here without poking iced internals; the WM uses the
        // .desktop's Name= for the window title anyway.
        (app, Task::none())
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::HistorySizeText(s) => {
                if s.chars().all(|c| c.is_ascii_digit()) && s.len() <= 5 {
                    self.history_size_text = s;
                }
            }
            Message::MaxCharsText(s) => {
                if s.chars().all(|c| c.is_ascii_digit()) && s.len() <= 8 {
                    self.max_chars_text = s;
                }
            }
            Message::PersistHistory(b) => self.persist_history = b,
            Message::ClearHistory => {
                let empty = History::new();
                self.status = match empty.save(&history_path()) {
                    Ok(()) => SaveStatus::Saved,
                    Err(e) => SaveStatus::Error(format!("clear-{e}")),
                };
            }
            Message::ClearEmojiRecents => {
                let empty = EmojiRecents::new();
                self.status = match empty.save(&emoji_recents_path()) {
                    Ok(()) => SaveStatus::Saved,
                    Err(e) => SaveStatus::Error(format!("clear-{e}")),
                };
            }
            Message::ClearColorRecents => {
                let empty = RecentColors::new();
                self.status = match empty.save(&color_recents_path()) {
                    Ok(()) => SaveStatus::Saved,
                    Err(e) => SaveStatus::Error(format!("clear-{e}")),
                };
            }
            Message::Save => {
                let cfg = self.build_config();
                self.status = match config::save(&cfg) {
                    Ok(()) => SaveStatus::Saved,
                    Err(e) => SaveStatus::Error(e.to_string()),
                };
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let history_section = widget::settings::section()
            .title(fl!("settings-section-history"))
            .add(widget::settings::item(
                fl!("settings-history-size"),
                widget::text_input("50", &self.history_size_text)
                    .on_input(Message::HistorySizeText)
                    .width(Length::Fixed(80.0)),
            ))
            .add(widget::settings::item(
                fl!("settings-max-chars"),
                widget::text_input("10000", &self.max_chars_text)
                    .on_input(Message::MaxCharsText)
                    .width(Length::Fixed(120.0)),
            ))
            .add(widget::settings::item(
                fl!("settings-persist"),
                widget::toggler(self.persist_history).on_toggle(Message::PersistHistory),
            ));

        let actions = widget::settings::section()
            .title(fl!("settings-section-actions"))
            .add(widget::settings::item(
                fl!("settings-clear-row"),
                widget::button::destructive(fl!("settings-clear-button"))
                    .on_press(Message::ClearHistory),
            ))
            .add(widget::settings::item(
                fl!("settings-clear-emoji-row"),
                widget::button::destructive(fl!("settings-clear-button"))
                    .on_press(Message::ClearEmojiRecents),
            ))
            .add(widget::settings::item(
                fl!("settings-clear-color-row"),
                widget::button::destructive(fl!("settings-clear-button"))
                    .on_press(Message::ClearColorRecents),
            ));

        let body = widget::settings::view_column(vec![history_section.into(), actions.into()]);
        let scroll = widget::scrollable(body).height(Length::Fill);

        let status_widget: Element<Message> = match &self.status {
            SaveStatus::Idle => widget::Space::new().into(),
            SaveStatus::Saving => widget::text(fl!("settings-saving")).into(),
            SaveStatus::Saved => widget::text(fl!("settings-saved")).into(),
            SaveStatus::Error(e) => {
                let msg = if let Some(rest) = e.strip_prefix("clear-") {
                    fl!("settings-error-clear", error = rest.to_string())
                } else {
                    fl!("settings-error", error = e.clone())
                };
                widget::text(msg).into()
            }
        };

        let footer = widget::row::with_children(vec![
            status_widget,
            space::horizontal().into(),
            widget::button::suggested(fl!("settings-save"))
                .on_press(Message::Save)
                .into(),
        ])
        .spacing(8)
        .align_y(Alignment::Center);

        let content = widget::column::with_children(vec![scroll.into(), footer.into()])
            .spacing(16)
            .padding(16);

        widget::container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
