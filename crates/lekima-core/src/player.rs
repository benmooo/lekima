//! Playback orchestration.

use std::{collections::HashSet, sync::Arc};

use lekima_domain::{
    AudioEngine, DomainError, MusicService, PlaybackState, PlaybackStatus, Quality, Queue,
    QueueKind, RepeatMode, Result, Track, TrackId,
};
use tokio::sync::Mutex;

/// What should be loaded into the queue.
#[derive(Debug, Clone)]
pub enum PlayRequest {
    Tracks {
        tracks: Vec<Track>,
        kind: QueueKind,
    },
    /// Jump to an existing queue position.
    Index(usize),
}

struct Inner {
    queue: Queue,
    status: PlaybackStatus,
    volume: f32,
    muted: bool,
    repeat: RepeatMode,
    shuffle: bool,
    position_ms: u64,
    kind: QueueKind,
    /// Incremented for every load; stale loads stop themselves.
    generation: u64,
    loading: bool,
}

/// Owns queue and transport state for the daemon.
pub struct Player {
    music: Arc<dyn MusicService>,
    audio: Arc<dyn AudioEngine>,
    quality: Mutex<Quality>,
    inner: Mutex<Inner>,
    liked: Mutex<HashSet<TrackId>>,
    trashed: Mutex<HashSet<TrackId>>,
    /// Track id currently appended to the audio device as a gapless preload.
    preloaded: Mutex<Option<TrackId>>,
}

impl Player {
    #[must_use]
    pub fn new(
        music: Arc<dyn MusicService>,
        audio: Arc<dyn AudioEngine>,
        quality: Quality,
        volume: f32,
    ) -> Self {
        let volume = volume.clamp(0.0, 1.0);
        let _ = audio.set_volume(volume);
        Self {
            music,
            audio,
            quality: Mutex::new(quality),
            inner: Mutex::new(Inner {
                queue: Queue::default(),
                status: PlaybackStatus::Stopped,
                volume,
                muted: false,
                repeat: RepeatMode::Off,
                shuffle: false,
                position_ms: 0,
                kind: QueueKind::Manual,
                generation: 0,
                loading: false,
            }),
            liked: Mutex::new(HashSet::new()),
            trashed: Mutex::new(HashSet::new()),
            preloaded: Mutex::new(None),
        }
    }

    #[must_use]
    pub fn music(&self) -> &Arc<dyn MusicService> {
        &self.music
    }

    /// Immutable view of the current state.
    pub async fn snapshot(&self) -> PlaybackState {
        // Lock order: liked, then inner. No other path takes them in reverse.
        let liked = self.liked.lock().await;
        let inner = self.inner.lock().await;
        let duration_ms = inner.queue.current().map_or(0, |track| track.duration_ms);
        PlaybackState {
            status: inner.status,
            track: inner.queue.current().cloned(),
            // The device is the source of truth while a track is loaded; `max`
            // here made backward seeks look like no-ops.
            position_ms: if inner.status == PlaybackStatus::Stopped {
                0
            } else if self.audio.has_track() {
                self.audio.position_ms()
            } else {
                inner.position_ms
            },
            volume: inner.volume,
            muted: inner.muted,
            repeat: inner.repeat,
            shuffle: inner.shuffle,
            queue_length: inner.queue.len(),
            queue_index: inner.queue.current_index(),
            queue_kind: inner.kind,
            liked: inner
                .queue
                .current()
                .is_some_and(|track| liked.contains(&track.id)),
            loading: inner.loading,
            buffering: self.audio.is_buffering(),
            quality: *self.quality.lock().await,
            sleep_remaining_ms: None,
            wave: self.audio.wave(),
            stream_duration_ms: self.audio.stream_duration_ms(),
            buffered: self.audio.buffered_ranges(duration_ms),
            revision: 0,
        }
    }

    /// Replaces the queue and starts the first track.
    pub async fn play(&self, request: PlayRequest) -> Result<PlaybackState> {
        {
            let mut inner = self.inner.lock().await;
            match request {
                PlayRequest::Tracks { tracks, kind } => {
                    if tracks.is_empty() {
                        return Err(DomainError::InvalidInput(
                            "nothing to play: queue would be empty".to_owned(),
                        ));
                    }
                    inner.queue.replace(tracks);
                    inner.kind = kind;
                }
                PlayRequest::Index(index) => {
                    if inner.queue.jump(index).is_none() {
                        return Err(DomainError::NotFound(format!("queue index {index}")));
                    }
                }
            }
        }
        self.load_current().await?;
        Ok(self.snapshot().await)
    }

    /// Appends tracks, optionally as "play next".
    pub async fn enqueue(&self, tracks: Vec<Track>, next: bool) -> Result<()> {
        if tracks.is_empty() {
            return Err(DomainError::InvalidInput("no tracks to enqueue".to_owned()));
        }
        let should_start = {
            let mut inner = self.inner.lock().await;
            let was_idle = inner.queue.is_empty();
            if next {
                for track in tracks.into_iter().rev() {
                    inner.queue.insert_next(track);
                }
            } else {
                inner.queue.extend(tracks);
            }
            was_idle
        };
        if should_start {
            self.load_current().await?;
        }
        Ok(())
    }

    pub async fn queue(&self) -> (Vec<Track>, Option<usize>) {
        let inner = self.inner.lock().await;
        (inner.queue.tracks().to_vec(), inner.queue.current_index())
    }

    /// Removes a queued track. Removing the current one stops it and loads the
    /// following track (or stops if the queue becomes empty).
    pub async fn remove_at(&self, index: usize) -> Result<()> {
        let (was_current, has_next) = {
            let mut inner = self.inner.lock().await;
            if index >= inner.queue.len() {
                return Err(DomainError::InvalidInput(format!(
                    "queue index {index} is out of range"
                )));
            }
            let was_current = inner.queue.current_index() == Some(index);
            if was_current {
                inner.status = PlaybackStatus::Stopped;
                inner.loading = false;
                inner.generation = inner.generation.wrapping_add(1);
            }
            inner.queue.remove_at(index);
            (was_current, !inner.queue.is_empty())
        };

        if was_current {
            self.audio.stop()?;
            if has_next {
                self.load_current().await?;
            }
        }
        Ok(())
    }

    /// Moves a queued track; the currently playing track is never restarted.
    pub async fn move_item(&self, from: usize, to: usize) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if !inner.queue.move_item(from, to) {
            return Err(DomainError::InvalidInput(format!(
                "cannot move queue item {from} to {to}"
            )));
        }
        Ok(())
    }

    pub async fn clear_queue(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.queue.clear();
        inner.status = PlaybackStatus::Stopped;
        inner.position_ms = 0;
        inner.kind = QueueKind::Manual;
        inner.loading = false;
        inner.generation = inner.generation.wrapping_add(1);
        drop(inner);
        *self.preloaded.lock().await = None;
        self.audio.stop()
    }

    /// Loads the queue's current track into the audio engine.
    ///
    /// The previous track is stopped **before** resolving or downloading the
    /// next stream: silence first, then buffering. A generation counter makes
    /// rapid skips cancel superseded loads.
    async fn load_current(&self) -> Result<()> {
        let (track, generation) = {
            let mut inner = self.inner.lock().await;
            let track = inner
                .queue
                .current()
                .cloned()
                .ok_or_else(|| DomainError::NotFound("current track".to_owned()))?;
            inner.generation = inner.generation.wrapping_add(1);
            inner.loading = true;
            // Do not show the previous track's position while the next URL is
            // resolved; otherwise B briefly reports A's 0:30 and then snaps to 0.
            inner.position_ms = 0;
            (track, inner.generation)
        };

        // A must go silent now, not after B has been fetched.
        *self.preloaded.lock().await = None;
        let _ = self.audio.stop();

        let quality = *self.quality.lock().await;
        let url = match self.music.stream_url(track.id, quality).await {
            Ok(url) => url,
            Err(error) => {
                self.fail_load(generation).await;
                return Err(error);
            }
        };
        if self.is_stale(generation).await {
            return Ok(());
        }

        if let Err(error) = self.audio.play_url(&url).await {
            self.fail_load(generation).await;
            return Err(error);
        }

        let mut inner = self.inner.lock().await;
        if inner.generation != generation {
            drop(inner);
            let _ = self.audio.stop();
            return Ok(());
        }
        inner.loading = false;
        if inner.status == PlaybackStatus::Paused {
            let _ = self.audio.pause();
        } else {
            inner.status = PlaybackStatus::Playing;
            inner.position_ms = 0;
            let gain = if inner.muted { 0.0 } else { inner.volume };
            let _ = self.audio.set_volume(gain);
        }
        tracing::debug!(track = %track.id, "playback started");
        Ok(())
    }

    async fn is_stale(&self, generation: u64) -> bool {
        self.inner.lock().await.generation != generation
    }

    async fn fail_load(&self, generation: u64) {
        let mut inner = self.inner.lock().await;
        if inner.generation == generation {
            inner.status = PlaybackStatus::Stopped;
            inner.loading = false;
            inner.position_ms = 0;
        }
    }

    pub async fn pause(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.status == PlaybackStatus::Playing {
            self.audio.pause()?;
            inner.status = PlaybackStatus::Paused;
            inner.position_ms = self.audio.position_ms();
        }
        Ok(())
    }

    pub async fn resume(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.status == PlaybackStatus::Paused {
            self.audio.resume()?;
            inner.status = PlaybackStatus::Playing;
        } else if inner.status == PlaybackStatus::Stopped && !inner.queue.is_empty() {
            drop(inner);
            return self.load_current().await;
        }
        Ok(())
    }

    pub async fn toggle(&self) -> Result<()> {
        match self.snapshot().await.status {
            PlaybackStatus::Playing => self.pause().await,
            PlaybackStatus::Paused | PlaybackStatus::Stopped => self.resume().await,
        }
    }

    pub async fn stop(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        self.audio.stop()?;
        *self.preloaded.lock().await = None;
        inner.status = PlaybackStatus::Stopped;
        inner.position_ms = 0;
        inner.loading = false;
        inner.generation = inner.generation.wrapping_add(1);
        Ok(())
    }

    /// Advances to the next track. Returns `None` at the end of the queue.
    pub async fn next(&self) -> Result<Option<Track>> {
        let advanced = {
            let mut inner = self.inner.lock().await;
            let repeat = effective_repeat(inner.kind, inner.repeat);
            inner.queue.advance(repeat).cloned()
        };
        match advanced {
            Some(track) => {
                self.load_current().await?;
                Ok(Some(track))
            }
            None => {
                self.stop().await?;
                Ok(None)
            }
        }
    }

    pub async fn previous(&self) -> Result<Option<Track>> {
        {
            let mut inner = self.inner.lock().await;
            let repeat = effective_repeat(inner.kind, inner.repeat);
            inner.queue.rewind(repeat);
        }
        self.load_current().await?;
        Ok(self.snapshot().await.track)
    }

    pub async fn seek(&self, position_ms: u64) -> Result<()> {
        self.audio.seek(position_ms)?;
        let mut inner = self.inner.lock().await;
        inner.position_ms = position_ms;
        Ok(())
    }

    pub async fn set_volume(&self, level: f32) -> Result<()> {
        let level = level.clamp(0.0, 1.0);
        let mut inner = self.inner.lock().await;
        inner.volume = level;
        let gain = if inner.muted { 0.0 } else { level };
        self.audio.set_volume(gain)?;
        Ok(())
    }

    pub async fn toggle_mute(&self) -> Result<bool> {
        let mut inner = self.inner.lock().await;
        inner.muted = !inner.muted;
        let gain = if inner.muted { 0.0 } else { inner.volume };
        self.audio.set_volume(gain)?;
        Ok(inner.muted)
    }

    pub async fn cycle_repeat(&self) -> RepeatMode {
        let mut inner = self.inner.lock().await;
        inner.repeat = inner.repeat.cycle();
        inner.repeat
    }

    pub async fn set_repeat(&self, mode: RepeatMode) {
        self.inner.lock().await.repeat = mode;
    }

    pub async fn set_shuffle(&self, enabled: bool) -> Result<()> {
        self.inner.lock().await.shuffle = enabled;
        Ok(())
    }

    /// Called periodically by the daemon to detect the end of a track.
    ///
    /// Returns the newly started track when playback rolled over.
    pub async fn tick(&self) -> Result<Option<Track>> {
        let (finished, playing) = {
            let inner = self.inner.lock().await;
            // `has_track` is already false when the sink reports finished, so it
            // must not gate the advance. `loading` does: a second tick while the
            // next stream is being resolved must not skip another queue entry.
            (
                self.audio.is_finished(),
                inner.status == PlaybackStatus::Playing
                    && !inner.loading
                    && inner.queue.current().is_some(),
            )
        };
        if finished && playing {
            tracing::debug!("track ended, advancing");
            return self.next().await;
        }
        Ok(None)
    }

    pub async fn remove_current(&self) -> Result<()> {
        {
            let mut inner = self.inner.lock().await;
            inner.queue.remove_current();
        }
        if self.inner.lock().await.queue.is_empty() {
            self.stop().await?;
        } else {
            self.load_current().await?;
        }
        Ok(())
    }

    /// The track after the current one, if the queue has one.
    pub async fn next_track(&self) -> Option<Track> {
        let inner = self.inner.lock().await;
        let current = inner.queue.current_index()?;
        inner.queue.get(current + 1).cloned()
    }

    pub async fn repeat_mode(&self) -> RepeatMode {
        self.inner.lock().await.repeat
    }

    /// True once when a preloaded source has begun playing.
    pub fn take_track_boundary(&self) -> Option<TrackId> {
        self.audio.take_track_boundary()
    }

    /// Prepares and appends the next source to the audio queue.
    ///
    /// Returns `true` when a new source was queued. A second call for the same
    /// queue position is a no-op, so transport commands such as volume changes
    /// cannot enqueue the next track twice.
    pub async fn queue_next(&self, id: TrackId, url: &str) -> Result<bool> {
        let (generation, next_id) = {
            let inner = self.inner.lock().await;
            let next_id = inner
                .queue
                .current_index()
                .and_then(|index| inner.queue.get(index + 1))
                .map(|track| track.id);
            (inner.generation, next_id)
        };
        if next_id != Some(id) {
            return Ok(false);
        }
        {
            let mut preloaded = self.preloaded.lock().await;
            if preloaded.is_some() {
                return Ok(false);
            }
            // Reserve the slot before the network/appending work. A concurrent
            // load clears it, and the generation check below detects that.
            *preloaded = Some(id);
        }

        if let Err(error) = self.audio.queue_next_url(id, url).await {
            let mut preloaded = self.preloaded.lock().await;
            if *preloaded == Some(id) {
                *preloaded = None;
            }
            return Err(error);
        }

        let current = {
            let inner = self.inner.lock().await;
            inner.generation == generation
                && inner
                    .queue
                    .current_index()
                    .and_then(|index| inner.queue.get(index + 1))
                    .map(|track| track.id)
                    == Some(id)
        };
        if !current {
            let mut preloaded = self.preloaded.lock().await;
            if *preloaded == Some(id) {
                *preloaded = None;
            }
            return Ok(false);
        }
        Ok(true)
    }

    /// Advances the queue because `boundary` has started on the audio device.
    ///
    /// - Boundary equal to the current track: a manual load/jump already applied
    ///   it, so nothing advances (this prevents a double skip when `n` and the
    ///   natural rollover race).
    /// - Boundary equal to the queue's next track: gapless advance.
    /// - Anything else is stale: stop the stale audio and continue with the
    ///   queue's next entry instead of replaying the track that just ended.
    pub async fn advance_gapless(&self, boundary: TrackId) -> Result<Option<Track>> {
        let (current, expected, kind, repeat, loading) = {
            let inner = self.inner.lock().await;
            let current = inner.queue.current().map(|track| track.id);
            let expected = inner
                .queue
                .current_index()
                .and_then(|index| inner.queue.get(index + 1))
                .map(|track| track.id);
            (current, expected, inner.kind, inner.repeat, inner.loading)
        };
        let repeat = effective_repeat(kind, repeat);
        if loading {
            let _ = self.preloaded.lock().await.take();
            return Ok(None);
        }

        if current == Some(boundary) {
            let _ = self.preloaded.lock().await.take();
            return Ok(None);
        }

        if Some(boundary) == expected && repeat != RepeatMode::One {
            let _ = self.preloaded.lock().await.take();
            let mut inner = self.inner.lock().await;
            let still_expected = inner
                .queue
                .current_index()
                .and_then(|index| inner.queue.get(index + 1))
                .map(|track| track.id)
                == Some(boundary);
            if !still_expected {
                drop(inner);
                self.audio.stop()?;
                self.load_current().await?;
                return Ok(None);
            }
            let next = inner.queue.advance(repeat).cloned();
            if next.is_some() {
                inner.status = PlaybackStatus::Playing;
                inner.loading = false;
                inner.position_ms = 0;
            }
            return Ok(next);
        }

        // Stale boundary, or repeat-one rejecting the preload: stop the audio
        // and advance/rewind according to the repeat mode.
        let _ = self.preloaded.lock().await.take();
        self.audio.stop()?;
        let next = {
            let mut inner = self.inner.lock().await;
            let next = inner.queue.advance(repeat).cloned();
            if next.is_some() {
                inner.status = PlaybackStatus::Playing;
                inner.loading = true;
                inner.position_ms = 0;
            }
            next
        };
        match next {
            Some(track) => {
                self.load_current().await?;
                Ok(Some(track))
            }
            None => {
                self.stop().await?;
                Ok(None)
            }
        }
    }

    /// Changes the bitrate class for the next track.
    pub async fn set_quality(&self, quality: Quality) {
        *self.quality.lock().await = quality;
    }

    #[must_use]
    pub async fn quality(&self) -> Quality {
        *self.quality.lock().await
    }

    /// Daily recommendations, played immediately.
    pub async fn play_daily(&self) -> Result<PlaybackState> {
        let tracks = self.music.daily().await?;
        self.play(PlayRequest::Tracks {
            tracks,
            kind: QueueKind::Daily,
        })
        .await
    }

    /// The account's music cloud, played immediately.
    pub async fn play_cloud(&self, limit: u32, offset: u32) -> Result<PlaybackState> {
        let tracks = self.music.cloud(limit, offset).await?;
        self.play(PlayRequest::Tracks {
            tracks,
            kind: QueueKind::Cloud,
        })
        .await
    }

    /// Starts personal FM.
    pub async fn fm_start(&self) -> Result<PlaybackState> {
        let tracks = self.music.fm_batch().await?;
        self.play(PlayRequest::Tracks {
            tracks,
            kind: QueueKind::Fm,
        })
        .await
    }

    /// Advances FM, optionally trashing the current track first.
    pub async fn fm_next(&self, trash: bool) -> Result<Option<Track>> {
        let before = self.snapshot().await.track.map(|track| track.id);
        if trash {
            if let Some(track) = self.snapshot().await.track {
                self.music.fm_trash(track.id).await?;
                self.trashed.lock().await.insert(track.id);
            }
            // The network call above can take long enough for the track to end
            // and a gapless boundary to advance the queue. Do not advance a
            // second time when that happened.
            let after = self.snapshot().await.track.map(|track| track.id);
            if after != before {
                return Ok(self.snapshot().await.track);
            }
        }
        self.next().await
    }

    /// Keeps the FM station fed: fetch a batch when two tracks remain.
    ///
    /// Returns the number of tracks appended so the daemon can publish a queue
    /// event even though no client command caused the change.
    pub async fn fm_maybe_prefetch(&self) -> Result<usize> {
        let need = {
            let inner = self.inner.lock().await;
            inner.kind == QueueKind::Fm
                && inner.status != PlaybackStatus::Stopped
                && inner.queue.len() < 200
                && inner
                    .queue
                    .current_index()
                    .is_none_or(|index| inner.queue.len().saturating_sub(index + 1) <= 2)
        };
        if !need {
            return Ok(0);
        }

        let batch = self.music.fm_batch().await?;
        let existing: HashSet<TrackId> = {
            let (tracks, _) = self.queue().await;
            tracks.into_iter().map(|track| track.id).collect()
        };
        let trashed = self.trashed.lock().await.clone();
        let fresh: Vec<Track> = batch
            .into_iter()
            .filter(|track| !existing.contains(&track.id) && !trashed.contains(&track.id))
            .collect();
        if fresh.is_empty() {
            return Ok(0);
        }

        let added = fresh.len();
        let mut inner = self.inner.lock().await;
        if inner.kind == QueueKind::Fm {
            inner.queue.extend(fresh);
            Ok(added)
        } else {
            Ok(0)
        }
    }

    /// Seeds the liked cache. Failures are non-fatal (anonymous sessions).
    pub async fn load_likes(&self) {
        if let Ok(ids) = self.music.liked_ids().await {
            let mut liked = self.liked.lock().await;
            liked.clear();
            liked.extend(ids);
        }
    }

    /// Idempotent like update with an optimistic cache.
    pub async fn set_like(&self, id: TrackId, liked: bool) -> Result<bool> {
        {
            let mut cache = self.liked.lock().await;
            if liked {
                cache.insert(id);
            } else {
                cache.remove(&id);
            }
        }

        if let Err(error) = self.music.set_like(id, liked).await {
            let mut cache = self.liked.lock().await;
            if liked {
                cache.remove(&id);
            } else {
                cache.insert(id);
            }
            return Err(error);
        }
        Ok(liked)
    }

    /// Liked tracks, resolved to metadata (played or listed by the caller).
    pub async fn liked_tracks(&self) -> Result<Vec<Track>> {
        let ids = self.music.liked_ids().await?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut tracks = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(500) {
            tracks.extend(self.music.tracks(chunk).await?);
        }
        Ok(tracks)
    }

    /// Resolves track ids to full metadata, rejecting unknown ids.
    pub async fn resolve_tracks(&self, ids: &[TrackId]) -> Result<Vec<Track>> {
        let tracks = self.music.tracks(ids).await?;
        if tracks.is_empty() {
            return Err(DomainError::NotFound("no tracks matched".to_owned()));
        }
        Ok(tracks)
    }
}

/// FM is an endless station: repeat modes should not wrap a finite batch back
/// to old tracks when the station runs dry.
const fn effective_repeat(kind: QueueKind, repeat: RepeatMode) -> RepeatMode {
    if matches!(kind, QueueKind::Fm) {
        RepeatMode::Off
    } else {
        repeat
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };

    use async_trait::async_trait;
    use lekima_domain::{
        Album, Artist, AudioEngine, MusicService, PlaybackStatus, Playlist, PlaylistId, Quality,
        RepeatMode, Result, SearchKind, SearchResults, Track, TrackId, UserId, UserProfile,
    };

    use super::{PlayRequest, Player};
    use lekima_domain::QueueKind;

    fn track(id: u64) -> Track {
        Track {
            id: TrackId(id),
            name: format!("song-{id}"),
            artists: vec![Artist {
                id: 1.into(),
                name: "artist".to_owned(),
            }],
            album: Some(Album {
                id: 1.into(),
                name: "album".to_owned(),
                cover_url: None,
            }),
            duration_ms: 1_000,
        }
    }

    struct StubMusic;

    #[async_trait]
    impl MusicService for StubMusic {
        async fn search(&self, _: &str, _: SearchKind, _: u32, _: u32) -> Result<SearchResults> {
            Ok(SearchResults::default())
        }

        async fn tracks(&self, ids: &[TrackId]) -> Result<Vec<Track>> {
            Ok(ids.iter().map(|id| track(id.0)).collect())
        }

        async fn stream_url(&self, id: TrackId, _: Quality) -> Result<String> {
            Ok(format!("https://example.test/{id}.mp3"))
        }

        async fn playlists(&self, _: Option<UserId>) -> Result<Vec<Playlist>> {
            Ok(Vec::new())
        }

        async fn playlist_tracks(&self, _: PlaylistId) -> Result<Vec<Track>> {
            Ok(Vec::new())
        }

        async fn current_user(&self) -> Result<Option<UserProfile>> {
            Ok(None)
        }

        async fn lyrics(&self, _: TrackId) -> Result<lekima_domain::Lyrics> {
            Ok(lekima_domain::Lyrics::default())
        }

        async fn daily(&self) -> Result<Vec<Track>> {
            Ok(vec![track(1), track(2)])
        }

        async fn fm_batch(&self) -> Result<Vec<Track>> {
            Ok(vec![track(7), track(8)])
        }

        async fn set_like(&self, _: TrackId, _: bool) -> Result<()> {
            Ok(())
        }

        async fn liked_ids(&self) -> Result<Vec<TrackId>> {
            Ok(vec![TrackId(1)])
        }

        async fn cloud(&self, _: u32, _: u32) -> Result<Vec<Track>> {
            Ok(vec![track(11)])
        }
    }

    #[derive(Default)]
    struct StubAudio {
        playing: AtomicBool,
        finished: AtomicBool,
        position: AtomicU64,
        volume: Mutex<f32>,
        last_url: Mutex<Option<String>>,
        stops: AtomicU64,
        queued: AtomicU64,
    }

    #[async_trait]
    impl AudioEngine for StubAudio {
        async fn play_url(&self, url: &str) -> Result<()> {
            *self.last_url.lock().unwrap() = Some(url.to_owned());
            self.playing.store(true, Ordering::SeqCst);
            self.finished.store(false, Ordering::SeqCst);
            self.position.store(0, Ordering::SeqCst);
            Ok(())
        }

        fn pause(&self) -> Result<()> {
            self.playing.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn resume(&self) -> Result<()> {
            self.playing.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn stop(&self) -> Result<()> {
            self.playing.store(false, Ordering::SeqCst);
            self.finished.store(false, Ordering::SeqCst);
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn seek(&self, position_ms: u64) -> Result<()> {
            self.position.store(position_ms, Ordering::SeqCst);
            Ok(())
        }

        fn set_volume(&self, gain: f32) -> Result<()> {
            *self.volume.lock().unwrap() = gain;
            Ok(())
        }

        fn position_ms(&self) -> u64 {
            self.position.load(Ordering::SeqCst)
        }

        fn is_finished(&self) -> bool {
            self.finished.load(Ordering::SeqCst)
        }

        fn has_track(&self) -> bool {
            self.playing.load(Ordering::SeqCst)
        }

        async fn queue_next_url(&self, _id: TrackId, _url: &str) -> Result<()> {
            self.queued.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn player() -> (Player, Arc<StubAudio>) {
        let audio = Arc::new(StubAudio::default());
        let player = Player::new(Arc::new(StubMusic), audio.clone(), Quality::Exhigh, 0.5);
        (player, audio)
    }

    #[tokio::test]
    async fn preloading_the_next_track_is_idempotent() {
        let (player, audio) = player();
        player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1), track(2), track(3)],
                kind: QueueKind::Manual,
            })
            .await
            .unwrap();

        assert!(
            player
                .queue_next(TrackId(2), "https://example.test/2.mp3")
                .await
                .unwrap()
        );
        assert!(
            !player
                .queue_next(TrackId(2), "https://example.test/2.mp3")
                .await
                .unwrap()
        );
        assert_eq!(audio.queued.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn gapless_advance_requires_a_matching_preload() {
        let (player, audio) = player();
        player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1), track(2), track(3)],
                kind: QueueKind::Manual,
            })
            .await
            .unwrap();
        player
            .queue_next(TrackId(2), "https://example.test/2.mp3")
            .await
            .unwrap();

        let next = player.advance_gapless(TrackId(2)).await.unwrap().unwrap();
        assert_eq!(next.id, TrackId(2));
        assert_eq!(player.snapshot().await.queue_index, Some(1));
        assert_eq!(
            audio.last_url.lock().unwrap().as_deref(),
            Some("https://example.test/1.mp3")
        );

        // A duplicate boundary for the track that is already current (the `n`
        // vs natural-rollover race) must not advance again.
        assert!(player.advance_gapless(TrackId(2)).await.unwrap().is_none());
        assert_eq!(player.snapshot().await.queue_index, Some(1));
    }

    #[tokio::test]
    async fn jumping_discards_a_previous_preload() {
        let (player, audio) = player();
        player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1), track(2), track(3), track(4)],
                kind: QueueKind::Manual,
            })
            .await
            .unwrap();
        player
            .queue_next(TrackId(2), "https://example.test/2.mp3")
            .await
            .unwrap();

        player.play(PlayRequest::Index(2)).await.unwrap();
        assert_eq!(player.snapshot().await.queue_index, Some(2));
        let next = player.advance_gapless(TrackId(2)).await.unwrap().unwrap();
        assert_eq!(next.id, TrackId(4));
        assert_eq!(player.snapshot().await.queue_index, Some(3));
        assert_eq!(
            audio.last_url.lock().unwrap().as_deref(),
            Some("https://example.test/4.mp3")
        );
    }

    #[tokio::test]
    async fn fm_prefetch_appends_and_reports_new_tracks() {
        let (player, _audio) = player();
        player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1), track(2)],
                kind: QueueKind::Fm,
            })
            .await
            .unwrap();
        let added = player.fm_maybe_prefetch().await.unwrap();
        assert_eq!(added, 2);
        let (tracks, index) = player.queue().await;
        assert_eq!(tracks.len(), 4);
        assert_eq!(index, Some(0));
        assert_eq!(tracks[2].id, TrackId(7));
        assert_eq!(tracks[3].id, TrackId(8));
    }

    #[tokio::test]
    async fn fm_does_not_wrap_repeat_all_back_to_old_tracks() {
        let (player, _audio) = player();
        player.fm_start().await.unwrap();
        player.set_repeat(RepeatMode::All).await;
        assert_eq!(player.next().await.unwrap().unwrap().id, TrackId(8));
        assert!(player.next().await.unwrap().is_none());
        let state = player.snapshot().await;
        assert_eq!(state.status, PlaybackStatus::Stopped);
        assert_eq!(state.track.unwrap().id, TrackId(8));
    }

    #[tokio::test]
    async fn queue_end_keeps_the_last_track_and_can_resume() {
        let (player, audio) = player();
        player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1), track(2)],
                kind: QueueKind::Manual,
            })
            .await
            .unwrap();
        assert_eq!(player.next().await.unwrap().unwrap().id, TrackId(2));
        assert!(player.next().await.unwrap().is_none());

        let stopped = player.snapshot().await;
        assert_eq!(stopped.status, PlaybackStatus::Stopped);
        assert_eq!(stopped.track.unwrap().id, TrackId(2));
        assert_eq!(stopped.queue_index, Some(1));

        player.resume().await.unwrap();
        let resumed = player.snapshot().await;
        assert_eq!(resumed.status, PlaybackStatus::Playing);
        assert_eq!(resumed.track.unwrap().id, TrackId(2));
        assert_eq!(
            audio.last_url.lock().unwrap().as_deref(),
            Some("https://example.test/2.mp3")
        );
    }

    #[tokio::test]
    async fn incomplete_track_hands_off_to_the_next_track_at_zero() {
        let (player, audio) = player();
        player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1), track(2)],
                kind: QueueKind::Manual,
            })
            .await
            .unwrap();
        // A 30 s preview of a 2 min track: it ends early, and the next track
        // must not inherit 0:30 as its starting position.
        audio.position.store(30_000, Ordering::SeqCst);
        audio.finished.store(true, Ordering::SeqCst);
        let next = player.tick().await.unwrap().unwrap();
        assert_eq!(next.id, TrackId(2));
        let state = player.snapshot().await;
        assert_eq!(state.track.unwrap().id, TrackId(2));
        assert_eq!(state.position_ms, 0);
    }

    #[tokio::test]
    async fn switching_tracks_does_not_inherit_the_previous_position() {
        let (player, _audio) = player();
        player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1), track(2)],
                kind: QueueKind::Manual,
            })
            .await
            .unwrap();
        player.inner.lock().await.position_ms = 30_000;
        assert_eq!(player.snapshot().await.position_ms, 0);

        player.next().await.unwrap();
        assert_eq!(player.snapshot().await.position_ms, 0);
        assert_eq!(player.snapshot().await.queue_index, Some(1));
    }

    #[tokio::test]
    async fn tick_waits_while_a_track_is_still_loading() {
        let (player, audio) = player();
        player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1), track(2)],
                kind: QueueKind::Manual,
            })
            .await
            .unwrap();
        audio.finished.store(true, Ordering::SeqCst);
        player.inner.lock().await.loading = true;
        assert!(player.tick().await.unwrap().is_none());
        player.inner.lock().await.loading = false;
        let next = player.tick().await.unwrap().unwrap();
        assert_eq!(next.id, TrackId(2));
    }

    #[tokio::test]
    async fn play_loads_the_first_track_and_reports_playing() {
        let (player, audio) = player();
        let state = player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1), track(2)],
                kind: QueueKind::Manual,
            })
            .await
            .unwrap();
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert_eq!(state.track.unwrap().id, TrackId(1));
        assert!(audio.last_url.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn empty_queue_is_rejected() {
        let (player, _) = player();
        assert!(
            player
                .play(PlayRequest::Tracks {
                    tracks: Vec::new(),
                    kind: QueueKind::Manual,
                })
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn finished_track_advances_and_stops_at_the_end() {
        let (player, audio) = player();
        player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1), track(2)],
                kind: QueueKind::Manual,
            })
            .await
            .unwrap();
        audio.finished.store(true, Ordering::SeqCst);
        let rolled = player.tick().await.unwrap();
        assert_eq!(rolled.unwrap().id, TrackId(2));

        audio.finished.store(true, Ordering::SeqCst);
        assert!(player.tick().await.unwrap().is_none());
        assert_eq!(player.snapshot().await.status, PlaybackStatus::Stopped);
    }

    #[tokio::test]
    async fn mute_keeps_the_stored_volume() {
        let (player, audio) = player();
        player.set_volume(0.9).await.unwrap();
        player.toggle_mute().await.unwrap();
        assert_eq!(*audio.volume.lock().unwrap(), 0.0);
        assert!((player.snapshot().await.volume - 0.9).abs() < f32::EPSILON);
        player.toggle_mute().await.unwrap();
        assert_eq!(*audio.volume.lock().unwrap(), 0.9);
    }

    #[tokio::test]
    async fn daily_and_cloud_set_the_queue_kind() {
        let (player, _) = player();
        let state = player.play_daily().await.unwrap();
        assert_eq!(state.queue_kind, QueueKind::Daily);
        let state = player.play_cloud(10, 0).await.unwrap();
        assert_eq!(state.queue_kind, QueueKind::Cloud);
    }

    struct SlowMusic {
        entered: Arc<tokio::sync::Notify>,
        gate: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl MusicService for SlowMusic {
        async fn search(&self, _: &str, _: SearchKind, _: u32, _: u32) -> Result<SearchResults> {
            Ok(SearchResults::default())
        }

        async fn tracks(&self, ids: &[TrackId]) -> Result<Vec<Track>> {
            Ok(ids.iter().map(|id| track(id.0)).collect())
        }

        async fn stream_url(&self, id: TrackId, _: Quality) -> Result<String> {
            self.entered.notify_one();
            self.gate.notified().await;
            Ok(format!("https://example.test/{id}.mp3"))
        }

        async fn playlists(&self, _: Option<UserId>) -> Result<Vec<Playlist>> {
            Ok(Vec::new())
        }

        async fn playlist_tracks(&self, _: PlaylistId) -> Result<Vec<Track>> {
            Ok(Vec::new())
        }

        async fn current_user(&self) -> Result<Option<UserProfile>> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn starting_a_track_stops_the_previous_one_before_streaming() {
        let entered = Arc::new(tokio::sync::Notify::new());
        let gate = Arc::new(tokio::sync::Notify::new());
        let audio = Arc::new(StubAudio::default());
        audio.playing.store(true, Ordering::SeqCst);

        let player = Arc::new(Player::new(
            Arc::new(SlowMusic {
                entered: Arc::clone(&entered),
                gate: Arc::clone(&gate),
            }),
            audio.clone(),
            Quality::Exhigh,
            0.5,
        ));
        let worker = Arc::clone(&player);
        let task = tokio::spawn(async move {
            worker
                .play(PlayRequest::Tracks {
                    tracks: vec![track(9)],
                    kind: QueueKind::Manual,
                })
                .await
        });

        entered.notified().await;
        assert!(audio.stops.load(Ordering::SeqCst) >= 1);
        assert!(!audio.playing.load(Ordering::SeqCst));
        let state = player.snapshot().await;
        assert!(state.loading);
        assert_eq!(state.track.unwrap().id, TrackId(9));

        gate.notify_one();
        task.await.unwrap().unwrap();
        let state = player.snapshot().await;
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert!(!state.loading);
    }

    #[tokio::test]
    async fn liked_state_tracks_the_cache() {
        let (player, _) = player();
        player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1)],
                kind: QueueKind::Manual,
            })
            .await
            .unwrap();
        player.load_likes().await;
        assert!(player.snapshot().await.liked);
        player.set_like(TrackId(1), false).await.unwrap();
        assert!(!player.snapshot().await.liked);
    }

    #[tokio::test]
    async fn fm_start_uses_the_fm_queue_kind() {
        let (player, _) = player();
        let state = player.fm_start().await.unwrap();
        assert_eq!(state.queue_kind, QueueKind::Fm);
    }

    #[tokio::test]
    async fn finished_track_advances_even_after_the_engine_unloads() {
        let (player, audio) = player();
        player
            .play(PlayRequest::Tracks {
                tracks: vec![track(1), track(2)],
                kind: QueueKind::Manual,
            })
            .await
            .unwrap();
        // The audio thread clears `loaded` when the sink empties and sets
        // `finished`; auto-advance must still fire.
        audio.playing.store(false, Ordering::SeqCst);
        audio.finished.store(true, Ordering::SeqCst);
        let rolled = player.tick().await.unwrap();
        assert_eq!(rolled.unwrap().id, TrackId(2));
    }

    #[tokio::test]
    async fn repeat_cycles_off_all_one() {
        let (player, _) = player();
        assert_eq!(player.cycle_repeat().await.as_str(), "all");
        assert_eq!(player.cycle_repeat().await.as_str(), "one");
        assert_eq!(player.cycle_repeat().await.as_str(), "off");
    }
}
