//! Durable playback queue for daemon restarts.
//!
//! This is intentionally a snapshot, not a live journal: the daemon rewrites it
//! periodically and on shutdown, and restores it in a stopped state. The audio
//! stream itself is never cached here.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write as _},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use lekima_domain::{Quality, QueueKind, RepeatMode, Track};
use serde::{Deserialize, Serialize};

/// Everything needed to rebuild the queue after a daemon restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedPlayback {
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub position_ms: u64,
    #[serde(default)]
    pub repeat: RepeatMode,
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub quality: Quality,
    #[serde(default)]
    pub kind: QueueKind,
}

const fn default_volume() -> f32 {
    0.7
}

impl Default for SavedPlayback {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            index: None,
            position_ms: 0,
            repeat: RepeatMode::Off,
            shuffle: false,
            volume: default_volume(),
            quality: Quality::Exhigh,
            kind: QueueKind::Manual,
        }
    }
}

/// Atomically reads and writes one `SavedPlayback`.
#[derive(Debug, Clone)]
pub struct PlaybackStore {
    path: PathBuf,
}

impl PlaybackStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads a snapshot. A missing file is `None`; malformed JSON is an error.
    pub fn load(&self) -> io::Result<Option<SavedPlayback>> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if contents.trim().is_empty() {
            return Ok(None);
        }
        serde_json::from_str(&contents)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Writes the snapshot atomically with `0600` permissions.
    pub fn save(&self, saved: &SavedPlayback) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = self.path.with_extension("json.tmp");
        let encoded = serde_json::to_vec_pretty(saved)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(&encoded)?;
            file.sync_all()?;
        }
        fs::rename(&temp, &self.path)
    }

    /// Removes the snapshot, if one exists.
    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// The default location under the cache directory.
#[must_use]
pub fn playback_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("playback.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lekima_domain::{Artist, TrackId};
    use std::os::unix::fs::PermissionsExt;

    fn track(id: u64) -> Track {
        Track {
            id: TrackId(id),
            name: format!("track {id}"),
            artists: vec![Artist {
                id: 1.into(),
                name: "artist".to_owned(),
            }],
            album: None,
            duration_ms: 120_000,
        }
    }

    #[test]
    fn round_trips_a_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let store = PlaybackStore::new(playback_path(dir.path()));
        let saved = SavedPlayback {
            tracks: vec![track(1), track(2)],
            index: Some(1),
            position_ms: 42_000,
            repeat: RepeatMode::All,
            shuffle: true,
            volume: 0.4,
            quality: Quality::Lossless,
            kind: QueueKind::Fm,
        };
        store.save(&saved).unwrap();
        assert_eq!(store.load().unwrap(), Some(saved));
        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn missing_or_empty_files_are_not_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = PlaybackStore::new(playback_path(dir.path()));
        assert!(store.load().unwrap().is_none());
        fs::write(store.path(), b"  \n").unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn malformed_snapshots_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let store = PlaybackStore::new(playback_path(dir.path()));
        fs::write(store.path(), b"{not json").unwrap();
        let error = store.load().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
