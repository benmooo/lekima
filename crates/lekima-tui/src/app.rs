//! View model and key handling for the TUI.
//!
//! The reducer is deliberately independent of the terminal so key sequences can
//! be unit-tested: `handle_key` maps a key to an [`Action`], and `apply` folds a
//! daemon reply back into the state.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use lekima_domain::{Lyrics, PlaybackState, PlaybackStatus, Playlist, Toplist, Track, TrackId};
use lekima_ipc::{Command, ErrorCode, IpcError, PlayTarget, Reply};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    Command,
    Help,
    Queue,
    Settings,
}

/// Persisted UI choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiPreferences {
    pub theme: String,
    pub glyphs: String,
    pub density: String,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            theme: "cyber".to_owned(),
            glyphs: "unicode".to_owned(),
            density: "compact".to_owned(),
        }
    }
}

const GLYPHS: [&str; 2] = ["unicode", "ascii"];
const DENSITIES: [&str; 2] = ["compact", "comfortable"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Tracks,
    Toplists,
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
    /// Theme, glyphs or density changed; rebuild the palette.
    UiChanged,
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

/// Lifecycle of the lyrics pane for one track.
#[derive(Debug, Clone, Default)]
pub enum LyricsState {
    #[default]
    Idle,
    Loading {
        track: TrackId,
    },
    Ready {
        track: TrackId,
        lyrics: Lyrics,
    },
    Empty {
        track: TrackId,
    },
    Error {
        track: TrackId,
        message: String,
    },
}

impl LyricsState {
    #[must_use]
    pub fn track(&self) -> Option<TrackId> {
        match self {
            Self::Idle => None,
            Self::Loading { track }
            | Self::Ready { track, .. }
            | Self::Empty { track }
            | Self::Error { track, .. } => Some(*track),
        }
    }

    #[must_use]
    pub fn ready(&self) -> Option<&Lyrics> {
        match self {
            Self::Ready { lyrics, .. } => Some(lyrics),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    view: View,
    tracks: Vec<Track>,
    toplists: Vec<Toplist>,
    source: String,
    selection: usize,
}

/// Sidebar entries that are not playlists, in display order.
pub const SIDEBAR_STATICS: [&str; 5] = ["DAILY", "FM", "TOPLISTS", "CLOUD", "LIKED"];

pub struct App {
    pub status: PlaybackState,
    pub tracks: Vec<Track>,
    pub toplists: Vec<Toplist>,
    pub view: View,
    pub lyrics_state: LyricsState,
    pub lyrics_open: bool,
    pub lyrics_offset_ms: i64,
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
    history_back: Vec<HistoryEntry>,
    history_forward: Vec<HistoryEntry>,
    pub preferences: UiPreferences,
    pub settings_selection: usize,
    settings_dirty: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            status: PlaybackState::default(),
            tracks: Vec::new(),
            toplists: Vec::new(),
            view: View::Tracks,
            lyrics_state: LyricsState::Idle,
            lyrics_open: false,
            lyrics_offset_ms: 0,
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
            history_back: Vec::new(),
            history_forward: Vec::new(),
            preferences: UiPreferences::default(),
            settings_selection: 0,
            settings_dirty: false,
        }
    }
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn history_snapshot(&self) -> HistoryEntry {
        HistoryEntry {
            view: self.view,
            tracks: self.tracks.clone(),
            toplists: self.toplists.clone(),
            source: self.source.clone(),
            selection: self.main_selection,
        }
    }

    fn restore_history(&mut self, entry: HistoryEntry) {
        self.view = entry.view;
        self.tracks = entry.tracks;
        self.toplists = entry.toplists;
        self.source = entry.source;
        self.main_selection = entry.selection;
        self.focus = Focus::Main;
    }

    /// Records the current middle-pane state before navigating away.
    fn remember(&mut self) {
        let current = self.history_snapshot();
        if self
            .history_back
            .last()
            .is_some_and(|entry| entry.source == current.source && entry.view == current.view)
        {
            return;
        }
        self.history_forward.clear();
        if self.history_back.len() == 32 {
            self.history_back.remove(0);
        }
        self.history_back.push(current);
    }

    fn go_back(&mut self) {
        match self.history_back.pop() {
            Some(entry) => {
                self.history_forward.push(self.history_snapshot());
                self.restore_history(entry);
            }
            None => self.notify("oldest view", false),
        }
    }

    fn go_forward(&mut self) {
        match self.history_forward.pop() {
            Some(entry) => {
                self.history_back.push(self.history_snapshot());
                self.restore_history(entry);
            }
            None => self.notify("newest view", false),
        }
    }

    #[must_use]
    pub fn theme(&self) -> crate::theme::Theme {
        crate::theme::Theme::resolve(
            &self.preferences.theme,
            &self.preferences.glyphs,
            &self.preferences.density,
        )
    }

    pub fn set_preferences(&mut self, preferences: UiPreferences) {
        self.preferences = preferences;
    }

    /// Returns the preferences to persist when the TUI exits.
    #[must_use]
    pub fn changed_preferences(&self) -> Option<UiPreferences> {
        self.settings_dirty.then(|| self.preferences.clone())
    }

    fn cycle_setting(&mut self, delta: isize) -> Action {
        let (list, current, field): (&[&str], &mut String, usize) = match self.settings_selection {
            0 => (&crate::theme::THEMES, &mut self.preferences.theme, 0),
            1 => (&GLYPHS, &mut self.preferences.glyphs, 1),
            _ => (&DENSITIES, &mut self.preferences.density, 2),
        };
        let index = list
            .iter()
            .position(|value| *value == current.as_str())
            .unwrap_or(0) as isize;
        let len = list.len() as isize;
        let next = (index + delta).rem_euclid(len) as usize;
        *current = list[next].to_owned();
        let _ = field;
        self.settings_dirty = true;
        Action::UiChanged
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc | KeyCode::Char('S') => {
                self.mode = Mode::Normal;
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.settings_selection = (self.settings_selection + 1).min(2);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.settings_selection = self.settings_selection.saturating_sub(1);
                Action::None
            }
            KeyCode::Char('h') | KeyCode::Left => self.cycle_setting(-1),
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                self.cycle_setting(1)
            }
            _ => Action::None,
        }
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

    #[must_use]
    pub fn sidebar_len(&self) -> usize {
        SIDEBAR_STATICS.len() + self.playlists.len()
    }

    fn clamp_selection(&mut self) {
        let main = match self.view {
            View::Tracks => self.tracks.len(),
            View::Toplists => self.toplists.len(),
        };
        self.main_selection = self.main_selection.min(main.saturating_sub(1));
        self.sidebar_selection = self
            .sidebar_selection
            .min(self.sidebar_len().saturating_sub(1));
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
            Mode::Settings => self.handle_settings_key(key),
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
                        self.remember();
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
            "theme" | "glyphs" | "density" if !rest.trim().is_empty() => {
                let value = rest.trim().to_owned();
                match name {
                    "theme" => self.preferences.theme = value,
                    "glyphs" => self.preferences.glyphs = value,
                    _ => self.preferences.density = value,
                }
                self.settings_dirty = true;
                Action::UiChanged
            }
            "ui" | "settings" => {
                self.mode = Mode::Settings;
                self.settings_selection = 0;
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
            KeyCode::Char('o') if ctrl => {
                self.go_back();
                Action::None
            }
            KeyCode::Char('i') if ctrl => {
                self.go_forward();
                Action::None
            }
            KeyCode::Char('c') if ctrl => Action::Quit,
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                Action::None
            }
            KeyCode::Char('S') => {
                self.mode = Mode::Settings;
                self.settings_selection = 0;
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
            KeyCode::Char('F') => Action::Call(Command::Fm {
                action: lekima_ipc::FmAction::Start,
            }),
            KeyCode::Char('t') => {
                self.remember();
                Action::Call(Command::Toplists)
            }
            KeyCode::Char('c') => {
                self.remember();
                self.source = "cloud".to_owned();
                Action::Call(Command::Cloud {
                    limit: 100,
                    offset: 0,
                    play: false,
                })
            }
            KeyCode::Char('f') => self.like_selected(),
            KeyCode::Char('L') => self.toggle_lyrics(),
            KeyCode::Char('[') => {
                if self.lyrics_open {
                    self.lyrics_offset_ms -= 250;
                }
                Action::None
            }
            KeyCode::Char(']') => {
                if self.lyrics_open {
                    self.lyrics_offset_ms += 250;
                }
                Action::None
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
            KeyCode::Char('d') => {
                self.remember();
                self.source = "daily".to_owned();
                Action::Call(Command::Daily { play: false })
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
            Focus::Main => {
                let len = match self.view {
                    View::Tracks => self.tracks.len(),
                    View::Toplists => self.toplists.len(),
                };
                self.main_selection = next.min(len.saturating_sub(1));
            }
            Focus::Sidebar => {
                self.sidebar_selection = next.min(self.sidebar_len().saturating_sub(1));
            }
        }
    }

    fn set_selection(&mut self, index: usize) {
        match self.focus {
            Focus::Main => {
                let len = match self.view {
                    View::Tracks => self.tracks.len(),
                    View::Toplists => self.toplists.len(),
                };
                self.main_selection = index.min(len.saturating_sub(1));
            }
            Focus::Sidebar => {
                self.sidebar_selection = index.min(self.sidebar_len().saturating_sub(1));
            }
        }
    }

    fn open_selection(&mut self) -> Action {
        match self.focus {
            Focus::Sidebar => {
                if let Some(action) = self.open_static(self.sidebar_selection) {
                    return action;
                }
                let index = self.sidebar_selection.saturating_sub(SIDEBAR_STATICS.len());
                match self.playlists.get(index) {
                    Some(playlist) => {
                        let (id, name) = (playlist.id, playlist.name.clone());
                        self.remember();
                        self.source = format!("playlist: {name}");
                        Action::Call(Command::PlaylistTracks { id })
                    }
                    None => Action::None,
                }
            }
            Focus::Main => match self.view {
                View::Toplists => match self.toplists.get(self.main_selection) {
                    Some(chart) => {
                        let (id, name) = (chart.id, chart.name.clone());
                        self.remember();
                        self.source = format!("chart: {name}");
                        Action::Call(Command::PlaylistTracks { id })
                    }
                    None => Action::None,
                },
                View::Tracks => {
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
            },
        }
    }

    fn open_static(&mut self, index: usize) -> Option<Action> {
        if matches!(index, 0 | 2 | 3 | 4) {
            self.remember();
        }
        Some(match index {
            0 => {
                self.source = "daily".to_owned();
                Action::Call(Command::Daily { play: false })
            }
            1 => Action::Call(Command::Fm {
                action: lekima_ipc::FmAction::Start,
            }),
            2 => Action::Call(Command::Toplists),
            3 => {
                self.source = "cloud".to_owned();
                Action::Call(Command::Cloud {
                    limit: 100,
                    offset: 0,
                    play: false,
                })
            }
            4 => {
                self.source = "liked".to_owned();
                Action::Call(Command::Liked { play: false })
            }
            _ => return None,
        })
    }

    fn like_selected(&mut self) -> Action {
        let track = if self.focus == Focus::Main {
            self.tracks.get(self.main_selection)
        } else {
            self.status.track.as_ref()
        };
        let Some(track) = track else {
            self.notify("nothing to like", true);
            return Action::None;
        };
        let is_current = self
            .status
            .track
            .as_ref()
            .is_some_and(|current| current.id == track.id);
        let liked = if is_current { !self.status.liked } else { true };
        Action::Call(Command::Like {
            id: Some(track.id),
            liked,
        })
    }

    fn toggle_lyrics(&mut self) -> Action {
        if self.lyrics_open {
            self.lyrics_open = false;
            return Action::None;
        }
        let Some(track) = self.status.track.as_ref() else {
            self.notify("nothing playing", true);
            return Action::None;
        };
        let id = track.id;
        self.lyrics_open = true;
        self.lyrics_state = LyricsState::Loading { track: id };
        Action::Call(Command::Lyrics { id: Some(id) })
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
                        let previous = self.status.track.as_ref().map(|track| track.id);
                        self.status = state;
                        let current = self.status.track.as_ref().map(|track| track.id);
                        if previous != current {
                            if self.lyrics_open {
                                match current {
                                    Some(id) => {
                                        self.lyrics_state = LyricsState::Loading { track: id };
                                        return Action::Call(Command::Lyrics { id: Some(id) });
                                    }
                                    None => {
                                        self.lyrics_state = LyricsState::Idle;
                                    }
                                }
                            } else {
                                self.lyrics_state = LyricsState::Idle;
                            }
                        }
                    }
                    Reply::Tracks { tracks } => {
                        self.tracks = tracks;
                        self.view = View::Tracks;
                        self.main_selection = 0;
                        self.focus = Focus::Main;
                    }
                    Reply::Search { results } => {
                        self.tracks = results.tracks;
                        self.view = View::Tracks;
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
                    Reply::Lyrics { id, lyrics } => {
                        // Ignore replies for a track that is no longer playing.
                        if self
                            .status
                            .track
                            .as_ref()
                            .is_some_and(|track| track.id == id)
                        {
                            if lyrics.is_empty() {
                                self.lyrics_state = LyricsState::Empty { track: id };
                            } else {
                                self.lyrics_state = LyricsState::Ready { track: id, lyrics };
                            }
                        }
                    }
                    Reply::Toplists { toplists } => {
                        self.toplists = toplists;
                        self.view = View::Toplists;
                        self.main_selection = 0;
                        self.focus = Focus::Main;
                        if self.toplists.is_empty() {
                            self.notify("no charts", true);
                        }
                    }
                    Reply::Like { id, liked } => {
                        if self
                            .status
                            .track
                            .as_ref()
                            .is_some_and(|track| track.id == id)
                        {
                            self.status.liked = liked;
                        }
                        self.notify(if liked { "♥ liked" } else { "♡ unliked" }, false);
                    }
                    Reply::Pong | Reply::Ack | Reply::Welcome { .. } | Reply::Subscribed { .. } => {
                    }
                }
                Action::None
            }
            Err(error) => {
                if let LyricsState::Loading { track } = self.lyrics_state {
                    self.lyrics_state = LyricsState::Error {
                        track,
                        message: crate::app::error_message(&error),
                    };
                }
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

    use lekima_domain::Track;

    use super::{Action, App, Focus, LyricsState, Mode};

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
    fn discovery_and_like_keys_emit_commands() {
        let mut app = App::new();
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('d'))),
            Action::Call(lekima_ipc::Command::Daily { .. })
        ));
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('t'))),
            Action::Call(lekima_ipc::Command::Toplists)
        ));
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('F'))),
            Action::Call(lekima_ipc::Command::Fm { .. })
        ));
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('c'))),
            Action::Call(lekima_ipc::Command::Cloud { .. })
        ));
    }

    fn sample_track(id: u64) -> Track {
        Track {
            id: id.into(),
            name: format!("track-{id}"),
            artists: Vec::new(),
            album: None,
            duration_ms: 0,
        }
    }

    #[test]
    fn track_changes_refresh_open_lyrics() {
        let mut app = App::new();
        app.status.track = Some(sample_track(1));
        app.lyrics_open = true;
        app.lyrics_state = LyricsState::Ready {
            track: 1.into(),
            lyrics: lekima_domain::parse_lrc("[00:01.00]old"),
        };

        let mut state = app.status.clone();
        state.track = Some(sample_track(2));
        let action = app.apply(Ok(lekima_ipc::Reply::Status { state }));

        assert!(matches!(
            action,
            Action::Call(lekima_ipc::Command::Lyrics { id: Some(id) }) if id == 2.into()
        ));
        assert!(matches!(
            app.lyrics_state,
            LyricsState::Loading { track } if track == 2.into()
        ));
    }

    #[test]
    fn stale_lyrics_replies_are_ignored() {
        let mut app = App::new();
        app.status.track = Some(sample_track(2));
        app.lyrics_open = true;
        app.lyrics_state = LyricsState::Loading { track: 2.into() };

        app.apply(Ok(lekima_ipc::Reply::Lyrics {
            id: 1.into(),
            lyrics: lekima_domain::parse_lrc("[00:01.00]stale"),
        }));

        assert!(matches!(
            app.lyrics_state,
            LyricsState::Loading { track } if track == 2.into()
        ));
    }

    #[test]
    fn lyrics_replies_become_ready_or_empty() {
        let mut app = App::new();
        app.status.track = Some(sample_track(3));
        app.lyrics_open = true;

        app.apply(Ok(lekima_ipc::Reply::Lyrics {
            id: 3.into(),
            lyrics: lekima_domain::parse_lrc("[00:01.00]line"),
        }));
        assert!(app.lyrics_state.ready().is_some());

        app.apply(Ok(lekima_ipc::Reply::Lyrics {
            id: 3.into(),
            lyrics: lekima_domain::Lyrics::default(),
        }));
        assert!(matches!(app.lyrics_state, LyricsState::Empty { .. }));
    }

    #[test]
    fn history_navigates_back_and_forward() {
        let mut app = App::new();
        app.tracks = vec![sample_track(1)];
        app.source = "search".to_owned();

        app.handle_key(key(KeyCode::Char('d')));
        app.apply(Ok(lekima_ipc::Reply::Tracks {
            tracks: vec![sample_track(2)],
        }));
        app.source = "daily".to_owned();

        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(app.source, "search");
        assert_eq!(app.tracks[0].id, 1.into());

        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL));
        assert_eq!(app.source, "daily");
        assert_eq!(app.tracks[0].id, 2.into());
    }

    #[test]
    fn new_navigation_clears_forward_history() {
        let mut app = App::new();
        app.tracks = vec![sample_track(1)];
        app.source = "search".to_owned();
        app.handle_key(key(KeyCode::Char('d')));
        app.source = "daily".to_owned();
        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(app.source, "search");

        app.handle_key(key(KeyCode::Char('t')));
        app.source = "charts".to_owned();
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL));
        assert_eq!(app.source, "charts");
    }

    #[test]
    fn settings_overlay_changes_and_marks_preferences() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('S')));
        assert_eq!(app.mode, Mode::Settings);
        let action = app.handle_key(key(KeyCode::Char('l')));
        assert!(matches!(action, Action::UiChanged));
        assert_eq!(app.preferences.theme, "amber");
        assert!(app.changed_preferences().is_some());
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
