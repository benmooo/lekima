//! Durable listening history.
//!
//! The file is a single versioned document rewritten atomically, the same shape
//! as `playback.json`: the daemon owns it, and a reader never sees a partial
//! write. A malformed file is reported rather than silently reset, so a bug that
//! corrupts history is visible instead of destroying it.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write as _},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use lekima_domain::{HistoryLog, PlayRecord};
use serde::{Deserialize, Serialize};

/// Current on-disk schema version.
const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct HistoryFile {
    #[serde(default = "version")]
    version: u32,
    #[serde(default)]
    entries: Vec<PlayRecord>,
}

const fn version() -> u32 {
    VERSION
}

/// Reads and writes the listening history for one state root.
#[derive(Debug, Clone)]
pub struct HistoryStore {
    path: PathBuf,
}

impl HistoryStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the log, keeping at most `capacity` records.
    ///
    /// A missing or empty file is an empty log, not an error.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidData`] when the file exists but is not a
    /// valid history document.
    pub fn load(&self, capacity: usize) -> io::Result<HistoryLog> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(HistoryLog::new(capacity));
            }
            Err(error) => return Err(error),
        };
        if contents.trim().is_empty() {
            return Ok(HistoryLog::new(capacity));
        }
        let file: HistoryFile = serde_json::from_str(&contents)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if file.version > VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "history format version {} is newer than this build understands ({VERSION})",
                    file.version
                ),
            ));
        }
        Ok(HistoryLog::from_records(file.entries, capacity))
    }

    /// Writes the log atomically with `0600` permissions.
    ///
    /// # Errors
    ///
    /// Propagates filesystem failures and serialization failures.
    pub fn save(&self, log: &HistoryLog) -> io::Result<()> {
        let records: Vec<PlayRecord> = log.records().iter().cloned().collect();
        self.save_records(&records)
    }

    /// Writes a slice of records atomically with `0600` permissions.
    ///
    /// # Errors
    ///
    /// Propagates filesystem failures and serialization failures.
    pub fn save_records(&self, records: &[PlayRecord]) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = HistoryFile {
            version: VERSION,
            entries: records.to_vec(),
        };
        let encoded = serde_json::to_vec_pretty(&file)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let temp = self.path.with_extension("json.tmp");
        {
            let mut handle = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp)?;
            handle.write_all(&encoded)?;
            handle.sync_all()?;
        }
        fs::rename(&temp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use lekima_domain::{QueueKind, Track, TrackId};

    use super::HistoryStore;

    fn track(id: u64) -> Track {
        Track {
            id: TrackId::from(id),
            name: format!("track {id}"),
            artists: Vec::new(),
            album: None,
            duration_ms: 60_000,
        }
    }

    fn log(ids: &[u64]) -> lekima_domain::HistoryLog {
        let records = ids.iter().map(|id| lekima_domain::PlayRecord {
            track: track(*id),
            started_at: 1_700_000_000_000,
            played_ms: 60_000,
            completed: true,
            kind: QueueKind::Manual,
        });
        lekima_domain::HistoryLog::from_records(records, 100)
    }

    #[test]
    fn round_trips_records() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::new(dir.path().join("history.json"));
        let log = log(&[1, 2, 3]);

        store.save(&log).unwrap();
        let loaded = store.load(100).unwrap();

        let ids: Vec<u64> = loaded.records().iter().map(|r| r.track.id.get()).collect();
        assert_eq!(ids, vec![1, 2, 3]);
        assert!(loaded.records()[0].completed);
        assert_eq!(
            std::fs::metadata(store.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn a_missing_file_is_an_empty_log() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::new(dir.path().join("history.json"));

        let loaded = store.load(50).unwrap();
        assert_eq!(loaded.capacity(), 50);
        assert!(loaded.is_empty());
    }

    #[test]
    fn capacity_trims_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::new(dir.path().join("history.json"));
        store.save(&log(&[1, 2, 3, 4])).unwrap();

        let loaded = store.load(2).unwrap();
        let ids: Vec<u64> = loaded.records().iter().map(|r| r.track.id.get()).collect();
        assert_eq!(ids, vec![3, 4]);
    }

    #[test]
    fn malformed_history_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::new(dir.path().join("history.json"));
        std::fs::write(store.path(), b"{not json").unwrap();

        let error = store.load(10).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_future_format_is_rejected_instead_of_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::new(dir.path().join("history.json"));
        std::fs::write(store.path(), br#"{"version":99,"entries":[]}"#).unwrap();

        let error = store.load(10).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn an_empty_file_is_an_empty_log() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::new(dir.path().join("history.json"));
        std::fs::write(store.path(), b"\n").unwrap();

        assert!(store.load(10).unwrap().is_empty());
    }
}
