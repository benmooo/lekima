#![forbid(unsafe_code)]
//! Keyboard-first terminal frontend.
//!
//! The TUI is a pure client: it owns no playback state, it renders replies from
//! the daemon and forwards intents. That keeps the UI responsive while the
//! network is slow and lets a CLI or an agent change what you are looking at.

pub mod app;
pub mod keymap;
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

    // Draw only when something changed. With reduced motion the 20 Hz ticker
    // does not repaint, so an idle player costs zero frames.
    let mut dirty = true;
    loop {
        if dirty {
            terminal.draw(|frame| ui::draw(frame, app, theme))?;
            dirty = false;
        }

        tokio::select! {
            key = keys.recv() => {
                let Some(key) = key else { break };
                let action = app.handle_key(key);
                if matches!(action, Action::UiChanged) {
                    *theme = app.theme();
                } else if apply_action(action, &socket, &sender, app) {
                    break;
                }
                dirty = true;
            }
            reply = replies.recv() => {
                let Some(reply) = reply else { break };
                let status_only = matches!(&reply, Ok(Reply::Status { .. }));
                let before = status_fingerprint(app);
                let follow_up = app.apply(reply);
                apply_action(follow_up, &socket, &sender, app);
                if !status_only
                    || !theme.motion.is_reduced()
                    || status_fingerprint(app) != before
                {
                    dirty = true;
                }
            }
            frame = events.recv() => {
                let Some(frame) = frame else { break };
                let status_only = matches!(frame.event, DaemonEvent::Status { .. });
                let before = status_fingerprint(app);
                app.note_event(frame.seq);
                let reply = match frame.event {
                    DaemonEvent::Status { state } => Some(Reply::Status { state: *state }),
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
                if !status_only
                    || !theme.motion.is_reduced()
                    || status_fingerprint(app) != before
                {
                    dirty = true;
                }
            }
            _ = ticker.tick() => {
                let visible_before = app
                    .message
                    .as_ref()
                    .is_some_and(|message| message.is_visible(app.frame));
                app.tick();
                let visible_after = app
                    .message
                    .as_ref()
                    .is_some_and(|message| message.is_visible(app.frame));
                if !theme.motion.is_reduced() || visible_before != visible_after {
                    dirty = true;
                }
                // 20 Hz while playing for the spectrum; 1 Hz while the daemon
                // is down so a restart is noticed without pressing R.
                let every = if app.is_playing() {
                    1
                } else if app.daemon_down {
                    20
                } else {
                    10
                };
                if app.frame.is_multiple_of(every) && !app.status_inflight {
                    app.status_inflight = true;
                    dispatch(&socket, Command::Status, &sender);
                }
            }
        }
    }
    Ok(())
}

/// Everything the frame depends on that can change without a key press.
/// Reduced motion compares fingerprints so an unchanged status poll does not
/// repaint the screen.
fn status_fingerprint(app: &App) -> String {
    // Only fields that affect a rendered cell. Volatile but invisible data
    // (`wave`, `revision`, `generation`) must not force a repaint.
    let playback = (
        app.status.track.as_ref().map(|track| {
            (
                track.id,
                track.name.clone(),
                track.artist_names(),
                track.duration_ms,
            )
        }),
        app.status.status,
        app.status.position_ms / 1000,
        app.status.queue_length,
        app.status.queue_index,
        (app.status.volume * 100.0) as i32,
        app.status.muted,
        app.status.liked,
        app.status.quality,
    );
    let surfaces = (
        &app.status.buffered,
        app.status.repeat,
        app.status.shuffle,
        app.status.loading,
        app.status.buffering,
        app.status.cached,
        app.status.sleep_remaining_ms,
        app.daemon_down,
        app.message
            .as_ref()
            .map(|message| (message.created, message.error)),
    );
    format!("{playback:?}|{surfaces:?}")
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
            app.notify("starting the daemon...", false);
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
