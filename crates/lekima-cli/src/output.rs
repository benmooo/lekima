//! Output and process-exit policy.
//!
//! Human output is for people; `--json` is the stable agent contract:
//!
//! ```json
//! {"ok":true,"data":{…}}
//! {"ok":false,"error":{"code":"unauthorized","message":"…"}}
//! ```

use lekima_domain::{
    Album, AlbumDetail, ArtistAlbums, ArtistDetail, PlaybackState, Playlist, SearchResults, Track,
    sanitize_line, sanitize_multiline,
};
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
        Reply::Search { results, .. } => print_search(results),
        Reply::Artist { artist } => print_artist(artist),
        Reply::ArtistAlbums { page } => print_artist_albums(page),
        Reply::Album { detail } => print_album(detail),
        Reply::Playlists { playlists } => print_playlists(playlists),
        Reply::Queue { tracks, index } => print_queue(tracks, *index),
        Reply::History { entries } => print_history(entries),
        Reply::Hooks { hooks } => print_hooks(hooks),
        Reply::User { user } => match user {
            Some(user) => crate::say!("{} ({})", sanitize_line(&user.nickname), user.id),
            None => crate::say!("not signed in"),
        },
        Reply::Lyrics { lyrics, .. } => crate::say!("{}", lyrics.plain()),
        Reply::Comments { comments, .. } => print_comments(comments),
        Reply::Toplists { toplists } => {
            for chart in toplists {
                crate::say!(
                    "{:>10}  {:<24} {:>5}  {}",
                    chart.id,
                    truncate(&sanitize_line(&chart.name), 24),
                    chart.track_count,
                    sanitize_line(&chart.update_frequency)
                );
            }
        }
        Reply::Like { id, liked } => {
            crate::say!("{} {id}", if *liked { "♥ liked" } else { "♡ unliked" });
        }
        Reply::Ack | Reply::Pong | Reply::Welcome { .. } | Reply::Subscribed { .. } => {}
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
        eprintln!("error: {}", sanitize_multiline(&failure.message));
    }
}

fn print_status(state: &PlaybackState) {
    let preview = state.track.as_ref().is_some_and(|track| {
        state
            .stream_duration_ms
            .is_some_and(|stream| stream > 0 && stream.saturating_add(2_000) < track.duration_ms)
    });
    let track = state.track.as_ref().map_or_else(
        || "nothing playing".to_owned(),
        |track| {
            format!(
                "{} — {}{}{}",
                sanitize_line(&track.name),
                sanitize_line(&track.artist_names()),
                if preview { "  [PREVIEW]" } else { "" },
                if state.cached { "  [CACHE]" } else { "" }
            )
        },
    );
    let duration = state.track.as_ref().map_or("--:--".to_owned(), |track| {
        let duration = if preview {
            state.stream_duration_ms.unwrap_or(track.duration_ms)
        } else {
            track.duration_ms
        };
        format_clock(duration)
    });
    let sleep = state
        .sleep_remaining_ms
        .map_or_else(String::new, |ms| format!("  sleep {}", format_clock(ms)));
    crate::say!(
        "{}  {}  [{}/{}]  vol {:.0}%  repeat {}  queue {}  {}{}",
        icon(state),
        track,
        format_clock(state.position_ms),
        duration,
        state.effective_volume() * 100.0,
        state.repeat.as_str(),
        state.queue_length,
        state.quality.as_str(),
        sleep,
    );
}

fn icon(state: &PlaybackState) -> &'static str {
    match state.status {
        lekima_domain::PlaybackStatus::Playing => "▶",
        lekima_domain::PlaybackStatus::Paused => "⏸",
        lekima_domain::PlaybackStatus::Stopped => "■",
    }
}

pub fn print_comments(comments: &lekima_domain::Comments) {
    let mut printed = false;
    for (label, list) in [("hot", &comments.hot), ("all", &comments.comments)] {
        if list.is_empty() {
            continue;
        }
        printed = true;
        crate::say!("// {label} · {}", list.len());
        for comment in list {
            let author = if comment.author.is_empty() {
                "anonymous"
            } else {
                comment.author.as_str()
            };
            crate::say!(
                "{:>5}  {}: {}",
                comment.liked_count,
                sanitize_line(author),
                sanitize_line(&comment.content)
            );
            for reply in &comment.replies {
                crate::say!(
                    "       ↳ {}: {}",
                    sanitize_line(&reply.author),
                    sanitize_line(&reply.content)
                );
            }
        }
    }
    if !printed {
        crate::say!("no comments");
    }
    if comments.has_more {
        crate::say!("more available · use --offset");
    }
}

fn print_tracks(tracks: &[Track]) {
    for (index, track) in tracks.iter().enumerate() {
        crate::say!(
            "{:>4}  {:<40} {:<22} {:>6}  {}",
            index + 1,
            truncate(&sanitize_line(&track.name), 40),
            truncate(&sanitize_line(&track.artist_names()), 22),
            track.duration_label(),
            track.id
        );
    }
    if tracks.is_empty() {
        crate::say!("no tracks");
    }
}

fn print_search(results: &SearchResults) {
    if results.is_empty() {
        crate::say!("no matches");
        return;
    }
    crate::say!("// SEARCH · {} RESULTS", results.total);
    if !results.tracks.is_empty() {
        crate::say!("// TRACKS · {}", results.tracks.len());
        print_tracks(&results.tracks);
    }
    if !results.albums.is_empty() {
        crate::say!("// ALBUMS · {}", results.totals.albums);
        print_albums(&results.albums);
    }
    if !results.artists.is_empty() {
        crate::say!("// ARTISTS · {}", results.artists.len());
        for artist in &results.artists {
            crate::say!(
                "{:>10}  {}",
                artist.id,
                truncate(&sanitize_line(&artist.name), 40)
            );
        }
    }
    if !results.playlists.is_empty() {
        crate::say!("// PLAYLISTS · {}", results.playlists.len());
        print_playlists(&results.playlists);
    }
}

fn print_albums(albums: &[Album]) {
    for album in albums {
        crate::say!(
            "{:>10}  {:<40} {:<22} {:>4}  {}",
            album.id,
            truncate(&sanitize_line(&album.name), 40),
            truncate(&sanitize_line(album.artist_name()), 22),
            album
                .year
                .map_or_else(|| "—".to_owned(), |year| year.to_string()),
            album.track_count.map_or_else(
                || "—".to_owned(),
                |count| format!("{count} {}", plural(count as usize)),
            )
        );
    }
    if albums.is_empty() {
        crate::say!("no albums");
    }
}

fn print_artist(artist: &ArtistDetail) {
    crate::say!(
        "// ARTIST · {} · {}",
        artist.id,
        sanitize_line(&artist.name)
    );
    crate::say!(
        "   {} albums · {} tracks",
        artist.album_count,
        artist.track_count
    );
    if !artist.aliases.is_empty() {
        crate::say!("   {}", sanitize_line(&artist.aliases.join(" · ")));
    }
    if !artist.description.is_empty() {
        // Biographies run to paragraphs; two wrapped lines are enough here.
        for line in wrap_text(&sanitize_line(&artist.description), 96)
            .into_iter()
            .take(2)
        {
            crate::say!("   {line}");
        }
    }
}

fn print_artist_albums(page: &ArtistAlbums) {
    print_albums(&page.albums);
    if page.has_more {
        crate::say!("more available · use --offset");
    }
}

fn print_album(detail: &AlbumDetail) {
    let album = &detail.album;
    crate::say!("// ALBUM · {} · {}", album.id, sanitize_line(&album.name));
    let artist = sanitize_line(album.artist_name());
    let artist = if artist.is_empty() { "—" } else { &artist };
    let year = album
        .year
        .map_or_else(|| "—".to_owned(), |year| year.to_string());
    let tracks = album
        .track_count
        .map_or(detail.tracks.len(), |count| count as usize);
    crate::say!("   {artist} · {year} · {tracks} {}", plural(tracks));
    if !album.description.is_empty() {
        for line in wrap_text(&sanitize_line(&album.description), 96)
            .into_iter()
            .take(2)
        {
            crate::say!("   {line}");
        }
    }
    print_tracks(&detail.tracks);
}

fn print_playlists(playlists: &[Playlist]) {
    for playlist in playlists {
        crate::say!(
            "{:>10}  {:<40} {}",
            playlist.id,
            truncate(&sanitize_line(&playlist.name), 40),
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
            sanitize_line(&track.name),
            sanitize_line(&track.artist_names())
        );
    }
}

fn print_hooks(hooks: &[lekima_ipc::HookStatus]) {
    if hooks.is_empty() {
        crate::say!("no hooks configured · add a [hooks.<name>] table to config.toml");
        return;
    }
    for hook in hooks {
        let state = match (&hook.error, hook.running) {
            (Some(_), _) => "invalid",
            (None, true) => "running",
            (None, false) => "stopped",
        };
        crate::say!(
            "{:<9} {:<20} events: {}  commands: {}",
            state,
            sanitize_line(&hook.name),
            hook.events.join(","),
            if hook.capabilities.is_empty() {
                "read-only".to_owned()
            } else {
                hook.capabilities.join(",")
            }
        );
        if let Some(error) = &hook.error {
            crate::say!("          {}", sanitize_line(error));
        }
    }
}

fn print_history(entries: &[lekima_domain::PlayRecord]) {
    if entries.is_empty() {
        crate::say!("no plays yet · start with `lekima play <query>`");
        return;
    }
    let now = now_ms();
    for entry in entries {
        crate::say!(
            "{:>8}  {} — {}  {:>5}  {}",
            format_age(entry.started_at, now),
            sanitize_line(&entry.track.name),
            sanitize_line(&entry.track.artist_names()),
            entry.played_label(),
            if entry.completed { "✓" } else { " " }
        );
    }
}

/// Relative age of a Unix-millisecond timestamp.
///
/// Relative rather than absolute so the output needs no timezone database and
/// stays readable in a terminal; agents read `started_at` from `--json`.
fn format_age(started_at_ms: i64, now_ms: i64) -> String {
    let elapsed = now_ms.saturating_sub(started_at_ms);
    if elapsed < 60_000 {
        "just now".to_owned()
    } else if elapsed < 3_600_000 {
        format!("{}m ago", elapsed / 60_000)
    } else if elapsed < 86_400_000 {
        format!("{}h ago", elapsed / 3_600_000)
    } else {
        format!("{}d ago", elapsed / 86_400_000)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

fn format_clock(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// Greedy wrap so long provider prose stays readable in a terminal.
///
/// Widths are terminal columns, not characters, and long unspaced runs (CJK
/// prose) break mid-run instead of overflowing the line.
fn wrap_text(value: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;

    let width = width.max(8);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;
    let mut last_space: Option<usize> = None;

    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if line_width + character_width > width {
            // Only rewind to a space when it leaves a reasonably full line;
            // otherwise a lone word before a long CJK run would sit alone.
            let space_break = last_space.filter(|index| {
                let prefix: usize = line[..*index].chars().map(|c| c.width().unwrap_or(0)).sum();
                prefix + 1 >= width / 2
            });
            if let Some(index) = space_break {
                let rest = line[index..].trim_start().to_owned();
                line.truncate(index);
                lines.push(line.trim_end().to_owned());
                line = rest;
                line_width = line.chars().map(|c| c.width().unwrap_or(0)).sum();
            } else {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            last_space = line.rfind(' ');
        }
        if character == ' ' {
            last_space = Some(line.len());
        }
        line.push(character);
        line_width += character_width;
    }
    if !line.trim().is_empty() {
        lines.push(line.trim_end().to_owned());
    }
    lines
}

/// `track`/`tracks` so counts read naturally.
fn plural(count: usize) -> &'static str {
    if count == 1 { "track" } else { "tracks" }
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    let mut truncated: String = value.chars().take(width.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::wrap_text;

    #[test]
    fn wraps_ascii_and_cjk_prose_by_display_width() {
        assert_eq!(
            wrap_text("alpha beta gamma", 11),
            vec!["alpha beta", "gamma"]
        );
        let lines = wrap_text("周杰伦（Jay Chou）是一位歌手", 10);
        assert!(
            lines.iter().all(|line| line.width() <= 10),
            "{lines:?} must fit 10 columns"
        );
        assert_eq!(
            lines.concat().replace(' ', ""),
            "周杰伦（JayChou）是一位歌手"
        );
    }
}
