#![forbid(unsafe_code)]
//! Keyboard-first terminal frontend.
//!
//! The TUI is a pure client: it owns no playback state, it renders replies from
//! the daemon and forwards intents. That keeps the UI responsive while the
//! network is slow and lets a CLI or an agent change what you are looking at.

pub mod app;
pub mod theme;
pub mod ui;

pub use app::UiPreferences;
pub use theme::{Density, Glyphs, THEMES, Theme};

use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyEvent, KeyEventKind};
use lekima_ipc::{Command, Event as DaemonEvent, EventFrame, IpcError, Reply, call, watch};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::app::{Action, App};

type ReplyResult = Result<Reply, IpcError>;

#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("terminal error: {0}")]
    Io(#[from] std::io::Error),
}

/// Runs the TUI until the user quits.
///
/// # Errors
///
/// Returns [`TuiError`] when the terminal cannot be drawn.
pub async fn run(
    socket: PathBuf,
    preferences: UiPreferences,
) -> Result<Option<UiPreferences>, TuiError> {
    let mut app = App::new();
    app.set_preferences(preferences);
    let mut theme = app.theme();

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, socket, &mut app, &mut theme).await;
    ratatui::restore();
    result.map(|()| app.changed_preferences())
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    socket: PathBuf,
    app: &mut App,
    theme: &mut Theme,
) -> Result<(), TuiError> {
    let mut keys = spawn_key_reader();
    let (sender, mut replies) = unbounded_channel::<ReplyResult>();
    let (event_sender, mut events) = unbounded_channel::<EventFrame>();
    spawn_event_watcher(socket.clone(), event_sender);

    dispatch(&socket, Command::Status, &sender);
    app.status_inflight = true;
    dispatch(&socket, Command::Playlists, &sender);
    dispatch(&socket, Command::Queue, &sender);
    dispatch(&socket, Command::Whoami, &sender);

    // 20 fps for the signal visualiser; status still polls at 4 Hz.
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(50));
    ticker.tick().await;

    loop {
        terminal.draw(|frame| ui::draw(frame, app, theme))?;

        tokio::select! {
            key = keys.recv() => {
                let Some(key) = key else { break };
                let action = app.handle_key(key);
                if matches!(action, Action::UiChanged) {
                    *theme = app.theme();
                } else if apply_action(action, &socket, &sender, app) {
                    break;
                }
            }
            reply = replies.recv() => {
                let Some(reply) = reply else { break };
                let follow_up = app.apply(reply);
                apply_action(follow_up, &socket, &sender, app);
            }
            frame = events.recv() => {
                let Some(frame) = frame else { break };
                let reply = match frame.event {
                    DaemonEvent::Status { state } => Some(Reply::Status { state }),
                    DaemonEvent::Queue { tracks, index } => {
                        Some(Reply::Queue { tracks, index })
                    }
                    DaemonEvent::Like { id, liked } => Some(Reply::Like { id, liked }),
                    DaemonEvent::Resync => {
                        dispatch(&socket, Command::Status, &sender);
                        None
                    }
                };
                if let Some(reply) = reply {
                    let follow_up = app.apply(Ok(reply));
                    apply_action(follow_up, &socket, &sender, app);
                }
            }
            _ = ticker.tick() => {
                app.tick();
                // 20 Hz while playing for the spectrum; 2 Hz otherwise.
                let every = if app.is_playing() { 1 } else { 10 };
                if app.frame.is_multiple_of(every) && !app.status_inflight && !app.daemon_down {
                    app.status_inflight = true;
                    dispatch(&socket, Command::Status, &sender);
                }
            }
        }
    }
    Ok(())
}

/// Returns `true` when the TUI should exit.
fn apply_action(
    action: Action,
    socket: &Path,
    sender: &UnboundedSender<ReplyResult>,
    app: &mut App,
) -> bool {
    match action {
        Action::None => false,
        Action::Quit => true,
        Action::Call(command) => {
            dispatch(socket, command, sender);
            false
        }
        Action::UiChanged => false,
        Action::SpawnDaemon => {
            spawn_daemon();
            app.notify("starting the daemon…", false);
            false
        }
    }
}

fn spawn_event_watcher(socket: PathBuf, sender: UnboundedSender<EventFrame>) {
    tokio::spawn(async move {
        loop {
            if watch(&socket, Vec::new(), None, sender.clone())
                .await
                .is_err()
            {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            } else {
                break;
            }
        }
    });
}

fn dispatch(socket: &Path, command: Command, sender: &UnboundedSender<ReplyResult>) {
    let socket = socket.to_path_buf();
    let sender = sender.clone();
    tokio::spawn(async move {
        let result = call(&socket, command).await;
        let _ = sender.send(result);
    });
}

fn spawn_daemon() {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(executable)
        .args(["daemon", "run", "--detached"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn spawn_key_reader() -> UnboundedReceiver<KeyEvent> {
    let (sender, receiver) = unbounded_channel();
    std::thread::spawn(move || {
        loop {
            match crossterm::event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    if sender.send(key).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    receiver
}
