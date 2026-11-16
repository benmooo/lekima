//! The player daemon: owns the audio engine, the provider session and the
//! queue. Every other process is a client.

use std::{
    collections::VecDeque,
    fs,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use lekima_config::{Config, FileCredentials, Paths, PlaybackStore, SavedPlayback};
use lekima_core::{PlayRequest, Player};
use lekima_domain::{CredentialsStore, PlaybackState, QueueKind, SearchKind};
use lekima_ipc::{
    ApiError, Command, Event, EventFrame, Handled, PlayTarget, Reply, Request, Response,
    Subscription, serve_connection,
};
use lekima_ncm::{NcmMusicService, NcmOptions};
use lekima_platform::{MediaCommand, MediaControls, NowPlaying};
use tokio::{
    net::UnixListener,
    sync::{Notify, broadcast},
};

use crate::output::Failure;

/// Runs the daemon until it is asked to shut down.
///
/// # Errors
///
/// Fails when the socket cannot be bound, or when the audio device or provider
/// client cannot be initialised.
pub async fn run(paths: &Paths, config: &Config, socket: &Path, _detached: bool) -> Result<()> {
    paths
        .ensure()
        .context("failed to create state directories")?;
    prepare_socket(socket)?;

    let listener = UnixListener::bind(socket)
        .with_context(|| format!("failed to bind {}", socket.display()))?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))
        .context("failed to restrict socket permissions")?;
    tracing::info!(socket = %socket.display(), "daemon listening");

    let music = Arc::new(NcmMusicService::new(NcmOptions {
        device_id_path: paths.device_file(),
        cache: config.provider.cache,
        cache_ttl: Duration::from_secs(config.provider.cache_ttl_secs),
        ..NcmOptions::new(paths.session_file())
    })?);
    let audio = Arc::new(
        lekima_audio::RodioAudioEngine::open(
            config.playback.volume,
            config.audio.device.as_deref(),
            lekima_audio::StreamCache::new(
                paths.cache_dir().join("streams"),
                config.cache.limit_mb,
                config.cache.enabled,
            ),
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?,
    );
    let player = Arc::new(Player::new(
        music,
        audio,
        config.playback.quality,
        config.playback.volume,
    ));
    let playback_store = PlaybackStore::new(paths.playback_file());
    match playback_store.load() {
        Ok(Some(saved)) => {
            if let Err(error) = player
                .restore(
                    saved.tracks,
                    saved.index,
                    saved.kind,
                    saved.repeat,
                    saved.shuffle,
                    saved.volume,
                    saved.quality,
                    saved.position_ms,
                )
                .await
            {
                tracing::warn!(%error, "failed to restore the saved queue");
            }
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "ignoring a malformed playback snapshot"),
    }

    let hub = Arc::new(EventHub::new());
    let sleep_until = Arc::new(StdMutex::new(None));
    let preloaded = Arc::new(AtomicBool::new(false));
    let controls = lekima_platform::controls().await;
    tracing::debug!(backend = controls.backend(), "media integration ready");
    let daemon = Arc::new(Daemon {
        player: Arc::clone(&player),
        credentials: FileCredentials::new(paths.session_file()),
        shutdown: Notify::new(),
        hub: Arc::clone(&hub),
        sleep_until: Arc::clone(&sleep_until),
        preloaded: Arc::clone(&preloaded),
        controls,
        playback_store: playback_store.clone(),
    });

    {
        let likes = Arc::clone(&player);
        tokio::spawn(async move { likes.load_likes().await });
    }
    spawn_ticker(
        Arc::clone(&player),
        Arc::clone(&hub),
        sleep_until,
        Arc::clone(&preloaded),
    );
    spawn_persister(Arc::clone(&player), playback_store.clone());
    spawn_preloader(Arc::clone(&daemon));
    spawn_platform(Arc::clone(&daemon));

    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("failed to install the SIGTERM handler")?;

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let daemon = Arc::clone(&daemon);
                        tokio::spawn(async move {
                            let handler = {
                                let daemon = Arc::clone(&daemon);
                                move |request: Request| {
                                    let daemon = Arc::clone(&daemon);
                                    async move { daemon.handle(request).await }
                                }
                            };
                            if let Err(error) = serve_connection(stream, handler).await {
                                tracing::warn!(%error, "ipc connection failed");
                            }
                        });
                    }
                    Err(error) => tracing::warn!(%error, "accept failed"),
                }
            }
            () = daemon.shutdown.notified() => break,
            _ = tokio::signal::ctrl_c() => break,
            _ = terminate.recv() => break,
        }
    }

    tracing::info!("shutting down");
    persist_playback(&daemon.player, &daemon.playback_store).await;
    let _ = daemon.player.stop().await;
    drop(listener);
    let _ = fs::remove_file(socket);
    Ok(())
}

struct Daemon {
    player: Arc<Player>,
    credentials: FileCredentials,
    shutdown: Notify,
    hub: Arc<EventHub>,
    sleep_until: Arc<StdMutex<Option<Instant>>>,
    preloaded: Arc<AtomicBool>,
    controls: Arc<dyn MediaControls>,
    playback_store: PlaybackStore,
}

/// Sequenced broadcast with a replay ring.
struct EventHub {
    sender: broadcast::Sender<EventFrame>,
    ring: StdMutex<VecDeque<EventFrame>>,
    seq: AtomicU64,
}

impl EventHub {
    const RING: usize = 256;

    fn new() -> Self {
        let (sender, _) = broadcast::channel(256);
        Self {
            sender,
            ring: StdMutex::new(VecDeque::with_capacity(Self::RING)),
            seq: AtomicU64::new(0),
        }
    }

    fn publish(&self, event: Event) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let frame = EventFrame { seq, event };
        if let Ok(mut ring) = self.ring.lock() {
            if ring.len() == Self::RING {
                ring.pop_front();
            }
            ring.push_back(frame.clone());
        }
        let _ = self.sender.send(frame);
    }

    #[allow(clippy::type_complexity)]
    fn subscribe(
        &self,
        since: Option<u64>,
    ) -> (u64, Vec<EventFrame>, bool, broadcast::Receiver<EventFrame>) {
        let last = self.seq.load(Ordering::SeqCst);
        let ring = self
            .ring
            .lock()
            .map_or_else(|_| VecDeque::new(), |ring| ring.clone());
        let resync = since.is_some_and(|since| {
            ring.front()
                .is_some_and(|first| since.saturating_add(1) < first.seq)
        });
        let initial = since.map_or_else(Vec::new, |since| {
            ring.into_iter().filter(|frame| frame.seq > since).collect()
        });
        (last, initial, resync, self.sender.subscribe())
    }
}

impl Daemon {
    async fn handle(&self, request: Request) -> Handled {
        let id = request.id;
        if let Command::Subscribe { kinds, since } = &request.command {
            let (seq, initial, resync, receiver) = self.hub.subscribe(*since);
            return Handled::stream(
                Response::ok(id, Reply::Subscribed { seq, resync }),
                Subscription {
                    receiver,
                    initial,
                    kinds: kinds.clone(),
                },
            );
        }

        let command = request.command;
        if invalidates_preload(&command) {
            self.preloaded.store(false, Ordering::SeqCst);
        }
        let outcome = self.execute(command.clone()).await;
        match outcome {
            Ok(mut reply) => {
                self.stamp_revision(&mut reply);
                self.publish(&command, &reply).await;
                Handled::reply(Response::ok(id, reply))
            }
            Err(error) => Handled::reply(Response::err(id, error)),
        }
    }

    /// Stamps status replies with the event revision clients should compare
    /// against; a poll handled before a newer event cannot overwrite it.
    fn stamp_revision(&self, reply: &mut Reply) {
        if let Reply::Status { state } = reply {
            state.revision = self.hub.seq.load(Ordering::SeqCst);
        }
    }

    async fn publish(&self, command: &Command, reply: &Reply) {
        match reply {
            Reply::Status { state } if !matches!(command, Command::Status) => {
                self.hub.publish(Event::Status {
                    state: Box::new(state.clone()),
                });
            }
            Reply::Queue { tracks, index } => self.hub.publish(Event::Queue {
                tracks: tracks.clone(),
                index: *index,
            }),
            Reply::Like { id, liked } => self.hub.publish(Event::Like {
                id: *id,
                liked: *liked,
            }),
            Reply::Ack if matches!(command, Command::Enqueue { .. } | Command::ClearQueue) => {
                let (tracks, index) = self.player.queue().await;
                self.hub.publish(Event::Queue { tracks, index });
            }
            _ => {}
        }

        if matches!(
            command,
            Command::Play { .. }
                | Command::Daily { play: true }
                | Command::Fm { .. }
                | Command::Cloud { play: true, .. }
                | Command::Liked { play: true }
                | Command::Jump { .. }
                | Command::Next
                | Command::Prev
                | Command::QueueRemove { .. }
                | Command::QueueMove { .. }
        ) {
            let (tracks, index) = self.player.queue().await;
            self.hub.publish(Event::Queue { tracks, index });
        }
    }

    async fn execute(&self, command: Command) -> std::result::Result<Reply, ApiError> {
        match command {
            Command::Ping => Ok(Reply::Pong),
            Command::Hello { protocol } => Ok(Reply::Welcome {
                protocol: protocol.min(2),
                seq: self.hub.seq.load(Ordering::SeqCst),
                capabilities: [
                    "events", "daily", "fm", "like", "toplists", "cloud", "lyrics", "comments",
                ]
                .iter()
                .map(|capability| (*capability).to_owned())
                .collect(),
            }),
            Command::Status => Ok(self.status().await),
            Command::Play { target } => {
                self.play(target).await?;
                Ok(self.status().await)
            }
            Command::Jump { index } => {
                self.player.play(PlayRequest::Index(index)).await?;
                Ok(self.status().await)
            }
            Command::Pause => {
                self.player.pause().await?;
                Ok(self.status().await)
            }
            Command::Resume => {
                self.player.resume().await?;
                Ok(self.status().await)
            }
            Command::Toggle => {
                self.player.toggle().await?;
                Ok(self.status().await)
            }
            Command::Stop => {
                self.player.stop().await?;
                Ok(self.status().await)
            }
            Command::Next => {
                self.player.next().await?;
                Ok(self.status().await)
            }
            Command::Prev => {
                self.player.previous().await?;
                Ok(self.status().await)
            }
            Command::Seek { position_ms } => {
                self.player.seek(position_ms).await?;
                Ok(self.status().await)
            }
            Command::SetVolume { level } => {
                self.player.set_volume(level).await?;
                Ok(self.status().await)
            }
            Command::ToggleMute => {
                self.player.toggle_mute().await?;
                Ok(self.status().await)
            }
            Command::CycleRepeat => {
                self.player.cycle_repeat().await;
                Ok(self.status().await)
            }
            Command::SetRepeat { mode } => {
                self.player.set_repeat(mode).await;
                Ok(self.status().await)
            }
            Command::SetShuffle { enabled } => {
                self.player.set_shuffle(enabled).await?;
                Ok(self.status().await)
            }
            Command::Queue => {
                let (tracks, index) = self.player.queue().await;
                Ok(Reply::Queue { tracks, index })
            }
            Command::QueueRemove { index } => {
                let was_current = self.player.queue().await.1 == Some(index);
                self.player.remove_at(index).await?;
                if was_current {
                    Ok(self.status().await)
                } else {
                    let (tracks, index) = self.player.queue().await;
                    Ok(Reply::Queue { tracks, index })
                }
            }
            Command::QueueMove { from, to } => {
                self.player.move_item(from, to).await?;
                let (tracks, index) = self.player.queue().await;
                Ok(Reply::Queue { tracks, index })
            }
            Command::Enqueue { tracks, next } => {
                let resolved = self.player.resolve_tracks(&tracks).await?;
                self.player.enqueue(resolved, next).await?;
                Ok(Reply::Ack)
            }
            Command::ClearQueue => {
                self.player.clear_queue().await?;
                Ok(Reply::Ack)
            }
            Command::Search {
                query,
                kind,
                limit,
                offset,
            } => {
                let results = self
                    .player
                    .music()
                    .search(&query, kind, limit, offset)
                    .await?;
                Ok(Reply::Search { results })
            }
            Command::Playlists => {
                let playlists = self.player.music().playlists(None).await?;
                Ok(Reply::Playlists { playlists })
            }
            Command::PlaylistTracks { id } => {
                let tracks = self.player.music().playlist_tracks(id).await?;
                Ok(Reply::Tracks { tracks })
            }
            Command::Lyrics { id } => {
                let track_id = self.current_track_id(id).await?;
                let lyrics = self.player.music().lyrics(track_id).await?;
                Ok(Reply::Lyrics {
                    id: track_id,
                    lyrics,
                })
            }
            Command::Comments { id, limit, offset } => {
                let track_id = self.current_track_id(id).await?;
                let comments = self
                    .player
                    .music()
                    .comments(track_id, limit.max(1), offset)
                    .await?;
                Ok(Reply::Comments {
                    id: track_id,
                    comments,
                })
            }
            Command::Daily { play } => {
                if play {
                    self.player.play_daily().await?;
                    Ok(self.status().await)
                } else {
                    let tracks = self.player.music().daily().await?;
                    Ok(Reply::Tracks { tracks })
                }
            }
            Command::Fm { action } => {
                use lekima_ipc::FmAction;
                match action {
                    FmAction::Start => {
                        self.player.fm_start().await?;
                    }
                    FmAction::Next { trash } => {
                        self.player.fm_next(trash).await?;
                    }
                    FmAction::Stop => {
                        self.player.stop().await?;
                    }
                }
                Ok(self.status().await)
            }
            Command::Toplists => {
                let toplists = self.player.music().toplists().await?;
                Ok(Reply::Toplists { toplists })
            }
            Command::Cloud {
                limit,
                offset,
                play,
            } => {
                if play {
                    self.player.play_cloud(limit, offset).await?;
                    Ok(self.status().await)
                } else {
                    let tracks = self.player.music().cloud(limit, offset).await?;
                    Ok(Reply::Tracks { tracks })
                }
            }
            Command::Like { id, liked } => {
                let track_id = self.current_track_id(id).await?;
                let liked = self.player.set_like(track_id, liked).await?;
                Ok(Reply::Like {
                    id: track_id,
                    liked,
                })
            }
            Command::Liked { play } => {
                let tracks = self.player.liked_tracks().await?;
                if play {
                    self.player
                        .play(PlayRequest::Tracks {
                            tracks,
                            kind: QueueKind::Liked,
                        })
                        .await?;
                    Ok(self.status().await)
                } else {
                    Ok(Reply::Tracks { tracks })
                }
            }
            Command::Whoami => {
                let user = self.player.music().current_user().await?;
                Ok(Reply::User { user })
            }
            Command::Login { cookie } => {
                self.credentials.save(&cookie)?;
                Ok(Reply::Ack)
            }
            Command::Logout => {
                self.credentials.clear()?;
                Ok(Reply::Ack)
            }
            Command::Subscribe { .. } => Err(ApiError::new(
                lekima_ipc::ErrorCode::BadRequest,
                "subscriptions are handled on the connection",
            )),
            Command::Sleep { minutes } => {
                let deadline = minutes
                    .map(|minutes| Instant::now() + Duration::from_secs(u64::from(minutes) * 60));
                if let Ok(mut slot) = self.sleep_until.lock() {
                    *slot = deadline;
                }
                Ok(self.status().await)
            }
            Command::SetQuality { quality } => {
                self.player.set_quality(quality).await;
                Ok(self.status().await)
            }
            Command::Shutdown => {
                self.shutdown.notify_waiters();
                Ok(Reply::Ack)
            }
        }
    }

    async fn play(&self, target: PlayTarget) -> std::result::Result<(), ApiError> {
        let kind = match &target {
            PlayTarget::Playlist { .. } => QueueKind::Playlist,
            _ => QueueKind::Manual,
        };
        let tracks = match target {
            PlayTarget::Query { query } => {
                let results = self
                    .player
                    .music()
                    .search(&query, SearchKind::Track, 25, 0)
                    .await?;
                if results.tracks.is_empty() {
                    return Err(ApiError::new(
                        lekima_ipc::ErrorCode::NotFound,
                        format!("no tracks matched `{query}`"),
                    ));
                }
                results.tracks
            }
            PlayTarget::Tracks { ids } => self.player.resolve_tracks(&ids).await?,
            PlayTarget::Playlist { id } => self.player.music().playlist_tracks(id).await?,
        };
        self.player
            .play(PlayRequest::Tracks { tracks, kind })
            .await?;
        Ok(())
    }

    async fn current_track_id(
        &self,
        explicit: Option<lekima_domain::TrackId>,
    ) -> std::result::Result<lekima_domain::TrackId, ApiError> {
        if let Some(id) = explicit {
            return Ok(id);
        }
        self.player
            .snapshot()
            .await
            .track
            .map(|track| track.id)
            .ok_or_else(|| ApiError::new(lekima_ipc::ErrorCode::NotFound, "nothing is playing"))
    }

    async fn status(&self) -> Reply {
        let mut state: PlaybackState = self.player.snapshot().await;
        state.sleep_remaining_ms = self.sleep_until.lock().ok().and_then(|slot| {
            slot.map(|deadline| {
                u64::try_from(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .as_millis(),
                )
                .unwrap_or(u64::MAX)
            })
        });
        Reply::Status { state }
    }
}

fn spawn_ticker(
    player: Arc<Player>,
    hub: Arc<EventHub>,
    sleep_until: Arc<StdMutex<Option<Instant>>>,
    preloaded: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            if let Some(boundary) = player.take_track_boundary() {
                match player.advance_gapless(boundary).await {
                    Ok(_) => {
                        preloaded.store(false, Ordering::SeqCst);
                        hub.publish(Event::Status {
                            state: Box::new(player.snapshot().await),
                        });
                        let (tracks, index) = player.queue().await;
                        hub.publish(Event::Queue { tracks, index });
                    }
                    Err(error) => tracing::warn!(%error, "gapless advance failed"),
                }
                continue;
            }
            match player.tick().await {
                Ok(Some(_)) => {
                    hub.publish(Event::Status {
                        state: Box::new(player.snapshot().await),
                    });
                    let (tracks, index) = player.queue().await;
                    hub.publish(Event::Queue { tracks, index });
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "auto-advance failed"),
            }
            match player.fm_maybe_prefetch().await {
                Ok(0) => {}
                Ok(_) => {
                    let (tracks, index) = player.queue().await;
                    hub.publish(Event::Queue { tracks, index });
                }
                Err(error) => tracing::debug!(%error, "fm prefetch skipped"),
            }
            let expired = sleep_until
                .lock()
                .ok()
                .is_some_and(|slot| slot.is_some_and(|deadline| Instant::now() >= deadline));
            if expired {
                if let Ok(mut slot) = sleep_until.lock() {
                    *slot = None;
                }
                let _ = player.pause().await;
                hub.publish(Event::Status {
                    state: Box::new(player.snapshot().await),
                });
            }
        }
    });
}

/// Mirrors player state to the OS and executes media-key/MPRIS commands.
fn spawn_platform(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            let state = daemon.player.snapshot().await;
            daemon.controls.update(&now_playing(&state));
            for action in daemon.controls.poll() {
                let Some(command) = platform_command(&state, action) else {
                    continue;
                };
                match daemon.execute(command.clone()).await {
                    Ok(reply) => daemon.publish(&command, &reply).await,
                    Err(error) => tracing::debug!(error = ?error, "media command failed"),
                }
            }
        }
    });
}

fn now_playing(state: &PlaybackState) -> NowPlaying {
    let track = state.track.as_ref();
    NowPlaying {
        track_id: track.map_or(0, |track| track.id.get()),
        title: track.map_or_else(String::new, |track| track.name.clone()),
        artist: track.map_or_else(String::new, lekima_domain::Track::artist_names),
        album: track.map_or_else(String::new, |track| track.album_name().to_owned()),
        duration_ms: track.map_or(0, |track| track.duration_ms),
        position_ms: state.position_ms,
        playing: state.status == lekima_domain::PlaybackStatus::Playing,
        has_track: track.is_some(),
        has_next: state
            .queue_index
            .is_some_and(|index| index + 1 < state.queue_length),
        has_previous: state.queue_index.is_some_and(|index| index > 0),
        volume: f64::from(state.volume),
        muted: state.muted,
        shuffle: state.shuffle,
        repeat: match state.repeat {
            lekima_domain::RepeatMode::Off => lekima_platform::Repeat::Off,
            lekima_domain::RepeatMode::All => lekima_platform::Repeat::All,
            lekima_domain::RepeatMode::One => lekima_platform::Repeat::One,
        },
    }
}

fn platform_command(state: &PlaybackState, action: MediaCommand) -> Option<Command> {
    Some(match action {
        MediaCommand::Play => Command::Resume,
        MediaCommand::Pause => Command::Pause,
        MediaCommand::Toggle => Command::Toggle,
        MediaCommand::Stop => Command::Stop,
        MediaCommand::Next => Command::Next,
        MediaCommand::Previous => Command::Prev,
        MediaCommand::Seek(position_ms) => Command::Seek { position_ms },
        MediaCommand::SeekBy(delta) => {
            let base = i64::try_from(state.position_ms).unwrap_or(i64::MAX);
            let target = (base + delta).max(0);
            let target = if state
                .track
                .as_ref()
                .is_some_and(|track| track.duration_ms > 0)
            {
                target.min(
                    i64::try_from(state.track.as_ref().map_or(0, |track| track.duration_ms))
                        .unwrap_or(i64::MAX),
                )
            } else {
                target
            };
            Command::Seek {
                position_ms: u64::try_from(target).unwrap_or(0),
            }
        }
        MediaCommand::SetVolume(level) => Command::SetVolume {
            level: (level as f32).clamp(0.0, 1.0),
        },
        MediaCommand::SetShuffle(enabled) => Command::SetShuffle { enabled },
        MediaCommand::SetRepeat(repeat) => Command::SetRepeat {
            mode: match repeat {
                lekima_platform::Repeat::Off => lekima_domain::RepeatMode::Off,
                lekima_platform::Repeat::All => lekima_domain::RepeatMode::All,
                lekima_platform::Repeat::One => lekima_domain::RepeatMode::One,
            },
        },
    })
}

fn spawn_preloader(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(1000));
        loop {
            interval.tick().await;
            if daemon.preloaded.load(Ordering::SeqCst)
                || daemon.player.repeat_mode().await == lekima_domain::RepeatMode::One
            {
                continue;
            }
            let state = daemon.player.snapshot().await;
            if state.status != lekima_domain::PlaybackStatus::Playing {
                continue;
            }
            let Some(next) = daemon.player.next_track().await else {
                continue;
            };
            let quality = daemon.player.quality().await;
            let cached = daemon.player.has_cached(next.id, quality);
            let url = if cached {
                String::new()
            } else {
                daemon
                    .player
                    .music()
                    .stream_url(next.id, quality)
                    .await
                    .unwrap_or_default()
            };
            if cached || !url.is_empty() {
                match daemon.player.queue_next(next.id, &url).await {
                    Ok(true) => tracing::debug!(track = %next.id, "preloaded next track"),
                    Ok(false) => tracing::debug!(track = %next.id, "next track already preloaded"),
                    Err(error) => {
                        tracing::debug!(%error, "next track cannot be preloaded");
                    }
                }
            }
            // Regardless of outcome, do not retry until the track changes.
            daemon.preloaded.store(true, Ordering::SeqCst);
        }
    });
}

fn spawn_persister(player: Arc<Player>, store: PlaybackStore) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        let mut last: Option<SavedPlayback> = None;
        loop {
            interval.tick().await;
            let saved = playback_snapshot(&player).await;
            if saved != last {
                match &saved {
                    Some(saved) => {
                        if let Err(error) = store.save(saved) {
                            tracing::warn!(%error, "failed to save the playback queue");
                        }
                    }
                    None => {
                        if let Err(error) = store.clear() {
                            tracing::warn!(%error, "failed to clear the playback queue");
                        }
                    }
                }
                last = saved;
            }
        }
    });
}

async fn playback_snapshot(player: &Player) -> Option<SavedPlayback> {
    let state = player.snapshot().await;
    let (tracks, index) = player.queue().await;
    if tracks.is_empty() {
        return None;
    }
    let pending = player.pending_position_ms().await.unwrap_or(0);
    Some(SavedPlayback {
        tracks,
        index,
        position_ms: state.position_ms.max(pending),
        repeat: state.repeat,
        shuffle: state.shuffle,
        volume: state.volume,
        quality: state.quality,
        kind: state.queue_kind,
    })
}

async fn persist_playback(player: &Player, store: &PlaybackStore) {
    match playback_snapshot(player).await {
        Some(saved) => {
            if let Err(error) = store.save(&saved) {
                tracing::warn!(%error, "failed to save the playback queue");
            }
        }
        None => {
            if let Err(error) = store.clear() {
                tracing::warn!(%error, "failed to clear the playback queue");
            }
        }
    }
}

fn invalidates_preload(command: &Command) -> bool {
    matches!(
        command,
        Command::Play { .. }
            | Command::Jump { .. }
            | Command::Stop
            | Command::Next
            | Command::Prev
            | Command::QueueRemove { .. }
            | Command::QueueMove { .. }
            | Command::Enqueue { .. }
            | Command::ClearQueue
            | Command::Daily { play: true }
            | Command::Fm { .. }
            | Command::Cloud { play: true, .. }
            | Command::Liked { play: true }
    )
}

fn prepare_socket(socket: &Path) -> Result<()> {
    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to restrict {}", parent.display()))?;
    }
    if socket.exists() {
        if std::os::unix::net::UnixStream::connect(socket).is_ok() {
            anyhow::bail!(
                "another lekima daemon is already listening on {}",
                socket.display()
            );
        }
        fs::remove_file(socket)
            .with_context(|| format!("failed to remove stale socket {}", socket.display()))?;
    }
    Ok(())
}

/// Starts a detached daemon that logs to `log_file`.
///
/// # Errors
///
/// Propagates process-spawn and log-file failures.
pub fn spawn_detached(socket: &Path, log_file: &Path) -> Result<()> {
    let executable = std::env::current_exe().context("failed to locate the current binary")?;
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)?;
    }
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log_file)
        .with_context(|| format!("failed to open {}", log_file.display()))?;
    let errors = log.try_clone().context("failed to clone the log file")?;
    let mut command = std::process::Command::new(executable);
    command
        .arg("--socket")
        .arg(socket)
        .args(["daemon", "run", "--detached"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(errors))
        .env("LEKIMA_LOG", "info");
    command.spawn().context("failed to spawn the daemon")?;
    Ok(())
}

/// Waits until the daemon answers a ping.
///
/// # Errors
///
/// Returns [`Failure`] with code `daemon` when the deadline passes.
pub async fn wait_until_running(
    socket: &Path,
    deadline: Duration,
) -> std::result::Result<(), Failure> {
    let started = Instant::now();
    while started.elapsed() < deadline {
        if lekima_ipc::is_running(socket).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(Failure::new(
        lekima_ipc::ErrorCode::Daemon,
        format!(
            "the daemon did not start within {}s; see {}",
            deadline.as_secs(),
            "the log file (`lekima daemon logs`)"
        ),
    ))
}
