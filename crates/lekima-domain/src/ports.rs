//! Ports implemented by infrastructure adapters.
//!
//! The core depends only on these traits. The NetEase adapter, the rodio audio
//! backend and the file-system credential store are swappable, and tests can
//! supply in-memory implementations.

use async_trait::async_trait;

use crate::{
    Result,
    lyrics::Lyrics,
    model::{QrLogin, QrStatus, Quality, SearchKind, SearchResults, Toplist, Track, TrackId},
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

    /// Timed lyrics, when the provider exposes them.
    async fn lyrics(&self, id: TrackId) -> Result<Lyrics> {
        let _ = id;
        Err(crate::DomainError::Unsupported(
            "lyrics are not supported by this provider".to_owned(),
        ))
    }

    /// Daily recommendations for the signed-in account.
    async fn daily(&self) -> Result<Vec<Track>> {
        Err(crate::DomainError::Unsupported(
            "daily recommendations are not supported by this provider".to_owned(),
        ))
    }

    /// Official chart list.
    async fn toplists(&self) -> Result<Vec<Toplist>> {
        Err(crate::DomainError::Unsupported(
            "toplists are not supported by this provider".to_owned(),
        ))
    }

    /// Personal FM batch.
    async fn fm_batch(&self) -> Result<Vec<Track>> {
        Err(crate::DomainError::Unsupported(
            "personal FM is not supported by this provider".to_owned(),
        ))
    }

    /// Removes a track from personal FM.
    async fn fm_trash(&self, id: TrackId) -> Result<()> {
        let _ = id;
        Err(crate::DomainError::Unsupported(
            "personal FM is not supported by this provider".to_owned(),
        ))
    }

    /// Idempotent like/unlike.
    async fn set_like(&self, id: TrackId, liked: bool) -> Result<()> {
        let _ = (id, liked);
        Err(crate::DomainError::Unsupported(
            "likes are not supported by this provider".to_owned(),
        ))
    }

    /// Every liked track id for the signed-in account.
    async fn liked_ids(&self) -> Result<Vec<TrackId>> {
        Err(crate::DomainError::Unsupported(
            "likes are not supported by this provider".to_owned(),
        ))
    }

    /// The account's music cloud (uploaded tracks).
    async fn cloud(&self, limit: u32, offset: u32) -> Result<Vec<Track>> {
        let _ = (limit, offset);
        Err(crate::DomainError::Unsupported(
            "the music cloud is not supported by this provider".to_owned(),
        ))
    }

    /// Starts a QR login flow.
    ///
    /// Providers without QR support report
    /// [`crate::DomainError::Unsupported`]; the default implementation does
    /// exactly that so adapters only implement what they can do.
    async fn qr_start(&self) -> Result<QrLogin> {
        Err(crate::DomainError::Unsupported(
            "QR login is not supported by this provider".to_owned(),
        ))
    }

    /// Polls a QR login started by [`MusicService::qr_start`].
    async fn qr_poll(&self, key: &str) -> Result<QrStatus> {
        let _ = key;
        Err(crate::DomainError::Unsupported(
            "QR login is not supported by this provider".to_owned(),
        ))
    }
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

    /// Recent amplitude envelope, oldest first, each value in `0.0..=1.0`.
    ///
    /// Adapters that cannot sample the output return an empty vector.
    fn wave(&self) -> Vec<f32> {
        Vec::new()
    }

    /// True while the decoder is waiting for network data mid-track.
    fn is_buffering(&self) -> bool {
        false
    }

    /// Prepares a source for the next track and appends it to the same output
    /// queue, so the transition can be gapless.
    async fn queue_next_url(&self, url: &str) -> Result<()> {
        let _ = url;
        Err(crate::DomainError::Unsupported(
            "this audio backend cannot preload tracks".to_owned(),
        ))
    }

    /// True exactly once when a preloaded track has started playing.
    fn take_track_boundary(&self) -> bool {
        false
    }
}

/// Extension point for `status` output in frontends that do not own the daemon.
pub type StateSnapshot = PlaybackState;

/// Persists the provider session (cookies) between runs.
pub trait CredentialsStore: Send + Sync {
    fn load(&self) -> Result<Option<String>>;

    fn save(&self, cookie: &str) -> Result<()>;

    fn clear(&self) -> Result<()>;
}
