//! Output and process-exit policy.
//!
//! Human output is for people; `--json` is the stable agent contract:
//!
//! ```json
//! {"ok":true,"data":{…}}
//! {"ok":false,"error":{"code":"unauthorized","message":"…"}}
//! ```

use lekima_domain::{PlaybackState, Playlist, SearchResults, Track};
use lekima_ipc::{ApiError, ErrorCode, IpcError, Reply};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
struct Envelope<T> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ApiError>,
}

/// A failure that maps to one JSON error and one exit code.
#[derive(Debug)]
pub struct Failure {
    pub code: ErrorCode,
    pub message: String,
}

impl Failure {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    #[must_use]
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, message)
    }

    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self.code {
            ErrorCode::BadRequest => 2,
            ErrorCode::Unauthorized => 3,
            ErrorCode::NotFound => 4,
            ErrorCode::Backend => 5,
            ErrorCode::Playback => 6,
            ErrorCode::Daemon => 7,
            ErrorCode::Unsupported => 8,
            ErrorCode::Internal => 1,
        }
    }

    #[must_use]
    pub fn api_error(&self) -> ApiError {
        ApiError::new(self.code, self.message.clone())
    }
}

impl From<IpcError> for Failure {
    fn from(error: IpcError) -> Self {
        match error {
            IpcError::NotRunning { path } => Self::new(
                ErrorCode::Daemon,
                format!(
                    "the player daemon is not running at {}; start it with `lekima daemon start`",
                    path.display()
                ),
            ),
            IpcError::Remote { code, message } => Self::new(code, message),
            other => Self::new(ErrorCode::Daemon, other.to_string()),
        }
    }
}

impl From<lekima_domain::DomainError> for Failure {
    fn from(error: lekima_domain::DomainError) -> Self {
        let api: ApiError = error.into();
        Self::new(api.code, api.message)
    }
}

impl From<lekima_config::PathError> for Failure {
    fn from(error: lekima_config::PathError) -> Self {
        Self::new(ErrorCode::Internal, error.to_string())
    }
}

/// Prints a successful reply and exits 0.
pub fn print_reply(reply: &Reply, json: bool) {
    if json {
        emit_json(true, Some(reply_data(reply)), None);
        return;
    }
    match reply {
        Reply::Status { state } => print_status(state),
        Reply::Tracks { tracks } => print_tracks(tracks),
        Reply::Search { results } => print_search(results),
        Reply::Playlists { playlists } => print_playlists(playlists),
        Reply::Queue { tracks, index } => print_queue(tracks, *index),
        Reply::User { user } => match user {
            Some(user) => crate::say!("{} ({})", user.nickname, user.id),
            None => crate::say!("not signed in"),
        },
        Reply::Lyrics { lyrics, .. } => crate::say!("{}", lyrics.plain()),
        Reply::Toplists { toplists } => {
            for chart in toplists {
                crate::say!(
                    "{:>10}  {:<24} {:>5}  {}",
                    chart.id,
                    truncate(&chart.name, 24),
                    chart.track_count,
                    chart.update_frequency
                );
            }
        }
        Reply::Like { id, liked } => {
            crate::say!("{} {id}", if *liked { "♥ liked" } else { "♡ unliked" });
        }
        Reply::Ack | Reply::Pong => {}
    }
}

/// Serializes a reply for agents. `status` is flattened so `data.status` is the
/// playback status, matching the documented contract.
#[must_use]
pub fn reply_data(reply: &Reply) -> Value {
    match reply {
        Reply::Status { state } => serde_json::to_value(state).unwrap_or(Value::Null),
        other => serde_json::to_value(other).unwrap_or(Value::Null),
    }
}

pub fn emit_json(data_ok: bool, data: Option<Value>, error: Option<ApiError>) {
    let envelope: Envelope<Value> = Envelope {
        ok: data_ok,
        data,
        error,
    };
    match serde_json::to_string(&envelope) {
        Ok(encoded) => crate::say!("{encoded}"),
        Err(_) => crate::say!(
            r#"{{"ok":false,"error":{{"code":"internal","message":"serialization failed"}}}}"#
        ),
    }
}

pub fn print_failure(failure: &Failure, json: bool) {
    if json {
        emit_json(false, None, Some(failure.api_error()));
    } else {
        eprintln!("error: {}", failure.message);
    }
}

fn print_status(state: &PlaybackState) {
    let track = state.track.as_ref().map_or_else(
        || "nothing playing".to_owned(),
        |track| format!("{} — {}", track.name, track.artist_names()),
    );
    crate::say!(
        "{}  {}  [{}/{}]  vol {:.0}%  repeat {}  queue {}",
        icon(state),
        track,
        format_ms(state.position_ms),
        state
            .track
            .as_ref()
            .map_or("--:--".to_owned(), |track| format_ms(track.duration_ms)),
        state.effective_volume() * 100.0,
        state.repeat.as_str(),
        state.queue_length,
    );
}

fn icon(state: &PlaybackState) -> &'static str {
    match state.status {
        lekima_domain::PlaybackStatus::Playing => "▶",
        lekima_domain::PlaybackStatus::Paused => "⏸",
        lekima_domain::PlaybackStatus::Stopped => "■",
    }
}

fn print_tracks(tracks: &[Track]) {
    for (index, track) in tracks.iter().enumerate() {
        crate::say!(
            "{:>4}  {:<40} {:<22} {:>6}  {}",
            index + 1,
            truncate(&track.name, 40),
            truncate(&track.artist_names(), 22),
            track.duration_label(),
            track.id
        );
    }
    if tracks.is_empty() {
        crate::say!("no tracks");
    }
}

fn print_search(results: &SearchResults) {
    print_tracks(&results.tracks);
}

fn print_playlists(playlists: &[Playlist]) {
    for playlist in playlists {
        crate::say!(
            "{:>10}  {:<40} {}",
            playlist.id,
            truncate(&playlist.name, 40),
            playlist.track_count
        );
    }
    if playlists.is_empty() {
        crate::say!("no playlists");
    }
}

fn print_queue(tracks: &[Track], index: Option<usize>) {
    for (position, track) in tracks.iter().enumerate() {
        let marker = if Some(position) == index { "▶" } else { " " };
        crate::say!(
            "{marker} {:>3}  {} — {}",
            position + 1,
            track.name,
            track.artist_names()
        );
    }
}

fn format_ms(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}/{:02}", seconds / 60, seconds % 60)
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    let mut truncated: String = value.chars().take(width.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}
