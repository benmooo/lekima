//! The `config.toml` schema.

use std::{collections::BTreeMap, path::Path};

use lekima_domain::Quality;
use serde::{Deserialize, Serialize};

use crate::paths::PathError;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub provider: ProviderConfig,
    pub audio: AudioConfig,
    pub playback: PlaybackConfig,
    pub ui: UiConfig,
    pub daemon: DaemonConfig,
    pub cache: CacheConfig,
    pub notifications: NotificationsConfig,
    pub history: HistoryConfig,
    /// External hook processes, keyed by name (config section `[hooks.<name>]`).
    pub hooks: BTreeMap<String, HookConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    /// Output device name; `None` uses the system default.
    pub device: Option<String>,
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
    /// `auto` (default), `full` or `reduced` animation policy.
    pub motion: String,
    /// Lyrics offset in milliseconds, applied when the TUI starts.
    pub lyrics_offset_ms: i64,
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
            motion: "auto".to_owned(),
            lyrics_offset_ms: 0,
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

/// Persistent stream cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CacheConfig {
    pub enabled: bool,
    /// Capacity in MiB. The default is 256 MiB.
    pub limit_mb: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            limit_mb: 256,
        }
    }
}

/// Desktop notifications pushed by the daemon.
///
/// Off by default: a notification is an OS-level interruption, so it is opt-in
/// rather than a surprise after an upgrade.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationsConfig {
    /// Show a notification when the playing track changes.
    pub on_track_change: bool,
}

/// One external hook process.
///
/// Hooks are configured, never compiled in: the daemon starts `command`, writes
/// sequenced events to its stdin and accepts the commands it is allowed to send
/// back. Capabilities are command tags; see `lekima_ipc::HOOK_CAPABILITIES`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HookConfig {
    /// Executable followed by its arguments. An empty command is rejected per
    /// hook by the daemon, so one bad table cannot stop every CLI command from
    /// loading the configuration.
    pub command: Vec<String>,
    /// Event kinds to receive (`status`, `queue`, `like`, `resync`). Empty means
    /// every kind.
    #[serde(default)]
    pub events: Vec<String>,
    /// Commands the hook may send back. Empty means read-only.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Listening history retained on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryConfig {
    /// Number of finished plays kept. Oldest records are dropped first; values
    /// below 1 are treated as 1.
    pub capacity: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self { capacity: 1_000 }
    }
}

/// Persists the output device, preserving every other setting.
///
/// # Errors
///
/// Returns [`PathError`] when the file cannot be read, parsed or written.
pub fn save_audio_device(path: &Path, device: Option<&str>) -> Result<(), PathError> {
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
    let audio = table
        .entry("audio")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| PathError::Structure("`audio` is not a table".to_owned()))?;
    match device {
        Some(device) => {
            audio.insert("device".to_owned(), toml::Value::String(device.to_owned()));
        }
        None => {
            audio.remove("device");
        }
    }
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

/// Persists the UI preferences, preserving every other setting.
///
/// # Errors
///
/// Returns [`PathError`] when the file cannot be read, parsed or written.
pub fn save_ui(
    path: &Path,
    theme: &str,
    glyphs: &str,
    density: &str,
    motion: &str,
    lyrics_offset_ms: i64,
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
    ui.insert("motion".to_owned(), toml::Value::String(motion.to_owned()));
    ui.insert(
        "lyrics_offset_ms".to_owned(),
        toml::Value::Integer(lyrics_offset_ms),
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

/// Persists the cache settings, preserving every other setting.
///
/// # Errors
///
/// Returns [`PathError`] when the file cannot be read, parsed or written.
pub fn save_cache(path: &Path, enabled: bool, limit_mb: u64) -> Result<(), PathError> {
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
    let cache = table
        .entry("cache")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| PathError::Structure("`cache` is not a table".to_owned()))?;
    cache.insert("enabled".to_owned(), toml::Value::Boolean(enabled));
    cache.insert(
        "limit_mb".to_owned(),
        toml::Value::Integer(i64::try_from(limit_mb).unwrap_or(i64::MAX)),
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
        save_ui(&path, "amber", "ascii", "comfortable", "reduced", 500, true).unwrap();
        let config = super::load(&path).unwrap();
        assert_eq!(config.ui.theme, "amber");
        assert_eq!(config.ui.glyphs, "ascii");
        assert_eq!(config.ui.density, "comfortable");
        assert_eq!(config.ui.motion, "reduced");
        assert_eq!(config.ui.lyrics_offset_ms, 500);
        assert!(config.ui.lyrics_translation);
        assert!((config.playback.volume - 0.25).abs() < f32::EPSILON);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn notifications_are_opt_in() {
        let config: Config = toml::from_str("[playback]\nvolume = 0.25\n").unwrap();
        assert!(!config.notifications.on_track_change);

        let enabled: Config = toml::from_str("[notifications]\non_track_change = true\n").unwrap();
        assert!(enabled.notifications.on_track_change);
    }

    #[test]
    fn a_hook_without_a_command_still_parses() {
        // The daemon rejects it per hook; parsing must not fail for every
        // command that loads the config.
        let config: Config = toml::from_str("[hooks.broken]\nevents = [\"status\"]\n").unwrap();
        assert!(config.hooks["broken"].command.is_empty());
        assert_eq!(config.hooks["broken"].events, vec!["status".to_owned()]);
    }

    #[test]
    fn history_defaults_to_a_thousand_records() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.history.capacity, 1_000);

        let tuned: Config = toml::from_str("[history]\ncapacity = 25\n").unwrap();
        assert_eq!(tuned.history.capacity, 25);
    }

    #[test]
    fn unknown_keys_are_rejected_so_typos_are_visible() {
        let parsed: Result<Config, _> = toml::from_str("[playback]\nvolum = 1.0\n");
        assert!(parsed.is_err());
    }
}
