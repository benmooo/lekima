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
use lekima_config::{
    Config, FileCredentials, HistoryStore, HookConfig, Paths, PlaybackStore, SavedPlayback,
};
use lekima_core::{PlayRequest, Player};
use lekima_domain::{
    CredentialsStore, HistoryLog, PlaybackState, PlaybackStatus, QueueKind, SearchKind, Track,
    TrackId, sanitize_line,
};
use lekima_ipc::{
    ApiError, Command, Event, EventFrame, Handled, PlayTarget, Reply, Request, Response,
    Subscription, serve_connection,
};
use lekima_ncm::{NcmMusicService, NcmOptions};
use lekima_platform::{MediaCommand, MediaControls, Notification, Notifier, NowPlaying};
use tokio::{
    net::UnixListener,
    sync::{Notify, broadcast, mpsc},
};

use crate::hooks;

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
    let history_store = HistoryStore::new(paths.history_file());
    let (loaded, pruned) = load_history(&history_store, config.history.capacity);
    let saved_revision = prune_history(&history_store, &loaded, pruned);
    let history = Arc::new(StdMutex::new(loaded));
    let (hook_commands, hook_receiver) = mpsc::channel(hooks::COMMAND_QUEUE);
    let running_hooks = hooks::start(&config.hooks, hub.sender.clone(), hook_commands);
    let controls = lekima_platform::controls().await;
    tracing::debug!(backend = controls.backend(), "media integration ready");
    let notifier = lekima_platform::notifier();
    tracing::debug!(backend = notifier.backend(), "notification backend ready");
    let daemon = Arc::new(Daemon {
        player: Arc::clone(&player),
        credentials: FileCredentials::new(paths.session_file()),
        shutdown: Notify::new(),
        hub: Arc::clone(&hub),
        sleep_until: Arc::clone(&sleep_until),
        preloaded: Arc::clone(&preloaded),
        controls,
        notifications: NotifierHandle::spawn(notifier),
        track_notifications: StdMutex::new(TrackNotifications::new(
            config.notifications.on_track_change,
        )),
        history: Arc::clone(&history),
        history_store,
        history_saved: StdMutex::new(saved_revision),
        playback_store: playback_store.clone(),
        hooks: running_hooks,
        hook_configs: config.hooks.clone(),
    });

    tracing::info!(hooks = daemon.hooks.len(), "hook processes started");
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
    spawn_history(Arc::clone(&daemon));
    spawn_hook_commands(Arc::clone(&daemon), hook_receiver);

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
    persist_history(&daemon).await;
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
    notifications: NotifierHandle,
    /// Which track changes are worth announcing.
    track_notifications: StdMutex<TrackNotifications>,
    /// Finished plays, appended as tracks stop being current.
    history: Arc<StdMutex<HistoryLog>>,
    history_store: HistoryStore,
    /// Revision already on disk; a stale poller snapshot cannot overwrite a
    /// newer one.
    history_saved: StdMutex<u64>,
    playback_store: PlaybackStore,
    /// Live hook processes; dropping the daemon closes their stdin and kills
    /// them.
    hooks: Vec<hooks::RunningHook>,
    /// The configured hook tables, kept for `hooks` reports.
    hook_configs: std::collections::BTreeMap<String, HookConfig>,
}

/// Delivers notifications off the async runtime.
///
/// The platform loop that calls this also publishes Now Playing and drains
/// media keys, so a slow helper (a first-run permission prompt, a wedged
/// notification daemon) must never run on it. Deliveries go to a dedicated
/// thread through a small queue; when the queue is full the announcement is
/// dropped rather than blocking the player.
#[derive(Debug)]
struct NotifierHandle {
    sender: std::sync::mpsc::SyncSender<Notification>,
    /// Set once a dead worker is reported, so it warns per run, not per track.
    reported: AtomicBool,
}

impl NotifierHandle {
    const QUEUE: usize = 8;

    fn spawn(notifier: Arc<dyn Notifier>) -> Self {
        let backend = notifier.backend();
        let (sender, receiver) = std::sync::mpsc::sync_channel(Self::QUEUE);
        let worker = std::thread::Builder::new()
            .name("lekima-notify".to_owned())
            .spawn(move || {
                while let Ok(notification) = receiver.recv() {
                    if !notifier.notify(&notification) {
                        tracing::debug!(backend, "desktop notification was not shown");
                    }
                }
            });
        if let Err(error) = worker {
            tracing::warn!(%error, "notification thread failed to start");
        }
        Self {
            sender,
            reported: AtomicBool::new(false),
        }
    }

    fn send(&self, notification: Notification) {
        match self.sender.try_send(notification) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                tracing::debug!("notification queue is full; dropping an announcement");
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                if !self.reported.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        "notification worker stopped; notifications are off until the daemon restarts"
                    );
                }
            }
        }
    }
}

/// Decides which track changes become notifications.
///
/// Kept out of the daemon so the rule is testable without a player, an audio
/// device or an OS helper: the daemon only forwards what this returns.
#[derive(Debug)]
struct TrackNotifications {
    enabled: bool,
    /// The track the user was already told about.
    announced: Option<TrackId>,
}

impl TrackNotifications {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            announced: None,
        }
    }

    /// Records the current track and returns a notification when it is new and
    /// audible.
    ///
    /// - An empty queue clears the memory, so replaying a track after clearing
    ///   announces it again.
    /// - A new track seen while paused is marked as announced without
    ///   notifying: the listener paused deliberately, so resuming is not news.
    /// - A new track that is not audible yet (stopped, loading, buffering) is
    ///   left unannounced. A later snapshot that shows it playing announces it,
    ///   which covers a restored queue and a queue edit that replaces the
    ///   current track; a stream that fails to load never becomes playing, so it
    ///   is never announced.
    fn observe(&mut self, state: &PlaybackState) -> Option<Notification> {
        if !self.enabled {
            return None;
        }
        let Some(track) = state.track.as_ref() else {
            self.announced = None;
            return None;
        };
        let id = track.id;
        if self.announced == Some(id) {
            return None;
        }
        if state.status == PlaybackStatus::Paused {
            self.announced = Some(id);
            return None;
        }
        let audible = state.status == PlaybackStatus::Playing && !state.loading && !state.buffering;
        if !audible {
            return None;
        }
        self.announced = Some(id);
        Some(track_notification(track))
    }
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
            Reply::Ack
                if matches!(
                    command,
                    Command::Enqueue { .. }
                        | Command::ClearQueue
                        | Command::QueueAlbum { .. }
                        | Command::QueuePlaylist { .. }
                ) =>
            {
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
                    "history",
                ]
                .iter()
                .map(|capability| (*capability).to_owned())
                .collect(),
            }),
            Command::Status => Ok(self.status().await),
            Command::PlaylistCreate {
                name,
                description,
                private,
            } => {
                let playlist = self
                    .player
                    .music()
                    .playlist_create(&name, &description, private)
                    .await?;
                Ok(Reply::Playlist { playlist })
            }
            Command::PlaylistUpdate {
                id,
                name,
                description,
            } => {
                self.player
                    .music()
                    .playlist_update(id, name.as_deref(), description.as_deref())
                    .await?;
                Ok(Reply::Ack)
            }
            Command::PlaylistAdd { id, tracks } => {
                self.player.music().playlist_add(id, &tracks).await?;
                Ok(Reply::Ack)
            }
            Command::PlaylistRemove { id, tracks } => {
                self.player.music().playlist_remove(id, &tracks).await?;
                Ok(Reply::Ack)
            }
            Command::PlaylistDelete { id } => {
                self.player.music().playlist_delete(id).await?;
                Ok(Reply::Ack)
            }
            Command::Hooks => Ok(Reply::Hooks {
                hooks: self.hooks_report(),
            }),
            Command::History { limit } => {
                let limit = limit.unwrap_or(50).clamp(1, 500);
                let entries = self
                    .history
                    .lock()
                    .map_or_else(|_| Vec::new(), |log| log.recent(limit));
                Ok(Reply::History { entries })
            }
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
                Ok(Reply::Search {
                    query,
                    kind,
                    results,
                })
            }
            Command::QueueAlbum { id, next } => {
                self.player.queue_album(id, next).await?;
                Ok(Reply::Ack)
            }
            Command::QueuePlaylist { id, next } => {
                self.player.queue_playlist(id, next).await?;
                Ok(Reply::Ack)
            }
            Command::Artist { id } => {
                let artist = self.player.music().artist(id).await?;
                Ok(Reply::Artist { artist })
            }
            Command::ArtistAlbums { id, limit, offset } => {
                let page = self
                    .player
                    .music()
                    .artist_albums(id, limit.max(1), offset)
                    .await?;
                Ok(Reply::ArtistAlbums { page })
            }
            Command::Album { id } => {
                let detail = self.player.music().album(id).await?;
                Ok(Reply::Album { detail })
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
            PlayTarget::Album { .. } => QueueKind::Album,
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
            PlayTarget::Album { id } => {
                let detail = self.player.music().album(id).await?;
                if detail.tracks.is_empty() {
                    return Err(ApiError::new(
                        lekima_ipc::ErrorCode::NotFound,
                        format!("album {id} has no tracks"),
                    ));
                }
                detail.tracks
            }
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

    /// Configuration and liveness for every hook, in name order.
    fn hooks_report(&self) -> Vec<lekima_ipc::HookStatus> {
        self.hook_configs
            .iter()
            .map(
                |(name, config)| match hooks::HookSpec::from_config(name, config) {
                    Ok(spec) => lekima_ipc::HookStatus {
                        name: name.clone(),
                        events: spec
                            .event_names()
                            .iter()
                            .map(|kind| (*kind).to_owned())
                            .collect(),
                        capabilities: spec.capability_names(),
                        running: self
                            .hooks
                            .iter()
                            .any(|hook| hook.name() == name && hook.is_running()),
                        error: None,
                    },
                    Err(error) => lekima_ipc::HookStatus {
                        name: name.clone(),
                        events: Vec::new(),
                        capabilities: Vec::new(),
                        running: false,
                        error: Some(error),
                    },
                },
            )
            .collect()
    }

    /// Announces a track change once per track, and only once it is audible.
    fn maybe_notify(&self, state: &PlaybackState) {
        let Ok(mut notifications) = self.track_notifications.lock() else {
            return;
        };
        let Some(notification) = notifications.observe(state) else {
            return;
        };
        drop(notifications);
        self.notifications.send(notification);
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

/// Runs commands a hook asked for, through the same path as media keys.
///
/// Capability checks happen in `hooks`; by the time a command reaches this
/// queue it was declared in the hook's configuration.
fn spawn_hook_commands(daemon: Arc<Daemon>, mut commands: mpsc::Receiver<(String, Command)>) {
    tokio::spawn(async move {
        while let Some((hook, command)) = commands.recv().await {
            // Same pre/post handling as a client command: a hook must not leave
            // the preloader flag or the event revision behind.
            if invalidates_preload(&command) {
                daemon.preloaded.store(false, Ordering::SeqCst);
            }
            match daemon.execute(command.clone()).await {
                Ok(mut reply) => {
                    daemon.stamp_revision(&mut reply);
                    daemon.publish(&command, &reply).await;
                }
                Err(error) => tracing::debug!(%hook, error = ?error, "hook command failed"),
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
            daemon.maybe_notify(&state);
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

/// Builds the track-change notification.
///
/// Provider text is sanitized here rather than in `lekima-platform` so the
/// OS-facing string goes through the same helper as every terminal surface.
fn track_notification(track: &Track) -> Notification {
    let title = sanitize_line(&track.name);
    let artist = sanitize_line(&track.artist_names());
    let album = sanitize_line(track.album_name());
    let body = match (artist.is_empty(), album.is_empty()) {
        (true, true) => String::new(),
        (false, true) => artist,
        (true, false) => album,
        (false, false) => format!("{artist} · {album}"),
    };
    Notification {
        title: if title.is_empty() {
            "Unknown track".to_owned()
        } else {
            title
        },
        body,
    }
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

/// Loads the history, degrading to an empty log when the file is unreadable.
///
/// A corrupt file is reported but not deleted: the operator can inspect it, and
/// the daemon keeps working.
fn load_history(store: &HistoryStore, capacity: usize) -> (HistoryLog, bool) {
    match store.load_pruned(capacity) {
        Ok(loaded) => loaded,
        Err(error) => {
            tracing::warn!(%error, "ignoring an unreadable history file");
            (HistoryLog::new(capacity), false)
        }
    }
}

/// Rewrites a log that was trimmed or filtered on load, and returns the
/// revision now on disk.
///
/// Without this the file could stay over `[history] capacity` or keep
/// zero-played rows until the next play, and a raised capacity would resurrect
/// records the running build had already dropped. An all-filtered file is left
/// alone: deleting the last trace of a user's history is worse than keeping
/// junk that no surface shows.
fn prune_history(store: &HistoryStore, log: &HistoryLog, pruned: bool) -> u64 {
    if !pruned || log.is_empty() {
        return log.revision();
    }
    let records: Vec<_> = log.records().iter().cloned().collect();
    if let Err(error) = store.save_records(&records) {
        // The file still holds the unpruned records, so do not claim this
        // revision: the next save (shutdown, or a real play) retries.
        tracing::warn!(%error, "failed to rewrite the pruned listening history");
        return log.revision().saturating_sub(1);
    }
    log.revision()
}

/// Records one snapshot, returning whether the log changed enough to persist.
fn observe_history(daemon: &Daemon, state: &PlaybackState, now_ms: i64) -> bool {
    daemon
        .history
        .lock()
        .is_ok_and(|mut log| log.observe(state, now_ms))
}

/// Writes the history if it changed since the last write.
///
/// Two paths write (the poller and shutdown), and a poller snapshot taken
/// before the final play was closed must not overwrite it. The log revision
/// makes the newer snapshot win, and the write lock keeps the shared temp file
/// from being interleaved.
fn save_history(daemon: &Daemon) {
    let (records, revision) = {
        let Ok(log) = daemon.history.lock() else {
            return;
        };
        (
            log.records().iter().cloned().collect::<Vec<_>>(),
            log.revision(),
        )
    };
    write_if_newer(
        &daemon.history_store,
        &daemon.history_saved,
        &records,
        revision,
    );
}

/// Writes `records` unless `revision` is not newer than what is already on disk.
///
/// Returns whether the file was written. Extracted from the daemon so the
/// ordering rule can be tested without a player.
fn write_if_newer(
    store: &HistoryStore,
    saved: &StdMutex<u64>,
    records: &[lekima_domain::PlayRecord],
    revision: u64,
) -> bool {
    if records.is_empty() {
        return false;
    }
    let Ok(mut saved) = saved.lock() else {
        return false;
    };
    if revision <= *saved {
        return false;
    }
    if let Err(error) = store.save_records(records) {
        tracing::warn!(%error, "failed to save the listening history");
        return false;
    }
    *saved = revision;
    true
}

/// Writes the history on shutdown, including the track that was playing.
async fn persist_history(daemon: &Daemon) {
    let state = daemon.player.snapshot().await;
    let _ = observe_history(daemon, &state, now_ms());
    if let Ok(mut log) = daemon.history.lock() {
        log.finish();
    }
    save_history(daemon);
}

/// Appends finished plays to the history.
///
/// Samples at 4 Hz so a short track's play time is accurate to 250 ms; writes
/// happen only when a play ends, so a long track never rewrites the file.
fn spawn_history(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            let state = daemon.player.snapshot().await;
            if !observe_history(&daemon, &state, now_ms()) {
                continue;
            }
            save_history(&daemon);
        }
    });
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
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

#[cfg(test)]
mod tests {
    use lekima_domain::{
        Album, AlbumId, Artist, ArtistId, PlaybackState, PlaybackStatus, Quality, QueueKind, Track,
        TrackId,
    };

    use super::{TrackNotifications, track_notification};

    fn track(name: &str, artist: Option<&str>, album: Option<&str>) -> Track {
        Track {
            id: TrackId::from(1_u64),
            name: name.to_owned(),
            artists: artist
                .map(|name| {
                    vec![Artist {
                        id: ArtistId::from(7_u64),
                        name: name.to_owned(),
                    }]
                })
                .unwrap_or_default(),
            album: album.map(|name| Album {
                id: AlbumId::from(9_u64),
                name: name.to_owned(),
                description: String::new(),
                cover_url: None,
                artist: None,
                year: None,
                track_count: None,
            }),
            duration_ms: 0,
        }
    }

    #[test]
    fn track_notification_joins_artist_and_album() {
        let notification = track_notification(&track("Song", Some("Artist"), Some("Album")));
        assert_eq!(notification.title, "Song");
        assert_eq!(notification.body, "Artist · Album");
    }

    #[test]
    fn track_notification_falls_back_to_artist_when_there_is_no_album() {
        let without_album = track_notification(&track("Song", Some("Artist"), None));
        assert_eq!(without_album.body, "Artist");

        // `artist_names()` never returns an empty string, so a track with no
        // credited artist reads "Unknown artist · Album" rather than just the
        // album.
        let without_artist = track_notification(&track("Song", None, Some("Album")));
        assert_eq!(without_artist.body, "Unknown artist · Album");
    }

    #[test]
    fn track_notification_sanitizes_provider_text() {
        let notification = track_notification(&track(
            "Song\u{1b}[31m red",
            Some("Art\u{202e}ist"),
            Some("Al\u{7f}bum"),
        ));
        assert!(!notification.title.contains('\u{1b}'), "{notification:?}");
        assert!(!notification.body.contains('\u{202e}'), "{notification:?}");
        assert!(!notification.body.contains('\u{7f}'), "{notification:?}");
    }

    #[test]
    fn track_notification_names_an_empty_title() {
        let notification = track_notification(&track("   ", None, None));
        assert_eq!(notification.title, "Unknown track");
        assert_eq!(notification.body, "Unknown artist");
    }

    fn state(status: PlaybackStatus, id: Option<u64>) -> PlaybackState {
        PlaybackState {
            status,
            track: id.map(|id| Track {
                id: TrackId::from(id),
                ..track("Song", Some("Artist"), Some("Album"))
            }),
            position_ms: 0,
            volume: 0.7,
            muted: false,
            repeat: lekima_domain::RepeatMode::Off,
            shuffle: false,
            queue_length: usize::from(id.is_some()),
            queue_index: id.map(|_| 0),
            queue_kind: QueueKind::Manual,
            liked: false,
            loading: false,
            buffering: false,
            cached: false,
            quality: Quality::Exhigh,
            sleep_remaining_ms: None,
            wave: Vec::new(),
            stream_duration_ms: None,
            buffered: Vec::new(),
            revision: 0,
            generation: 0,
        }
    }

    #[test]
    fn an_older_snapshot_never_overwrites_a_newer_save() {
        let dir = tempfile::tempdir().unwrap();
        let store = lekima_config::HistoryStore::new(dir.path().join("history.json"));
        let saved = std::sync::Mutex::new(0_u64);

        let one = lekima_domain::PlayRecord {
            track: track("Song", None, None),
            started_at: 1,
            played_ms: 1_000,
            completed: false,
            kind: QueueKind::Manual,
        };
        let two = lekima_domain::PlayRecord {
            started_at: 2,
            ..one.clone()
        };

        assert!(super::write_if_newer(
            &store,
            &saved,
            std::slice::from_ref(&one),
            1
        ));
        // A poller that cloned before the final play closed must not win, even
        // with a non-empty (older) snapshot.
        assert!(!super::write_if_newer(
            &store,
            &saved,
            std::slice::from_ref(&one),
            1
        ));
        assert!(!super::write_if_newer(&store, &saved, &[], 2));
        let both = [one.clone(), two.clone()];
        assert!(super::write_if_newer(&store, &saved, &both, 2));
        assert!(!super::write_if_newer(
            &store,
            &saved,
            std::slice::from_ref(&one),
            1
        ));
        assert!(!super::write_if_newer(&store, &saved, &both, 2));

        let loaded = store.load(10).unwrap();
        assert_eq!(loaded.len(), 2, "the stale one-record snapshot lost");
        assert_eq!(loaded.records()[1].started_at, 2);
    }

    #[test]
    fn a_failed_prune_does_not_claim_the_disk_revision() {
        let dir = tempfile::tempdir().unwrap();
        // A regular file where the store expects a directory makes every write
        // fail.
        let blocker = dir.path().join("blocked");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let store = lekima_config::HistoryStore::new(blocker.join("history.json"));

        let record = lekima_domain::PlayRecord {
            track: track("Song", None, None),
            started_at: 1,
            played_ms: 1_000,
            completed: false,
            kind: QueueKind::Manual,
        };
        let log = lekima_domain::HistoryLog::from_records([record], 1);

        let baseline = super::prune_history(&store, &log, true);
        assert_eq!(
            baseline, 0,
            "a failed rewrite must stay below the log revision so a later save retries"
        );
    }

    #[test]
    fn notifications_are_silent_when_disabled() {
        let mut notifications = TrackNotifications::new(false);
        assert!(
            notifications
                .observe(&state(PlaybackStatus::Playing, Some(1)))
                .is_none()
        );
    }

    #[test]
    fn a_new_track_announces_once() {
        let mut notifications = TrackNotifications::new(true);
        let playing = state(PlaybackStatus::Playing, Some(1));

        assert!(notifications.observe(&playing).is_some());
        assert!(notifications.observe(&playing).is_none());
    }

    #[test]
    fn pausing_and_seeking_do_not_announce_again() {
        let mut notifications = TrackNotifications::new(true);
        let mut playing = state(PlaybackStatus::Playing, Some(1));
        assert!(notifications.observe(&playing).is_some());

        playing.status = PlaybackStatus::Paused;
        playing.position_ms = 30_000;
        assert!(notifications.observe(&playing).is_none());

        playing.status = PlaybackStatus::Playing;
        assert!(notifications.observe(&playing).is_none());
    }

    #[test]
    fn a_track_seen_while_paused_is_not_announced_on_resume() {
        let mut notifications = TrackNotifications::new(true);

        assert!(
            notifications
                .observe(&state(PlaybackStatus::Paused, Some(2)))
                .is_none()
        );
        assert!(
            notifications
                .observe(&state(PlaybackStatus::Playing, Some(2)))
                .is_none()
        );
        assert!(
            notifications
                .observe(&state(PlaybackStatus::Playing, Some(3)))
                .is_some()
        );
    }

    #[test]
    fn an_empty_queue_never_announces() {
        let mut notifications = TrackNotifications::new(true);
        assert!(
            notifications
                .observe(&state(PlaybackStatus::Stopped, None))
                .is_none()
        );
    }

    #[test]
    fn starting_from_stopped_announces_when_playback_begins() {
        let mut notifications = TrackNotifications::new(true);

        // A restored queue arrives stopped: nothing was heard yet.
        assert!(
            notifications
                .observe(&state(PlaybackStatus::Stopped, Some(1)))
                .is_none()
        );
        assert!(
            notifications
                .observe(&state(PlaybackStatus::Playing, Some(1)))
                .is_some()
        );
        assert!(
            notifications
                .observe(&state(PlaybackStatus::Playing, Some(1)))
                .is_none()
        );
    }

    #[test]
    fn a_loading_track_is_announced_only_when_it_starts() {
        let mut notifications = TrackNotifications::new(true);
        let mut playing = state(PlaybackStatus::Playing, Some(4));
        playing.loading = true;

        assert!(notifications.observe(&playing).is_none());
        playing.loading = false;
        assert!(notifications.observe(&playing).is_some());
    }

    #[test]
    fn a_buffering_track_is_announced_only_when_it_starts() {
        let mut notifications = TrackNotifications::new(true);
        let mut playing = state(PlaybackStatus::Playing, Some(5));
        playing.buffering = true;

        assert!(notifications.observe(&playing).is_none());
        playing.buffering = false;
        assert!(notifications.observe(&playing).is_some());
    }

    #[test]
    fn a_track_that_fails_to_load_is_never_announced() {
        let mut notifications = TrackNotifications::new(true);
        let mut loading = state(PlaybackStatus::Playing, Some(6));
        loading.loading = true;
        assert!(notifications.observe(&loading).is_none());

        // The stream failed: the track never becomes current audio.
        assert!(
            notifications
                .observe(&state(PlaybackStatus::Stopped, Some(6)))
                .is_none()
        );
    }

    #[test]
    fn clearing_the_queue_allows_the_same_track_again() {
        let mut notifications = TrackNotifications::new(true);
        let playing = state(PlaybackStatus::Playing, Some(1));

        assert!(notifications.observe(&playing).is_some());
        assert!(
            notifications
                .observe(&state(PlaybackStatus::Stopped, None))
                .is_none()
        );
        assert!(notifications.observe(&playing).is_some());
    }
}
