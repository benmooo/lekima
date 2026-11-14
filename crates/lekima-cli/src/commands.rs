//! Command dispatch: the composition root that wires adapters to the CLI.

use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use clap::CommandFactory;
use lekima_config::{Config, FileCredentials, Paths};
use lekima_domain::{CredentialsStore, MusicService, PlaybackState};
use lekima_ipc::{Command, ErrorCode, PlayTarget, Reply, call, is_running};
use lekima_ncm::{NcmMusicService, NcmOptions};

use crate::{
    cli::{Cli, DaemonCommand, PlaylistCommand, QueueCommand, TopCommand},
    daemon,
    output::{self, Failure},
};

pub async fn execute(cli: Cli, paths: Paths, config: Config) -> Result<(), Failure> {
    let socket = cli.socket.clone().unwrap_or_else(|| paths.socket());
    let json = cli.json;

    match cli.command {
        TopCommand::Tui => {
            ensure_daemon(&socket, &paths, &config).await?;
            lekima_tui::run(socket)
                .await
                .map_err(|error| Failure::internal(error.to_string()))?;
            Ok(())
        }

        TopCommand::Play {
            query,
            ids,
            playlist,
        } => {
            let target = if !ids.is_empty() {
                PlayTarget::Tracks {
                    ids: ids.into_iter().map(Into::into).collect(),
                }
            } else if let Some(id) = playlist {
                PlayTarget::Playlist { id: id.into() }
            } else {
                let query = query.join(" ");
                if query.trim().is_empty() {
                    return Err(Failure::usage(
                        "nothing to play: pass a query, --id or --playlist",
                    ));
                }
                PlayTarget::Query { query }
            };
            let reply = daemon_call(&socket, &paths, &config, Command::Play { target }).await?;
            output::print_reply(&reply, json);
            Ok(())
        }

        TopCommand::Pause => simple(&socket, &paths, &config, Command::Pause, json).await,
        TopCommand::Resume => simple(&socket, &paths, &config, Command::Resume, json).await,
        TopCommand::Toggle => simple(&socket, &paths, &config, Command::Toggle, json).await,
        TopCommand::Stop => simple(&socket, &paths, &config, Command::Stop, json).await,
        TopCommand::Next => simple(&socket, &paths, &config, Command::Next, json).await,
        TopCommand::Prev => simple(&socket, &paths, &config, Command::Prev, json).await,

        TopCommand::Seek { position } => {
            let offset = parse_position(&position)?;
            let target = if offset < 0 || position.trim_start().starts_with('+') {
                let reply = daemon_call(&socket, &paths, &config, Command::Status).await?;
                let state = state_of(&reply)?;
                let base = i64::try_from(state.position_ms).unwrap_or(i64::MAX);
                u64::try_from((base + offset).max(0)).unwrap_or(0)
            } else {
                u64::try_from(offset).unwrap_or(0)
            };
            let reply = daemon_call(
                &socket,
                &paths,
                &config,
                Command::Seek {
                    position_ms: target,
                },
            )
            .await?;
            output::print_reply(&reply, json);
            Ok(())
        }

        TopCommand::Volume {
            level,
            mute,
            unmute,
        } => {
            let command = if mute || unmute {
                let reply = daemon_call(&socket, &paths, &config, Command::Status).await?;
                let state = state_of(&reply)?;
                let desired = mute;
                if state.muted == desired {
                    Command::Status
                } else {
                    Command::ToggleMute
                }
            } else {
                let level = level
                    .ok_or_else(|| Failure::usage("pass a level (0-100), --mute or --unmute"))?;
                Command::SetVolume {
                    level: parse_volume(&level)?,
                }
            };
            let reply = daemon_call(&socket, &paths, &config, command).await?;
            output::print_reply(&reply, json);
            Ok(())
        }

        TopCommand::Queue { action } => match action.unwrap_or(QueueCommand::List) {
            QueueCommand::List => {
                let reply = daemon_call(&socket, &paths, &config, Command::Queue).await?;
                output::print_reply(&reply, json);
                Ok(())
            }
            QueueCommand::Add { query, ids, next } => {
                let ids = if ids.is_empty() {
                    let query = query.join(" ");
                    if query.trim().is_empty() {
                        return Err(Failure::usage("pass a query or --id"));
                    }
                    let service = service(&paths, &config)?;
                    let results =
                        bounded(service.search(&query, lekima_domain::SearchKind::Track, 25, 0))
                            .await?;
                    if results.tracks.is_empty() {
                        return Err(Failure::new(
                            ErrorCode::NotFound,
                            format!("no tracks matched `{query}`"),
                        ));
                    }
                    results.tracks.into_iter().map(|track| track.id).collect()
                } else {
                    ids.into_iter().map(Into::into).collect()
                };
                let reply = daemon_call(
                    &socket,
                    &paths,
                    &config,
                    Command::Enqueue { tracks: ids, next },
                )
                .await?;
                output::print_reply(&reply, json);
                Ok(())
            }
            QueueCommand::Clear => {
                simple(&socket, &paths, &config, Command::ClearQueue, json).await
            }
        },

        TopCommand::Search {
            query,
            kind,
            limit,
            offset,
        } => {
            let query = query.join(" ");
            let service = service(&paths, &config)?;
            let results = bounded(service.search(&query, kind.into(), limit, offset)).await?;
            output::print_reply(&Reply::Search { results }, json);
            Ok(())
        }

        TopCommand::Playlist { action } => {
            let service = service(&paths, &config)?;
            match action {
                PlaylistCommand::List => {
                    let playlists = bounded(service.playlists(None)).await?;
                    output::print_reply(&Reply::Playlists { playlists }, json);
                }
                PlaylistCommand::Show { id } => {
                    let tracks = bounded(service.playlist_tracks(id.into())).await?;
                    output::print_reply(&Reply::Tracks { tracks }, json);
                }
            }
            Ok(())
        }

        TopCommand::Lyrics { id } => {
            let reply = daemon_call(
                &socket,
                &paths,
                &config,
                Command::Lyrics {
                    id: id.map(Into::into),
                },
            )
            .await?;
            output::print_reply(&reply, json);
            Ok(())
        }

        TopCommand::Status => {
            let reply = daemon_call(&socket, &paths, &config, Command::Status).await?;
            output::print_reply(&reply, json);
            Ok(())
        }

        TopCommand::Login { cookie_file, qr } => {
            login(&paths, &socket, &config, cookie_file, qr, json).await
        }

        TopCommand::Logout => logout(&paths, &socket, json).await,

        TopCommand::Whoami => {
            let service = service(&paths, &config)?;
            let user = bounded(service.current_user()).await?;
            output::print_reply(&Reply::User { user }, json);
            Ok(())
        }

        TopCommand::Daemon { action } => {
            daemon_command(action, &socket, &paths, &config, json).await
        }

        TopCommand::Doctor => doctor(&socket, &paths, &config, json).await,

        TopCommand::Capabilities => {
            let tree = command_tree(&Cli::command());
            if json {
                output::emit_json(true, Some(tree), None);
            } else {
                let encoded = serde_json::to_string_pretty(&tree)
                    .map_err(|error| Failure::internal(error.to_string()))?;
                crate::say!("{encoded}");
            }
            Ok(())
        }

        TopCommand::Completions { shell } => {
            let mut command = Cli::command();
            clap_complete::generate(shell, &mut command, "lekima", &mut std::io::stdout());
            Ok(())
        }
    }
}

async fn simple(
    socket: &Path,
    paths: &Paths,
    config: &Config,
    command: Command,
    json: bool,
) -> Result<(), Failure> {
    let reply = daemon_call(socket, paths, config, command).await?;
    output::print_reply(&reply, json);
    Ok(())
}

/// Provider calls are bounded so an agent invocation never hangs.
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(30);

async fn bounded<F, T>(future: F) -> Result<T, Failure>
where
    F: std::future::Future<Output = Result<T, lekima_domain::DomainError>>,
{
    match tokio::time::timeout(PROVIDER_TIMEOUT, future).await {
        Ok(result) => result.map_err(Into::into),
        Err(_) => Err(Failure::new(
            ErrorCode::Backend,
            "the music provider did not answer within 30 seconds",
        )),
    }
}

fn service(paths: &Paths, config: &Config) -> Result<NcmMusicService, Failure> {
    NcmMusicService::new(NcmOptions {
        cookie_path: paths.session_file(),
        cache: config.provider.cache,
        cache_ttl: Duration::from_secs(config.provider.cache_ttl_secs),
    })
    .map_err(Into::into)
}

async fn daemon_call(
    socket: &Path,
    paths: &Paths,
    config: &Config,
    command: Command,
) -> Result<Reply, Failure> {
    match call(socket, command.clone()).await {
        Ok(reply) => Ok(reply),
        Err(error) if error.is_not_running() && config.daemon.autostart => {
            daemon::spawn_detached(socket, &paths.log_file())
                .map_err(|error| Failure::new(ErrorCode::Daemon, error.to_string()))?;
            daemon::wait_until_running(socket, Duration::from_secs(10)).await?;
            call(socket, command).await.map_err(Into::into)
        }
        Err(error) => Err(error.into()),
    }
}

async fn ensure_daemon(socket: &Path, paths: &Paths, config: &Config) -> Result<(), Failure> {
    if is_running(socket).await {
        return Ok(());
    }
    if !config.daemon.autostart {
        return Err(Failure::new(
            ErrorCode::Daemon,
            format!(
                "no daemon at {} and autostart is disabled",
                socket.display()
            ),
        ));
    }
    daemon::spawn_detached(socket, &paths.log_file())
        .map_err(|error| Failure::new(ErrorCode::Daemon, error.to_string()))?;
    daemon::wait_until_running(socket, Duration::from_secs(10)).await
}

fn state_of(reply: &Reply) -> Result<PlaybackState, Failure> {
    match reply {
        Reply::Status { state } => Ok(state.clone()),
        _ => Err(Failure::internal("daemon returned an unexpected reply")),
    }
}

async fn login(
    paths: &Paths,
    socket: &Path,
    config: &Config,
    cookie_file: Option<PathBuf>,
    qr: bool,
    json: bool,
) -> Result<(), Failure> {
    if qr {
        return login_with_qr(paths, socket, config, json).await;
    }

    let cookie = match cookie_file {
        Some(path) if path == Path::new("-") => read_stdin()?,
        Some(path) => std::fs::read_to_string(&path).map_err(|error| {
            Failure::usage(format!("failed to read {}: {error}", path.display()))
        })?,
        None => read_stdin()?,
    };
    let cookie = cookie.trim().to_owned();
    if !cookie.contains('=') || !cookie.to_ascii_uppercase().contains("MUSIC_U") {
        return Err(Failure::usage(
            "that does not look like a music.163.com cookie header (expected `MUSIC_U=…`)",
        ));
    }

    FileCredentials::new(paths.session_file()).save(&cookie)?;
    restart_daemon(socket).await;

    if json {
        output::emit_json(
            true,
            Some(serde_json::json!({ "saved": true, "session_file": paths.session_file() })),
            None,
        );
    } else {
        crate::say!(
            "session saved to {}; the next command starts a fresh daemon",
            paths.session_file().display()
        );
    }
    Ok(())
}

async fn login_with_qr(
    paths: &Paths,
    socket: &Path,
    config: &Config,
    json: bool,
) -> Result<(), Failure> {
    use lekima_domain::QrStatus;

    let service = service(paths, config)?;
    let started = bounded(service.qr_start()).await?;
    if json {
        output::emit_json(
            true,
            Some(serde_json::json!({ "key": started.key, "url": started.url })),
            None,
        );
    } else {
        crate::say!("Scan with the NetEase Cloud Music mobile app:\n");
        crate::say!("{}", render_qr(&started.url)?);
        crate::say!("or open {}\n", started.url);
    }

    let mut announced_scanned = false;
    for _ in 0..90 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        match bounded(service.qr_poll(&started.key)).await {
            Ok(QrStatus::Waiting) => {}
            Ok(QrStatus::Scanned) => {
                if !announced_scanned {
                    announced_scanned = true;
                    if !json {
                        crate::say!("scanned — confirm on your phone");
                    }
                }
            }
            Ok(QrStatus::Confirmed { nickname }) => {
                restart_daemon(socket).await;
                if json {
                    output::emit_json(
                        true,
                        Some(serde_json::json!({ "signed_in": true, "nickname": nickname })),
                        None,
                    );
                } else {
                    let label = nickname.map_or_else(String::new, |name| format!(" as {name}"));
                    crate::say!("signed in{label}");
                }
                return Ok(());
            }
            Ok(QrStatus::Expired) => {
                return Err(Failure::new(
                    ErrorCode::Unauthorized,
                    "the QR code expired; run `lekima login --qr` again",
                ));
            }
            Err(failure) => return Err(failure),
        }
    }

    Err(Failure::new(
        ErrorCode::Backend,
        "timed out waiting for QR confirmation",
    ))
}

fn render_qr(url: &str) -> Result<String, Failure> {
    use qrcode::render::unicode;
    let code = qrcode::QrCode::new(url.as_bytes())
        .map_err(|error| Failure::internal(format!("failed to encode the QR code: {error}")))?;
    Ok(code.render::<unicode::Dense1x2>().quiet_zone(true).build())
}

async fn logout(paths: &Paths, socket: &Path, json: bool) -> Result<(), Failure> {
    FileCredentials::new(paths.session_file()).clear()?;
    restart_daemon(socket).await;
    if json {
        output::emit_json(true, Some(serde_json::json!({ "saved": false })), None);
    } else {
        crate::say!("session removed");
    }
    Ok(())
}

async fn restart_daemon(socket: &Path) {
    if is_running(socket).await {
        let _ = call(socket, Command::Shutdown).await;
        for _ in 0..30 {
            if !is_running(socket).await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

async fn daemon_command(
    action: DaemonCommand,
    socket: &Path,
    paths: &Paths,
    config: &Config,
    json: bool,
) -> Result<(), Failure> {
    match action {
        DaemonCommand::Start => {
            if is_running(socket).await {
                return report_running(socket, false, json);
            }
            daemon::spawn_detached(socket, &paths.log_file())
                .map_err(|error| Failure::new(ErrorCode::Daemon, error.to_string()))?;
            daemon::wait_until_running(socket, Duration::from_secs(10)).await?;
            report_running(socket, true, json)
        }
        DaemonCommand::Stop => {
            if !is_running(socket).await {
                if json {
                    output::emit_json(true, Some(serde_json::json!({ "running": false })), None);
                } else {
                    crate::say!("daemon is not running");
                }
                return Ok(());
            }
            let _ = call(socket, Command::Shutdown).await;
            for _ in 0..30 {
                if !is_running(socket).await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if json {
                output::emit_json(true, Some(serde_json::json!({ "running": false })), None);
            } else {
                crate::say!("daemon stopped");
            }
            Ok(())
        }
        DaemonCommand::Status => report_running(socket, false, json),
        DaemonCommand::Run { detached } => daemon::run(paths, config, socket, detached)
            .await
            .map_err(|error| Failure::internal(error.to_string())),
        DaemonCommand::Logs { lines } => {
            let raw = std::fs::read_to_string(paths.log_file()).unwrap_or_default();
            let tail: Vec<_> = raw
                .lines()
                .rev()
                .take(lines)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            crate::say!("{}", tail.join("\n"));
            Ok(())
        }
    }
}

fn report_running(socket: &Path, started: bool, json: bool) -> Result<(), Failure> {
    let running = std::os::unix::net::UnixStream::connect(socket).is_ok();
    if json {
        output::emit_json(
            true,
            Some(serde_json::json!({
                "running": running,
                "started": started,
                "socket": socket,
            })),
            None,
        );
    } else if running {
        crate::say!("daemon running at {}", socket.display());
    } else {
        crate::say!("daemon is not running");
    }
    Ok(())
}

async fn doctor(socket: &Path, paths: &Paths, config: &Config, json: bool) -> Result<(), Failure> {
    let session_present = FileCredentials::new(paths.session_file()).load()?.is_some();
    let daemon_running = is_running(socket).await;

    let audio = match lekima_audio::RodioAudioEngine::open(0.0) {
        Ok(engine) => {
            drop(engine);
            serde_json::json!({ "ok": true })
        }
        Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }),
    };

    let provider = match service(paths, config) {
        Ok(service) => {
            match tokio::time::timeout(Duration::from_secs(8), service.current_user()).await {
                Ok(Ok(user)) => serde_json::json!({
                    "ok": true,
                    "signed_in": user.is_some(),
                }),
                Ok(Err(error)) => serde_json::json!({ "ok": false, "error": error.to_string() }),
                Err(_) => serde_json::json!({ "ok": false, "error": "timed out" }),
            }
        }
        Err(error) => serde_json::json!({ "ok": false, "error": error.message }),
    };

    let report = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "socket": { "path": socket, "daemon_running": daemon_running },
        "session": { "present": session_present, "file": paths.session_file() },
        "config": { "file": paths.config_file(), "loaded": true },
        "audio": audio,
        "provider": provider,
        "log": paths.log_file(),
    });

    if json {
        output::emit_json(true, Some(report), None);
        return Ok(());
    }

    crate::say!(
        "lekima {} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    );
    crate::say!(
        "daemon   {} at {}",
        if daemon_running {
            "running"
        } else {
            "not running"
        },
        socket.display()
    );
    crate::say!(
        "session  {} ({})",
        if session_present {
            "present"
        } else {
            "missing"
        },
        paths.session_file().display()
    );
    crate::say!("config   {}", paths.config_file().display());
    crate::say!("audio    {audio}");
    crate::say!("provider {provider}");
    crate::say!("log      {}", paths.log_file().display());
    Ok(())
}

fn command_tree(command: &clap::Command) -> serde_json::Value {
    let subcommands: Vec<_> = command
        .get_subcommands()
        .map(command_tree)
        .filter(|value| value["name"] != "help")
        .collect();
    serde_json::json!({
        "name": command.get_name(),
        "about": command.get_about().map(ToString::to_string),
        "options": command
            .get_arguments()
            .filter(|argument| argument.get_id() != "help" && argument.get_id() != "version")
            .map(|argument| serde_json::json!({
                "name": argument.get_id().to_string(),
                "short": argument.get_short(),
                "long": argument.get_long(),
                "help": argument.get_help().map(ToString::to_string),
                "takes_value": argument.get_action().takes_values(),
            }))
            .collect::<Vec<_>>(),
        "subcommands": subcommands,
    })
}

fn parse_volume(raw: &str) -> Result<f32, Failure> {
    let trimmed = raw.trim().trim_end_matches('%');
    let value: f32 = trimmed
        .parse()
        .map_err(|_| Failure::usage(format!("`{raw}` is not a volume level")))?;
    let fraction = if value > 1.0 { value / 100.0 } else { value };
    if !(0.0..=1.0).contains(&fraction) {
        return Err(Failure::usage("volume must be between 0 and 100"));
    }
    Ok(fraction)
}

/// Parses `1:23`, `83`, `83s`, `2m`, `+10s`, `-10s` into milliseconds.
fn parse_position(raw: &str) -> Result<i64, Failure> {
    let raw = raw.trim();
    let (sign, body) = match raw.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, raw.strip_prefix('+').unwrap_or(raw)),
    };
    if body.is_empty() {
        return Err(Failure::usage("empty position"));
    }

    let millis = if body.contains(':') {
        let mut total = 0i64;
        for part in body.split(':') {
            let value: i64 = part
                .parse()
                .map_err(|_| Failure::usage(format!("`{raw}` is not a position")))?;
            total = total * 60 + value;
        }
        total * 1000
    } else if let Some(hours) = body.strip_suffix('h') {
        hours
            .parse::<i64>()
            .map_err(|_| Failure::usage("invalid hours"))?
            * 3_600_000
    } else if let Some(minutes) = body.strip_suffix('m') {
        minutes
            .parse::<i64>()
            .map_err(|_| Failure::usage("invalid minutes"))?
            * 60_000
    } else {
        let seconds = body.strip_suffix('s').unwrap_or(body);
        seconds
            .parse::<f64>()
            .map_err(|_| Failure::usage(format!("`{raw}` is not a position")))?
            .mul_add(1000.0, 0.0) as i64
    };

    Ok(sign * millis)
}

fn read_stdin() -> Result<String, Failure> {
    let mut buffer = String::new();
    std::io::stdin()
        .read_to_string(&mut buffer)
        .map_err(|error| Failure::usage(format!("failed to read stdin: {error}")))?;
    if buffer.trim().is_empty() {
        return Err(Failure::usage(
            "stdin was empty; pipe the cookie header, e.g. `lekima login --cookie-file -`",
        ));
    }
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::{parse_position, parse_volume};

    #[test]
    fn positions_support_colons_and_units() {
        assert_eq!(parse_position("1:23").unwrap(), 83_000);
        assert_eq!(parse_position("83s").unwrap(), 83_000);
        assert_eq!(parse_position("2m").unwrap(), 120_000);
        assert_eq!(parse_position("+10s").unwrap(), 10_000);
        assert_eq!(parse_position("-10s").unwrap(), -10_000);
    }

    #[test]
    fn volume_accepts_percent_and_fraction() {
        assert!((parse_volume("70").unwrap() - 0.7).abs() < f32::EPSILON);
        assert!((parse_volume("70%").unwrap() - 0.7).abs() < f32::EPSILON);
        assert!((parse_volume("0.7").unwrap() - 0.7).abs() < f32::EPSILON);
        assert!(parse_volume("140").is_err());
    }
}
