//! View model and key handling for the TUI.
//!
//! The reducer is deliberately independent of the terminal so key sequences can
//! be unit-tested: `handle_key` maps a key to an [`Action`], and `apply` folds a
//! daemon reply back into the state.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use lekima_domain::{
    Album, AlbumDetail, AlbumId, Artist, ArtistDetail, ArtistId, Lyrics, PlaybackState,
    PlaybackStatus, Playlist, PlaylistId, SearchKind, SearchResults, Toplist, Track, TrackId,
    sanitize_line,
};
use lekima_ipc::{Command, ErrorCode, IpcError, PlayTarget, Reply};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    Command,
    Help,
    Queue,
    Settings,
    Comments,
    History,
    Playlists,
}

/// Persisted UI choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiPreferences {
    pub theme: String,
    pub glyphs: String,
    pub density: String,
    /// `auto`, `full` or `reduced`; the TUI only honours the reduced flag.
    pub motion: String,
    /// Starting lyrics offset; `[`/`]` adjust it for the session.
    pub lyrics_offset_ms: i64,
    pub lyrics_translation: bool,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            theme: "cyber".to_owned(),
            glyphs: "unicode".to_owned(),
            density: "compact".to_owned(),
            motion: "auto".to_owned(),
            lyrics_offset_ms: 0,
            lyrics_translation: false,
        }
    }
}

const GLYPHS: [&str; 2] = ["unicode", "ascii"];
const DENSITIES: [&str; 2] = ["compact", "comfortable"];
/// Rows in the settings overlay: theme, glyphs, density, translation, output.
pub const SETTINGS_ROWS: usize = 5;
/// Prompt history is capped per mode.
const HISTORY_LIMIT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Tracks,
    Toplists,
    Search,
    Artist,
    Album,
}

/// Result filter shown above the search prompt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchFilter {
    #[default]
    All,
    Tracks,
    Albums,
    Artists,
    Playlists,
}

impl SearchFilter {
    /// Display order, also the `Ctrl-1..5` order.
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Tracks,
        Self::Albums,
        Self::Artists,
        Self::Playlists,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Tracks => "TRACKS",
            Self::Albums => "ALBUMS",
            Self::Artists => "ARTISTS",
            Self::Playlists => "PLAYLISTS",
        }
    }

    #[must_use]
    pub const fn kind(self) -> SearchKind {
        match self {
            Self::All => SearchKind::All,
            Self::Tracks => SearchKind::Track,
            Self::Albums => SearchKind::Album,
            Self::Artists => SearchKind::Artist,
            Self::Playlists => SearchKind::Playlist,
        }
    }

    #[must_use]
    pub fn shifted(self, delta: isize) -> Self {
        let index = Self::ALL
            .iter()
            .position(|filter| *filter == self)
            .unwrap_or(0) as isize;
        Self::ALL[(index + delta).rem_euclid(Self::ALL.len() as isize) as usize]
    }

    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
}

/// One row of a search result page, in section order.
#[derive(Debug, Clone)]
pub enum SearchEntry {
    Track(Track),
    Album(Album),
    Artist(Artist),
    Playlist(Playlist),
}

impl SearchEntry {
    fn flatten(results: &SearchResults, filter: SearchFilter) -> Vec<Self> {
        let mut entries = Vec::new();
        if matches!(filter, SearchFilter::All | SearchFilter::Tracks) {
            entries.extend(results.tracks.iter().cloned().map(Self::Track));
        }
        if matches!(filter, SearchFilter::All | SearchFilter::Albums) {
            entries.extend(results.albums.iter().cloned().map(Self::Album));
        }
        if matches!(filter, SearchFilter::All | SearchFilter::Artists) {
            entries.extend(results.artists.iter().cloned().map(Self::Artist));
        }
        if matches!(filter, SearchFilter::All | SearchFilter::Playlists) {
            entries.extend(results.playlists.iter().cloned().map(Self::Playlist));
        }
        entries
    }
}

/// The search page: query, filter and flattened entries.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub filter: SearchFilter,
    /// Filter used for the results currently on screen.
    pub applied_filter: SearchFilter,
    pub loading: bool,
    pub results: Option<SearchResults>,
    pub entries: Vec<SearchEntry>,
    pub error: Option<String>,
}

/// The artist page: profile, one page of albums and paging state.
#[derive(Debug, Clone, Default)]
pub struct ArtistState {
    pub id: Option<ArtistId>,
    pub detail: Option<ArtistDetail>,
    pub albums: Vec<Album>,
    pub has_more: bool,
    /// True while the next album page is in flight.
    pub paging: bool,
    pub loading: bool,
    pub error: Option<String>,
}

/// The album page: metadata plus its ordered tracks.
#[derive(Debug, Clone, Default)]
pub struct AlbumState {
    pub id: Option<AlbumId>,
    pub detail: Option<AlbumDetail>,
    pub loading: bool,
    pub error: Option<String>,
}

impl AlbumState {
    pub fn tracks(&self) -> &[Track] {
        self.detail.as_ref().map_or(&[], |detail| &detail.tracks)
    }
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

/// Lifecycle of the comments pane for one track.
#[derive(Debug, Clone, Default)]
pub enum CommentsState {
    #[default]
    Idle,
    Loading {
        track: TrackId,
    },
    Ready {
        track: TrackId,
        comments: lekima_domain::Comments,
    },
    Empty {
        track: TrackId,
    },
    Error {
        track: TrackId,
        message: String,
    },
}

impl CommentsState {
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
    pub fn ready(&self) -> Option<&lekima_domain::Comments> {
        match self {
            Self::Ready { comments, .. } => Some(comments),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    view: View,
    tracks: Vec<Track>,
    toplists: Vec<Toplist>,
    search: SearchState,
    artist: ArtistState,
    album: AlbumState,
    source: String,
    selection: usize,
}

/// Sidebar entries that are not playlists, in display order.
pub const SIDEBAR_STATICS: [&str; 5] = ["DAILY", "FM", "TOPLISTS", "CLOUD", "LIKED"];

pub struct App {
    pub status: PlaybackState,
    pub tracks: Vec<Track>,
    pub toplists: Vec<Toplist>,
    pub search: SearchState,
    pub artist: ArtistState,
    pub album: AlbumState,
    pub view: View,
    pub lyrics_state: LyricsState,
    pub lyrics_open: bool,
    pub comments_state: CommentsState,
    pub comments_scroll: usize,
    pub comments_view_height: u16,
    /// Offset for the next page of regular comments.
    pub comments_next_offset: usize,
    /// Offset of an in-flight "load more" request.
    pub comments_pending_offset: Option<usize>,
    pub lyrics_offset_ms: i64,
    pub playlists: Vec<Playlist>,
    pub playlists_selection: usize,
    pub playlists_loading: bool,
    pub playlists_error: Option<String>,
    /// Playlist awaiting a `y`/`n` delete confirmation.
    pub playlist_confirm: Option<PlaylistId>,
    pub queue: Vec<Track>,
    /// Configured hooks, as reported by the daemon at startup.
    pub hooks: Vec<lekima_ipc::HookStatus>,
    /// A playlist write is in flight; the ack refreshes the sidebar.
    playlist_sync: bool,
    /// Mode to return to after a prompt that belongs to a pane.
    prompt_return: Option<Mode>,
    /// Finished plays, newest first, as returned by the daemon.
    pub plays: Vec<lekima_domain::PlayRecord>,
    pub plays_selection: usize,
    pub plays_loading: bool,
    pub plays_error: Option<String>,
    pub mode: Mode,
    pub focus: Focus,
    pub input: String,
    /// Byte cursor inside `input` for the prompt editor.
    pub cursor: usize,
    /// Offset into the help overlay; `draw_help` clamps it to the content.
    pub help_scroll: usize,
    /// Largest valid help offset, recomputed on every draw.
    pub help_max_scroll: usize,
    /// Visible body rows, used by half-page motion.
    pub main_view_height: u16,
    pub sidebar_view_height: u16,
    pub queue_view_height: u16,
    /// Queue entry awaiting a `y`/`n` confirmation.
    pub queue_confirm: Option<usize>,
    /// True while the queue selection follows the playing entry.
    pub queue_follow: bool,
    /// Count prefix (`5j`, `12G`); `None` when no digits are pending.
    pub pending_count: Option<u32>,
    /// True after `Ctrl-w`; the next key selects a pane.
    pub window_prefix: bool,
    search_history: Vec<String>,
    command_history: Vec<String>,
    history_cursor: Option<usize>,
    history_draft: String,
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
    /// Highest daemon event sequence applied; stale status replies are ignored.
    pub last_event_seq: u64,
    /// Whether the first daemon status has been folded into the view.
    bootstrapped: bool,
    history_back: Vec<HistoryEntry>,
    history_forward: Vec<HistoryEntry>,
    pub preferences: UiPreferences,
    pub available_devices: Vec<String>,
    pub settings_selection: usize,
    settings_dirty: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            status: PlaybackState::default(),
            tracks: Vec::new(),
            toplists: Vec::new(),
            search: SearchState::default(),
            artist: ArtistState::default(),
            album: AlbumState::default(),
            view: View::Tracks,
            lyrics_state: LyricsState::Idle,
            lyrics_open: false,
            comments_state: CommentsState::Idle,
            comments_scroll: 0,
            comments_view_height: 0,
            comments_next_offset: 0,
            comments_pending_offset: None,
            lyrics_offset_ms: 0,
            playlists: Vec::new(),
            playlists_selection: 0,
            playlists_loading: false,
            playlists_error: None,
            playlist_confirm: None,
            queue: Vec::new(),
            hooks: Vec::new(),
            playlist_sync: false,
            prompt_return: None,
            plays: Vec::new(),
            plays_selection: 0,
            plays_loading: false,
            plays_error: None,
            mode: Mode::Normal,
            focus: Focus::Main,
            input: String::new(),
            cursor: 0,
            help_scroll: 0,
            help_max_scroll: 0,
            main_view_height: 0,
            sidebar_view_height: 0,
            queue_view_height: 0,
            queue_confirm: None,
            queue_follow: true,
            pending_count: None,
            window_prefix: false,
            search_history: Vec::new(),
            command_history: Vec::new(),
            history_cursor: None,
            history_draft: String::new(),
            main_selection: 0,
            sidebar_selection: 0,
            queue_selection: 0,
            message: None,
            source: "search".to_owned(),
            user: None,
            status_inflight: false,
            daemon_down: false,
            frame: 0,
            last_event_seq: 0,
            bootstrapped: false,
            history_back: Vec::new(),
            history_forward: Vec::new(),
            preferences: UiPreferences::default(),
            available_devices: Vec::new(),
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
            search: self.search.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            source: self.source.clone(),
            selection: self.main_selection,
        }
    }

    fn restore_history(&mut self, entry: HistoryEntry) -> Action {
        self.view = entry.view;
        self.tracks = entry.tracks;
        self.toplists = entry.toplists;
        self.search = entry.search;
        self.artist = entry.artist;
        self.album = entry.album;
        self.source = entry.source;
        self.main_selection = entry.selection;
        self.focus = Focus::Main;

        // A page that left the screen mid-request has no data in the snapshot.
        // Re-issue the request instead of restoring a permanent spinner.
        match self.view {
            View::Artist => match self.artist.id {
                Some(id)
                    if self.artist.detail.is_none()
                        || (self.artist.loading && self.artist.albums.is_empty()) =>
                {
                    self.artist.loading = true;
                    self.artist.paging = false;
                    self.artist.error = None;
                    return Action::Call(Command::Artist { id });
                }
                _ => {
                    self.artist.loading = false;
                    self.artist.paging = false;
                }
            },
            View::Album => match self.album.id {
                Some(id) if self.album.detail.is_none() || self.album.loading => {
                    self.album.loading = true;
                    self.album.error = None;
                    return Action::Call(Command::Album { id });
                }
                _ => self.album.loading = false,
            },
            View::Search
                if self.search.loading
                    || (self.search.entries.is_empty() && !self.search.query.is_empty()) =>
            {
                self.search.loading = true;
                self.search.error = None;
                return Action::Call(Command::Search {
                    query: self.search.query.clone(),
                    kind: self.search.applied_filter.kind(),
                    limit: 40,
                    offset: 0,
                });
            }
            _ => {}
        }
        Action::None
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

    fn go_back(&mut self) -> Action {
        match self.history_back.pop() {
            Some(entry) => {
                self.history_forward.push(self.history_snapshot());
                self.restore_history(entry)
            }
            None => {
                self.notify("oldest view", false);
                Action::None
            }
        }
    }

    fn go_forward(&mut self) -> Action {
        match self.history_forward.pop() {
            Some(entry) => {
                self.history_back.push(self.history_snapshot());
                self.restore_history(entry)
            }
            None => {
                self.notify("newest view", false);
                Action::None
            }
        }
    }

    #[must_use]
    pub fn theme(&self) -> crate::theme::Theme {
        crate::theme::Theme::resolve(
            &self.preferences.theme,
            &self.preferences.glyphs,
            &self.preferences.density,
        )
        .with_motion(&self.preferences.motion)
    }

    pub fn set_preferences(&mut self, preferences: UiPreferences) {
        self.lyrics_offset_ms = preferences.lyrics_offset_ms;
        self.preferences = preferences;
    }

    pub fn set_devices(&mut self, devices: Vec<String>) {
        self.available_devices = devices;
    }

    /// Returns the preferences to persist when the TUI exits.
    #[must_use]
    pub fn changed_preferences(&self) -> Option<UiPreferences> {
        self.settings_dirty.then(|| self.preferences.clone())
    }

    fn cycle_setting(&mut self, delta: isize) -> Action {
        match self.settings_selection {
            0 => cycle_list(&crate::theme::THEMES, &mut self.preferences.theme, delta),
            1 => cycle_list(&GLYPHS, &mut self.preferences.glyphs, delta),
            2 => cycle_list(&DENSITIES, &mut self.preferences.density, delta),
            3 => {
                self.preferences.lyrics_translation = !self.preferences.lyrics_translation;
            }
            _ => {
                if self.available_devices.is_empty() {
                    self.notify("no output devices available", true);
                    return Action::None;
                }
                let mut options: Vec<Option<String>> = vec![None];
                options.extend(self.available_devices.iter().cloned().map(Some));
                let current_idx = self
                    .status
                    .output_device
                    .as_ref()
                    .and_then(|active| {
                        options
                            .iter()
                            .position(|opt| opt.as_deref() == Some(active.as_str()))
                    })
                    .unwrap_or(0);
                let len = options.len() as isize;
                let next_idx = ((current_idx as isize + delta).rem_euclid(len)) as usize;
                let next_device = options[next_idx].clone();
                return Action::Call(Command::SetAudioDevice {
                    device: next_device,
                });
            }
        }
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
                self.settings_selection =
                    (self.settings_selection + 1).min(SETTINGS_ROWS.saturating_sub(1));
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

    /// Records the newest daemon event sequence.
    pub fn note_event(&mut self, seq: u64) {
        self.last_event_seq = self.last_event_seq.max(seq);
    }

    #[must_use]
    pub fn sidebar_len(&self) -> usize {
        SIDEBAR_STATICS.len() + self.playlists.len()
    }

    fn view_len(&self) -> usize {
        match self.view {
            View::Tracks => self.tracks.len(),
            View::Toplists => self.toplists.len(),
            View::Search => self.search.entries.len(),
            View::Artist => self.artist.albums.len(),
            View::Album => self.album.tracks().len(),
        }
    }

    fn clamp_selection(&mut self) {
        let main = self.view_len();
        self.main_selection = self.main_selection.min(main.saturating_sub(1));
        self.sidebar_selection = self
            .sidebar_selection
            .min(self.sidebar_len().saturating_sub(1));
        self.playlists_selection = self
            .playlists_selection
            .min(self.playlists.len().saturating_sub(1));
        self.queue_selection = self.queue_selection.min(self.queue.len().saturating_sub(1));
    }

    fn selected_track(&self) -> Option<&Track> {
        match self.view {
            View::Tracks => self.tracks.get(self.main_selection),
            View::Search => match self.search.entries.get(self.main_selection) {
                Some(SearchEntry::Track(track)) => Some(track),
                _ => None,
            },
            View::Album => self.album.tracks().get(self.main_selection),
            View::Artist | View::Toplists => None,
        }
    }

    /// Handles a key press and returns what the loop should do.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.kind != KeyEventKind::Press {
            return Action::None;
        }
        match self.mode {
            Mode::Help => {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q' | '?') => {
                        self.mode = Mode::Normal;
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.help_scroll = (self.help_scroll + 1).min(self.help_max_scroll);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.help_scroll = self.help_scroll.saturating_sub(1);
                    }
                    KeyCode::PageDown => {
                        self.help_scroll = (self.help_scroll + 10).min(self.help_max_scroll);
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.help_scroll = (self.help_scroll + 10).min(self.help_max_scroll);
                    }
                    KeyCode::PageUp => {
                        self.help_scroll = self.help_scroll.saturating_sub(10);
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.help_scroll = self.help_scroll.saturating_sub(10);
                    }
                    KeyCode::Char('g') => self.help_scroll = 0,
                    KeyCode::Char('G') => self.help_scroll = self.help_max_scroll,
                    _ => {}
                }
                Action::None
            }
            Mode::Queue => self.handle_queue_key(key),
            Mode::Comments => self.handle_comments_key(key),
            Mode::History => self.handle_history_key(key),
            Mode::Playlists => self.handle_playlists_key(key),
            Mode::Search | Mode::Command => self.handle_prompt_key(key),
            Mode::Settings => self.handle_settings_key(key),
            Mode::Normal => self.handle_normal_key(key),
        }
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.mode == Mode::Search {
            match key.code {
                KeyCode::Tab => {
                    self.search.filter = self.search.filter.shifted(1);
                    return Action::None;
                }
                KeyCode::BackTab => {
                    self.search.filter = self.search.filter.shifted(-1);
                    return Action::None;
                }
                KeyCode::Char(digit @ '1'..='5') if ctrl => {
                    if let Some(filter) = SearchFilter::from_index(digit as usize - '1' as usize) {
                        self.search.filter = filter;
                    }
                    return Action::None;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Esc => {
                if self.mode == Mode::Search {
                    self.search.filter = self.search.applied_filter;
                }
                self.mode = self.prompt_return.take().unwrap_or(Mode::Normal);
                self.input.clear();
                self.cursor = 0;
                self.history_cursor = None;
                Action::None
            }
            KeyCode::Enter => {
                let input = std::mem::take(&mut self.input);
                let mode = self.mode;
                self.cursor = 0;
                self.history_cursor = None;
                self.mode = self.prompt_return.take().unwrap_or(Mode::Normal);
                if input.trim().is_empty() {
                    return Action::None;
                }
                self.remember_prompt(mode, input.trim());
                match mode {
                    Mode::Search => {
                        let query = input.trim().to_owned();
                        self.remember();
                        self.search.query.clone_from(&query);
                        self.search.applied_filter = self.search.filter;
                        self.search.loading = true;
                        self.search.error = None;
                        self.search.results = None;
                        self.search.entries.clear();
                        self.view = View::Search;
                        self.main_selection = 0;
                        self.source = format!("search: {query}");
                        Action::Call(Command::Search {
                            query,
                            kind: self.search.filter.kind(),
                            limit: 40,
                            offset: 0,
                        })
                    }
                    _ => self.run_command(input.trim()),
                }
            }
            KeyCode::Backspace => {
                self.delete_before_cursor();
                Action::None
            }
            KeyCode::Delete => {
                self.delete_at_cursor();
                Action::None
            }
            KeyCode::Left => {
                self.cursor = prev_boundary(&self.input, self.cursor);
                Action::None
            }
            KeyCode::Right => {
                self.cursor = next_boundary(&self.input, self.cursor);
                Action::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                Action::None
            }
            KeyCode::End => {
                self.cursor = self.input.len();
                Action::None
            }
            KeyCode::Up => {
                self.history_up();
                Action::None
            }
            KeyCode::Down => {
                self.history_down();
                Action::None
            }
            KeyCode::Char('w') if ctrl => {
                self.delete_word_before_cursor();
                Action::None
            }
            KeyCode::Char('u') if ctrl => {
                self.input.drain(..self.cursor);
                self.cursor = 0;
                Action::None
            }
            KeyCode::Char(character) if !ctrl => {
                self.input.insert(self.cursor, character);
                self.cursor += character.len_utf8();
                Action::None
            }
            _ => Action::None,
        }
    }

    fn delete_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = prev_boundary(&self.input, self.cursor);
        self.input.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    fn delete_at_cursor(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        let end = next_boundary(&self.input, self.cursor);
        self.input.replace_range(self.cursor..end, "");
    }

    fn delete_word_before_cursor(&mut self) {
        let mut start = self.cursor;
        while start > 0 {
            let character = self.input[..start].chars().next_back();
            if !character.is_some_and(char::is_whitespace) {
                break;
            }
            start = prev_boundary(&self.input, start);
        }
        while start > 0 {
            let character = self.input[..start].chars().next_back();
            if character.is_some_and(char::is_whitespace) {
                break;
            }
            start = prev_boundary(&self.input, start);
        }
        self.input.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    fn history(&self, mode: Mode) -> &[String] {
        match mode {
            Mode::Search => &self.search_history,
            _ => &self.command_history,
        }
    }

    fn remember_prompt(&mut self, mode: Mode, text: &str) {
        let history = match mode {
            Mode::Search => &mut self.search_history,
            _ => &mut self.command_history,
        };
        if history.last().is_none_or(|entry| entry != text) {
            history.push(text.to_owned());
            if history.len() > HISTORY_LIMIT {
                history.remove(0);
            }
        }
    }

    fn history_up(&mut self) {
        let (len, last) = {
            let history = self.history(self.mode);
            (history.len(), history.last().cloned())
        };
        if len == 0 {
            return;
        }
        let index = match self.history_cursor {
            None => {
                self.history_draft.clone_from(&self.input);
                len - 1
            }
            Some(index) => index.saturating_sub(1),
        };
        self.history_cursor = Some(index);
        let entry = {
            let history = self.history(self.mode);
            history.get(index).cloned().or(last)
        };
        if let Some(entry) = entry {
            self.input = entry;
        }
        self.cursor = self.input.len();
    }

    fn history_down(&mut self) {
        let next = {
            let history = self.history(self.mode);
            match self.history_cursor {
                Some(index) if index + 1 < history.len() => Some(history[index + 1].clone()),
                Some(_) => None,
                None => return,
            }
        };
        match next {
            Some(entry) => {
                if let Some(index) = self.history_cursor {
                    self.history_cursor = Some(index + 1);
                }
                self.input = entry;
            }
            None => {
                self.history_cursor = None;
                self.input.clone_from(&self.history_draft);
            }
        }
        self.cursor = self.input.len();
    }

    /// Re-runs the current search with a different catalogue kind.
    ///
    /// Used from the results view, where the prompt (and its rail) is closed.
    fn refilter(&mut self, filter: SearchFilter) -> Action {
        self.search.filter = filter;
        if self.search.query.is_empty() {
            return Action::None;
        }
        self.search.applied_filter = filter;
        self.search.loading = true;
        self.search.error = None;
        self.search.results = None;
        self.search.entries.clear();
        self.main_selection = 0;
        self.source = format!("search: {}", self.search.query);
        Action::Call(Command::Search {
            query: self.search.query.clone(),
            kind: filter.kind(),
            limit: 40,
            offset: 0,
        })
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
            "search" if !rest.trim().is_empty() => {
                let query = rest.trim().to_owned();
                self.remember();
                self.search.query.clone_from(&query);
                self.search.applied_filter = self.search.filter;
                self.search.loading = true;
                self.search.error = None;
                self.search.results = None;
                self.search.entries.clear();
                self.view = View::Search;
                self.main_selection = 0;
                self.source = format!("search: {query}");
                Action::Call(Command::Search {
                    query,
                    kind: self.search.applied_filter.kind(),
                    limit: 40,
                    offset: 0,
                })
            }
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
            "album" => self.album_command(rest.trim()),
            "playlist" => self.playlist_command(rest.trim()),
            "artist" => self.artist_command(rest.trim()),
            "sleep" => match parse_sleep_minutes(rest.trim()) {
                Ok(minutes) => Action::Call(Command::Sleep { minutes }),
                Err(message) => {
                    self.notify(message, true);
                    Action::None
                }
            },
            "quality" => match rest.trim() {
                "standard" => Action::Call(Command::SetQuality {
                    quality: lekima_domain::Quality::Standard,
                }),
                "higher" => Action::Call(Command::SetQuality {
                    quality: lekima_domain::Quality::Higher,
                }),
                "exhigh" => Action::Call(Command::SetQuality {
                    quality: lekima_domain::Quality::Exhigh,
                }),
                "lossless" => Action::Call(Command::SetQuality {
                    quality: lekima_domain::Quality::Lossless,
                }),
                other => {
                    self.notify(
                        format!("usage: :quality standard|higher|exhigh|lossless (got `{other}`)"),
                        true,
                    );
                    Action::None
                }
            },
            "device" => {
                let target = rest.trim();
                if target.is_empty() {
                    let current = self.status.output_device.as_deref().unwrap_or("default");
                    self.notify(
                        format!("output: {}", lekima_domain::sanitize_line(current)),
                        false,
                    );
                    Action::None
                } else if target == "default" {
                    Action::Call(Command::SetAudioDevice { device: None })
                } else {
                    Action::Call(Command::SetAudioDevice {
                        device: Some(target.to_owned()),
                    })
                }
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
            "translate" => {
                self.preferences.lyrics_translation = !self.preferences.lyrics_translation;
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

    fn current_album_id(&self) -> Option<AlbumId> {
        if self.view == View::Album {
            return self.album.id.filter(|id| id.get() > 0);
        }
        self.status
            .track
            .as_ref()
            .and_then(|track| track.album.as_ref())
            .map(|album| album.id)
            .filter(|id| id.get() > 0)
    }

    fn current_artist_id(&self) -> Option<ArtistId> {
        if self.view == View::Artist {
            return self.artist.id.filter(|id| id.get() > 0);
        }
        self.status
            .track
            .as_ref()
            .and_then(|track| track.artists.first())
            .map(|artist| artist.id)
            .filter(|id| id.get() > 0)
    }

    /// `:album`, `:album play`, `:album queue`, `:album <id>`.
    /// `:playlist create NAME`, `:playlist add ID`, `:playlist delete ID yes`.
    fn playlist_command(&mut self, spec: &str) -> Action {
        let (sub, rest) = spec.split_once(' ').unwrap_or((spec, ""));
        match sub {
            "create" if !rest.trim().is_empty() => Action::Call(Command::PlaylistCreate {
                name: rest.trim().to_owned(),
                description: String::new(),
                private: false,
            }),
            "add" => {
                let Ok(id) = rest.trim().parse::<u64>() else {
                    self.notify("usage: :playlist add <playlist-id>", true);
                    return Action::None;
                };
                let Some(track) = self.status.track.as_ref().map(|track| track.id) else {
                    self.notify("nothing playing", true);
                    return Action::None;
                };
                self.playlist_sync = true;
                Action::Call(Command::PlaylistAdd {
                    id: id.into(),
                    tracks: vec![track],
                })
            }
            "delete" => {
                let mut parts = rest.split_whitespace();
                let Some(id) = parts.next().and_then(|value| value.parse::<u64>().ok()) else {
                    self.notify("usage: :playlist delete <playlist-id> yes", true);
                    return Action::None;
                };
                if parts.next() != Some("yes") {
                    self.notify("confirm with :playlist delete <id> yes", true);
                    return Action::None;
                }
                self.playlist_sync = true;
                Action::Call(Command::PlaylistDelete { id: id.into() })
            }
            "rename" => {
                let (id, name) = rest.split_once(' ').unwrap_or((rest, ""));
                let Ok(id) = id.trim().parse::<u64>() else {
                    self.notify("usage: :playlist rename <id> <name>", true);
                    return Action::None;
                };
                if name.trim().is_empty() {
                    self.notify("playlist name cannot be empty", true);
                    return Action::None;
                }
                self.playlist_sync = true;
                Action::Call(Command::PlaylistUpdate {
                    id: id.into(),
                    name: Some(name.trim().to_owned()),
                    description: None,
                })
            }
            "describe" => {
                let (id, description) = rest.split_once(' ').unwrap_or((rest, ""));
                let Ok(id) = id.trim().parse::<u64>() else {
                    self.notify("usage: :playlist describe <id> <text>", true);
                    return Action::None;
                };
                self.playlist_sync = true;
                Action::Call(Command::PlaylistUpdate {
                    id: id.into(),
                    name: None,
                    description: Some(description.trim().to_owned()),
                })
            }
            _ => {
                self.notify(
                    "usage: :playlist create NAME · rename ID NAME · describe ID TEXT · add ID · delete ID yes",
                    true,
                );
                Action::None
            }
        }
    }

    fn album_command(&mut self, spec: &str) -> Action {
        match spec {
            "play" | "queue" => {
                let Some(id) = self.current_album_id() else {
                    self.notify("no album for the current track", true);
                    return Action::None;
                };
                if spec == "play" {
                    Action::Call(Command::Play {
                        target: PlayTarget::Album { id },
                    })
                } else {
                    self.notify("queued album", false);
                    Action::Call(Command::QueueAlbum { id, next: false })
                }
            }
            "" => match self.current_album_id() {
                Some(id) => self.open_album(id),
                None => {
                    self.notify("nothing playing has an album", true);
                    Action::None
                }
            },
            other => match other.parse::<u64>() {
                Ok(id) if id > 0 => self.open_album(AlbumId(id)),
                _ => {
                    self.notify("usage: :album [play|queue|<id>]", true);
                    Action::None
                }
            },
        }
    }

    /// `:artist` or `:artist <id>`.
    fn artist_command(&mut self, spec: &str) -> Action {
        if spec.is_empty() {
            return match self.current_artist_id() {
                Some(id) => self.open_artist(id),
                None => {
                    self.notify("nothing playing has an artist", true);
                    Action::None
                }
            };
        }
        match spec.parse::<u64>() {
            Ok(id) if id > 0 => self.open_artist(ArtistId(id)),
            _ => {
                self.notify("usage: :artist [<id>]", true);
                Action::None
            }
        }
    }

    fn handle_queue_key(&mut self, key: KeyEvent) -> Action {
        if let Some(index) = self.queue_confirm {
            return match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.queue_confirm = None;
                    self.notify(format!("removed queue entry {:03}", index + 1), false);
                    Action::Call(Command::QueueRemove { index })
                }
                KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                    self.queue_confirm = None;
                    Action::None
                }
                _ => Action::None,
            };
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'Q') => {
                self.mode = Mode::Normal;
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
            KeyCode::Char('h') | KeyCode::Left => Action::Call(Command::Seek {
                position_ms: self.status.position_ms.saturating_sub(10_000),
            }),
            KeyCode::Char('l') | KeyCode::Right => Action::Call(Command::Seek {
                position_ms: self.status.position_ms.saturating_add(10_000),
            }),
            KeyCode::Char('L') => self.toggle_lyrics(),
            KeyCode::Char('j') | KeyCode::Down => {
                self.queue_follow = false;
                self.queue_selection =
                    (self.queue_selection + 1).min(self.queue.len().saturating_sub(1));
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.queue_follow = false;
                self.queue_selection = self.queue_selection.saturating_sub(1);
                Action::None
            }
            KeyCode::Char('g') => {
                self.queue_follow = false;
                self.queue_selection = 0;
                Action::None
            }
            KeyCode::Char('G') => {
                self.queue_follow = false;
                self.queue_selection = self.queue.len().saturating_sub(1);
                Action::None
            }
            KeyCode::Char('x') => {
                self.queue_follow = false;
                if self.queue.is_empty() {
                    self.notify("queue is empty", true);
                } else {
                    self.queue_confirm = Some(self.queue_selection);
                }
                Action::None
            }
            KeyCode::Char('J') => {
                self.queue_follow = false;
                if self.queue_selection + 1 < self.queue.len() {
                    let from = self.queue_selection;
                    self.queue_selection += 1;
                    Action::Call(Command::QueueMove {
                        from,
                        to: self.queue_selection,
                    })
                } else {
                    Action::None
                }
            }
            KeyCode::Char('K') => {
                self.queue_follow = false;
                if self.queue_selection > 0 {
                    let from = self.queue_selection;
                    self.queue_selection -= 1;
                    Action::Call(Command::QueueMove {
                        from,
                        to: self.queue_selection,
                    })
                } else {
                    Action::None
                }
            }
            KeyCode::Enter => {
                self.queue_follow = true;
                Action::Call(Command::Jump {
                    index: self.queue_selection,
                })
            }
            _ => Action::None,
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // In the results view the rail is gone, so the kind stays changeable
        // from here. Tab keeps cycling focus.
        if ctrl
            && self.view == View::Search
            && let KeyCode::Char(digit @ '1'..='5') = key.code
            && let Some(filter) = SearchFilter::from_index(digit as usize - '1' as usize)
        {
            return self.refilter(filter);
        }
        if self.window_prefix {
            self.window_prefix = false;
            self.pending_count = None;
            return match key.code {
                KeyCode::Char('1') => {
                    self.focus = Focus::Sidebar;
                    Action::None
                }
                KeyCode::Char('2') => {
                    self.focus = Focus::Main;
                    Action::None
                }
                KeyCode::Char('3') => {
                    self.mode = Mode::Queue;
                    self.queue_confirm = None;
                    self.queue_follow = true;
                    self.queue_selection = self.status.queue_index.unwrap_or(0);
                    Action::Call(Command::Queue)
                }
                KeyCode::Char('4') => self.toggle_lyrics(),
                _ => Action::None,
            };
        }
        if ctrl && matches!(key.code, KeyCode::Char('w')) {
            self.window_prefix = true;
            self.pending_count = None;
            return Action::None;
        }
        if let KeyCode::Char(character) = key.code
            && !ctrl
            && character.is_ascii_digit()
        {
            if character == '0' && self.pending_count.is_none() {
                return Action::None;
            }
            let digit = character.to_digit(10).unwrap_or(0);
            let next = self
                .pending_count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(digit)
                .min(9_999);
            if next > 0 {
                self.pending_count = Some(next);
            }
            return Action::None;
        }
        let count = self.pending_count.take();
        let repeat = count.unwrap_or(1).min(10_000) as isize;
        match key.code {
            KeyCode::Char('o') if ctrl => self.go_back(),
            KeyCode::Char('i') if ctrl => self.go_forward(),
            KeyCode::Char('c') if ctrl => Action::Quit,
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                self.help_scroll = 0;
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
                self.cursor = 0;
                self.history_cursor = None;
                Action::None
            }
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.input.clear();
                self.cursor = 0;
                self.history_cursor = None;
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
                self.queue_confirm = None;
                self.queue_follow = true;
                self.queue_selection = self.status.queue_index.unwrap_or(0);
                Action::Call(Command::Queue)
            }
            KeyCode::Char('P') => self.open_playlists(),
            KeyCode::Char('F') => Action::Call(Command::Fm {
                action: lekima_ipc::FmAction::Start,
            }),
            KeyCode::Char('t') => {
                if self.lyrics_open {
                    self.preferences.lyrics_translation = !self.preferences.lyrics_translation;
                    self.settings_dirty = true;
                    Action::UiChanged
                } else {
                    self.remember();
                    Action::Call(Command::Toplists)
                }
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
            KeyCode::Char('C') => self.toggle_comments(),
            KeyCode::Char('H') => self.toggle_history(),
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
            KeyCode::Char('n') if self.view == View::Artist && self.artist.has_more => {
                self.artist_next_page()
            }
            KeyCode::Char('n') => Action::Call(Command::Next),
            KeyCode::Char('p') => Action::Call(Command::Prev),
            KeyCode::Char('m') => Action::Call(Command::ToggleMute),
            KeyCode::Esc if matches!(self.view, View::Search | View::Artist | View::Album) => {
                self.go_back()
            }
            KeyCode::Char('r') if self.view == View::Search => self.retry_search(),
            KeyCode::Char('r') if matches!(self.view, View::Artist | View::Album) => {
                self.refresh_page()
            }
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
                self.move_selection(repeat);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_selection(-repeat);
                Action::None
            }
            KeyCode::Char('d') if ctrl => {
                let height = if self.focus == Focus::Sidebar {
                    self.sidebar_view_height
                } else {
                    self.main_view_height
                };
                let page = i64::from(height.max(2) / 2).max(1);
                let delta = page.saturating_mul(i64::from(count.unwrap_or(1).min(1_000)));
                self.move_selection(delta.min(1_000_000) as isize);
                Action::None
            }
            KeyCode::Char('u') if ctrl => {
                let height = if self.focus == Focus::Sidebar {
                    self.sidebar_view_height
                } else {
                    self.main_view_height
                };
                let page = i64::from(height.max(2) / 2).max(1);
                let delta = page.saturating_mul(i64::from(count.unwrap_or(1).min(1_000)));
                self.move_selection(-(delta.min(1_000_000) as isize));
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
                if let Some(line) = count.filter(|line| *line > 0) {
                    self.set_selection(line.saturating_sub(1) as usize);
                } else {
                    self.set_selection(usize::MAX);
                }
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
                let len = self.view_len();
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
                let len = self.view_len();
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
                    self.play_ids(ids)
                }
                View::Album => {
                    let ids: Vec<_> = self
                        .album
                        .tracks()
                        .iter()
                        .skip(self.main_selection)
                        .map(|track| track.id)
                        .collect();
                    self.play_ids(ids)
                }
                View::Search => match self.search.entries.get(self.main_selection).cloned() {
                    Some(SearchEntry::Track(track)) => self.play_ids(vec![track.id]),
                    Some(SearchEntry::Album(album)) => self.open_album(album.id),
                    Some(SearchEntry::Artist(artist)) => self.open_artist(artist.id),
                    Some(SearchEntry::Playlist(playlist)) => {
                        let (id, name) = (playlist.id, playlist.name.clone());
                        self.remember();
                        self.source = format!("playlist: {name}");
                        Action::Call(Command::PlaylistTracks { id })
                    }
                    None => Action::None,
                },
                View::Artist => match self.artist.albums.get(self.main_selection) {
                    Some(album) => self.open_album(album.id),
                    None => Action::None,
                },
            },
        }
    }

    fn play_ids(&mut self, ids: Vec<TrackId>) -> Action {
        if ids.is_empty() {
            self.notify("nothing selected", true);
            Action::None
        } else {
            Action::Call(Command::Play {
                target: PlayTarget::Tracks { ids },
            })
        }
    }

    /// Opens an album page and asks the daemon for its tracks.
    fn open_album(&mut self, id: AlbumId) -> Action {
        self.remember();
        let name = self
            .search
            .entries
            .iter()
            .find_map(|entry| match entry {
                SearchEntry::Album(album) if album.id == id => Some(album.name.clone()),
                _ => None,
            })
            .or_else(|| {
                self.artist
                    .albums
                    .iter()
                    .find(|album| album.id == id)
                    .map(|album| album.name.clone())
            });
        self.album = AlbumState {
            id: Some(id),
            loading: true,
            ..AlbumState::default()
        };
        self.view = View::Album;
        self.source = name.map_or_else(|| format!("album {id}"), |name| format!("album: {name}"));
        self.main_selection = 0;
        self.focus = Focus::Main;
        Action::Call(Command::Album { id })
    }

    /// Opens an artist page and asks the daemon for the profile and first page.
    fn open_artist(&mut self, id: ArtistId) -> Action {
        self.remember();
        let name = self
            .search
            .entries
            .iter()
            .find_map(|entry| match entry {
                SearchEntry::Artist(artist) if artist.id == id => Some(artist.name.clone()),
                _ => None,
            })
            .or_else(|| {
                self.status.track.as_ref().and_then(|track| {
                    track
                        .artists
                        .iter()
                        .find(|artist| artist.id == id)
                        .map(|artist| artist.name.clone())
                })
            });
        self.artist = ArtistState {
            id: Some(id),
            loading: true,
            ..ArtistState::default()
        };
        self.view = View::Artist;
        self.source = name.map_or_else(|| format!("artist {id}"), |name| format!("artist: {name}"));
        self.main_selection = 0;
        self.focus = Focus::Main;
        Action::Call(Command::Artist { id })
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

    fn toggle_comments(&mut self) -> Action {
        if self.mode == Mode::Comments {
            self.mode = Mode::Normal;
            return Action::None;
        }
        let Some(track) = self.status.track.as_ref() else {
            self.notify("nothing playing", true);
            return Action::None;
        };
        let id = track.id;
        self.mode = Mode::Comments;
        self.comments_scroll = 0;
        self.comments_next_offset = 0;
        self.comments_pending_offset = None;
        self.comments_state = CommentsState::Loading { track: id };
        Action::Call(Command::Comments {
            id: Some(id),
            limit: 20,
            offset: 0,
        })
    }

    fn toggle_history(&mut self) -> Action {
        if self.mode == Mode::History {
            self.mode = Mode::Normal;
            return Action::None;
        }
        self.mode = Mode::History;
        self.plays_selection = 0;
        self.plays_loading = true;
        self.plays_error = None;
        Action::Call(Command::History { limit: Some(200) })
    }

    fn open_playlists(&mut self) -> Action {
        self.mode = Mode::Playlists;
        self.playlist_confirm = None;
        self.playlists_loading = true;
        self.playlists_error = None;
        self.playlists_selection = self
            .playlists_selection
            .min(self.playlists.len().saturating_sub(1));
        Action::Call(Command::Playlists)
    }

    /// Opens the command line with a prefix so the pane reuses the editor.
    fn begin_command(&mut self, text: String) -> Action {
        self.prompt_return = Some(Mode::Playlists);
        self.mode = Mode::Command;
        self.input = text;
        self.cursor = self.input.len();
        self.history_cursor = None;
        Action::None
    }

    fn selected_playlist(&self) -> Option<&Playlist> {
        self.playlists.get(self.playlists_selection)
    }

    fn open_playlist_selection(&mut self) -> Action {
        let Some((id, name)) = self
            .selected_playlist()
            .map(|playlist| (playlist.id, playlist.name.clone()))
        else {
            self.notify("no playlist selected", true);
            return Action::None;
        };
        self.remember();
        self.source = format!("playlist: {name}");
        self.mode = Mode::Normal;
        Action::Call(Command::PlaylistTracks { id })
    }

    fn add_current_track_to_playlist(&mut self) -> Action {
        let Some(id) = self.selected_playlist().map(|playlist| playlist.id) else {
            self.notify("no playlist selected", true);
            return Action::None;
        };
        let Some(track) = self.status.track.as_ref().map(|track| track.id) else {
            self.notify("nothing playing", true);
            return Action::None;
        };
        self.playlist_sync = true;
        Action::Call(Command::PlaylistAdd {
            id,
            tracks: vec![track],
        })
    }

    fn handle_playlists_key(&mut self, key: KeyEvent) -> Action {
        if let Some(id) = self.playlist_confirm {
            return match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.playlist_confirm = None;
                    self.playlist_sync = true;
                    Action::Call(Command::PlaylistDelete { id })
                }
                KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                    self.playlist_confirm = None;
                    Action::None
                }
                // Closing the pane cancels as well; never leave a trap.
                KeyCode::Char('P' | 'q') => {
                    self.playlist_confirm = None;
                    self.mode = Mode::Normal;
                    Action::None
                }
                _ => Action::None,
            };
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let last = self.playlists.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'P') => {
                self.mode = Mode::Normal;
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.playlists_selection = (self.playlists_selection + 1).min(last);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.playlists_selection = self.playlists_selection.saturating_sub(1);
                Action::None
            }
            KeyCode::Char('g') => {
                self.playlists_selection = 0;
                Action::None
            }
            KeyCode::Char('G') => {
                self.playlists_selection = last;
                Action::None
            }
            KeyCode::Char('d') if ctrl => {
                self.playlists_selection = (self.playlists_selection + 10).min(last);
                Action::None
            }
            KeyCode::Char('u') if ctrl => {
                self.playlists_selection = self.playlists_selection.saturating_sub(10);
                Action::None
            }
            KeyCode::Enter => self.open_playlist_selection(),
            KeyCode::Char('n') => self.begin_command("playlist create ".to_owned()),
            KeyCode::Char('R') => match self.selected_playlist() {
                Some(playlist) => {
                    let id = playlist.id;
                    self.begin_command(format!("playlist rename {id} "))
                }
                None => {
                    self.notify("no playlist selected", true);
                    Action::None
                }
            },
            KeyCode::Char('d') => match self.selected_playlist() {
                Some(playlist) => {
                    let id = playlist.id;
                    self.begin_command(format!("playlist describe {id} "))
                }
                None => {
                    self.notify("no playlist selected", true);
                    Action::None
                }
            },
            KeyCode::Char('a') => self.add_current_track_to_playlist(),
            KeyCode::Char('x') => {
                if let Some(id) = self.selected_playlist().map(|playlist| playlist.id) {
                    self.playlist_confirm = Some(id);
                } else {
                    self.notify("no playlist selected", true);
                }
                Action::None
            }
            KeyCode::Char('r') => {
                self.playlists_loading = true;
                self.playlists_error = None;
                Action::Call(Command::Playlists)
            }
            _ => Action::None,
        }
    }

    fn handle_history_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let last = self.plays.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'H') => {
                self.mode = Mode::Normal;
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.plays_selection = (self.plays_selection + 1).min(last);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.plays_selection = self.plays_selection.saturating_sub(1);
                Action::None
            }
            KeyCode::Char('g') => {
                self.plays_selection = 0;
                Action::None
            }
            KeyCode::Char('G') => {
                self.plays_selection = last;
                Action::None
            }
            KeyCode::Char('d') if ctrl => {
                self.plays_selection = (self.plays_selection + 10).min(last);
                Action::None
            }
            KeyCode::Char('u') if ctrl => {
                self.plays_selection = self.plays_selection.saturating_sub(10);
                Action::None
            }
            KeyCode::Char('r') => {
                self.plays_loading = true;
                self.plays_error = None;
                Action::Call(Command::History { limit: Some(200) })
            }
            KeyCode::Char(' ') => Action::Call(Command::Toggle),
            KeyCode::Enter => self.replay_history_selection(),
            _ => Action::None,
        }
    }

    /// Plays the track under the history cursor.
    fn replay_history_selection(&mut self) -> Action {
        let Some(id) = self
            .plays
            .get(self.plays_selection)
            .map(|entry| entry.track.id)
        else {
            self.notify("no plays yet", true);
            return Action::None;
        };
        let label = self
            .plays
            .get(self.plays_selection)
            .map_or_else(String::new, |entry| sanitize_line(&entry.track.name));
        self.notify(format!("playing {label}"), false);
        self.mode = Mode::Normal;
        Action::Call(Command::Play {
            target: PlayTarget::Tracks { ids: vec![id] },
        })
    }

    fn handle_comments_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc | KeyCode::Char('C' | 'q') => {
                self.mode = Mode::Normal;
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.comments_scroll = self.comments_scroll.saturating_add(1);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.comments_scroll = self.comments_scroll.saturating_sub(1);
                Action::None
            }
            KeyCode::Char('d') if ctrl => {
                self.comments_scroll = self.comments_scroll.saturating_add(10);
                Action::None
            }
            KeyCode::Char('u') if ctrl => {
                self.comments_scroll = self.comments_scroll.saturating_sub(10);
                Action::None
            }
            KeyCode::Char('g') => {
                self.comments_scroll = 0;
                Action::None
            }
            KeyCode::Char('r') => {
                let Some(id) = self.status.track.as_ref().map(|track| track.id) else {
                    return Action::None;
                };
                self.comments_scroll = 0;
                self.comments_next_offset = 0;
                self.comments_pending_offset = None;
                self.comments_state = CommentsState::Loading { track: id };
                Action::Call(Command::Comments {
                    id: Some(id),
                    limit: 20,
                    offset: 0,
                })
            }
            KeyCode::Char('n') => {
                if self.comments_pending_offset.is_some() {
                    return Action::None;
                }
                let Some(id) = self.status.track.as_ref().map(|track| track.id) else {
                    return Action::None;
                };
                let has_more = self
                    .comments_state
                    .ready()
                    .is_some_and(|comments| comments.has_more);
                if !has_more {
                    return Action::None;
                }
                let offset = self.comments_next_offset;
                self.comments_pending_offset = Some(offset);
                Action::Call(Command::Comments {
                    id: Some(id),
                    limit: 20,
                    offset: u32::try_from(offset).unwrap_or(u32::MAX),
                })
            }
            _ => Action::None,
        }
    }

    /// Re-runs the last search, keeping its query and filter.
    fn retry_search(&mut self) -> Action {
        if self.search.query.is_empty() {
            return Action::None;
        }
        self.search.error = None;
        Action::Call(Command::Search {
            query: self.search.query.clone(),
            kind: self.search.filter.kind(),
            limit: 40,
            offset: 0,
        })
    }

    fn refresh_page(&mut self) -> Action {
        match self.view {
            View::Artist => {
                let Some(id) = self.artist.id else {
                    return Action::None;
                };
                self.artist.loading = true;
                self.artist.error = None;
                Action::Call(Command::ArtistAlbums {
                    id,
                    limit: 20,
                    offset: 0,
                })
            }
            View::Album => {
                let Some(id) = self.album.id else {
                    return Action::None;
                };
                self.album.loading = true;
                self.album.error = None;
                Action::Call(Command::Album { id })
            }
            _ => Action::None,
        }
    }

    /// Loads the next page of an artist's albums (`n` while `has_more`).
    fn artist_next_page(&mut self) -> Action {
        let Some(id) = self.artist.id else {
            return Action::None;
        };
        if !self.artist.has_more || self.artist.paging {
            return Action::None;
        }
        self.artist.paging = true;
        Action::Call(Command::ArtistAlbums {
            id,
            limit: 20,
            offset: u32::try_from(self.artist.albums.len()).unwrap_or(u32::MAX),
        })
    }

    fn enqueue_selected(&mut self, next: bool) -> Action {
        if self.focus == Focus::Sidebar {
            self.notify("select a track to queue", true);
            return Action::None;
        }
        match self.view {
            View::Search => match self.search.entries.get(self.main_selection).cloned() {
                Some(SearchEntry::Track(track)) => self.enqueue_tracks(vec![track.id], next),
                Some(SearchEntry::Album(album)) => self.enqueue_album(album.id, next),
                Some(SearchEntry::Playlist(playlist)) => {
                    self.notify("queued playlist", false);
                    Action::Call(Command::QueuePlaylist {
                        id: playlist.id,
                        next,
                    })
                }
                Some(SearchEntry::Artist(artist)) => {
                    self.notify(format!("open {} to queue an album", artist.name), false);
                    Action::None
                }
                None => Action::None,
            },
            View::Artist => match self.artist.albums.get(self.main_selection).map(|a| a.id) {
                Some(id) => self.enqueue_album(id, next),
                None => {
                    self.notify("no album selected", true);
                    Action::None
                }
            },
            _ => match self.selected_track().map(|track| track.id) {
                Some(id) => self.enqueue_tracks(vec![id], next),
                None => Action::None,
            },
        }
    }

    fn enqueue_tracks(&mut self, ids: Vec<TrackId>, next: bool) -> Action {
        self.notify(
            if next {
                "queued to play next"
            } else {
                "queued"
            },
            false,
        );
        Action::Call(Command::Enqueue { tracks: ids, next })
    }

    fn enqueue_album(&mut self, id: AlbumId, next: bool) -> Action {
        self.notify(
            if next {
                "album queued to play next"
            } else {
                "album queued"
            },
            false,
        );
        Action::Call(Command::QueueAlbum { id, next })
    }

    /// Folds a daemon reply into the view state. Returns a follow-up command.
    pub fn apply(&mut self, result: Result<Reply, IpcError>) -> Action {
        self.status_inflight = false;
        let was_down = self.daemon_down;
        match result {
            Ok(reply) => {
                self.daemon_down = false;
                match reply {
                    Reply::Status { state } => {
                        if state.revision != 0 && state.revision < self.last_event_seq {
                            return Action::None;
                        }
                        let previous = self.status.track.as_ref().map(|track| track.id);
                        let previous_device = self.status.output_device.clone();
                        let previous_status = self.status.status;
                        let current_index = state.queue_index;
                        self.status = state;
                        let first = !self.bootstrapped;
                        self.bootstrapped = true;
                        if was_down {
                            self.notify(
                                if self.status.queue_length > 0 {
                                    "daemon restarted; queue restored; space to resume"
                                } else {
                                    "daemon restarted"
                                },
                                false,
                            );
                        } else if first
                            && self.status.queue_length > 0
                            && self.status.status == lekima_domain::PlaybackStatus::Stopped
                        {
                            self.notify("queue restored; space to resume", false);
                        } else if !first {
                            if previous_device.is_some()
                                && self.status.output_device.is_none()
                                && previous_status == PlaybackStatus::Playing
                                && self.status.status == PlaybackStatus::Paused
                            {
                                self.notify("output device lost; playback paused", true);
                            } else if previous_device.is_some()
                                && self.status.output_device != previous_device
                                && let Some(ref device) = self.status.output_device
                            {
                                self.notify(
                                    format!("output: {}", lekima_domain::sanitize_line(device)),
                                    false,
                                );
                            }
                        }
                        if self.mode == Mode::Queue
                            && self.queue_follow
                            && let Some(index) = current_index
                        {
                            self.queue_selection = index.min(self.queue.len().saturating_sub(1));
                        }
                        let current = self.status.track.as_ref().map(|track| track.id);
                        if previous != current {
                            if self.mode == Mode::Comments {
                                match current {
                                    Some(id) => {
                                        self.comments_scroll = 0;
                                        self.comments_next_offset = 0;
                                        self.comments_pending_offset = None;
                                        self.comments_state = CommentsState::Loading { track: id };
                                        return Action::Call(Command::Comments {
                                            id: Some(id),
                                            limit: 20,
                                            offset: 0,
                                        });
                                    }
                                    None => {
                                        self.comments_state = CommentsState::Idle;
                                        self.mode = Mode::Normal;
                                    }
                                }
                            }
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
                    Reply::Search {
                        query,
                        kind,
                        results,
                    } => {
                        if !self.search.loading
                            || self.view != View::Search
                            || query != self.search.query
                            || kind != self.search.applied_filter.kind()
                        {
                            return Action::None;
                        }
                        let entries = SearchEntry::flatten(&results, self.search.applied_filter);
                        let empty = entries.is_empty();
                        self.search.loading = false;
                        self.search.error = None;
                        self.search.results = Some(results);
                        self.search.entries = entries;
                        self.view = View::Search;
                        self.main_selection = 0;
                        self.focus = Focus::Main;
                        if empty {
                            self.notify(
                                format!("no matches for \"{}\" ; / to refine", self.search.query),
                                false,
                            );
                        }
                    }
                    Reply::Artist { artist } => {
                        if self.view == View::Artist
                            && self.artist.id.is_none_or(|id| id == artist.id)
                        {
                            let id = artist.id;
                            self.artist.detail = Some(artist);
                            self.artist.error = None;
                            // The profile and its first album page are separate
                            // commands; chain the page so one key press loads both.
                            self.artist.loading = true;
                            return Action::Call(Command::ArtistAlbums {
                                id,
                                limit: 20,
                                offset: 0,
                            });
                        }
                    }
                    Reply::ArtistAlbums { page } => {
                        // Only fold a page into the artist view that asked for it.
                        if self.view == View::Artist && (self.artist.loading || self.artist.paging)
                        {
                            if self.artist.paging {
                                self.artist.albums.extend(page.albums);
                            } else {
                                self.artist.albums = page.albums;
                            }
                            self.artist.paging = false;
                            self.artist.loading = false;
                            self.artist.has_more = page.has_more;
                            self.artist.error = None;
                            if self.artist.albums.is_empty() {
                                self.notify("no albums", false);
                            }
                        }
                    }
                    Reply::Album { detail } => {
                        if self.view == View::Album && self.album.id == Some(detail.album.id) {
                            self.album.detail = Some(detail);
                            self.album.loading = false;
                            self.album.error = None;
                        }
                    }
                    Reply::Playlists { playlists } => {
                        self.playlists = playlists;
                        self.playlists_loading = false;
                        self.playlists_error = None;
                        self.playlists_selection = self
                            .playlists_selection
                            .min(self.playlists.len().saturating_sub(1));
                    }
                    Reply::Queue { tracks, index } => {
                        self.queue = tracks;
                        if self.mode == Mode::Queue
                            && self.queue_follow
                            && let Some(index) = index
                        {
                            self.queue_selection = index.min(self.queue.len().saturating_sub(1));
                        }
                        self.queue_selection =
                            self.queue_selection.min(self.queue.len().saturating_sub(1));
                    }
                    Reply::Playlist { playlist } => {
                        self.notify(
                            format!("created playlist {}", sanitize_line(&playlist.name)),
                            false,
                        );
                        return Action::Call(Command::Playlists);
                    }
                    Reply::Hooks { hooks } => {
                        self.hooks = hooks;
                    }
                    Reply::History { entries } => {
                        self.plays = entries;
                        self.plays_loading = false;
                        self.plays_error = None;
                        self.plays_selection =
                            self.plays_selection.min(self.plays.len().saturating_sub(1));
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
                    Reply::Comments { id, comments } => {
                        if self
                            .status
                            .track
                            .as_ref()
                            .is_some_and(|track| track.id == id)
                        {
                            if let Some(_offset) = self.comments_pending_offset.take() {
                                if let CommentsState::Ready {
                                    track,
                                    comments: existing,
                                } = &mut self.comments_state
                                    && *track == id
                                {
                                    let added = comments.comments.len();
                                    existing.total = comments.total;
                                    existing.has_more = comments.has_more;
                                    existing.comments.extend(comments.comments);
                                    self.comments_next_offset =
                                        self.comments_next_offset.saturating_add(added);
                                }
                            } else if comments.hot.is_empty() && comments.comments.is_empty() {
                                self.comments_state = CommentsState::Empty { track: id };
                                self.comments_next_offset = 0;
                            } else {
                                self.comments_next_offset = comments.comments.len();
                                self.comments_state = CommentsState::Ready {
                                    track: id,
                                    comments,
                                };
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
                        self.notify(if liked { "+ liked" } else { "- unliked" }, false);
                    }
                    Reply::Pong | Reply::Welcome { .. } | Reply::Subscribed { .. } => {}
                    Reply::Ack => {
                        if self.playlist_sync {
                            self.playlist_sync = false;
                            self.notify("playlist updated", false);
                            return Action::Call(Command::Playlists);
                        }
                    }
                }
                Action::None
            }
            Err(error) => {
                // A failed playlist write must not make the next unrelated ack
                // look like a successful one.
                self.playlist_sync = false;
                if self.playlists_loading {
                    self.playlists_loading = false;
                    self.playlists_error = Some(error_message(&error));
                }
                if self.artist.loading || self.artist.paging {
                    self.artist.loading = false;
                    self.artist.paging = false;
                    self.artist.error = Some(error_message(&error));
                }
                if self.album.loading {
                    self.album.loading = false;
                    self.album.error = Some(error_message(&error));
                }
                if self.view == View::Search && self.search.loading {
                    self.search.loading = false;
                    self.search.error = Some(error_message(&error));
                }
                if self.plays_loading {
                    self.plays_loading = false;
                    self.plays_error = Some(error_message(&error));
                }
                if let LyricsState::Loading { track } = self.lyrics_state {
                    self.lyrics_state = LyricsState::Error {
                        track,
                        message: crate::app::error_message(&error),
                    };
                }
                if let CommentsState::Loading { track } = self.comments_state {
                    self.comments_state = CommentsState::Error {
                        track,
                        message: crate::app::error_message(&error),
                    };
                }
                if error.is_not_running() {
                    self.daemon_down = true;
                    if !was_down {
                        self.notify("daemon not running; press R to start it", true);
                    }
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

fn cycle_list(list: &[&str], current: &mut String, delta: isize) {
    let index = list
        .iter()
        .position(|value| *value == current.as_str())
        .unwrap_or(0) as isize;
    let len = list.len() as isize;
    let next = (index + delta).rem_euclid(len) as usize;
    *current = list[next].to_owned();
}

fn prev_boundary(value: &str, index: usize) -> usize {
    let mut index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    value[..index]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(value: &str, index: usize) -> usize {
    let mut index = index.min(value.len());
    while index < value.len() && !value.is_char_boundary(index) {
        index += 1;
    }
    value[index..]
        .char_indices()
        .nth(1)
        .map_or(value.len(), |(offset, _)| index + offset)
}

fn parse_sleep_minutes(spec: &str) -> Result<Option<u32>, String> {
    if spec.is_empty() || spec == "off" || spec == "0" {
        return Ok(None);
    }
    let (value, multiplier) = if let Some(value) = spec.strip_suffix('h') {
        (value, 60.0)
    } else if let Some(value) = spec.strip_suffix('m') {
        (value, 1.0)
    } else if let Some(value) = spec.strip_suffix('s') {
        (value, 1.0 / 60.0)
    } else {
        (spec, 1.0)
    };
    let value: f64 = value
        .parse()
        .map_err(|_| format!("`{spec}` is not a duration"))?;
    if value <= 0.0 {
        return Err("sleep duration must be positive".to_owned());
    }
    Ok(Some((value * multiplier).ceil().clamp(1.0, 1440.0) as u32))
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

    use lekima_domain::{SearchKind, Track};

    use super::{
        Action, AlbumState, App, ArtistState, Focus, LyricsState, Mode, SearchEntry, SearchFilter,
        View,
    };

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
    fn album_commands_target_the_current_track() {
        let mut app = App::new();
        app.status.track = Some(Track {
            id: 1.into(),
            name: "Mota".to_owned(),
            artists: Vec::new(),
            album: Some(lekima_domain::Album {
                id: 7.into(),
                name: "Mota".to_owned(),
                cover_url: None,
                artist: None,
                year: None,
                track_count: None,
                description: String::new(),
            }),
            duration_ms: 1_000,
        });
        assert!(matches!(
            app.run_command("album play"),
            Action::Call(lekima_ipc::Command::Play {
                target: lekima_ipc::PlayTarget::Album { id }
            }) if id == 7.into()
        ));
        assert!(matches!(
            app.run_command("album queue"),
            Action::Call(lekima_ipc::Command::QueueAlbum { id, next: false }) if id == 7.into()
        ));
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
    fn queue_editing_emits_remove_and_move_commands() {
        let mut app = App::new();
        app.mode = Mode::Queue;
        app.queue = vec![sample_track(1), sample_track(2), sample_track(3)];
        app.queue_selection = 1;

        match app.handle_key(key(KeyCode::Char('J'))) {
            Action::Call(lekima_ipc::Command::QueueMove { from, to }) => {
                assert_eq!((from, to), (1, 2));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(app.queue_selection, 2);

        match app.handle_key(key(KeyCode::Char('K'))) {
            Action::Call(lekima_ipc::Command::QueueMove { from, to }) => {
                assert_eq!((from, to), (2, 1));
            }
            other => panic!("unexpected {other:?}"),
        }

        app.queue_selection = 1;
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('x'))),
            Action::None
        ));
        assert_eq!(app.queue_confirm, Some(1));
        match app.handle_key(key(KeyCode::Char('y'))) {
            Action::Call(lekima_ipc::Command::QueueRemove { index }) => {
                assert_eq!(index, 1);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(app.queue_confirm, None);
    }

    #[test]
    fn settings_translation_row_toggles_without_touching_density() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('S')));
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Char('j')));
        }
        assert_eq!(app.settings_selection, 3);
        let density = app.preferences.density.clone();
        let action = app.handle_key(key(KeyCode::Char('l')));
        assert!(matches!(action, Action::UiChanged));
        assert!(app.preferences.lyrics_translation);
        assert_eq!(app.preferences.density, density);
    }

    #[test]
    fn prompt_editing_supports_midline_and_word_delete() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('/')));
        for character in "mota".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        app.handle_key(key(KeyCode::Home));
        app.handle_key(key(KeyCode::Char('X')));
        assert_eq!(app.input, "Xmota");
        app.handle_key(key(KeyCode::End));
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.input, "Xmot");
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(app.input, "");
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn prompt_history_recalls_a_previous_search() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('/')));
        for character in "mota".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        app.handle_key(key(KeyCode::Enter));

        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.input, "mota");
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.input, "");
    }

    #[test]
    fn count_prefix_repeats_motion_and_jumps() {
        let mut app = App::new();
        app.tracks = (0..10).map(sample_track).collect();
        app.handle_key(key(KeyCode::Char('3')));
        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.main_selection, 3);
        assert_eq!(app.pending_count, None);
        app.handle_key(key(KeyCode::Char('2')));
        app.handle_key(key(KeyCode::Char('G')));
        assert_eq!(app.main_selection, 1);
    }

    #[test]
    fn half_page_uses_the_visible_viewport() {
        let mut app = App::new();
        app.tracks = (0..40).map(sample_track).collect();
        app.main_view_height = 20;
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(app.main_selection, 10);
    }

    #[test]
    fn queue_remove_can_be_cancelled() {
        let mut app = App::new();
        app.mode = Mode::Queue;
        app.queue = vec![sample_track(1)];
        app.handle_key(key(KeyCode::Char('x')));
        assert_eq!(app.queue_confirm, Some(0));
        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(app.queue_confirm, None);
        assert_eq!(app.queue.len(), 1);
    }

    #[test]
    fn ctrl_w_pane_keys_do_not_steal_count_prefixes() {
        let mut app = App::new();
        app.tracks = (0..10).map(sample_track).collect();
        app.status.track = Some(sample_track(1));

        let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        app.handle_key(ctrl_w);
        app.handle_key(key(KeyCode::Char('1')));
        assert_eq!(app.focus, Focus::Sidebar);

        app.handle_key(ctrl_w);
        app.handle_key(key(KeyCode::Char('2')));
        assert_eq!(app.focus, Focus::Main);

        app.handle_key(ctrl_w);
        let action = app.handle_key(key(KeyCode::Char('3')));
        assert!(matches!(action, Action::Call(lekima_ipc::Command::Queue)));
        assert_eq!(app.mode, Mode::Queue);
        app.handle_key(key(KeyCode::Esc));

        app.handle_key(ctrl_w);
        app.handle_key(key(KeyCode::Char('4')));
        assert!(app.lyrics_open);

        app.handle_key(key(KeyCode::Char('5')));
        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.main_selection, 5);
    }

    #[test]
    fn queue_mode_keeps_playback_controls() {
        let mut app = App::new();
        app.mode = Mode::Queue;
        app.queue = vec![sample_track(1), sample_track(2)];
        assert!(matches!(
            app.handle_key(key(KeyCode::Char(' '))),
            Action::Call(lekima_ipc::Command::Toggle)
        ));
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('n'))),
            Action::Call(lekima_ipc::Command::Next)
        ));
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('p'))),
            Action::Call(lekima_ipc::Command::Prev)
        ));
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('r'))),
            Action::Call(lekima_ipc::Command::CycleRepeat)
        ));
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('s'))),
            Action::Call(lekima_ipc::Command::SetShuffle { .. })
        ));
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('l'))),
            Action::Call(lekima_ipc::Command::Seek { .. })
        ));
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('+'))),
            Action::Call(lekima_ipc::Command::SetVolume { .. })
        ));
    }

    #[test]
    fn queue_enter_keeps_the_queue_open() {
        let mut app = App::new();
        app.mode = Mode::Queue;
        app.queue = vec![sample_track(1), sample_track(2)];
        app.queue_selection = 1;
        let action = app.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            action,
            Action::Call(lekima_ipc::Command::Jump { index: 1 })
        ));
        assert_eq!(app.mode, Mode::Queue);
    }

    #[test]
    fn queue_follows_the_current_track_until_the_user_moves() {
        let mut app = App::new();
        app.mode = Mode::Queue;
        app.queue = vec![sample_track(1), sample_track(2), sample_track(3)];
        let mut state = app.status.clone();
        state.queue_index = Some(1);
        app.apply(Ok(lekima_ipc::Reply::Status { state }));
        assert_eq!(app.queue_selection, 1);

        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.queue_selection, 2);
        assert!(!app.queue_follow);

        let mut state = app.status.clone();
        state.queue_index = Some(0);
        app.apply(Ok(lekima_ipc::Reply::Status { state }));
        assert_eq!(app.queue_selection, 2);
    }

    #[test]
    fn queue_events_append_new_tracks_while_open() {
        let mut app = App::new();
        app.mode = Mode::Queue;
        app.queue = vec![sample_track(1)];
        app.apply(Ok(lekima_ipc::Reply::Queue {
            tracks: vec![
                sample_track(1),
                sample_track(2),
                sample_track(7),
                sample_track(8),
            ],
            index: Some(1),
        }));
        assert_eq!(app.queue.len(), 4);
        assert_eq!(app.queue_selection, 1);
    }

    #[test]
    fn stale_status_replies_are_ignored_after_a_newer_event() {
        let mut app = App::new();
        app.note_event(10);
        let mut stale = app.status.clone();
        stale.revision = 9;
        stale.position_ms = 30_000;
        app.apply(Ok(lekima_ipc::Reply::Status { state: stale }));
        assert_eq!(app.status.position_ms, 0);

        let mut fresh = app.status.clone();
        fresh.revision = 10;
        fresh.position_ms = 500;
        app.apply(Ok(lekima_ipc::Reply::Status { state: fresh }));
        assert_eq!(app.status.position_ms, 500);
    }

    #[test]
    fn restored_queue_is_announced_on_first_status() {
        let mut app = App::new();
        let mut state = app.status.clone();
        state.queue_length = 2;
        state.queue_index = Some(0);
        state.status = lekima_domain::PlaybackStatus::Stopped;
        state.track = Some(sample_track(1));
        app.apply(Ok(lekima_ipc::Reply::Status { state }));
        let message = app.message.as_ref().expect("restore notice");
        assert!(message.text.contains("queue restored"), "{}", message.text);
        assert!(message.text.contains("space to resume"));
    }

    #[test]
    fn daemon_restart_is_announced_and_queue_restored() {
        let mut app = App::new();
        app.daemon_down = true;
        let mut state = app.status.clone();
        state.queue_length = 2;
        state.status = lekima_domain::PlaybackStatus::Stopped;
        app.apply(Ok(lekima_ipc::Reply::Status { state }));
        assert!(!app.daemon_down);
        let message = app.message.as_ref().expect("restart notice");
        assert!(
            message.text.contains("daemon restarted"),
            "{}",
            message.text
        );
        assert!(message.text.contains("queue restored"));
    }

    #[test]
    fn comments_mode_opens_and_folds_replies() {
        let mut app = App::new();
        app.status.track = Some(sample_track(1));
        let action = app.handle_key(key(KeyCode::Char('C')));
        assert!(matches!(
            action,
            Action::Call(lekima_ipc::Command::Comments {
                limit: 20,
                offset: 0,
                ..
            })
        ));
        assert_eq!(app.mode, Mode::Comments);

        app.apply(Ok(lekima_ipc::Reply::Comments {
            id: 1.into(),
            comments: lekima_domain::Comments {
                total: 1,
                has_more: false,
                hot: vec![lekima_domain::Comment {
                    id: 1,
                    author: "ben".to_owned(),
                    avatar_url: None,
                    content: "great".to_owned(),
                    timestamp_ms: 0,
                    liked_count: 3,
                    replies: Vec::new(),
                }],
                comments: Vec::new(),
            },
        }));
        let comments = app.comments_state.ready().expect("ready comments");
        assert_eq!(comments.hot[0].content, "great");
    }

    #[test]
    fn comments_can_load_the_next_page() {
        let mut app = App::new();
        app.status.track = Some(sample_track(1));
        app.handle_key(key(KeyCode::Char('C')));
        app.apply(Ok(lekima_ipc::Reply::Comments {
            id: 1.into(),
            comments: lekima_domain::Comments {
                total: 2,
                has_more: true,
                hot: Vec::new(),
                comments: vec![lekima_domain::Comment {
                    id: 1,
                    author: "a".to_owned(),
                    avatar_url: None,
                    content: "first".to_owned(),
                    timestamp_ms: 0,
                    liked_count: 0,
                    replies: Vec::new(),
                }],
            },
        }));
        assert_eq!(app.comments_next_offset, 1);
        let action = app.handle_key(key(KeyCode::Char('n')));
        assert!(matches!(
            action,
            Action::Call(lekima_ipc::Command::Comments { offset: 1, .. })
        ));
        app.apply(Ok(lekima_ipc::Reply::Comments {
            id: 1.into(),
            comments: lekima_domain::Comments {
                total: 2,
                has_more: false,
                hot: Vec::new(),
                comments: vec![lekima_domain::Comment {
                    id: 2,
                    author: "b".to_owned(),
                    avatar_url: None,
                    content: "second".to_owned(),
                    timestamp_ms: 0,
                    liked_count: 0,
                    replies: Vec::new(),
                }],
            },
        }));
        let comments = app.comments_state.ready().unwrap();
        assert_eq!(comments.comments.len(), 2);
        assert!(!comments.has_more);
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

    fn catalogue_results() -> lekima_domain::SearchResults {
        use lekima_domain::{Album, Artist, Playlist, SearchResults, SearchTotals};
        SearchResults {
            tracks: vec![sample_track(1)],
            albums: vec![Album {
                id: 9.into(),
                name: "Mota".to_owned(),
                cover_url: None,
                artist: Some(Artist {
                    id: 2.into(),
                    name: "Aash".to_owned(),
                }),
                year: Some(2024),
                track_count: Some(1),
                description: String::new(),
            }],
            artists: vec![Artist {
                id: 6452.into(),
                name: "Jay Chou".to_owned(),
            }],
            playlists: vec![Playlist {
                id: 5.into(),
                name: "Focus".to_owned(),
                description: String::new(),
                track_count: 42,
                cover_url: None,
                owner: None,
            }],
            totals: SearchTotals {
                tracks: 1,
                albums: 1,
                artists: 1,
                playlists: 1,
            },
            total: 4,
        }
    }

    fn open_search_results(app: &mut App) {
        app.search.query = "mota".to_owned();
        app.search.loading = true;
        app.view = View::Search;
        app.apply(Ok(lekima_ipc::Reply::Search {
            query: "mota".to_owned(),
            kind: lekima_domain::SearchKind::All,
            results: catalogue_results(),
        }));
    }

    fn sample_artist() -> lekima_domain::ArtistDetail {
        lekima_domain::ArtistDetail {
            id: 6452.into(),
            name: "Jay Chou".to_owned(),
            avatar_url: None,
            cover_url: None,
            album_count: 49,
            track_count: 592,
            description: "bio".to_owned(),
            aliases: vec!["周董".to_owned()],
        }
    }

    fn sample_album() -> lekima_domain::AlbumDetail {
        use lekima_domain::{Album, AlbumDetail};
        AlbumDetail {
            album: Album {
                id: 9.into(),
                name: "Mota".to_owned(),
                cover_url: None,
                artist: None,
                year: Some(2024),
                track_count: Some(2),
                description: String::new(),
            },
            tracks: vec![sample_track(1), sample_track(2)],
        }
    }

    #[test]
    fn the_category_can_be_changed_from_the_results_view() {
        let mut app = App::new();
        app.view = View::Search;
        app.search.query = "mota".to_owned();
        app.search.applied_filter = SearchFilter::All;

        match app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::CONTROL)) {
            Action::Call(lekima_ipc::Command::Search { kind, query, .. }) => {
                assert_eq!(kind, SearchKind::Album);
                assert_eq!(query, "mota");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(app.search.applied_filter, SearchFilter::Albums);
        assert!(app.search.loading);
    }

    #[test]
    fn search_filter_cycles_and_drives_the_command() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('/')));
        assert_eq!(app.search.filter, SearchFilter::All);

        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.search.filter, SearchFilter::Tracks);
        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.search.filter, SearchFilter::All);
        app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::CONTROL));
        assert_eq!(app.search.filter, SearchFilter::Albums);

        for character in "mota".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        match app.handle_key(key(KeyCode::Enter)) {
            Action::Call(lekima_ipc::Command::Search { kind, .. }) => {
                assert_eq!(kind, lekima_domain::SearchKind::Album);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(app.view, View::Search);
    }

    #[test]
    fn escaping_search_restores_the_applied_filter() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::CONTROL));
        assert_eq!(app.search.filter, SearchFilter::Artists);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.search.filter, SearchFilter::All);
        assert_eq!(app.search.applied_filter, SearchFilter::All);
    }

    #[test]
    fn stale_search_replies_are_ignored() {
        let mut app = App::new();
        app.search.query = "b".to_owned();
        app.search.loading = true;
        app.view = View::Search;
        app.apply(Ok(lekima_ipc::Reply::Search {
            query: "a".to_owned(),
            kind: lekima_domain::SearchKind::All,
            results: catalogue_results(),
        }));
        assert!(app.search.entries.is_empty());
        assert!(app.search.loading);
    }

    #[test]
    fn search_results_flatten_by_section() {
        let mut app = App::new();
        open_search_results(&mut app);
        assert_eq!(app.view, View::Search);
        assert_eq!(app.search.entries.len(), 4);
        assert!(matches!(app.search.entries[0], SearchEntry::Track(_)));
        assert!(matches!(app.search.entries[1], SearchEntry::Album(_)));
        assert!(matches!(app.search.entries[2], SearchEntry::Artist(_)));
        assert!(matches!(app.search.entries[3], SearchEntry::Playlist(_)));
    }

    #[test]
    fn enter_opens_albums_and_artists_then_history_returns() {
        let mut app = App::new();
        open_search_results(&mut app);

        app.main_selection = 1;
        match app.handle_key(key(KeyCode::Enter)) {
            Action::Call(lekima_ipc::Command::Album { id }) => assert_eq!(id, 9.into()),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(app.view, View::Album);
        assert!(app.album.loading);
        app.apply(Ok(lekima_ipc::Reply::Album {
            detail: sample_album(),
        }));
        assert_eq!(app.album.tracks().len(), 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(app.view, View::Search);

        app.main_selection = 2;
        match app.handle_key(key(KeyCode::Enter)) {
            Action::Call(lekima_ipc::Command::Artist { id }) => assert_eq!(id, 6452.into()),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(app.view, View::Artist);
        let action = app.apply(Ok(lekima_ipc::Reply::Artist {
            artist: sample_artist(),
        }));
        assert!(
            matches!(
                action,
                Action::Call(lekima_ipc::Command::ArtistAlbums { .. })
            ),
            "the profile chains its first album page"
        );
        assert!(app.artist.loading);
        app.apply(Ok(lekima_ipc::Reply::ArtistAlbums {
            page: lekima_domain::ArtistAlbums {
                albums: vec![sample_album().album],
                has_more: false,
            },
        }));
        assert_eq!(app.artist.albums.len(), 1);
        assert!(!app.artist.loading);
    }

    #[test]
    fn catalogue_rows_queue_albums_and_artists_do_not() {
        let mut app = App::new();
        open_search_results(&mut app);

        app.main_selection = 1;
        match app.handle_key(key(KeyCode::Char('a'))) {
            Action::Call(lekima_ipc::Command::QueueAlbum { id, next }) => {
                assert_eq!((id, next), (9.into(), false));
            }
            other => panic!("unexpected {other:?}"),
        }
        match app.handle_key(key(KeyCode::Char('A'))) {
            Action::Call(lekima_ipc::Command::QueueAlbum { id, next }) => {
                assert_eq!((id, next), (9.into(), true));
            }
            other => panic!("unexpected {other:?}"),
        }

        app.main_selection = 2;
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('a'))),
            Action::None
        ));
        assert!(app.message.as_ref().is_some_and(|message| !message.error));

        app.main_selection = 3;
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('a'))),
            Action::Call(lekima_ipc::Command::QueuePlaylist { .. })
        ));
    }

    #[test]
    fn artist_page_pages_albums_and_queues_on_a() {
        let mut app = App::new();
        app.view = View::Artist;
        app.artist = ArtistState {
            id: Some(6452.into()),
            detail: Some(sample_artist()),
            albums: vec![sample_album().album],
            has_more: true,
            ..ArtistState::default()
        };

        match app.handle_key(key(KeyCode::Char('n'))) {
            Action::Call(lekima_ipc::Command::ArtistAlbums { id, offset, .. }) => {
                assert_eq!((id, offset), (6452.into(), 1));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(app.artist.paging);

        // A paging failure clears the flag and surfaces on the page.
        app.apply(Err(lekima_ipc::IpcError::Remote {
            code: lekima_ipc::ErrorCode::Backend,
            message: "provider down".to_owned(),
        }));
        assert!(!app.artist.paging);
        assert_eq!(
            app.artist.error.as_deref(),
            Some("provider error: provider down")
        );

        match app.handle_key(key(KeyCode::Char('A'))) {
            Action::Call(lekima_ipc::Command::QueueAlbum { next, .. }) => assert!(next),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn album_tracks_play_from_the_selection() {
        let mut app = App::new();
        app.view = View::Album;
        app.album = AlbumState {
            id: Some(9.into()),
            detail: Some(sample_album()),
            ..AlbumState::default()
        };
        app.main_selection = 1;
        match app.handle_key(key(KeyCode::Enter)) {
            Action::Call(lekima_ipc::Command::Play {
                target: lekima_ipc::PlayTarget::Tracks { ids },
            }) => assert_eq!(ids, vec![2.into()]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn colon_commands_jump_from_the_current_track() {
        let mut app = App::new();
        let mut track = sample_track(1);
        track.album = Some(sample_album().album);
        track.artists = vec![lekima_domain::Artist {
            id: 6452.into(),
            name: "Jay Chou".to_owned(),
        }];
        app.status.track = Some(track);

        app.handle_key(key(KeyCode::Char(':')));
        for character in "album".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        match app.handle_key(key(KeyCode::Enter)) {
            Action::Call(lekima_ipc::Command::Album { id }) => assert_eq!(id, 9.into()),
            other => panic!("unexpected {other:?}"),
        }

        app.handle_key(key(KeyCode::Char(':')));
        for character in "artist".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        match app.handle_key(key(KeyCode::Enter)) {
            Action::Call(lekima_ipc::Command::Artist { id }) => assert_eq!(id, 6452.into()),
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[cfg(test)]
mod history_tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use lekima_domain::{PlayRecord, QueueKind, Track, TrackId};
    use lekima_ipc::{Command, PlayTarget, Reply};

    use super::{Action, App, Mode};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn record(id: u64, completed: bool) -> PlayRecord {
        PlayRecord {
            track: Track {
                id: TrackId::from(id),
                name: format!("Track {id}"),
                artists: Vec::new(),
                album: None,
                duration_ms: 60_000,
            },
            started_at: 1_700_000_000_000,
            played_ms: 60_000,
            completed,
            kind: QueueKind::Manual,
        }
    }

    #[test]
    fn h_opens_the_history_and_asks_for_plays() {
        let mut app = App::new();
        match app.handle_key(key(KeyCode::Char('H'))) {
            Action::Call(Command::History { limit }) => {
                assert_eq!(limit, Some(200));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(app.mode, Mode::History);
        assert!(app.plays_loading);

        app.handle_key(key(KeyCode::Char('H')));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn history_replies_populate_the_list_and_clamp_the_cursor() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('H')));
        app.plays_selection = 9;

        app.apply(Ok(Reply::History {
            entries: vec![record(2, false), record(1, true)],
        }));

        assert_eq!(app.plays.len(), 2);
        assert_eq!(app.plays_selection, 1, "cursor stays inside the list");
        assert!(!app.plays_loading);
    }

    #[test]
    fn movement_stays_inside_the_history() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('H')));
        app.apply(Ok(Reply::History {
            entries: vec![record(2, false), record(1, true)],
        }));

        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.plays_selection, 1);
        app.handle_key(key(KeyCode::Char('g')));
        assert_eq!(app.plays_selection, 0);
        app.handle_key(key(KeyCode::Char('G')));
        assert_eq!(app.plays_selection, 1);
    }

    #[test]
    fn enter_replays_the_selected_play_and_closes_the_pane() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('H')));
        app.apply(Ok(Reply::History {
            entries: vec![record(2, false), record(1, true)],
        }));

        match app.handle_key(key(KeyCode::Enter)) {
            Action::Call(Command::Play {
                target: PlayTarget::Tracks { ids },
            }) => assert_eq!(ids, vec![TrackId::from(2_u64)]),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn enter_on_an_empty_history_does_nothing() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('H')));

        assert!(matches!(app.handle_key(key(KeyCode::Enter)), Action::None));
        assert_eq!(app.mode, Mode::History);
    }

    fn run(app: &mut App, command: &str) -> Action {
        app.handle_key(key(KeyCode::Char(':')));
        for character in command.chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        app.handle_key(key(KeyCode::Enter))
    }

    #[test]
    fn playlist_commands_map_to_writes() {
        let mut app = App::new();

        match run(&mut app, "playlist create Late night") {
            Action::Call(Command::PlaylistCreate { name, private, .. }) => {
                assert_eq!(name, "Late night");
                assert!(!private);
            }
            other => panic!("unexpected {other:?}"),
        }

        app.status.track = Some(Track {
            id: TrackId::from(42_u64),
            name: "Song".to_owned(),
            artists: Vec::new(),
            album: None,
            duration_ms: 1_000,
        });
        match run(&mut app, "playlist add 7") {
            Action::Call(Command::PlaylistAdd { id, tracks }) => {
                assert_eq!(id, 7.into());
                assert_eq!(tracks, vec![TrackId::from(42_u64)]);
            }
            other => panic!("unexpected {other:?}"),
        }

        // Without the literal `yes` nothing is deleted.
        assert!(matches!(run(&mut app, "playlist delete 7"), Action::None));
        match run(&mut app, "playlist delete 7 yes") {
            Action::Call(Command::PlaylistDelete { id }) => assert_eq!(id, 7.into()),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_failed_playlist_write_does_not_poison_the_next_ack() {
        let mut app = App::new();
        app.status.track = Some(Track {
            id: TrackId::from(42_u64),
            name: "Song".to_owned(),
            artists: Vec::new(),
            album: None,
            duration_ms: 1_000,
        });

        run(&mut app, "playlist add 7");
        app.apply(Err(lekima_ipc::IpcError::Remote {
            code: lekima_ipc::ErrorCode::Backend,
            message: "rejected".to_owned(),
        }));

        assert!(matches!(app.apply(Ok(Reply::Ack)), Action::None));
    }

    #[test]
    fn a_playlist_write_refreshes_the_sidebar_when_acked() {
        let mut app = App::new();
        app.status.track = Some(Track {
            id: TrackId::from(42_u64),
            name: "Song".to_owned(),
            artists: Vec::new(),
            album: None,
            duration_ms: 1_000,
        });

        run(&mut app, "playlist add 7");
        assert!(matches!(
            app.apply(Ok(Reply::Ack)),
            Action::Call(Command::Playlists)
        ));

        // A later unrelated ack does not refresh again.
        assert!(matches!(app.apply(Ok(Reply::Ack)), Action::None));
    }

    #[test]
    fn hook_reports_are_stored_for_the_badge() {
        let mut app = App::new();
        app.apply(Ok(Reply::Hooks {
            hooks: vec![lekima_ipc::HookStatus {
                name: "nowplaying".to_owned(),
                events: vec!["status".to_owned()],
                capabilities: vec!["pause".to_owned()],
                running: true,
                error: None,
            }],
        }));

        assert_eq!(app.hooks.len(), 1);
        assert!(app.hooks[0].running);
    }

    #[test]
    fn a_failed_request_is_visible_and_retryable() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('H')));
        app.apply(Err(lekima_ipc::IpcError::Remote {
            code: lekima_ipc::ErrorCode::Daemon,
            message: "boom".to_owned(),
        }));

        assert!(!app.plays_loading);
        assert_eq!(app.plays_error.as_deref(), Some("daemon error: boom"));

        app.handle_key(key(KeyCode::Char('r')));
        assert!(app.plays_loading, "a retry goes back to loading");
        assert!(app.plays_error.is_none(), "the old error is cleared");
    }

    #[test]
    fn r_refreshes_without_leaving_the_pane() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('H')));

        assert!(matches!(
            app.handle_key(key(KeyCode::Char('r'))),
            Action::Call(Command::History { .. })
        ));
        assert_eq!(app.mode, Mode::History);
    }

    fn playlist(id: u64, name: &str) -> lekima_domain::Playlist {
        lekima_domain::Playlist {
            id: id.into(),
            name: name.to_owned(),
            description: String::new(),
            track_count: id,
            cover_url: None,
            owner: None,
        }
    }

    fn three_playlists() -> Vec<lekima_domain::Playlist> {
        vec![
            playlist(7, "Late night"),
            playlist(8, "Focus"),
            playlist(9, "Daily"),
        ]
    }

    #[test]
    fn p_opens_the_playlist_pane_and_refreshes_it() {
        let mut app = App::new();
        let action = app.handle_key(key(KeyCode::Char('P')));

        assert!(matches!(action, Action::Call(Command::Playlists)));
        assert_eq!(app.mode, Mode::Playlists);
        assert!(app.playlists_loading);
        assert!(app.playlists_error.is_none());
    }

    #[test]
    fn playlist_navigation_clamps_and_supports_half_pages() {
        let mut app = App::new();
        app.mode = Mode::Playlists;
        app.playlists = three_playlists();

        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.playlists_selection, 2);
        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.playlists_selection, 2, "clamped at the bottom");

        app.handle_key(key(KeyCode::Char('g')));
        assert_eq!(app.playlists_selection, 0);
        app.handle_key(key(KeyCode::Char('G')));
        assert_eq!(app.playlists_selection, 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.playlists_selection, 0);
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(app.playlists_selection, 2);

        app.handle_key(key(KeyCode::Char('k')));
        assert_eq!(app.playlists_selection, 1);
    }

    #[test]
    fn enter_opens_the_selected_playlist_and_closes_the_pane() {
        let mut app = App::new();
        app.mode = Mode::Playlists;
        app.playlists = three_playlists();
        app.playlists_selection = 1;

        match app.handle_key(key(KeyCode::Enter)) {
            Action::Call(Command::PlaylistTracks { id }) => assert_eq!(id, 8.into()),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.source, "playlist: Focus");
    }

    #[test]
    fn playlist_write_prompts_use_the_command_line() {
        let mut app = App::new();
        app.mode = Mode::Playlists;
        app.playlists = three_playlists();
        app.playlists_selection = 1;

        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.input, "playlist create ");
        assert_eq!(app.cursor, app.input.len());

        app.mode = Mode::Playlists;
        app.handle_key(key(KeyCode::Char('R')));
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.input, "playlist rename 8 ");

        app.mode = Mode::Playlists;
        app.handle_key(key(KeyCode::Char('d')));
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.input, "playlist describe 8 ");
    }

    #[test]
    fn rename_and_describe_submit_playlist_updates() {
        let mut app = App::new();

        match run(&mut app, "playlist rename 8 Focus 2") {
            Action::Call(Command::PlaylistUpdate {
                id,
                name,
                description,
            }) => {
                assert_eq!(id, 8.into());
                assert_eq!(name.as_deref(), Some("Focus 2"));
                assert!(description.is_none());
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(app.playlist_sync);

        app.playlist_sync = false;
        match run(&mut app, "playlist describe 8 quiet hours") {
            Action::Call(Command::PlaylistUpdate {
                id,
                name,
                description,
            }) => {
                assert_eq!(id, 8.into());
                assert!(name.is_none());
                assert_eq!(description.as_deref(), Some("quiet hours"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(app.playlist_sync);

        // An empty name is rejected before it reaches the daemon.
        assert!(matches!(run(&mut app, "playlist rename 8 "), Action::None));
    }

    #[test]
    fn a_adds_the_current_track_to_the_selected_playlist() {
        let mut app = App::new();
        app.mode = Mode::Playlists;
        app.playlists = three_playlists();
        app.playlists_selection = 1;
        app.status.track = Some(Track {
            id: TrackId::from(42_u64),
            name: "Song".to_owned(),
            artists: Vec::new(),
            album: None,
            duration_ms: 1_000,
        });

        match app.handle_key(key(KeyCode::Char('a'))) {
            Action::Call(Command::PlaylistAdd { id, tracks }) => {
                assert_eq!(id, 8.into());
                assert_eq!(tracks, vec![TrackId::from(42_u64)]);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(app.playlist_sync);

        app.playlist_sync = false;
        app.status.track = None;
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('a'))),
            Action::None
        ));
        assert!(!app.playlist_sync, "nothing playing must not write");
    }

    #[test]
    fn delete_requires_a_confirmation_key() {
        let mut app = App::new();
        app.mode = Mode::Playlists;
        app.playlists = three_playlists();
        app.playlists_selection = 1;

        assert!(matches!(
            app.handle_key(key(KeyCode::Char('x'))),
            Action::None
        ));
        assert_eq!(app.playlist_confirm, Some(8.into()));
        assert!(!app.playlist_sync, "x alone must not write");

        // A stray key keeps the confirmation open and writes nothing.
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('j'))),
            Action::None
        ));
        assert_eq!(app.playlist_confirm, Some(8.into()));

        app.handle_key(key(KeyCode::Char('n')));
        assert!(app.playlist_confirm.is_none());
        assert!(!app.playlist_sync);

        app.handle_key(key(KeyCode::Char('x')));
        match app.handle_key(key(KeyCode::Char('y'))) {
            Action::Call(Command::PlaylistDelete { id }) => assert_eq!(id, 8.into()),
            other => panic!("unexpected {other:?}"),
        }
        assert!(app.playlist_sync);
        assert!(app.playlist_confirm.is_none());
    }

    #[test]
    fn a_playlist_prompt_returns_to_the_pane() {
        let mut app = App::new();
        app.mode = Mode::Playlists;

        // `n` prefills the command line from the pane.
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('n'))),
            Action::None
        ));
        assert_eq!(app.mode, Mode::Command);
        for character in "Late night".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        match app.handle_key(key(KeyCode::Enter)) {
            Action::Call(Command::PlaylistCreate { name, .. }) => {
                assert_eq!(name, "Late night");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(app.mode, Mode::Playlists, "the pane is still open");
    }

    #[test]
    fn p_cancels_the_delete_confirmation_and_closes_the_pane() {
        let mut app = App::new();
        app.mode = Mode::Playlists;
        app.playlists = vec![lekima_domain::Playlist {
            id: 7.into(),
            name: "Late night".to_owned(),
            description: String::new(),
            track_count: 0,
            cover_url: None,
            owner: None,
        }];

        app.handle_key(key(KeyCode::Char('x')));
        assert_eq!(app.playlist_confirm, Some(7.into()));
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('P'))),
            Action::None
        ));
        assert!(app.playlist_confirm.is_none());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn escape_cancels_the_delete_confirmation_without_deleting() {
        let mut app = App::new();
        app.mode = Mode::Playlists;
        app.playlists = vec![lekima_domain::Playlist {
            id: 7.into(),
            name: "Late night".to_owned(),
            description: String::new(),
            track_count: 0,
            cover_url: None,
            owner: None,
        }];

        assert!(matches!(
            app.handle_key(key(KeyCode::Char('x'))),
            Action::None
        ));
        assert_eq!(app.playlist_confirm, Some(7.into()));
        assert!(matches!(app.handle_key(key(KeyCode::Esc)), Action::None));
        assert!(app.playlist_confirm.is_none());
        assert_eq!(app.mode, Mode::Playlists, "esc stays in the pane");
    }

    #[test]
    fn a_adds_the_current_track_even_when_playback_is_stopped() {
        let mut app = App::new();
        app.mode = Mode::Playlists;
        app.playlists = vec![lekima_domain::Playlist {
            id: 7.into(),
            name: "Late night".to_owned(),
            description: String::new(),
            track_count: 0,
            cover_url: None,
            owner: None,
        }];
        app.status.track = Some(Track {
            id: TrackId::from(42_u64),
            name: "Song".to_owned(),
            artists: Vec::new(),
            album: None,
            duration_ms: 1_000,
        });
        app.status.status = lekima_domain::PlaybackStatus::Stopped;

        match app.handle_key(key(KeyCode::Char('a'))) {
            Action::Call(Command::PlaylistAdd { id, tracks }) => {
                assert_eq!(id, 7.into());
                assert_eq!(tracks, vec![TrackId::from(42_u64)]);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_failed_playlist_write_does_not_refresh_the_pane() {
        let mut app = App::new();
        app.mode = Mode::Playlists;
        app.playlists = three_playlists();

        app.handle_key(key(KeyCode::Char('x')));
        app.handle_key(key(KeyCode::Char('y')));
        assert!(app.playlist_sync);

        let action = app.apply(Err(lekima_ipc::IpcError::Remote {
            code: lekima_ipc::ErrorCode::Backend,
            message: "rejected".to_owned(),
        }));
        assert!(matches!(action, Action::None));
        assert!(!app.playlist_sync);
        assert_eq!(app.mode, Mode::Playlists);
        assert!(app.message.as_ref().is_some_and(|message| message.error));

        // The next unrelated ack cannot look like this write's success.
        assert!(matches!(app.apply(Ok(Reply::Ack)), Action::None));
    }

    #[test]
    fn a_successful_playlist_write_refreshes_the_pane() {
        let mut app = App::new();
        app.mode = Mode::Playlists;
        app.playlists = three_playlists();
        app.playlists_selection = 1;
        app.status.track = Some(Track {
            id: TrackId::from(42_u64),
            name: "Song".to_owned(),
            artists: Vec::new(),
            album: None,
            duration_ms: 1_000,
        });

        app.handle_key(key(KeyCode::Char('a')));
        assert!(matches!(
            app.apply(Ok(Reply::Ack)),
            Action::Call(Command::Playlists)
        ));
        assert_eq!(app.mode, Mode::Playlists);
        assert_eq!(app.playlists[1].name, "Focus");
    }

    #[test]
    fn r_refreshes_the_playlist_pane() {
        let mut app = App::new();
        app.mode = Mode::Playlists;
        app.playlists = three_playlists();
        app.playlists_loading = false;

        assert!(matches!(
            app.handle_key(key(KeyCode::Char('r'))),
            Action::Call(Command::Playlists)
        ));
        assert_eq!(app.mode, Mode::Playlists);
        assert!(app.playlists_loading);
    }

    #[test]
    fn a_failed_playlist_list_shows_an_inline_error_with_a_retry() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('P')));
        app.apply(Err(lekima_ipc::IpcError::Remote {
            code: lekima_ipc::ErrorCode::Daemon,
            message: "boom".to_owned(),
        }));

        assert!(!app.playlists_loading);
        assert_eq!(app.playlists_error.as_deref(), Some("daemon error: boom"));

        app.handle_key(key(KeyCode::Char('r')));
        assert!(app.playlists_loading);
        assert!(app.playlists_error.is_none());
    }

    #[test]
    fn device_command_and_settings_row_dispatch_set_audio_device() {
        let mut app = App::new();
        app.set_devices(vec!["Built-in".to_owned(), "Headphones".to_owned()]);

        app.handle_key(key(KeyCode::Char(':')));
        for ch in "device Headphones".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            Action::Call(Command::SetAudioDevice { device: Some(name) }) if name == "Headphones"
        ));

        app.handle_key(key(KeyCode::Char(':')));
        for ch in "device default".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            Action::Call(Command::SetAudioDevice { device: None })
        ));

        app.mode = Mode::Settings;
        app.settings_selection = 4;
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('l'))),
            Action::Call(Command::SetAudioDevice { device: Some(name) }) if name == "Built-in"
        ));
    }

    #[test]
    fn status_update_notifies_on_device_loss_or_switch() {
        use lekima_domain::{PlaybackState, PlaybackStatus};
        let mut app = App::new();
        let initial = PlaybackState {
            status: PlaybackStatus::Playing,
            output_device: Some("USB DAC".to_owned()),
            revision: 1,
            ..PlaybackState::default()
        };
        app.apply(Ok(lekima_ipc::Reply::Status { state: initial }));

        let switched = PlaybackState {
            status: PlaybackStatus::Playing,
            output_device: Some("Built-in".to_owned()),
            revision: 2,
            ..PlaybackState::default()
        };
        app.apply(Ok(lekima_ipc::Reply::Status { state: switched }));
        assert_eq!(
            app.message.as_ref().map(|m| m.text.as_str()),
            Some("output: Built-in")
        );

        let lost = PlaybackState {
            status: PlaybackStatus::Paused,
            output_device: None,
            revision: 3,
            ..PlaybackState::default()
        };
        app.apply(Ok(lekima_ipc::Reply::Status { state: lost }));
        assert_eq!(
            app.message.as_ref().map(|m| (m.text.as_str(), m.error)),
            Some(("output device lost; playback paused", true))
        );
    }
}
