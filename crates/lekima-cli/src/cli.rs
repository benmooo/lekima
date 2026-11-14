//! Command line surface. This file is the contract with humans and agents:
//! names, flags and help text are API.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "lekima",
    version,
    about = "NetEase Cloud Music in your terminal — vim-first, agent-native",
    long_about = "A keyboard-driven NetEase Cloud Music player.\n\n\
                  Playback is owned by a small background daemon, so music keeps \
                  playing after the TUI closes and every command works from scripts \
                  and code agents. Use --json for machine-readable output.",
    propagate_version = true
)]
pub struct Cli {
    /// Unix socket of the player daemon.
    #[arg(long, global = true, env = "LEKIMA_SOCKET", value_name = "PATH")]
    pub socket: Option<PathBuf>,

    /// Emit one JSON object instead of human-readable output.
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress non-essential output.
    #[arg(long, short, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: TopCommand,
}

#[derive(Debug, Subcommand)]
pub enum TopCommand {
    /// Open the interactive player.
    Tui,

    /// Play a search query, track ids, or a playlist.
    Play {
        /// Search query, e.g. `lekima play bohemian rhapsody`.
        #[arg(value_name = "QUERY")]
        query: Vec<String>,

        /// Play specific track ids (repeatable).
        #[arg(long = "id", value_name = "ID")]
        ids: Vec<u64>,

        /// Play every track of a playlist.
        #[arg(long, value_name = "ID")]
        playlist: Option<u64>,
    },

    /// Pause playback.
    Pause,

    /// Resume playback.
    Resume,

    /// Toggle between playing and paused.
    Toggle,

    /// Stop playback and clear the current track.
    Stop,

    /// Play the next track.
    Next,

    /// Play the previous track.
    Prev,

    /// Seek to an absolute position or by a relative offset.
    Seek {
        /// `1:23`, `83`, `83s`, `+10s`, `-10s`, `2m`.
        #[arg(value_name = "POSITION", allow_hyphen_values = true)]
        position: String,
    },

    /// Show or change the volume.
    Volume {
        /// Percent (`70`, `70%`) or fraction (`0.7`).
        #[arg(value_name = "LEVEL")]
        level: Option<String>,

        /// Mute output.
        #[arg(long, conflicts_with = "unmute")]
        mute: bool,

        /// Unmute output.
        #[arg(long)]
        unmute: bool,
    },

    /// Inspect or modify the play queue.
    Queue {
        #[command(subcommand)]
        action: Option<QueueCommand>,
    },

    /// Search the catalogue.
    Search {
        #[arg(value_name = "QUERY", required = true)]
        query: Vec<String>,

        /// What to search for.
        #[arg(long, value_enum, default_value_t = Kind::Track)]
        kind: Kind,

        #[arg(long, default_value_t = 40)]
        limit: u32,

        #[arg(long, default_value_t = 0)]
        offset: u32,
    },

    /// List or open playlists.
    Playlist {
        #[command(subcommand)]
        action: PlaylistCommand,
    },

    /// Print lyrics for a track (requires ncmapi 1.1).
    Lyrics {
        #[arg(long, value_name = "ID")]
        id: Option<u64>,
    },

    /// Show the current playback state.
    Status,

    /// Save a session cookie so the provider accepts requests.
    Login {
        /// File containing the cookie header; `-` reads stdin.
        #[arg(long, value_name = "PATH")]
        cookie_file: Option<PathBuf>,

        /// Start the QR flow in the terminal (not available yet).
        #[arg(long)]
        qr: bool,
    },

    /// Remove the stored session.
    Logout,

    /// Show the signed-in account.
    Whoami,

    /// Manage the background player daemon.
    Daemon {
        #[command(subcommand)]
        action: DaemonCommand,
    },

    /// Check installation, configuration, session and audio device.
    Doctor,

    /// Machine-readable command catalogue.
    Capabilities,

    /// Generate shell completions.
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Debug, Subcommand)]
pub enum QueueCommand {
    /// List queued tracks.
    List,
    /// Append tracks to the queue.
    Add {
        #[arg(value_name = "QUERY")]
        query: Vec<String>,
        #[arg(long = "id", value_name = "ID")]
        ids: Vec<u64>,
        /// Insert after the current track instead of at the end.
        #[arg(long)]
        next: bool,
    },
    /// Remove everything from the queue.
    Clear,
}

#[derive(Debug, Subcommand)]
pub enum PlaylistCommand {
    /// List the signed-in user's playlists.
    List,
    /// Show the tracks of a playlist.
    Show {
        #[arg(value_name = "ID")]
        id: u64,
    },
}

#[derive(Debug, Subcommand)]
pub enum DaemonCommand {
    /// Start the daemon in the background.
    Start,
    /// Stop the daemon.
    Stop,
    /// Report whether the daemon is running.
    Status,
    /// Run in the foreground (internal; used by autostart).
    Run {
        /// Detach from the terminal and log to file.
        #[arg(long)]
        detached: bool,
    },
    /// Print the tail of the daemon log.
    Logs {
        #[arg(long, default_value_t = 40)]
        lines: usize,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Kind {
    Track,
    Album,
    Artist,
    Playlist,
}

impl From<Kind> for lekima_domain::SearchKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Track => Self::Track,
            Kind::Album => Self::Album,
            Kind::Artist => Self::Artist,
            Kind::Playlist => Self::Playlist,
        }
    }
}
