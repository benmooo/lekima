//! Persistent cache for fully downloaded streams.
//!
//! Only streams small enough to load into memory are cached. Large/unknown
//! streams keep using the ephemeral spool and are intentionally not copied to
//! disk, so the cache never tries to compete with the streaming path.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::Serialize;

/// Summary returned by `lekima cache status`.
#[derive(Debug, Clone, Serialize)]
pub struct CacheStats {
    pub enabled: bool,
    pub path: PathBuf,
    pub limit_bytes: u64,
    pub files: usize,
    pub bytes: u64,
}

/// A bounded directory of `<key>.audio` files.
#[derive(Debug, Clone)]
pub struct StreamCache {
    root: PathBuf,
    limit_bytes: u64,
    enabled: bool,
}

impl StreamCache {
    /// Creates/opens the cache directory. A disabled cache is still described,
    /// so `cache status` can report the configured size.
    #[must_use]
    pub fn new(root: PathBuf, limit_mb: u64, enabled: bool) -> Self {
        let limit_mb = limit_mb.max(1);
        if let Err(error) = fs::create_dir_all(&root) {
            tracing::warn!(%error, path = %root.display(), "failed to create the stream cache");
        } else {
            let _ = fs::set_permissions(&root, fs::Permissions::from_mode(0o700));
        }
        Self {
            root,
            limit_bytes: limit_mb.saturating_mul(1024 * 1024),
            enabled,
        }
    }

    /// A cache that never stores anything, for doctor/headless checks.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            root: std::env::temp_dir().join("lekima-disabled-cache"),
            limit_bytes: 0,
            enabled: false,
        }
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn limit_bytes(&self) -> u64 {
        self.limit_bytes
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn path_for(&self, key: &str) -> PathBuf {
        self.root.join(format!("{}.audio", safe_key(key)))
    }

    /// True when a non-empty cache file exists.
    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.enabled
            && fs::metadata(self.path_for(key))
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
    }

    /// Reads a cached stream and marks it recently used.
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        if !self.enabled {
            return None;
        }
        let path = self.path_for(key);
        let mut bytes = Vec::new();
        match File::open(&path).and_then(|mut file| file.read_to_end(&mut bytes)) {
            Ok(_) if !bytes.is_empty() => {
                touch(&path);
                Some(bytes)
            }
            Ok(_) => None,
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                tracing::debug!(%error, path = %path.display(), "failed to read the stream cache");
                None
            }
        }
    }

    /// Stores a stream if it fits the cache. Returns whether it was written.
    pub fn put(&self, key: &str, bytes: &[u8]) -> bool {
        if !self.enabled || bytes.is_empty() || bytes.len() as u64 > self.limit_bytes {
            return false;
        }
        let path = self.path_for(key);
        let temp = path.with_extension("audio.tmp");
        let result = (|| -> io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &path)?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temp);
            tracing::debug!(%error, path = %path.display(), "failed to write the stream cache");
            return false;
        }
        touch(&path);
        self.evict();
        true
    }

    /// Removes every cache file, leaving unrelated files alone.
    pub fn clear(&self) -> io::Result<usize> {
        let mut removed = 0;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if (name.ends_with(".audio") || name.ends_with(".audio.tmp"))
                && fs::remove_file(entry.path()).is_ok()
            {
                removed += 1;
            }
        }
        Ok(removed)
    }

    #[must_use]
    pub fn stats(&self) -> CacheStats {
        let mut files = 0;
        let mut bytes = 0;
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if !name.to_string_lossy().ends_with(".audio") {
                    continue;
                }
                if let Ok(metadata) = entry.metadata()
                    && metadata.is_file()
                {
                    files += 1;
                    bytes += metadata.len();
                }
            }
        }
        CacheStats {
            enabled: self.enabled,
            path: self.root.clone(),
            limit_bytes: self.limit_bytes,
            files,
            bytes,
        }
    }

    fn evict(&self) {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            if !name.to_string_lossy().ends_with(".audio") {
                continue;
            }
            if let Ok(metadata) = entry.metadata()
                && metadata.is_file()
            {
                let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                files.push((modified, metadata.len(), entry.path()));
            }
        }
        let mut total: u64 = files.iter().map(|(_, size, _)| *size).sum();
        if total <= self.limit_bytes {
            return;
        }
        files.sort_by_key(|(modified, _, _)| *modified);
        for (_, size, path) in files {
            if total <= self.limit_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
    }
}

fn safe_key(key: &str) -> String {
    if !key.is_empty()
        && key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return key.to_owned();
    }
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn touch(path: &Path) {
    let now = SystemTime::now();
    if let Ok(file) = OpenOptions::new().write(true).open(path) {
        let times = fs::FileTimes::new().set_accessed(now).set_modified(now);
        let _ = file.set_times(times);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_reports_stats() {
        let dir = tempfile::tempdir().unwrap();
        let cache = StreamCache::new(dir.path().to_path_buf(), 1, true);
        assert!(!cache.has("track-1-exhigh"));
        assert!(cache.put("track-1-exhigh", b"audio"));
        assert!(cache.has("track-1-exhigh"));
        assert_eq!(
            cache.get("track-1-exhigh").as_deref(),
            Some(b"audio".as_slice())
        );
        let stats = cache.stats();
        assert_eq!(stats.files, 1);
        assert_eq!(stats.bytes, 5);
        assert_eq!(stats.limit_bytes, 1024 * 1024);
        assert_eq!(
            fs::metadata(cache.path_for("track-1-exhigh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn refuses_oversized_items_and_evicts_old_ones() {
        let dir = tempfile::tempdir().unwrap();
        let cache = StreamCache::new(dir.path().to_path_buf(), 1, true);
        let one_mib = 1024 * 1024;
        assert!(!cache.put("too-big", &vec![0u8; one_mib + 1]));
        assert!(cache.put("first", &vec![1u8; 700 * 1024]));
        assert!(cache.put("second", &vec![2u8; 700 * 1024]));
        let stats = cache.stats();
        assert!(stats.bytes <= stats.limit_bytes, "{stats:?}");
        assert_eq!(stats.files, 1);
    }

    #[test]
    fn disabled_cache_never_stores() {
        let dir = tempfile::tempdir().unwrap();
        let cache = StreamCache::new(dir.path().to_path_buf(), 1, false);
        assert!(!cache.put("track", b"audio"));
        assert!(!cache.has("track"));
        assert!(cache.get("track").is_none());
    }

    #[test]
    fn clear_only_removes_cache_files() {
        let dir = tempfile::tempdir().unwrap();
        let cache = StreamCache::new(dir.path().to_path_buf(), 1, true);
        cache.put("track", b"audio");
        fs::write(dir.path().join("keep.txt"), b"keep").unwrap();
        assert_eq!(cache.clear().unwrap(), 1);
        assert!(dir.path().join("keep.txt").exists());
    }
}
