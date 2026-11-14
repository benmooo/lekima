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
    /// `unicode` (default) or `ascii` block-drawing set.
    pub glyphs: String,
    /// `compact` (default) or `comfortable` row rhythm.
    pub density: String,
    /// Show translated lyrics under the original when available.
    pub lyrics_translation: bool,
    /// `true` keeps the terminal contents when the TUI exits.
    pub alternate_screen: bool,
    pub show_help_on_start: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "cyber".to_owned(),
            glyphs: "unicode".to_owned(),
            density: "compact".to_owned(),
            lyrics_translation: false,
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

/// Persists the playback quality, preserving every other setting.
///
/// # Errors
///
/// Returns [`PathError`] when the file cannot be read, parsed or written.
pub fn save_quality(path: &Path, quality: &str) -> Result<(), PathError> {
    let mut root: toml::Value = match std::fs::read_to_string(path) {
        Ok(raw) => toml::from_str(&raw).map_err(|source| PathError::Parse {
            path: path.to_path_buf(),
            source,
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            toml::Value::Table(toml::map::Map::new())
        }
        Err(source) => {
            return Err(PathError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let table = root
        .as_table_mut()
        .ok_or_else(|| PathError::Structure("config root is not a table".to_owned()))?;
    let playback = table
        .entry("playback")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| PathError::Structure("`playback` is not a table".to_owned()))?;
    playback.insert(
        "quality".to_owned(),
        toml::Value::String(quality.to_owned()),
    );
    let encoded = toml::to_string_pretty(&root).map_err(|source| PathError::Serialize {
        path: path.to_path_buf(),
        source,
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| PathError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(path, encoded).map_err(|source| PathError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// Persists the three UI preferences, preserving every other setting.
///
/// # Errors
///
/// Returns [`PathError`] when the file cannot be read, parsed or written.
pub fn save_ui(
    path: &Path,
    theme: &str,
    glyphs: &str,
    density: &str,
    lyrics_translation: bool,
) -> Result<(), PathError> {
    let mut root: toml::Value = match std::fs::read_to_string(path) {
        Ok(raw) => toml::from_str(&raw).map_err(|source| PathError::Parse {
            path: path.to_path_buf(),
            source,
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            toml::Value::Table(toml::map::Map::new())
        }
        Err(source) => {
            return Err(PathError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    let table = root
        .as_table_mut()
        .ok_or_else(|| PathError::Structure("config root is not a table".to_owned()))?;
    let ui = table
        .entry("ui")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| PathError::Structure("`ui` is not a table".to_owned()))?;
    ui.insert("theme".to_owned(), toml::Value::String(theme.to_owned()));
    ui.insert("glyphs".to_owned(), toml::Value::String(glyphs.to_owned()));
    ui.insert(
        "density".to_owned(),
        toml::Value::String(density.to_owned()),
    );
    ui.insert(
        "lyrics_translation".to_owned(),
        toml::Value::Boolean(lyrics_translation),
    );

    let encoded = toml::to_string_pretty(&root).map_err(|source| PathError::Serialize {
        path: path.to_path_buf(),
        source,
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| PathError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(path, encoded).map_err(|source| PathError::Write {
        path: path.to_path_buf(),
        source,
    })
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
    use super::{Config, save_ui};

    #[test]
    fn partial_config_keeps_defaults_for_missing_sections() {
        let config: Config = toml::from_str("[playback]\nvolume = 0.25\n").unwrap();
        assert!((config.playback.volume - 0.25).abs() < f32::EPSILON);
        assert_eq!(config.daemon.log_level, "info");
        assert_eq!(config.ui.theme, "cyber");
        assert_eq!(config.ui.glyphs, "unicode");
        assert_eq!(config.ui.density, "compact");
        assert!(config.provider.cache);
    }

    #[test]
    fn save_ui_preserves_other_sections() {
        let dir = std::env::temp_dir().join(format!("lekima-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[playback]\nvolume = 0.25\n").unwrap();
        save_ui(&path, "amber", "ascii", "comfortable", true).unwrap();
        let config = super::load(&path).unwrap();
        assert_eq!(config.ui.theme, "amber");
        assert_eq!(config.ui.glyphs, "ascii");
        assert_eq!(config.ui.density, "comfortable");
        assert!(config.ui.lyrics_translation);
        assert!((config.playback.volume - 0.25).abs() < f32::EPSILON);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_keys_are_rejected_so_typos_are_visible() {
        let parsed: Result<Config, _> = toml::from_str("[playback]\nvolum = 1.0\n");
        assert!(parsed.is_err());
    }
}
