//! The player daemon: owns the audio engine, the provider session and the
//! queue. Every other process is a client.

use std::{
    fs,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use lekima_config::{Config, FileCredentials, Paths};
use lekima_core::{PlayRequest, Player};
use lekima_domain::{CredentialsStore, PlaybackState, QueueKind, SearchKind};
use lekima_ipc::{ApiError, Command, PlayTarget, Reply, Request, Response, serve_connection};
use lekima_ncm::{NcmMusicService, NcmOptions};
use tokio::{net::UnixListener, sync::Notify};

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
        cookie_path: paths.session_file(),
        cache: config.provider.cache,
        cache_ttl: Duration::from_secs(config.provider.cache_ttl_secs),
    })?);
    let audio = Arc::new(
        lekima_audio::RodioAudioEngine::open(config.playback.volume)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?,
    );
    let player = Arc::new(Player::new(
        music,
        audio,
        config.playback.quality,
        config.playback.volume,
    ));

    let daemon = Arc::new(Daemon {
        player: Arc::clone(&player),
        credentials: FileCredentials::new(paths.session_file()),
        shutdown: Notify::new(),
    });

    {
        let likes = Arc::clone(&player);
        tokio::spawn(async move { likes.load_likes().await });
    }
    spawn_ticker(Arc::clone(&player));

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
    let _ = daemon.player.stop().await;
    drop(listener);
    let _ = fs::remove_file(socket);
    Ok(())
}

struct Daemon {
    player: Arc<Player>,
    credentials: FileCredentials,
    shutdown: Notify,
}

impl Daemon {
    async fn handle(&self, request: Request) -> Response {
        let id = request.id;
        match self.execute(request.command).await {
            Ok(reply) => Response::ok(id, reply),
            Err(error) => Response::err(id, error),
        }
    }

    async fn execute(&self, command: Command) -> std::result::Result<Reply, ApiError> {
        match command {
            Command::Ping => Ok(Reply::Pong),
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
                Ok(Reply::Lyrics { lyrics })
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
        let state: PlaybackState = self.player.snapshot().await;
        Reply::Status { state }
    }
}

fn spawn_ticker(player: Arc<Player>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            if let Err(error) = player.tick().await {
                tracing::warn!(%error, "auto-advance failed");
            }
            if let Err(error) = player.fm_maybe_prefetch().await {
                tracing::debug!(%error, "fm prefetch skipped");
            }
        }
    });
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
