//! View model and key handling for the TUI.
//!
//! The reducer is deliberately independent of the terminal so key sequences can
//! be unit-tested: `handle_key` maps a key to an [`Action`], and `apply` folds a
//! daemon reply back into the state.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use lekima_domain::{PlaybackState, PlaybackStatus, Playlist, Track};
use lekima_ipc::{Command, ErrorCode, IpcError, PlayTarget, Reply};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    Command,
    Help,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Main,
    Sidebar,
}

/// What the event loop should do after a key press.
#[derive(Debug, Clone)]
pub enum Action {
    None,
    Quit,
    Call(Command),
    /// Start a daemon because none was reachable.
    SpawnDaemon,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub text: String,
    pub error: bool,
    /// Frame the message was created on, for auto-dismissal.
    pub created: u64,
}

/// How long a toast stays on screen (50 ms ticks × 80 = 4 s).
pub const TOAST_FRAMES: u64 = 80;

impl Message {
    #[must_use]
    pub const fn is_visible(&self, now: u64) -> bool {
        now.saturating_sub(self.created) < TOAST_FRAMES
    }
}

pub struct App {
    pub status: PlaybackState,
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub queue: Vec<Track>,
    pub mode: Mode,
    pub focus: Focus,
    pub input: String,
    pub main_selection: usize,
    pub sidebar_selection: usize,
    pub queue_selection: usize,
    pub message: Option<Message>,
    pub source: String,
    pub user: Option<String>,
    pub status_inflight: bool,
    pub daemon_down: bool,
    /// Monotonic tick counter; drives cursor blink and subtle animation.
    pub frame: u64,
}

impl Default for App {
    fn default() -> Self {
        Self {
            status: PlaybackState::default(),
            tracks: Vec::new(),
            playlists: Vec::new(),
            queue: Vec::new(),
            mode: Mode::Normal,
            focus: Focus::Main,
            input: String::new(),
            main_selection: 0,
            sidebar_selection: 0,
            queue_selection: 0,
            message: None,
            source: "search".to_owned(),
            user: None,
            status_inflight: false,
            daemon_down: false,
            frame: 0,
        }
    }
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advances animation state; called on every UI tick.
    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.status.status == PlaybackStatus::Playing
    }

    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.message = Some(Message {
            text: text.into(),
            error,
            created: self.frame,
        });
    }

    fn clamp_selection(&mut self) {
        self.main_selection = self.main_selection.min(self.tracks.len().saturating_sub(1));
        self.sidebar_selection = self
            .sidebar_selection
            .min(self.playlists.len().saturating_sub(1));
        self.queue_selection = self.queue_selection.min(self.queue.len().saturating_sub(1));
    }

    fn selected_track(&self) -> Option<&Track> {
        self.tracks.get(self.main_selection)
    }

    /// Handles a key press and returns what the loop should do.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.kind != KeyEventKind::Press {
            return Action::None;
        }
        match self.mode {
            Mode::Help => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('q' | '?')) {
                    self.mode = Mode::Normal;
                }
                Action::None
            }
            Mode::Queue => self.handle_queue_key(key),
            Mode::Search | Mode::Command => self.handle_prompt_key(key),
            Mode::Normal => self.handle_normal_key(key),
        }
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.input.clear();
                Action::None
            }
            KeyCode::Backspace => {
                self.input.pop();
                Action::None
            }
            KeyCode::Enter => {
                let input = std::mem::take(&mut self.input);
                let mode = self.mode;
                self.mode = Mode::Normal;
                if input.trim().is_empty() {
                    return Action::None;
                }
                match mode {
                    Mode::Search => {
                        self.source = format!("search: {}", input.trim());
                        Action::Call(Command::Search {
                            query: input.trim().to_owned(),
                            kind: lekima_domain::SearchKind::Track,
                            limit: 40,
                            offset: 0,
                        })
                    }
                    _ => self.run_command(input.trim()),
                }
            }
            KeyCode::Char(character) => {
                self.input.push(character);
                Action::None
            }
            _ => Action::None,
        }
    }

    /// Minimal ex-style command line. Anything longer belongs in the CLI.
    fn run_command(&mut self, input: &str) -> Action {
        let (name, rest) = input.split_once(' ').unwrap_or((input, ""));
        match name {
            "q" | "quit" => Action::Quit,
            "play" if !rest.trim().is_empty() => Action::Call(Command::Play {
                target: PlayTarget::Query {
                    query: rest.trim().to_owned(),
                },
            }),
            "search" if !rest.trim().is_empty() => Action::Call(Command::Search {
                query: rest.trim().to_owned(),
                kind: lekima_domain::SearchKind::Track,
                limit: 40,
                offset: 0,
            }),
            "vol" | "volume" => match rest.trim().parse::<f32>() {
                Ok(level) => Action::Call(Command::SetVolume {
                    level: (level / 100.0).clamp(0.0, 1.0),
                }),
                Err(_) => {
                    self.notify("usage: :vol 0-100", true);
                    Action::None
                }
            },
            "pause" => Action::Call(Command::Pause),
            "resume" => Action::Call(Command::Resume),
            "toggle" => Action::Call(Command::Toggle),
            "next" => Action::Call(Command::Next),
            "prev" => Action::Call(Command::Prev),
            "queue" => {
                self.mode = Mode::Queue;
                Action::Call(Command::Queue)
            }
            "help" => {
                self.mode = Mode::Help;
                Action::None
            }
            "login" => {
                self.notify("run `lekima login --qr` in a shell", false);
                Action::None
            }
            other => {
                let text = format!("unknown command: {other}");
                self.notify(text, true);
                Action::None
            }
        }
    }

    fn handle_queue_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'Q') => {
                self.mode = Mode::Normal;
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.queue_selection =
                    (self.queue_selection + 1).min(self.queue.len().saturating_sub(1));
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.queue_selection = self.queue_selection.saturating_sub(1);
                Action::None
            }
            KeyCode::Char('g') => {
                self.queue_selection = 0;
                Action::None
            }
            KeyCode::Char('G') => {
                self.queue_selection = self.queue.len().saturating_sub(1);
                Action::None
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                Action::Call(Command::Jump {
                    index: self.queue_selection,
                })
            }
            _ => Action::None,
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => Action::Quit,
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                Action::None
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.input.clear();
                Action::None
            }
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.input.clear();
                Action::None
            }
            KeyCode::Char('R') => Action::SpawnDaemon,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Main => Focus::Sidebar,
                    Focus::Sidebar => Focus::Main,
                };
                Action::None
            }
            KeyCode::Char('Q') => {
                self.mode = Mode::Queue;
                Action::Call(Command::Queue)
            }
            KeyCode::Char(' ') => Action::Call(Command::Toggle),
            KeyCode::Char('n') => Action::Call(Command::Next),
            KeyCode::Char('p') => Action::Call(Command::Prev),
            KeyCode::Char('m') => Action::Call(Command::ToggleMute),
            KeyCode::Char('r') => Action::Call(Command::CycleRepeat),
            KeyCode::Char('s') => Action::Call(Command::SetShuffle {
                enabled: !self.status.shuffle,
            }),
            KeyCode::Char('+') | KeyCode::Char('=') => Action::Call(Command::SetVolume {
                level: (self.status.volume + 0.05).min(1.0),
            }),
            KeyCode::Char('-') => Action::Call(Command::SetVolume {
                level: (self.status.volume - 0.05).max(0.0),
            }),
            KeyCode::Char('l') | KeyCode::Right => Action::Call(Command::Seek {
                position_ms: self.status.position_ms.saturating_add(10_000),
            }),
            KeyCode::Char('h') | KeyCode::Left => Action::Call(Command::Seek {
                position_ms: self.status.position_ms.saturating_sub(10_000),
            }),
            KeyCode::Char('a') => self.enqueue_selected(false),
            KeyCode::Char('A') => self.enqueue_selected(true),
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_selection(1);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_selection(-1);
                Action::None
            }
            KeyCode::Char('d') if ctrl => {
                self.move_selection(10);
                Action::None
            }
            KeyCode::Char('u') if ctrl => {
                self.move_selection(-10);
                Action::None
            }
            KeyCode::Char('g') => {
                self.set_selection(0);
                Action::None
            }
            KeyCode::Char('G') => {
                self.set_selection(usize::MAX);
                Action::None
            }
            KeyCode::Enter => self.open_selection(),
            _ => Action::None,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let current = match self.focus {
            Focus::Main => self.main_selection,
            Focus::Sidebar => self.sidebar_selection,
        } as isize;
        let next = (current + delta).max(0) as usize;
        match self.focus {
            Focus::Main => self.main_selection = next.min(self.tracks.len().saturating_sub(1)),
            Focus::Sidebar => {
                self.sidebar_selection = next.min(self.playlists.len().saturating_sub(1));
            }
        }
    }

    fn set_selection(&mut self, index: usize) {
        match self.focus {
            Focus::Main => self.main_selection = index.min(self.tracks.len().saturating_sub(1)),
            Focus::Sidebar => {
                self.sidebar_selection = index.min(self.playlists.len().saturating_sub(1));
            }
        }
    }

    fn open_selection(&mut self) -> Action {
        match self.focus {
            Focus::Sidebar => match self.playlists.get(self.sidebar_selection) {
                Some(playlist) => {
                    self.source = format!("playlist: {}", playlist.name);
                    Action::Call(Command::PlaylistTracks { id: playlist.id })
                }
                None => Action::None,
            },
            Focus::Main => {
                let ids: Vec<_> = self
                    .tracks
                    .iter()
                    .skip(self.main_selection)
                    .map(|track| track.id)
                    .collect();
                if ids.is_empty() {
                    self.notify("nothing selected", true);
                    Action::None
                } else {
                    Action::Call(Command::Play {
                        target: PlayTarget::Tracks { ids },
                    })
                }
            }
        }
    }

    fn enqueue_selected(&mut self, next: bool) -> Action {
        if self.focus == Focus::Sidebar {
            self.notify("select a track to queue", true);
            return Action::None;
        }
        match self.selected_track().map(|track| track.id) {
            Some(id) => {
                self.notify(
                    if next {
                        "queued to play next"
                    } else {
                        "queued"
                    },
                    false,
                );
                Action::Call(Command::Enqueue {
                    tracks: vec![id],
                    next,
                })
            }
            None => Action::None,
        }
    }

    /// Folds a daemon reply into the view state. Returns a follow-up command.
    pub fn apply(&mut self, result: Result<Reply, IpcError>) -> Action {
        self.status_inflight = false;
        match result {
            Ok(reply) => {
                self.daemon_down = false;
                match reply {
                    Reply::Status { state } => {
                        self.status = state;
                    }
                    Reply::Tracks { tracks } => {
                        self.tracks = tracks;
                        self.main_selection = 0;
                    }
                    Reply::Search { results } => {
                        self.tracks = results.tracks;
                        self.main_selection = 0;
                        self.focus = Focus::Main;
                        if self.tracks.is_empty() {
                            self.notify("no matches", true);
                        }
                    }
                    Reply::Playlists { playlists } => {
                        self.playlists = playlists;
                    }
                    Reply::Queue { tracks, index } => {
                        self.queue = tracks;
                        self.queue_selection = index.unwrap_or(0);
                    }
                    Reply::User { user } => {
                        self.user = user.map(|user| user.nickname);
                    }
                    Reply::Lyrics { text } => {
                        if text.trim().is_empty() {
                            self.notify("no lyrics", true);
                        } else {
                            self.notify("lyrics available in the CLI for now", false);
                        }
                    }
                    Reply::Pong | Reply::Ack => {}
                }
                Action::None
            }
            Err(error) => {
                if error.is_not_running() {
                    self.daemon_down = true;
                    self.notify("daemon not running — press R to start it", true);
                } else {
                    self.notify(error_message(&error), true);
                }
                Action::None
            }
        }
    }

    pub fn clamp(&mut self) {
        self.clamp_selection();
    }
}

fn error_message(error: &IpcError) -> String {
    match error {
        IpcError::Remote { code, message } => format!("{}: {message}", code_label(*code)),
        other => other.to_string(),
    }
}

const fn code_label(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::BadRequest => "bad request",
        ErrorCode::Unauthorized => "unauthorized",
        ErrorCode::NotFound => "not found",
        ErrorCode::Unsupported => "unsupported",
        ErrorCode::Backend => "provider error",
        ErrorCode::Playback => "playback error",
        ErrorCode::Daemon => "daemon error",
        ErrorCode::Internal => "internal error",
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{Action, App, Focus, Mode};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn slash_enters_search_and_escape_returns() {
        let mut app = App::new();
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('/'))),
            Action::None
        ));
        assert_eq!(app.mode, Mode::Search);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn search_submit_emits_a_search_command() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('/')));
        for character in "mota".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        let action = app.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            action,
            Action::Call(lekima_ipc::Command::Search { .. })
        ));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn horizontal_navigation_emits_seek_commands_safely() {
        let mut app = App::new();
        app.status.position_ms = 5_000;
        match app.handle_key(key(KeyCode::Char('h'))) {
            Action::Call(lekima_ipc::Command::Seek { position_ms }) => {
                assert_eq!(position_ms, 0);
            }
            other => panic!("unexpected action: {other:?}"),
        }
        app.status.position_ms = 5_000;
        match app.handle_key(key(KeyCode::Char('l'))) {
            Action::Call(lekima_ipc::Command::Seek { position_ms }) => {
                assert_eq!(position_ms, 15_000);
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn tab_switches_focus() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Sidebar);
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Main);
    }

    #[test]
    fn queue_overlay_opens_and_closes() {
        let mut app = App::new();
        let action = app.handle_key(key(KeyCode::Char('Q')));
        assert!(matches!(action, Action::Call(lekima_ipc::Command::Queue)));
        assert_eq!(app.mode, Mode::Queue);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn quit_keys_quit() {
        let mut app = App::new();
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('q'))),
            Action::Quit
        ));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(matches!(app.handle_key(ctrl_c), Action::Quit));
    }
}
