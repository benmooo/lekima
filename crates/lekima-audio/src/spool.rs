//! Growing-file spool for streaming playback.
//!
//! The downloader appends chunks; the decoder reads from a second file handle.
//! When the reader catches up it waits on a condition variable (with a timeout
//! so cancellation is observed) instead of hitting a premature EOF.

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const READ_WAIT: Duration = Duration::from_millis(100);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

struct State {
    written: u64,
    total: Option<u64>,
    complete: bool,
    error: Option<String>,
    cancelled: bool,
}

pub struct Spool {
    state: Mutex<State>,
    ready: Condvar,
    buffering: AtomicBool,
}

impl Spool {
    #[must_use]
    pub fn new(total: Option<u64>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                written: 0,
                total,
                complete: false,
                error: None,
                cancelled: false,
            }),
            ready: Condvar::new(),
            buffering: AtomicBool::new(false),
        })
    }

    /// Appends one chunk. Returns `false` when the spool was cancelled.
    pub fn push(&self, writer: &mut File, chunk: &[u8]) -> io::Result<bool> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.cancelled {
            return Ok(false);
        }
        writer.write_all(chunk)?;
        state.written += chunk.len() as u64;
        self.ready.notify_all();
        Ok(true)
    }

    pub fn finish(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.complete = true;
        self.ready.notify_all();
    }

    pub fn fail(&self, error: impl Into<String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.error = Some(error.into());
        state.complete = true;
        self.ready.notify_all();
    }

    pub fn cancel(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.cancelled = true;
        self.ready.notify_all();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.lock().map_or(true, |state| state.cancelled)
    }

    /// Blocks until `threshold` bytes exist, the download completes, or it fails.
    pub fn wait_ready(&self, threshold: u64, timeout: Duration) -> Result<(), String> {
        let started = Instant::now();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        loop {
            if let Some(error) = &state.error {
                return Err(error.clone());
            }
            if state.cancelled {
                return Err("stream was cancelled".to_owned());
            }
            let target = state.total.map_or(threshold, |total| total.min(threshold));
            if state.complete || state.written >= target {
                return Ok(());
            }
            if started.elapsed() >= timeout {
                return Err("timed out while buffering the stream".to_owned());
            }
            let (next, _) = self
                .ready
                .wait_timeout(state, READ_WAIT)
                .unwrap_or_else(|poison| poison.into_inner());
            state = next;
        }
    }

    #[must_use]
    pub fn total(&self) -> Option<u64> {
        self.state.lock().map_or(None, |state| state.total)
    }

    #[must_use]
    pub fn is_buffering(&self) -> bool {
        self.buffering.load(Ordering::Relaxed)
    }

    fn progress(&self) -> (u64, bool, Option<String>, bool) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        (
            state.written,
            state.complete,
            state.error.clone(),
            state.cancelled,
        )
    }

    fn wait_more(&self) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _ = self
            .ready
            .wait_timeout(state, READ_WAIT)
            .unwrap_or_else(|poison| poison.into_inner());
    }
}

/// `Read + Seek` view over the growing file.
pub struct GrowingFile {
    file: File,
    position: u64,
    spool: Arc<Spool>,
}

impl GrowingFile {
    #[must_use]
    pub const fn new(file: File, spool: Arc<Spool>) -> Self {
        Self {
            file,
            position: 0,
            spool,
        }
    }
}

impl Read for GrowingFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let started = Instant::now();
        loop {
            let (written, complete, error, cancelled) = self.spool.progress();
            if let Some(error) = error {
                return Err(io::Error::other(error));
            }
            if cancelled {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "stream was cancelled",
                ));
            }
            if self.position < written {
                let available = (written - self.position).min(buffer.len() as u64) as usize;
                self.file.seek(SeekFrom::Start(self.position))?;
                let read = self.file.read(&mut buffer[..available])?;
                self.position += read as u64;
                self.spool.buffering.store(false, Ordering::Relaxed);
                return Ok(read);
            }
            if complete {
                self.spool.buffering.store(false, Ordering::Relaxed);
                return Ok(0);
            }
            if started.elapsed() >= READ_TIMEOUT {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "stream stalled for 30 seconds",
                ));
            }
            self.spool.buffering.store(true, Ordering::Relaxed);
            self.spool.wait_more();
        }
    }
}

impl Seek for GrowingFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let (written, _complete, error, _) = self.spool.progress();
        if let Some(error) = error {
            return Err(io::Error::other(error));
        }
        let total = self.spool.total();
        let target = match position {
            SeekFrom::Start(offset) => i64::try_from(offset).unwrap_or(i64::MAX),
            SeekFrom::Current(delta) => i64::try_from(self.position).unwrap_or(i64::MAX) + delta,
            SeekFrom::End(delta) => match total {
                Some(total) => i64::try_from(total).unwrap_or(i64::MAX) + delta,
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "stream length is unknown",
                    ));
                }
            },
        };
        if target < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "negative seek position",
            ));
        }
        let target = target as u64;
        if target > written {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "seek beyond the buffered region",
            ));
        }
        self.file.seek(SeekFrom::Start(target))?;
        self.position = target;
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Seek, SeekFrom},
        sync::Arc,
        thread,
        time::Duration,
    };

    use tempfile::NamedTempFile;

    use super::{GrowingFile, Spool};

    #[test]
    fn reads_block_until_data_arrives_and_end_after_finish() {
        let temp = NamedTempFile::new().unwrap();
        let mut writer = temp.reopen().unwrap();
        let reader = temp.reopen().unwrap();
        let spool = Spool::new(Some(6));
        let mut file = GrowingFile::new(reader, Arc::clone(&spool));

        let handle = thread::spawn(move || {
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer).unwrap();
            buffer
        });

        thread::sleep(Duration::from_millis(50));
        assert!(spool.push(&mut writer, b"abc").unwrap());
        thread::sleep(Duration::from_millis(50));
        assert!(spool.push(&mut writer, b"def").unwrap());
        spool.finish();

        assert_eq!(handle.join().unwrap(), b"abcdef");
    }

    #[test]
    fn seek_within_buffered_region_works_beyond_is_rejected() {
        let temp = NamedTempFile::new().unwrap();
        let mut writer = temp.reopen().unwrap();
        let reader = temp.reopen().unwrap();
        let spool = Spool::new(Some(6));
        spool.push(&mut writer, b"abcdef").unwrap();
        spool.finish();

        let mut file = GrowingFile::new(reader, spool);
        assert_eq!(file.seek(SeekFrom::Start(2)).unwrap(), 2);
        let mut buffer = [0u8; 3];
        file.read_exact(&mut buffer).unwrap();
        assert_eq!(&buffer, b"cde");
        assert!(file.seek(SeekFrom::Start(99)).is_err());
    }

    #[test]
    fn errors_and_cancellation_wake_the_reader() {
        let temp = NamedTempFile::new().unwrap();
        let reader = temp.reopen().unwrap();
        let spool = Spool::new(None);
        let mut file = GrowingFile::new(reader, Arc::clone(&spool));

        let handle = thread::spawn(move || {
            let mut byte = [0u8; 1];
            file.read(&mut byte).unwrap_err().kind()
        });
        thread::sleep(Duration::from_millis(50));
        spool.cancel();
        assert_eq!(handle.join().unwrap(), std::io::ErrorKind::Interrupted);
    }
}
