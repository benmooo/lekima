//! The wire protocol: newline-delimited JSON over a Unix socket.
//!
//! Every frame is one JSON object. Requests carry a client-chosen `id` that is
//! echoed back so a future streaming mode can multiplex. The protocol is
//! intentionally line-oriented and stable: shell scripts, the TUI and code
//! agents all speak the same language.

use lekima_domain::{
    Comments, Lyrics, PlaybackState, Playlist, PlaylistId, Quality, RepeatMode, SearchKind,
    SearchResults, Toplist, Track, TrackId, UserProfile,
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
        results: SearchResults,
    },
    Playlists {
        playlists: Vec<Playlist>,
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
}
