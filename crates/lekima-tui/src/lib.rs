#![forbid(unsafe_code)]
//! Keyboard-first terminal frontend.
//!
//! The TUI is a pure client: it owns no playback state, it renders replies from
//! the daemon and forwards intents. That keeps the UI responsive while the
//! network is slow and lets a CLI or an agent change what you are looking at.

pub mod app;
pub mod theme;
pub mod ui;

pub use theme::Theme;

use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyEvent, KeyEventKind};
use lekima_ipc::{Command, IpcError, Reply, call};
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
pub async fn run(socket: PathBuf, theme: Theme) -> Result<(), TuiError> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, socket, theme).await;
    ratatui::restore();
    result
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    socket: PathBuf,
    theme: Theme,
) -> Result<(), TuiError> {
    let mut app = App::new();
    let mut keys = spawn_key_reader();
    let (sender, mut replies) = unbounded_channel::<ReplyResult>();

    dispatch(&socket, Command::Status, &sender);
    app.status_inflight = true;
    dispatch(&socket, Command::Playlists, &sender);
    dispatch(&socket, Command::Queue, &sender);
    dispatch(&socket, Command::Whoami, &sender);

    // 20 fps for the signal visualiser; status still polls at 4 Hz.
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(50));
    ticker.tick().await;

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app, &theme))?;

        tokio::select! {
            key = keys.recv() => {
                let Some(key) = key else { break };
                let action = app.handle_key(key);
                if apply_action(action, &socket, &sender, &mut app) {
                    break;
                }
            }
            reply = replies.recv() => {
                let Some(reply) = reply else { break };
                let follow_up = app.apply(reply);
                apply_action(follow_up, &socket, &sender, &mut app);
            }
            _ = ticker.tick() => {
                app.tick();
                // 10 Hz while playing so the waveform moves; 2 Hz otherwise.
                let every = if app.is_playing() { 2 } else { 10 };
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
        Action::SpawnDaemon => {
            spawn_daemon();
            app.notify("starting the daemon…", false);
            false
        }
    }
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
