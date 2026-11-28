#![forbid(unsafe_code)]
//! The pure heart of lekima.
//!
//! This crate has no I/O, no runtime and no knowledge of HTTP, audio devices or
//! terminals. It defines the vocabulary shared by every adapter:
//!
//! - [`model`] — catalogue entities (tracks, playlists, users, …)
//! - [`playback`] — the queue and the observable playback state
//! - [`ports`] — the traits adapters must implement
//!
//! Keeping this layer dependency-free is what allows the core rules (queue
//! semantics, repeat/shuffle behaviour, state transitions) to be tested in
//! microseconds and reused by the CLI, the TUI and any future frontend.

pub mod error;
pub mod history;
pub mod lyrics;
pub mod model;
pub mod playback;
pub mod ports;
pub mod sanitize;

pub use error::{DomainError, Result};
pub use history::{COMPLETION_PERCENT, HistoryLog, PlayRecord, is_completed};
pub use lyrics::{LyricLine, Lyrics, parse_lrc, parse_lrc_with_translation};
pub use model::{
    Album, AlbumDetail, AlbumId, Artist, ArtistAlbums, ArtistDetail, ArtistId, Comment,
    CommentReply, Comments, Playlist, PlaylistId, QrLogin, QrStatus, Quality, SearchKind,
    SearchResults, SearchTotals, Toplist, Track, TrackId, UserId, UserProfile, format_duration_ms,
};
pub use playback::{BufferedRange, PlaybackState, PlaybackStatus, Queue, QueueKind, RepeatMode};
pub use ports::{AudioDeviceEvent, AudioEngine, CredentialsStore, MusicService};
pub use sanitize::{sanitize_line, sanitize_multiline};
