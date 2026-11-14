#![forbid(unsafe_code)]
//! Configuration, XDG paths and the persisted provider session.
//!
//! Precedence is: built-in defaults, then `config.toml`, then environment
//! variables. `LEKIMA_CONFIG_DIR` relocates all state, which keeps tests and
//! CI hermetic.

mod config;
mod paths;
mod session;

pub use config::{AudioConfig, Config, DaemonConfig, PlaybackConfig, ProviderConfig, UiConfig};
pub use paths::{PathError, Paths};
pub use session::FileCredentials;

pub use config::{load as load_config, save_audio_device, save_quality, save_ui};
