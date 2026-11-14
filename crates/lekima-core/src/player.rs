//! Playback orchestration.

use std::sync::Arc;

use lekima_domain::{
    AudioEngine, DomainError, MusicService, PlaybackState, PlaybackStatus, Quality, Queue,
    RepeatMode, Result, Track, TrackId,
};
use tokio::sync::Mutex;

/// What should be loaded into the queue.
#[derive(Debug, Clone)]
pub enum PlayRequest {
    Tracks(Vec<Track>),
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
}

/// Owns queue and transport state for the daemon.
pub struct Player {
    music: Arc<dyn MusicService>,
    audio: Arc<dyn AudioEngine>,
    quality: Quality,
    inner: Mutex<Inner>,
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
            quality,
            inner: Mutex::new(Inner {
                queue: Queue::default(),
                status: PlaybackStatus::Stopped,
                volume,
                muted: false,
                repeat: RepeatMode::Off,
                shuffle: false,
                position_ms: 0,
            }),
        }
    }

    #[must_use]
    pub fn music(&self) -> &Arc<dyn MusicService> {
        &self.music
    }

    /// Immutable view of the current state.
    pub async fn snapshot(&self) -> PlaybackState {
        let inner = self.inner.lock().await;
        let mut state = PlaybackState {
            status: inner.status,
            track: inner.queue.current().cloned(),
            position_ms: if inner.status == PlaybackStatus::Stopped {
                0
            } else {
                self.audio.position_ms().max(inner.position_ms)
            },
            volume: inner.volume,
            muted: inner.muted,
            repeat: inner.repeat,
            shuffle: inner.shuffle,
            queue_length: inner.queue.len(),
            queue_index: inner.queue.current_index(),
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
                PlayRequest::Tracks(tracks) => {
                    if tracks.is_empty() {
                        return Err(DomainError::InvalidInput(
                            "nothing to play: queue would be empty".to_owned(),
                        ));
                    }
                    inner.queue.replace(tracks);
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

    pub async fn clear_queue(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.queue.clear();
        inner.status = PlaybackStatus::Stopped;
        inner.position_ms = 0;
        drop(inner);
        self.audio.stop()
    }

    /// Loads the queue's current track into the audio engine.
    async fn load_current(&self) -> Result<()> {
        let track = {
            let inner = self.inner.lock().await;
            inner
                .queue
                .current()
                .cloned()
                .ok_or_else(|| DomainError::NotFound("current track".to_owned()))?
        };

        let url = self.music.stream_url(track.id, self.quality).await?;
        if let Err(error) = self.audio.play_url(&url).await {
            let mut inner = self.inner.lock().await;
            inner.status = PlaybackStatus::Stopped;
            return Err(error);
        }

        let mut inner = self.inner.lock().await;
        inner.status = PlaybackStatus::Playing;
        inner.position_ms = 0;
        let gain = if inner.muted { 0.0 } else { inner.volume };
        let _ = self.audio.set_volume(gain);
        tracing::debug!(track = %track.id, "playback started");
        Ok(())
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

        async fn lyrics(&self, _: TrackId) -> Result<String> {
            Ok(String::new())
        }
    }

    #[derive(Default)]
    struct StubAudio {
        playing: AtomicBool,
        finished: AtomicBool,
        position: AtomicU64,
        volume: Mutex<f32>,
        last_url: Mutex<Option<String>>,
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
            .play(PlayRequest::Tracks(vec![track(1), track(2)]))
            .await
            .unwrap();
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert_eq!(state.track.unwrap().id, TrackId(1));
        assert!(audio.last_url.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn empty_queue_is_rejected() {
        let (player, _) = player();
        assert!(player.play(PlayRequest::Tracks(Vec::new())).await.is_err());
    }

    #[tokio::test]
    async fn finished_track_advances_and_stops_at_the_end() {
        let (player, audio) = player();
        player
            .play(PlayRequest::Tracks(vec![track(1), track(2)]))
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
    async fn repeat_cycles_off_all_one() {
        let (player, _) = player();
        assert_eq!(player.cycle_repeat().await.as_str(), "all");
        assert_eq!(player.cycle_repeat().await.as_str(), "one");
        assert_eq!(player.cycle_repeat().await.as_str(), "off");
    }
}
