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
        atomic::{AtomicBool, AtomicU64, Ordering},
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
    progress: Arc<Progress>,
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
            progress: Arc::new(Progress::default()),
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

    /// Snapshot of the downloaded byte ranges, sorted and merged.
    #[must_use]
    pub fn ranges(&self) -> Vec<Range<u64>> {
        self.lock().ranges.clone()
    }

    /// The byte-to-time curve the decoder learns for this stream.
    #[must_use]
    pub fn progress(&self) -> Arc<Progress> {
        Arc::clone(&self.progress)
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

/// One point where the decoder had read `bytes` of the file and produced `ms`
/// of audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Anchor {
    bytes: u64,
    ms: u64,
}

/// Smallest gap between recorded anchors, so a long track stays bounded.
const MIN_ANCHOR_SPACING_MS: u64 = 500;
/// Anchors kept before the list is thinned by half.
const MAX_ANCHORS: usize = 512;

/// Byte-to-time mapping learned from the decoder.
///
/// A variable-bitrate file cannot be mapped by dividing bytes by the average
/// bitrate: the first minute of a track may be 320 kbps and the rest 96 kbps,
/// which shifts every later range by tens of seconds. The decoder records an
/// anchor as it reads — "at this file offset it had produced this much audio" —
/// and buffered byte ranges are translated through those anchors.
#[derive(Debug, Default)]
pub struct Progress {
    bytes_read: AtomicU64,
    anchors: Mutex<Vec<Anchor>>,
}

impl Progress {
    /// Records where the decoder is in the file.
    pub fn set_bytes(&self, bytes: u64) {
        self.bytes_read.store(bytes, Ordering::Relaxed);
    }

    #[must_use]
    pub fn bytes_read(&self) -> u64 {
        self.bytes_read.load(Ordering::Relaxed)
    }

    /// Records that `ms` of audio has been produced at the current read offset.
    ///
    /// Anchors must advance on both axes; a backward seek resets the list, so
    /// this only ever stores a monotonic curve.
    pub fn record(&self, ms: u64) {
        let bytes = self.bytes_read();
        let Ok(mut anchors) = self.anchors.lock() else {
            return;
        };
        match anchors.last() {
            None => anchors.push(Anchor { bytes, ms }),
            Some(last) => {
                if bytes <= last.bytes || ms <= last.ms {
                    return;
                }
                if ms - last.ms < MIN_ANCHOR_SPACING_MS {
                    return;
                }
                if anchors.len() >= MAX_ANCHORS {
                    let thinned: Vec<Anchor> = anchors.iter().copied().step_by(2).collect();
                    *anchors = thinned;
                }
                anchors.push(Anchor { bytes, ms });
            }
        }
    }

    /// Drops the learned curve, e.g. after a seek.
    pub fn reset(&self) {
        if let Ok(mut anchors) = self.anchors.lock() {
            anchors.clear();
        }
    }

    /// Approximate track time of one file offset.
    ///
    /// Falls back to the average bitrate until two anchors exist, and clamps to
    /// `duration_ms` so shading never runs past the end of the track.
    #[must_use]
    pub fn time_ms(&self, bytes: u64, total_bytes: u64, duration_ms: u64) -> u64 {
        if total_bytes == 0 || duration_ms == 0 {
            return 0;
        }
        let linear = |value: u64| {
            let scaled = u128::from(value.min(total_bytes)) * u128::from(duration_ms)
                / u128::from(total_bytes);
            u64::try_from(scaled).unwrap_or(u64::MAX)
        };

        let Ok(anchors) = self.anchors.lock() else {
            return linear(bytes);
        };
        if anchors.len() < 2 {
            return linear(bytes);
        }

        let first = anchors[0];
        if bytes <= first.bytes {
            return interpolate(0, 0, first.bytes, first.ms, bytes).min(duration_ms);
        }
        for pair in anchors.windows(2) {
            let (left, right) = (pair[0], pair[1]);
            if bytes <= right.bytes {
                return interpolate(left.bytes, left.ms, right.bytes, right.ms, bytes)
                    .min(duration_ms);
            }
        }
        let (left, right) = (anchors[anchors.len() - 2], anchors[anchors.len() - 1]);
        let extra = extrapolate(left, right, bytes);
        right.ms.saturating_add(extra).min(duration_ms)
    }
}

fn interpolate(left_bytes: u64, left_ms: u64, right_bytes: u64, right_ms: u64, at: u64) -> u64 {
    if right_bytes <= left_bytes {
        return right_ms;
    }
    let span = u128::from(right_bytes - left_bytes);
    let offset = u128::from(at.saturating_sub(left_bytes));
    let worked = offset * u128::from(right_ms.saturating_sub(left_ms)) / span;
    left_ms.saturating_add(u64::try_from(worked).unwrap_or(u64::MAX))
}

fn extrapolate(left: Anchor, right: Anchor, at: u64) -> u64 {
    if right.bytes <= left.bytes {
        return 0;
    }
    let span = u128::from(right.bytes - left.bytes);
    let offset = u128::from(at.saturating_sub(right.bytes));
    let worked = offset * u128::from(right.ms.saturating_sub(left.ms)) / span;
    u64::try_from(worked).unwrap_or(u64::MAX)
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
    progress: Arc<Progress>,
    spool: Arc<Spool>,
}

impl GrowingFile {
    #[must_use]
    pub fn new(file: File, spool: Arc<Spool>) -> Self {
        Self {
            file,
            position: 0,
            progress: spool.progress(),
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
                    self.progress.set_bytes(self.position);
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
        self.progress.set_bytes(target);
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

    use super::{GrowingFile, Progress, Spool};

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

    #[test]
    fn anchors_must_advance_on_both_axes() {
        let progress = Progress::default();
        progress.set_bytes(100);
        progress.record(1_000);
        progress.set_bytes(50);
        progress.record(2_000);
        progress.set_bytes(200);
        progress.record(500);
        progress.set_bytes(300);
        progress.record(1_600);

        // Byte 100 → 1 000 ms survives; the backward move and the too-close
        // one are ignored, so the curve stays monotonic and interpolates
        // between the two accepted anchors.
        assert_eq!(progress.time_ms(100, 1_000, 2_000), 1_000);
        assert_eq!(progress.time_ms(200, 1_000, 2_000), 1_300);
    }

    #[test]
    fn a_seek_resets_the_curve() {
        let progress = Progress::default();
        progress.set_bytes(100);
        progress.record(1_000);
        progress.set_bytes(200);
        progress.record(2_000);
        progress.reset();

        // With no anchors left the average bitrate is the fallback.
        assert_eq!(progress.time_ms(100, 200, 2_000), 1_000);
    }

    #[test]
    fn recording_a_long_track_stays_bounded() {
        let progress = Progress::default();
        for step in 1..=4_000u64 {
            progress.set_bytes(step * 1_000);
            progress.record(step * 500);
        }
        // 4 000 s at 500 ms spacing would be 8 000 anchors; the list thins
        // instead of growing without bound, and still maps sensibly.
        let mapped = progress.time_ms(2_000_000, 4_000_000, 2_000_000);
        assert!(mapped > 0 && mapped <= 2_000_000, "mapped {mapped}");
    }
}
