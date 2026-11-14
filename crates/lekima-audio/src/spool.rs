//! Sparse growing-file spool for streaming playback.
//!
//! The downloader writes at arbitrary offsets (sequential and HTTP `Range`
//! fetches), and the decoder reads from a second handle. Reads and seeks outside
//! downloaded ranges block until a range worker fills them, while the
//! `buffering` flag is set.
//!
//! Reads block on the *decoder prefetch thread*, never on the audio callback,
//! so a slow range fetch pauses output instead of killing the stream.

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    ops::Range,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::sync::Notify;

const WAIT: Duration = Duration::from_millis(100);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const SEEK_TIMEOUT: Duration = Duration::from_secs(30);

struct State {
    /// Sorted, merged downloaded byte ranges.
    ranges: Vec<Range<u64>>,
    total: Option<u64>,
    complete: bool,
    error: Option<String>,
    cancelled: bool,
}

pub struct Spool {
    state: Mutex<State>,
    ready: Condvar,
    buffering: AtomicBool,
    pending: Mutex<Vec<u64>>,
    wake: Notify,
}

impl Spool {
    #[must_use]
    pub fn new(total: Option<u64>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                ranges: Vec::new(),
                total,
                complete: false,
                error: None,
                cancelled: false,
            }),
            ready: Condvar::new(),
            buffering: AtomicBool::new(false),
            pending: Mutex::new(Vec::new()),
            wake: Notify::new(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Writes one chunk at an absolute offset and merges its range.
    pub fn write_at(&self, file: &mut File, offset: u64, chunk: &[u8]) -> io::Result<bool> {
        let mut state = self.lock();
        if state.cancelled {
            return Ok(false);
        }
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(chunk)?;
        merge(&mut state.ranges, offset, offset + chunk.len() as u64);
        drop(state);
        self.ready.notify_all();
        Ok(true)
    }

    /// Queues a request for the bytes starting at `offset`.
    pub fn request(&self, offset: u64) {
        if self.is_covered(offset) {
            return;
        }
        if let Ok(mut pending) = self.pending.lock()
            && !pending.contains(&offset)
        {
            pending.push(offset);
        }
        self.wake.notify_one();
    }

    /// Next byte offset the range worker should fetch, skipping covered ranges.
    ///
    /// Returns `None` once the spool is cancelled. Newest requests win, so an
    /// interrupted seek refetches its own target rather than every position it
    /// passed through.
    pub async fn next_request(&self) -> Option<u64> {
        loop {
            if self.is_cancelled() {
                return None;
            }
            let candidates = {
                let Ok(mut pending) = self.pending.lock() else {
                    continue;
                };
                std::mem::take(&mut *pending)
            };
            if let Some(offset) = candidates
                .into_iter()
                .rev()
                .find(|offset| !self.is_covered(*offset))
            {
                return Some(offset);
            }
            self.wake.notified().await;
        }
    }

    pub fn mark_complete(&self) {
        let mut state = self.lock();
        state.complete = true;
        drop(state);
        self.ready.notify_all();
    }

    pub fn fail(&self, error: impl Into<String>) {
        let mut state = self.lock();
        state.error = Some(error.into());
        state.complete = true;
        drop(state);
        self.ready.notify_all();
    }

    pub fn cancel(&self) {
        let mut state = self.lock();
        state.cancelled = true;
        drop(state);
        self.ready.notify_all();
        self.wake.notify_one();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    #[must_use]
    pub fn total(&self) -> Option<u64> {
        self.lock().total
    }

    /// Records the total length once it is known from a response header.
    pub fn set_total(&self, total: u64) {
        let mut state = self.lock();
        if state.total.is_none() {
            state.total = Some(total);
        }
        drop(state);
        self.ready.notify_all();
    }

    #[must_use]
    pub fn is_buffering(&self) -> bool {
        self.buffering.load(Ordering::Relaxed)
    }

    /// True when `offset` can be read without waiting.
    #[must_use]
    pub fn is_covered(&self, offset: u64) -> bool {
        covered_end(&self.lock().ranges, offset).is_some()
    }

    /// Blocks until `[0, threshold)` is covered, the download completes, or it fails.
    pub fn wait_ready(&self, threshold: u64, timeout: Duration) -> Result<(), String> {
        let started = Instant::now();
        let mut state = self.lock();
        loop {
            if let Some(error) = &state.error {
                return Err(error.clone());
            }
            if state.cancelled {
                return Err("stream was cancelled".to_owned());
            }
            let target = state.total.map_or(threshold, |total| total.min(threshold));
            if state
                .ranges
                .first()
                .is_some_and(|range| range.start == 0 && range.end >= target)
            {
                return Ok(());
            }
            if state.complete && state.total.is_none() {
                return Ok(());
            }
            if started.elapsed() >= timeout {
                return Err("timed out while buffering the stream".to_owned());
            }
            let (next, _) = self
                .ready
                .wait_timeout(state, WAIT)
                .unwrap_or_else(|poison| poison.into_inner());
            state = next;
        }
    }

    fn wait_for(&self, offset: u64, timeout: Duration) -> Result<(), String> {
        let started = Instant::now();
        self.buffering.store(true, Ordering::Relaxed);
        let result = loop {
            self.request(offset);
            let state = self.lock();
            if let Some(error) = &state.error {
                break Err(error.clone());
            }
            if state.cancelled {
                break Err("stream was cancelled".to_owned());
            }
            if covered_end(&state.ranges, offset).is_some() {
                break Ok(());
            }
            if state.complete && state.total.is_some_and(|total| offset >= total) {
                break Ok(());
            }
            if started.elapsed() >= timeout {
                break Err("timed out while seeking the stream".to_owned());
            }
            let _ = self
                .ready
                .wait_timeout(state, WAIT)
                .unwrap_or_else(|poison| poison.into_inner());
        };
        self.buffering.store(false, Ordering::Relaxed);
        result
    }
}

fn covered_end(ranges: &[Range<u64>], offset: u64) -> Option<u64> {
    ranges
        .iter()
        .find(|range| range.start <= offset && offset < range.end)
        .map(|range| range.end)
}

fn merge(ranges: &mut Vec<Range<u64>>, start: u64, end: u64) {
    let mut merged = Vec::with_capacity(ranges.len() + 1);
    let mut insertion = Some((start, end));
    for range in ranges.iter() {
        match insertion.take() {
            Some((mut new_start, mut new_end))
                if range.start <= new_end && new_start <= range.end =>
            {
                new_start = new_start.min(range.start);
                new_end = new_end.max(range.end);
                insertion = Some((new_start, new_end));
            }
            Some(pending) if pending.0 < range.start => {
                merged.push(pending.0..pending.1);
                merged.push(range.clone());
            }
            Some(pending) => {
                merged.push(range.clone());
                insertion = Some(pending);
            }
            None => merged.push(range.clone()),
        }
    }
    if let Some((start, end)) = insertion {
        merged.push(start..end);
    }
    *ranges = merged;
}

/// `Read + Seek` view over the sparse spool.
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
            if let Some(total) = self.spool.total()
                && self.position >= total
            {
                return Ok(0);
            }
            {
                let state = self.spool.lock();
                if let Some(error) = &state.error {
                    return Err(io::Error::other(error.clone()));
                }
                if state.cancelled {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "stream was cancelled",
                    ));
                }
                if let Some(end) = covered_end(&state.ranges, self.position) {
                    let available = (end - self.position).min(buffer.len() as u64) as usize;
                    drop(state);
                    self.file.seek(SeekFrom::Start(self.position))?;
                    let read = self.file.read(&mut buffer[..available])?;
                    self.position += read as u64;
                    self.spool.buffering.store(false, Ordering::Relaxed);
                    return Ok(read);
                }
            }
            if started.elapsed() >= READ_TIMEOUT {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "stream stalled for 30 seconds",
                ));
            }
            self.spool.buffering.store(true, Ordering::Relaxed);
            self.spool.request(self.position);
            let state = self.spool.lock();
            let _ = self
                .spool
                .ready
                .wait_timeout(state, WAIT)
                .unwrap_or_else(|poison| poison.into_inner());
        }
    }
}

impl Seek for GrowingFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
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
        if let Some(total) = total
            && target > total
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek past the end of the stream",
            ));
        }
        if target < total.unwrap_or(u64::MAX) && !self.spool.is_covered(target) {
            self.spool
                .wait_for(target, SEEK_TIMEOUT)
                .map_err(io::Error::other)?;
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
    fn sequential_writes_are_readable_and_out_of_order_writes_merge() {
        let temp = NamedTempFile::new().unwrap();
        let mut writer = temp.reopen().unwrap();
        let spool = Spool::new(Some(9));
        spool.write_at(&mut writer, 0, b"abc").unwrap();
        spool.write_at(&mut writer, 6, b"ghi").unwrap();
        spool.write_at(&mut writer, 3, b"def").unwrap();
        spool.mark_complete();

        let mut file = GrowingFile::new(temp.reopen().unwrap(), Arc::clone(&spool));
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).unwrap();
        assert_eq!(buffer, b"abcdefghi");
        assert_eq!(file.seek(SeekFrom::Start(2)).unwrap(), 2);
        let mut part = [0u8; 3];
        file.read_exact(&mut part).unwrap();
        assert_eq!(&part, b"cde");
    }

    #[test]
    fn seek_beyond_buffered_bytes_waits_for_the_range() {
        let temp = NamedTempFile::new().unwrap();
        let spool = Spool::new(Some(10));
        let mut file = GrowingFile::new(temp.reopen().unwrap(), Arc::clone(&spool));

        let reader_spool = Arc::clone(&spool);
        let handle = thread::spawn(move || {
            file.seek(SeekFrom::Start(6)).unwrap();
            let mut buffer = [0u8; 2];
            file.read_exact(&mut buffer).unwrap();
            buffer
        });

        thread::sleep(Duration::from_millis(80));
        let mut writer = temp.reopen().unwrap();
        spool.write_at(&mut writer, 6, b"gh").unwrap();
        spool.mark_complete();
        assert_eq!(&handle.join().unwrap(), b"gh");
        assert!(reader_spool.total().is_some());
    }

    #[test]
    fn cancellation_wakes_readers() {
        let temp = NamedTempFile::new().unwrap();
        let spool = Spool::new(Some(10));
        let mut file = GrowingFile::new(temp.reopen().unwrap(), Arc::clone(&spool));
        let handle = thread::spawn(move || {
            let mut buffer = [0u8; 1];
            file.read(&mut buffer).unwrap_err().kind()
        });
        thread::sleep(Duration::from_millis(50));
        spool.cancel();
        assert_eq!(handle.join().unwrap(), std::io::ErrorKind::Interrupted);
    }
}
