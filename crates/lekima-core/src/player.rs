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
        let mut state = PlaybackState {
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
        };
        if state.status != PlaybackStatus::Stopped
            && state.position_ms == 0
            && inner.position_ms > 0
        {
            state.position_ms = inner.position_ms;
        }
        state
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
            (track, inner.generation)
        };

        // A must go silent now, not after B has been fetched.
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
            let repeat = inner.repeat;
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
            let repeat = inner.repeat;
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
            (
                self.audio.is_finished(),
                inner.status == PlaybackStatus::Playing && self.audio.has_track(),
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
        if trash {
            let current = self.snapshot().await.track;
            if let Some(track) = current {
                self.music.fm_trash(track.id).await?;
                self.trashed.lock().await.insert(track.id);
            }
        }
        self.next().await
    }

    /// Keeps the FM station fed: fetch a batch when two tracks remain.
    pub async fn fm_maybe_prefetch(&self) -> Result<()> {
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
            return Ok(());
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
            return Ok(());
        }

        let mut inner = self.inner.lock().await;
        if inner.kind == QueueKind::Fm {
            inner.queue.extend(fresh);
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };

    use async_trait::async_trait;
    use lekima_domain::{
        Album, Artist, AudioEngine, MusicService, PlaybackStatus, Playlist, PlaylistId, Quality,
        Result, SearchKind, SearchResults, Track, TrackId, UserId, UserProfile,
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
    }

    #[async_trait]
    impl AudioEngine for StubAudio {
        async fn play_url(&self, url: &str) -> Result<()> {
            *self.last_url.lock().unwrap() = Some(url.to_owned());
            self.playing.store(true, Ordering::SeqCst);
            self.finished.store(false, Ordering::SeqCst);
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
    }

    fn player() -> (Player, Arc<StubAudio>) {
        let audio = Arc::new(StubAudio::default());
        let player = Player::new(Arc::new(StubMusic), audio.clone(), Quality::Exhigh, 0.5);
        (player, audio)
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
    async fn repeat_cycles_off_all_one() {
        let (player, _) = player();
        assert_eq!(player.cycle_repeat().await.as_str(), "all");
        assert_eq!(player.cycle_repeat().await.as_str(), "one");
        assert_eq!(player.cycle_repeat().await.as_str(), "off");
    }
}
