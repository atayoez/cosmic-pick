// cosmic-clip: Wayland-native clipboard history manager.
//
// - Polls the clipboard every N ms (default 500) and keeps the last N
//   distinct text entries.
// - Tray icon exposes recent entries; clicking one re-copies it back.
// - "Settings…" launches the libcosmic GUI; "Open History" / "Clear" /
//   "Quit" are inline.
//
// Image / file clipboard payloads are out of scope for the POC.

use ksni::{menu::*, Tray, TrayMethods};
use std::process::Command;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};

use cosmic_clip::config::{self, Config};
use cosmic_clip::history::{Entry, History};
use cosmic_clip::paths::{config_path, history_path, settings_exec};

#[derive(Clone)]
struct ClipTray {
    state: Arc<TrayState>,
}

struct TrayState {
    history: Mutex<History>,
    cfg: Mutex<Config>,
}

impl Tray for ClipTray {
    fn id(&self) -> String {
        "cosmic-clip".into()
    }
    fn title(&self) -> String {
        "Clipboard".into()
    }
    fn icon_name(&self) -> String {
        "cosmic-clip-symbolic".into()
    }
    fn icon_theme_path(&self) -> String {
        String::new()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        let n = self.state.history.lock().map(|h| h.len()).unwrap_or(0);
        ksni::ToolTip {
            title: "Clipboard".into(),
            description: if n == 0 {
                "No history yet".into()
            } else {
                format!("{n} recent {}", if n == 1 { "entry" } else { "entries" })
            },
            icon_name: "cosmic-clip-symbolic".into(),
            icon_pixmap: vec![],
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let snapshot: Vec<Entry> = self
            .state
            .history
            .lock()
            .map(|h| h.iter().cloned().collect())
            .unwrap_or_default();
        let cap = self
            .state
            .cfg
            .lock()
            .map(|c| c.history_size.min(15))
            .unwrap_or(15);

        let mut items: Vec<MenuItem<Self>> = Vec::new();

        if snapshot.is_empty() {
            items.push(
                StandardItem {
                    label: "(empty)".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        } else {
            for (idx, entry) in snapshot.iter().take(cap).enumerate() {
                let preview = preview_label(&entry.text);
                items.push(
                    StandardItem {
                        label: preview,
                        activate: Box::new(move |t: &mut ClipTray| {
                            t.restore_index(idx);
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }
        }

        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Clear History".into(),
                icon_name: "edit-clear-all-symbolic".into(),
                activate: Box::new(|t: &mut ClipTray| {
                    if let Ok(mut h) = t.state.history.lock() {
                        h.clear();
                    }
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Settings...".into(),
                icon_name: "preferences-system-symbolic".into(),
                activate: Box::new(|_| {
                    if let Some(exe) = settings_exec() {
                        let _ = Command::new(exe).spawn();
                    } else {
                        let _ = Command::new("xdg-open").arg(config_path()).spawn();
                    }
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

impl ClipTray {
    fn restore_index(&mut self, idx: usize) {
        let entry: Option<Entry> = self
            .state
            .history
            .lock()
            .ok()
            .and_then(|h| h.get(idx).cloned());
        if let Some(e) = entry {
            // arboard::Clipboard::set_text re-publishes the text. The watcher
            // loop will see this on the next poll and dedupe via push().
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(e.text);
            }
        }
    }
}

fn preview_label(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    let max = 50;
    if trimmed.chars().count() > max {
        let cut: String = trimmed.chars().take(max).collect();
        format!("{cut}…")
    } else if trimmed.is_empty() {
        "(whitespace)".into()
    } else {
        trimmed.to_string()
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    if let Some(cmd) = args.next() {
        match cmd.as_str() {
            "--help" | "-h" => {
                println!("cosmic-clip: Wayland-native clipboard history manager.");
                println!();
                println!("Usage:");
                println!("  cosmic-clip            run the tray daemon");
                println!("  cosmic-clip --help     this help");
                println!();
                println!("Run `cosmic-clip-settings` for the GUI settings editor.");
                return Ok(());
            }
            other => {
                eprintln!("cosmic-clip: unknown argument {other:?}");
                std::process::exit(2);
            }
        }
    }

    let cfg_path = config_path();
    let cfg = config::read(&cfg_path).unwrap_or_default();
    if !cfg_path.exists() {
        let _ = config::write(&cfg_path, &cfg);
    }

    let hist_path = history_path();
    let initial_history = if cfg.persist_history {
        History::load(&hist_path)
    } else {
        History::new()
    };

    let state = Arc::new(TrayState {
        history: Mutex::new(initial_history),
        cfg: Mutex::new(cfg.clone()),
    });

    let tray = ClipTray {
        state: state.clone(),
    };
    let _handle = tray.spawn().await?;

    // Watch loop: poll arboard at the configured cadence, dedupe via History.
    let watch_state = state.clone();
    tokio::spawn(async move {
        let mut last_seen: Option<String> = None;
        loop {
            let interval = watch_state
                .cfg
                .lock()
                .map(|c| c.poll_interval_ms.max(100))
                .unwrap_or(500);
            sleep(Duration::from_millis(interval)).await;

            let text_opt = tokio::task::spawn_blocking(|| {
                arboard::Clipboard::new()
                    .ok()
                    .and_then(|mut cb| cb.get_text().ok())
            })
            .await
            .ok()
            .flatten();

            let Some(text) = text_opt else { continue };
            if last_seen.as_deref() == Some(text.as_str()) {
                continue;
            }
            last_seen = Some(text.clone());

            let (cap, max_chars) = watch_state
                .cfg
                .lock()
                .map(|c| (c.history_size.max(1), c.max_entry_chars))
                .unwrap_or((50, 10_000));
            if text.chars().count() > max_chars {
                continue;
            }
            if let Ok(mut hist) = watch_state.history.lock() {
                hist.push(text, cap);
            }
        }
    });

    // Periodic save loop: serialize history to disk every 5s if configured.
    let save_state = state.clone();
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(5)).await;
            let persist = save_state
                .cfg
                .lock()
                .map(|c| c.persist_history)
                .unwrap_or(true);
            if persist {
                let snapshot = save_state
                    .history
                    .lock()
                    .map(|h| h.clone())
                    .unwrap_or_default();
                let path = history_path();
                let _ = tokio::task::spawn_blocking(move || snapshot.save(&path)).await;
            }
        }
    });

    std::future::pending::<()>().await;
    Ok(())
}
