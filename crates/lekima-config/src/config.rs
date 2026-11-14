//! The `config.toml` schema.

use std::path::Path;

use lekima_domain::Quality;
use serde::{Deserialize, Serialize};

use crate::paths::PathError;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub provider: ProviderConfig,
    pub playback: PlaybackConfig,
    pub ui: UiConfig,
    pub daemon: DaemonConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderConfig {
    /// Enables the short-lived HTTP response cache.
    pub cache: bool,
    /// Seconds a cached response stays valid.
    pub cache_ttl_secs: u64,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            cache: true,
            cache_ttl_secs: 180,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlaybackConfig {
    pub volume: f32,
    pub quality: Quality,
    /// Start playback automatically once a queue is loaded.
    pub autoplay: bool,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            volume: 0.7,
            quality: Quality::Exhigh,
            autoplay: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// `cyber` (default), `amber` or `mono`.
    pub theme: String,
    /// `true` keeps the terminal contents when the TUI exits.
    pub alternate_screen: bool,
    pub show_help_on_start: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "cyber".to_owned(),
            alternate_screen: true,
            show_help_on_start: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    /// Spawn a daemon automatically when a client cannot reach one.
    pub autostart: bool,
    pub log_level: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            autostart: true,
            log_level: "info".to_owned(),
        }
    }
}

/// Loads `path`, falling back to defaults when the file does not exist.
///
/// # Errors
///
/// Returns [`PathError`] when the file exists but cannot be read or parsed.
pub fn load(path: &Path) -> Result<Config, PathError> {
    match std::fs::read_to_string(path) {
        Ok(raw) => toml::from_str(&raw).map_err(|source| PathError::Parse {
            path: path.to_path_buf(),
            source,
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(source) => Err(PathError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn partial_config_keeps_defaults_for_missing_sections() {
        let config: Config = toml::from_str("[playback]\nvolume = 0.25\n").unwrap();
        assert!((config.playback.volume - 0.25).abs() < f32::EPSILON);
        assert_eq!(config.daemon.log_level, "info");
        assert_eq!(config.ui.theme, "cyber");
        assert!(config.provider.cache);
    }

    #[test]
    fn unknown_keys_are_rejected_so_typos_are_visible() {
        let parsed: Result<Config, _> = toml::from_str("[playback]\nvolum = 1.0\n");
        assert!(parsed.is_err());
    }
}
