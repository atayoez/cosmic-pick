// libcosmic settings GUI for cosmic-clip. Edits are kept in memory until
// Save; the Autostart toggle and Clear-history button apply immediately.

use cosmic::app::{Core, Settings, Task};
use cosmic::iced::{Alignment, Length, Size};
use cosmic::prelude::*;
use cosmic::widget::{self, space};
use cosmic::Action;

use cosmic_clip::autostart;
use cosmic_clip::config::{self, Config};
use cosmic_clip::history::History;
use cosmic_clip::paths::{config_path, history_path, APP_ID};

fn main() -> cosmic::iced::Result {
    let settings = Settings::default()
        .size(Size::new(640.0, 520.0))
        .exit_on_close(true);
    cosmic::app::run::<App>(settings, ())
}

#[derive(Clone, Debug)]
pub enum Message {
    HistorySizeText(String),
    PollIntervalText(String),
    MaxCharsText(String),
    PersistHistory(bool),
    Autostart(bool),
    ClearHistory,
    Save,
    SaveResult(Result<(), String>),
}

#[derive(Clone, Debug, Default)]
enum SaveStatus {
    #[default]
    Idle,
    Saving,
    Saved,
    Error(String),
}

pub struct App {
    core: Core,
    history_size_text: String,
    poll_interval_text: String,
    max_chars_text: String,
    persist_history: bool,
    autostart_enabled: bool,
    status: SaveStatus,
}

impl App {
    fn build_config(&self) -> Config {
        Config {
            history_size: self.history_size_text.parse().unwrap_or(50).max(1),
            poll_interval_ms: self.poll_interval_text.parse().unwrap_or(500).max(100),
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
        let cfg = config::read(&config_path()).unwrap_or_default();
        let mut app = App {
            core,
            history_size_text: cfg.history_size.to_string(),
            poll_interval_text: cfg.poll_interval_ms.to_string(),
            max_chars_text: cfg.max_entry_chars.to_string(),
            persist_history: cfg.persist_history,
            autostart_enabled: autostart::is_enabled(),
            status: SaveStatus::Idle,
        };
        let title = app.set_window_title("cosmic-clip Settings".into());
        (app, title)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::HistorySizeText(s) => {
                if s.chars().all(|c| c.is_ascii_digit()) && s.len() <= 5 {
                    self.history_size_text = s;
                }
            }
            Message::PollIntervalText(s) => {
                if s.chars().all(|c| c.is_ascii_digit()) && s.len() <= 6 {
                    self.poll_interval_text = s;
                }
            }
            Message::MaxCharsText(s) => {
                if s.chars().all(|c| c.is_ascii_digit()) && s.len() <= 8 {
                    self.max_chars_text = s;
                }
            }
            Message::PersistHistory(b) => self.persist_history = b,
            Message::Autostart(on) => {
                let res = if on {
                    autostart::enable()
                } else {
                    autostart::disable()
                };
                if let Err(e) = res {
                    self.status = SaveStatus::Error(format!("autostart: {e}"));
                }
                self.autostart_enabled = autostart::is_enabled();
            }
            Message::ClearHistory => {
                let path = history_path();
                let empty = History::new();
                let res = empty.save(&path);
                self.status = match res {
                    Ok(()) => SaveStatus::Saved,
                    Err(e) => SaveStatus::Error(format!("clear: {e}")),
                };
            }
            Message::Save => {
                self.status = SaveStatus::Saving;
                let cfg = self.build_config();
                let path = config_path();
                return Task::perform(
                    async move { config::write(&path, &cfg).map_err(|e| e.to_string()) },
                    |r| Action::App(Message::SaveResult(r)),
                );
            }
            Message::SaveResult(Ok(())) => self.status = SaveStatus::Saved,
            Message::SaveResult(Err(e)) => self.status = SaveStatus::Error(e),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let general = widget::settings::section()
            .title("History")
            .add(widget::settings::item(
                "History size (entries)",
                widget::text_input("50", &self.history_size_text)
                    .on_input(Message::HistorySizeText)
                    .width(Length::Fixed(80.0)),
            ))
            .add(widget::settings::item(
                "Poll interval (ms)",
                widget::text_input("500", &self.poll_interval_text)
                    .on_input(Message::PollIntervalText)
                    .width(Length::Fixed(80.0)),
            ))
            .add(widget::settings::item(
                "Max characters per entry",
                widget::text_input("10000", &self.max_chars_text)
                    .on_input(Message::MaxCharsText)
                    .width(Length::Fixed(120.0)),
            ))
            .add(widget::settings::item(
                "Persist history across restarts",
                widget::toggler(self.persist_history).on_toggle(Message::PersistHistory),
            ));

        let startup = widget::settings::section()
            .title("Startup")
            .add(widget::settings::item(
                "Start cosmic-clip on login",
                widget::toggler(self.autostart_enabled).on_toggle(Message::Autostart),
            ));

        let actions = widget::settings::section().title("Actions").add(
            widget::settings::item(
                "Clear clipboard history",
                widget::button::destructive("Clear now").on_press(Message::ClearHistory),
            ),
        );

        let body =
            widget::settings::view_column(vec![general.into(), startup.into(), actions.into()]);
        let scroll = widget::scrollable(body).height(Length::Fill);

        let status_widget: Element<Message> = match &self.status {
            SaveStatus::Idle => widget::Space::new().into(),
            SaveStatus::Saving => widget::text("Saving…").into(),
            SaveStatus::Saved => widget::text("Saved.").into(),
            SaveStatus::Error(e) => widget::text(format!("Error: {e}")).into(),
        };

        let footer = widget::row::with_children(vec![
            status_widget,
            space::horizontal().into(),
            widget::button::suggested("Save")
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
