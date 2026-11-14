//! Ports implemented by infrastructure adapters.
//!
//! The core depends only on these traits. The NetEase adapter, the rodio audio
//! backend and the file-system credential store are swappable, and tests can
//! supply in-memory implementations.

use async_trait::async_trait;

use crate::{
    Result,
    model::{Quality, SearchKind, SearchResults, Track, TrackId},
    playback::PlaybackState,
};

/// Catalogue and account operations backed by a music provider.
#[async_trait]
pub trait MusicService: Send + Sync {
    /// Searches the catalogue.
    async fn search(
        &self,
        query: &str,
        kind: SearchKind,
        limit: u32,
        offset: u32,
    ) -> Result<SearchResults>;

    /// Fetches metadata for a batch of tracks, preserving request order where
    /// the provider allows it.
    async fn tracks(&self, ids: &[TrackId]) -> Result<Vec<Track>>;

    /// Single-track convenience wrapper.
    async fn track(&self, id: TrackId) -> Result<Track> {
        self.tracks(&[id])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| crate::DomainError::NotFound(format!("track {id}")))
    }

    /// Resolves a playable URL for a track.
    async fn stream_url(&self, id: TrackId, quality: Quality) -> Result<String>;

    /// The signed-in user's playlists, or another user's when supplied.
    async fn playlists(
        &self,
        user: Option<crate::model::UserId>,
    ) -> Result<Vec<crate::model::Playlist>>;

    /// Tracks of a playlist. Providers that only expose playlist metadata
    /// return [`crate::DomainError::Unsupported`].
    async fn playlist_tracks(&self, id: crate::model::PlaylistId) -> Result<Vec<Track>>;

    /// The account currently authenticated against the provider, if any.
    async fn current_user(&self) -> Result<Option<crate::model::UserProfile>>;

    /// Plain-text lyrics, when the provider exposes them.
    async fn lyrics(&self, id: TrackId) -> Result<String>;
}

/// Decides what to play and how the device reports progress.
#[async_trait]
pub trait AudioEngine: Send + Sync {
    /// Starts playback of a remote stream, replacing whatever was playing.
    async fn play_url(&self, url: &str) -> Result<()>;

    fn pause(&self) -> Result<()>;

    fn resume(&self) -> Result<()>;

    fn stop(&self) -> Result<()>;

    /// Seeks within the current track.
    fn seek(&self, position_ms: u64) -> Result<()>;

    /// Sets linear gain in `0.0..=1.0`.
    fn set_volume(&self, gain: f32) -> Result<()>;

    /// Current position of the playing track.
    fn position_ms(&self) -> u64;

    /// True when the current track has played to completion.
    fn is_finished(&self) -> bool;

    /// True while a track is loaded (playing or paused).
    fn has_track(&self) -> bool;
}

/// Extension point for `status` output in frontends that do not own the daemon.
pub type StateSnapshot = PlaybackState;

/// Persists the provider session (cookies) between runs.
pub trait CredentialsStore: Send + Sync {
    fn load(&self) -> Result<Option<String>>;

    fn save(&self, cookie: &str) -> Result<()>;

    fn clear(&self) -> Result<()>;
}
