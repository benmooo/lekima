//! The wire protocol: newline-delimited JSON over a Unix socket.
//!
//! Every frame is one JSON object. Requests carry a client-chosen `id` that is
//! echoed back so a future streaming mode can multiplex. The protocol is
//! intentionally line-oriented and stable: shell scripts, the TUI and code
//! agents all speak the same language.

use lekima_domain::{
    AlbumDetail, AlbumId, ArtistAlbums, ArtistDetail, ArtistId, Comments, Lyrics, PlayRecord,
    PlaybackState, Playlist, PlaylistId, Quality, RepeatMode, SearchKind, SearchResults, Toplist,
    Track, TrackId, UserProfile,
};
use serde::{Deserialize, Serialize};

/// Largest accepted frame. Guards against a wedged peer exhausting memory.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub command: Command,
}

impl Request {
    #[must_use]
    pub const fn new(id: u64, command: Command) -> Self {
        Self { id, command }
    }
}

/// Client-to-daemon commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Ping,
    Status,
    /// What would `Play` target?
    Play {
        target: PlayTarget,
    },
    /// Jump to a position in the current queue.
    Jump {
        index: usize,
    },
    Pause,
    Resume,
    Toggle,
    Stop,
    Next,
    Prev,
    Seek {
        position_ms: u64,
    },
    SetVolume {
        level: f32,
    },
    ToggleMute,
    CycleRepeat,
    SetRepeat {
        mode: RepeatMode,
    },
    SetShuffle {
        enabled: bool,
    },
    Queue,
    QueueRemove {
        index: usize,
    },
    QueueMove {
        from: usize,
        to: usize,
    },
    Enqueue {
        tracks: Vec<TrackId>,
        next: bool,
    },
    ClearQueue,
    /// Daily recommendations; list or play.
    Daily {
        play: bool,
    },
    /// Personal FM controls.
    Fm {
        action: FmAction,
    },
    /// Official charts.
    Toplists,
    /// The account's music cloud; list or play.
    Cloud {
        limit: u32,
        offset: u32,
        play: bool,
    },
    /// Like or unlike a track (defaults to the current track).
    Like {
        id: Option<TrackId>,
        liked: bool,
    },
    /// Listening history, newest first. `limit` defaults to 50.
    History {
        limit: Option<usize>,
    },
    /// Public comments for a track (defaults to the current track).
    Comments {
        id: Option<TrackId>,
        limit: u16,
        offset: u32,
    },
    /// The account's liked tracks; list or play.
    Liked {
        play: bool,
    },
    /// Protocol handshake.
    Hello {
        protocol: u16,
    },
    /// Configure or cancel the sleep timer (`None` cancels).
    Sleep {
        minutes: Option<u32>,
    },
    /// Bitrate class for subsequent tracks.
    SetQuality {
        quality: Quality,
    },
    /// Stream sequenced events on this connection after the ack.
    Subscribe {
        #[serde(default)]
        kinds: Vec<crate::event::EventKind>,
        since: Option<u64>,
    },
    Search {
        query: String,
        kind: SearchKind,
        limit: u32,
        offset: u32,
    },
    Playlists,
    PlaylistTracks {
        id: PlaylistId,
    },
    /// A full artist profile.
    Artist {
        id: ArtistId,
    },
    /// One page of an artist's albums.
    ArtistAlbums {
        id: ArtistId,
        limit: u32,
        offset: u32,
    },
    /// An album with its track list.
    Album {
        id: AlbumId,
    },
    /// Append every track of an album to the queue.
    QueueAlbum {
        id: AlbumId,
        next: bool,
    },
    /// Append every track of a playlist to the queue.
    QueuePlaylist {
        id: PlaylistId,
        next: bool,
    },
    Lyrics {
        id: Option<TrackId>,
    },
    Whoami,
    /// Persist a browser cookie header (`MUSIC_U=…`) and reload the provider.
    Login {
        cookie: String,
    },
    Logout,
    Shutdown,
}

impl Command {
    /// Wire tag for this command.
    ///
    /// The tag is also the capability name a hook must declare before the
    /// daemon will run the command for it, so the two cannot drift.
    #[must_use]
    pub const fn tag(&self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::Status => "status",
            Self::Play { .. } => "play",
            Self::Jump { .. } => "jump",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Toggle => "toggle",
            Self::Stop => "stop",
            Self::Next => "next",
            Self::Prev => "prev",
            Self::Seek { .. } => "seek",
            Self::SetVolume { .. } => "set_volume",
            Self::ToggleMute => "toggle_mute",
            Self::CycleRepeat => "cycle_repeat",
            Self::SetRepeat { .. } => "set_repeat",
            Self::SetShuffle { .. } => "set_shuffle",
            Self::Queue => "queue",
            Self::QueueRemove { .. } => "queue_remove",
            Self::QueueMove { .. } => "queue_move",
            Self::Enqueue { .. } => "enqueue",
            Self::ClearQueue => "clear_queue",
            Self::Daily { .. } => "daily",
            Self::Fm { .. } => "fm",
            Self::Toplists => "toplists",
            Self::Cloud { .. } => "cloud",
            Self::Like { .. } => "like",
            Self::History { .. } => "history",
            Self::Comments { .. } => "comments",
            Self::Liked { .. } => "liked",
            Self::Hello { .. } => "hello",
            Self::Sleep { .. } => "sleep",
            Self::SetQuality { .. } => "set_quality",
            Self::Subscribe { .. } => "subscribe",
            Self::Search { .. } => "search",
            Self::Playlists => "playlists",
            Self::PlaylistTracks { .. } => "playlist_tracks",
            Self::Artist { .. } => "artist",
            Self::ArtistAlbums { .. } => "artist_albums",
            Self::Album { .. } => "album",
            Self::QueueAlbum { .. } => "queue_album",
            Self::QueuePlaylist { .. } => "queue_playlist",
            Self::Lyrics { .. } => "lyrics",
            Self::Whoami => "whoami",
            Self::Login { .. } => "login",
            Self::Logout => "logout",
            Self::Shutdown => "shutdown",
        }
    }
}

/// Command tags a hook may be granted.
///
/// Only commands with a player-visible effect appear here. Read-only commands
/// are absent because a hook cannot receive a reply on its command stream; it
/// reads state from events, from the public state files, or by connecting to the
/// socket like any other client. Session and lifecycle commands (`login`,
/// `logout`, `shutdown`) are absent so a mistaken hook cannot take over the
/// account or stop the daemon.
pub const HOOK_CAPABILITIES: &[&str] = &[
    "play",
    "jump",
    "pause",
    "resume",
    "toggle",
    "stop",
    "next",
    "prev",
    "seek",
    "set_volume",
    "toggle_mute",
    "cycle_repeat",
    "set_repeat",
    "set_shuffle",
    "queue_remove",
    "queue_move",
    "enqueue",
    "clear_queue",
    "daily",
    "fm",
    "cloud",
    "like",
    "sleep",
    "set_quality",
    "queue_album",
    "queue_playlist",
];

/// Whether `tag` names a capability a hook may declare.
#[must_use]
pub fn is_hook_capability(tag: &str) -> bool {
    HOOK_CAPABILITIES.contains(&tag)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FmAction {
    Start,
    Next { trash: bool },
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlayTarget {
    /// Search the catalogue and queue the results.
    Query { query: String },
    /// Play known track ids.
    Tracks { ids: Vec<TrackId> },
    /// Play every track of a playlist.
    Playlist { id: PlaylistId },
    /// Play every track of an album.
    Album { id: AlbumId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Reply>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

impl Response {
    #[must_use]
    pub const fn ok(id: u64, result: Reply) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    #[must_use]
    pub fn err(id: u64, error: ApiError) -> Self {
        Self {
            id,
            result: None,
            error: Some(error),
        }
    }

    #[must_use]
    pub const fn is_ok(&self) -> bool {
        self.error.is_none()
    }
}

/// Daemon-to-client payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Pong,
    Ack,
    Status {
        state: PlaybackState,
    },
    Tracks {
        tracks: Vec<Track>,
    },
    Search {
        #[serde(default)]
        query: String,
        #[serde(default)]
        kind: SearchKind,
        results: SearchResults,
    },
    Playlists {
        playlists: Vec<Playlist>,
    },
    Artist {
        artist: ArtistDetail,
    },
    ArtistAlbums {
        page: ArtistAlbums,
    },
    Album {
        detail: AlbumDetail,
    },
    Lyrics {
        id: TrackId,
        lyrics: Lyrics,
    },
    Comments {
        id: TrackId,
        comments: Comments,
    },
    Toplists {
        toplists: Vec<Toplist>,
    },
    Like {
        id: TrackId,
        liked: bool,
    },
    Welcome {
        protocol: u16,
        seq: u64,
        capabilities: Vec<String>,
    },
    Subscribed {
        seq: u64,
        resync: bool,
    },
    User {
        user: Option<UserProfile>,
    },
    Queue {
        tracks: Vec<Track>,
        index: Option<usize>,
    },
    /// Finished plays, newest first.
    History {
        entries: Vec<PlayRecord>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

impl ApiError {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Stable, machine-readable failure categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Malformed request or invalid argument.
    BadRequest,
    /// The provider needs a session.
    Unauthorized,
    NotFound,
    Unsupported,
    /// The provider or network failed.
    Backend,
    /// Playback device failure.
    Playback,
    /// The daemon could not be reached or crashed.
    Daemon,
    Internal,
}

impl From<lekima_domain::DomainError> for ApiError {
    fn from(error: lekima_domain::DomainError) -> Self {
        use lekima_domain::DomainError;
        match error {
            DomainError::InvalidInput(message) => Self::new(ErrorCode::BadRequest, message),
            DomainError::NotFound(message) => Self::new(ErrorCode::NotFound, message),
            DomainError::AuthenticationRequired(message) => {
                Self::new(ErrorCode::Unauthorized, message)
            }
            DomainError::Unsupported(message) => Self::new(ErrorCode::Unsupported, message),
            DomainError::Unavailable(message) => Self::new(ErrorCode::Backend, message),
            DomainError::Playback(message) => Self::new(ErrorCode::Playback, message),
        }
    }
}

#[cfg(test)]
mod tests {
    use lekima_domain::{SearchKind, TrackId};

    use super::{Command, PlayTarget, Reply, Request, Response};

    #[test]
    fn commands_round_trip_through_json() {
        let request = Request::new(
            7,
            Command::Play {
                target: PlayTarget::Tracks {
                    ids: vec![TrackId(1), TrackId(2)],
                },
            },
        );
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(encoded.contains("\"type\":\"play\""));
        let decoded: Request = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.id, 7);
    }

    #[test]
    fn search_kind_is_lowercase_on_the_wire() {
        let encoded = serde_json::to_string(&Command::Search {
            query: "mota".into(),
            kind: SearchKind::Track,
            limit: 20,
            offset: 0,
        })
        .unwrap();
        assert!(encoded.contains("\"kind\":\"track\""));
    }

    #[test]
    fn response_without_result_or_error_is_detectable() {
        let response = Response::ok(1, Reply::Ack);
        assert!(response.is_ok());
        assert!(serde_json::to_string(&response).unwrap().contains("ack"));
    }

    #[test]
    fn hook_capabilities_are_command_tags_without_session_commands() {
        for tag in super::HOOK_CAPABILITIES {
            assert!(!tag.is_empty());
            assert_eq!(*tag, tag.to_ascii_lowercase(), "wire tags are snake_case");
            assert!(super::is_hook_capability(tag));
        }
        // Lifecycle, session and read-only commands are never grantable.
        for forbidden in [
            "login",
            "logout",
            "shutdown",
            "subscribe",
            "hello",
            "ping",
            "status",
            "queue",
            "history",
            "search",
            "lyrics",
            "whoami",
        ] {
            assert!(
                !super::is_hook_capability(forbidden),
                "{forbidden} must not be grantable to a hook"
            );
        }

        // No duplicates, and every entry is a real command tag.
        let mut seen = std::collections::HashSet::new();
        for tag in super::HOOK_CAPABILITIES {
            assert!(seen.insert(*tag), "duplicate capability {tag}");
        }

        assert_eq!(Command::Pause.tag(), "pause");
        assert_eq!(Command::SetVolume { level: 0.5 }.tag(), "set_volume");
        assert_eq!(Command::History { limit: None }.tag(), "history");
        assert_eq!(
            Command::QueueAlbum {
                id: 1.into(),
                next: true
            }
            .tag(),
            "queue_album"
        );
        assert_eq!(Command::Shutdown.tag(), "shutdown");
    }

    #[test]
    fn history_round_trips_and_defaults_to_the_daemon_limit() {
        let encoded = serde_json::to_string(&Command::History { limit: Some(5) }).unwrap();
        assert_eq!(encoded, r#"{"type":"history","limit":5}"#);

        let decoded: Command = serde_json::from_str(r#"{"type":"history"}"#).unwrap();
        assert!(matches!(decoded, Command::History { limit: None }));

        let reply = Reply::History { entries: vec![] };
        let encoded = serde_json::to_string(&reply).unwrap();
        assert_eq!(encoded, r#"{"type":"history","entries":[]}"#);
    }

    #[test]
    fn catalogue_commands_round_trip() {
        let cases = vec![
            Command::Artist {
                id: lekima_domain::ArtistId(6452),
            },
            Command::ArtistAlbums {
                id: lekima_domain::ArtistId(6452),
                limit: 20,
                offset: 0,
            },
            Command::Album {
                id: lekima_domain::AlbumId(242_091_487),
            },
            Command::QueueAlbum {
                id: lekima_domain::AlbumId(242_091_487),
                next: true,
            },
            Command::Play {
                target: PlayTarget::Album {
                    id: lekima_domain::AlbumId(9),
                },
            },
        ];
        for command in cases {
            let encoded = serde_json::to_string(&command).unwrap();
            let decoded: Command = serde_json::from_str(&encoded).unwrap();
            assert_eq!(
                serde_json::to_string(&decoded).unwrap(),
                encoded,
                "round trip must be stable"
            );
        }
    }

    #[test]
    fn catalogue_replies_round_trip() {
        let artist = lekima_domain::ArtistDetail {
            id: lekima_domain::ArtistId(6452),
            name: "周杰伦".to_owned(),
            avatar_url: None,
            cover_url: None,
            album_count: 49,
            track_count: 592,
            description: String::new(),
            aliases: vec!["Jay Chou".to_owned()],
        };
        let album = lekima_domain::AlbumDetail {
            album: lekima_domain::Album {
                id: lekima_domain::AlbumId(9),
                name: "Mota".to_owned(),
                cover_url: None,
                artist: None,
                year: Some(2024),
                track_count: Some(1),
                description: String::new(),
            },
            tracks: Vec::new(),
        };
        for reply in [
            Reply::Artist { artist },
            Reply::ArtistAlbums {
                page: lekima_domain::ArtistAlbums {
                    albums: Vec::new(),
                    has_more: true,
                },
            },
            Reply::Album { detail: album },
        ] {
            let encoded = serde_json::to_string(&reply).unwrap();
            let decoded: Reply = serde_json::from_str(&encoded).unwrap();
            assert_eq!(
                serde_json::to_string(&decoded).unwrap(),
                encoded,
                "round trip must be stable"
            );
        }
    }
}
