#![deny(unsafe_code)]
//! Operating-system media integration.
//!
//! The daemon owns one [`MediaControls`] implementation. It pushes the current
//! [`NowPlaying`] snapshot on every state change, and drains [`MediaCommand`]s
//! that come from the OS (MPRIS clients on Linux, media keys and Control Center
//! on macOS) and feeds them into the same command path as the CLI and TUI.
//!
//! Backends:
//!
//! | OS | Mechanism |
//! | --- | --- |
//! | Linux | `org.mpris.MediaPlayer2.lekima` over D-Bus |
//! | macOS | `MPNowPlayingInfoCenter` + `MPRemoteCommandCenter` |
//! | other | no-op |

use std::sync::Arc;

mod noop;

#[cfg(target_os = "macos")]
mod host;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos;

pub use noop::NoopControls;

#[cfg(target_os = "macos")]
pub use macos::run_host;

/// Repeat mode as exposed to the OS.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(target_os = "macos", derive(serde::Serialize, serde::Deserialize))]
pub enum Repeat {
    /// Stop after the queue.
    #[default]
    Off,
    /// Repeat the whole queue.
    All,
    /// Repeat the current track.
    One,
}

/// What the operating system should display and how it should behave.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(target_os = "macos", derive(serde::Serialize, serde::Deserialize))]
pub struct NowPlaying {
    pub track_id: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub playing: bool,
    pub has_track: bool,
    pub has_next: bool,
    pub has_previous: bool,
    pub volume: f64,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
}

/// Transport actions initiated by the OS.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(target_os = "macos", derive(serde::Serialize, serde::Deserialize))]
pub enum MediaCommand {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    /// Absolute position in milliseconds.
    Seek(u64),
    /// Relative position change in milliseconds.
    SeekBy(i64),
    /// Linear volume in `0.0..=1.0`.
    SetVolume(f64),
    SetShuffle(bool),
    SetRepeat(Repeat),
}

/// An OS media integration backend.
pub trait MediaControls: Send + Sync {
    /// Publishes a new snapshot; backends diff it against the last one.
    fn update(&self, state: &NowPlaying);

    /// Drains actions requested by the OS since the last call.
    fn poll(&self) -> Vec<MediaCommand>;

    /// Short backend name for `doctor` and logs.
    fn backend(&self) -> &'static str;
}

/// Name of the media integration available on this platform.
#[must_use]
pub const fn backend_name() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "mpris"
    }
    #[cfg(target_os = "macos")]
    {
        "now-playing"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        "none"
    }
}

/// Creates the backend for this OS.
///
/// Must be called from inside a Tokio runtime on Linux (the D-Bus server needs
/// one). Falls back to [`NoopControls`] when the OS integration is unavailable.
pub async fn controls() -> Arc<dyn MediaControls> {
    #[cfg(target_os = "linux")]
    {
        match linux::start().await {
            Ok(controls) => controls,
            Err(error) => {
                tracing::warn!(%error, "MPRIS is unavailable");
                Arc::new(NoopControls)
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        host::spawn()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Arc::new(NoopControls)
    }
}

#[cfg(test)]
mod tests {
    use super::{MediaCommand, NowPlaying, Repeat};

    #[test]
    fn now_playing_defaults_are_inert() {
        let state = NowPlaying::default();
        assert!(!state.playing);
        assert!(!state.has_track);
        assert_eq!(state.repeat, Repeat::Off);
    }

    #[test]
    fn commands_are_copyable_and_comparable() {
        let command = MediaCommand::SeekBy(-1500);
        assert_eq!(command, MediaCommand::SeekBy(-1500));
        assert_ne!(command, MediaCommand::Seek(0));
    }
}
