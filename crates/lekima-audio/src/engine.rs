//! The rodio-backed [`AudioEngine`] implementation.

use std::{
    fs::File,
    io::Cursor,
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    time::Duration,
};

use async_trait::async_trait;
use lekima_domain::{AudioEngine, BufferedRange, DomainError, Result, TrackId};
use rodio::{Decoder, DeviceSinkBuilder, Player, cpal, cpal::traits::HostTrait};

use crate::{
    prefetch::PrefetchSource,
    spool::{GrowingFile, Spool},
    wave::{Metered, Waveform},
};
use tempfile::NamedTempFile;
use tokio::sync::oneshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
}

/// Lists output devices. Returns an empty list on headless systems.
///
/// # Errors
///
/// Returns [`DomainError::Playback`] when the host cannot enumerate devices.
pub fn devices() -> Result<Vec<DeviceInfo>> {
    let host = cpal::default_host();
    let default = host
        .default_output_device()
        .map(|device| device.to_string());
    let devices = host.output_devices().map_err(|error| {
        DomainError::Playback(format!("failed to enumerate audio devices: {error}"))
    })?;
    Ok(devices
        .map(|device| {
            let name = device.to_string();
            DeviceInfo {
                is_default: default.as_deref() == Some(name.as_str()),
                name,
            }
        })
        .collect())
}

/// Tracks at or below this size are downloaded fully into memory (fast seeks).
pub const MAX_MEMORY_BYTES: u64 = 64 * 1024 * 1024;

/// Bytes to buffer before starting a spooled stream.
const SPOOL_THRESHOLD: u64 = 512 * 1024;
/// Give the network this long to reach the threshold.
const SPOOL_TIMEOUT: Duration = Duration::from_secs(30);

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const DEVICE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";
const REFERER: &str = "https://music.163.com/";

enum Command {
    Load {
        bytes: Vec<u8>,
        ack: oneshot::Sender<std::result::Result<(), String>>,
    },
    LoadStream {
        reader: File,
        total: Option<u64>,
        spool: Arc<Spool>,
        ack: oneshot::Sender<std::result::Result<(), String>>,
    },
    QueueNext {
        track: TrackId,
        bytes: Vec<u8>,
        ack: oneshot::Sender<std::result::Result<(), String>>,
    },
    Pause,
    Resume,
    Stop,
    Seek(u64),
    Volume(f32),
    Shutdown,
}

struct Shared {
    position_ms: AtomicU64,
    loaded: AtomicBool,
    finished: AtomicBool,
    volume_bits: AtomicU32,
    wave: Arc<Waveform>,
    boundary: Mutex<Option<TrackId>>,
}

impl Shared {
    fn new(volume: f32) -> Self {
        Self {
            position_ms: AtomicU64::new(0),
            loaded: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            volume_bits: AtomicU32::new(volume.clamp(0.0, 1.0).to_bits()),
            wave: Arc::new(Waveform::new()),
            boundary: Mutex::new(None),
        }
    }

    fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    fn set_volume(&self, volume: f32) {
        self.volume_bits
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
}

/// Audio engine backed by the system's default output device.
pub struct RodioAudioEngine {
    commands: Sender<Command>,
    shared: Arc<Shared>,
    client: reqwest::Client,
    spool: Mutex<Option<Arc<Spool>>>,
}

impl RodioAudioEngine {
    /// Opens the default device and starts the audio thread.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::Playback`] when no output device is available.
    pub fn open(volume: f32, device: Option<&str>) -> Result<Self> {
        let shared = Arc::new(Shared::new(volume));
        let (commands, receiver) = channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let thread_shared = Arc::clone(&shared);
        let device = device.map(str::to_owned);

        std::thread::Builder::new()
            .name("lekima-audio".to_owned())
            .spawn(move || audio_thread(receiver, thread_shared, &ready_tx, device))
            .map_err(|error| {
                DomainError::Playback(format!("failed to start the audio thread: {error}"))
            })?;

        match ready_rx.recv_timeout(DEVICE_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(message)) => return Err(DomainError::Playback(message)),
            Err(_) => {
                return Err(DomainError::Playback(
                    "timed out while opening the default audio device".to_owned(),
                ));
            }
        }

        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| {
                DomainError::Playback(format!("failed to build HTTP client: {error}"))
            })?;

        Ok(Self {
            commands,
            shared,
            client,
            spool: Mutex::new(None),
        })
    }

    fn cancel_spool(&self) {
        if let Ok(mut slot) = self.spool.lock()
            && let Some(spool) = slot.take()
        {
            spool.cancel();
        }
    }

    fn send(&self, command: Command) -> Result<()> {
        self.commands
            .send(command)
            .map_err(|_| DomainError::Playback("the audio thread is no longer running".to_owned()))
    }
}

#[async_trait]
impl AudioEngine for RodioAudioEngine {
    async fn play_url(&self, url: &str) -> Result<()> {
        self.cancel_spool();
        // Stop the previous track before the network work.
        let _ = self.send(Command::Stop);

        let response = self
            .client
            .get(url)
            .header(reqwest::header::REFERER, REFERER)
            .send()
            .await
            .map_err(|error| {
                DomainError::Playback(format!("failed to fetch the stream: {error}"))
            })?;

        if !response.status().is_success() {
            return Err(DomainError::Playback(format!(
                "the stream server returned {}",
                response.status()
            )));
        }

        let total = response.content_length();
        let memory = total.is_some_and(|length| length <= MAX_MEMORY_BYTES);

        if memory {
            let bytes = response.bytes().await.map_err(|error| {
                DomainError::Playback(format!("failed to read the stream: {error}"))
            })?;
            if bytes.is_empty() {
                return Err(DomainError::Playback("stream is empty".to_owned()));
            }
            let (ack, ack_rx) = oneshot::channel();
            self.send(Command::Load {
                bytes: bytes.to_vec(),
                ack,
            })?;
            return await_load(ack_rx).await;
        }

        // Large or unknown-length stream: spool to an anonymous temp file.
        let temp = NamedTempFile::new().map_err(|error| {
            DomainError::Playback(format!("failed to create the stream spool: {error}"))
        })?;
        let reader = temp.reopen().map_err(|error| {
            DomainError::Playback(format!("failed to open the stream spool: {error}"))
        })?;

        let spool = Spool::new(total);
        if let Ok(mut slot) = self.spool.lock() {
            *slot = Some(Arc::clone(&spool));
        }

        let sequential_writer = temp.reopen().map_err(|error| {
            DomainError::Playback(format!("failed to open the stream spool: {error}"))
        })?;
        let range_writer = temp.reopen().map_err(|error| {
            DomainError::Playback(format!("failed to open the stream spool: {error}"))
        })?;

        let sequential_spool = Arc::clone(&spool);
        tokio::spawn(async move {
            download_sequential(response, sequential_spool, sequential_writer).await;
        });

        let range_spool = Arc::clone(&spool);
        let range_client = self.client.clone();
        let range_url = url.to_owned();
        tokio::spawn(async move {
            let mut writer = range_writer;
            while let Some(target) = range_spool.next_request().await {
                fetch_range(&range_client, &range_url, target, &range_spool, &mut writer).await;
            }
        });

        let wait_spool = Arc::clone(&spool);
        let ready = tokio::task::spawn_blocking(move || {
            wait_spool.wait_ready(SPOOL_THRESHOLD, SPOOL_TIMEOUT)
        })
        .await
        .map_err(|error| DomainError::Playback(format!("buffering task failed: {error}")))?;

        if let Err(message) = ready {
            spool.cancel();
            return Err(DomainError::Playback(message));
        }

        let (ack, ack_rx) = oneshot::channel();
        self.send(Command::LoadStream {
            reader,
            total,
            spool: Arc::clone(&spool),
            ack,
        })?;
        await_load(ack_rx).await
    }

    fn pause(&self) -> Result<()> {
        self.send(Command::Pause)
    }

    fn resume(&self) -> Result<()> {
        self.send(Command::Resume)
    }

    fn stop(&self) -> Result<()> {
        self.cancel_spool();
        self.send(Command::Stop)
    }

    fn seek(&self, position_ms: u64) -> Result<()> {
        self.send(Command::Seek(position_ms))
    }

    fn set_volume(&self, gain: f32) -> Result<()> {
        self.shared.set_volume(gain);
        self.send(Command::Volume(gain))
    }

    fn position_ms(&self) -> u64 {
        self.shared.position_ms.load(Ordering::Relaxed)
    }

    fn is_finished(&self) -> bool {
        self.shared.finished.load(Ordering::Relaxed)
    }

    fn has_track(&self) -> bool {
        self.shared.loaded.load(Ordering::Relaxed)
    }

    fn wave(&self) -> Vec<f32> {
        self.shared.wave.snapshot()
    }

    fn is_buffering(&self) -> bool {
        self.spool
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|spool| spool.is_buffering()))
            .unwrap_or(false)
    }

    fn buffered_ranges(&self, duration_ms: u64) -> Vec<BufferedRange> {
        let Ok(slot) = self.spool.lock() else {
            return Vec::new();
        };
        let Some(spool) = slot.as_ref() else {
            return Vec::new();
        };
        let Some(total) = spool.total().filter(|total| *total > 0) else {
            return Vec::new();
        };
        map_buffered_ranges(&spool.ranges(), total, duration_ms)
    }

    async fn queue_next_url(&self, id: TrackId, url: &str) -> Result<()> {
        let response = self
            .client
            .get(url)
            .header(reqwest::header::REFERER, REFERER)
            .send()
            .await
            .map_err(|error| {
                DomainError::Playback(format!("failed to fetch the next stream: {error}"))
            })?;
        if !response.status().is_success() {
            return Err(DomainError::Playback(format!(
                "the next stream returned {}",
                response.status()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_MEMORY_BYTES)
        {
            return Err(DomainError::Unsupported(
                "next track is too large to preload".to_owned(),
            ));
        }
        let bytes = response.bytes().await.map_err(|error| {
            DomainError::Playback(format!("failed to read the next stream: {error}"))
        })?;
        let (ack, ack_rx) = oneshot::channel();
        self.send(Command::QueueNext {
            track: id,
            bytes: bytes.to_vec(),
            ack,
        })?;
        await_load(ack_rx).await
    }

    fn take_track_boundary(&self) -> Option<TrackId> {
        self.shared
            .boundary
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
    }
}

async fn await_load(ack_rx: oneshot::Receiver<std::result::Result<(), String>>) -> Result<()> {
    match ack_rx.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => Err(DomainError::Playback(message)),
        Err(_) => Err(DomainError::Playback(
            "the audio thread dropped the load request".to_owned(),
        )),
    }
}

impl Drop for RodioAudioEngine {
    fn drop(&mut self) {
        self.cancel_spool();
        let _ = self.commands.send(Command::Shutdown);
    }
}

/// Marks the atomic boundary when its first sample is pulled.
struct Boundary<S> {
    inner: S,
    track: TrackId,
    shared: Arc<Shared>,
}

impl<S: rodio::Source> Iterator for Boundary<S> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        let mut boundary = self
            .shared
            .boundary
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if boundary.is_none() {
            *boundary = Some(self.track);
            self.shared.position_ms.store(0, Ordering::Relaxed);
        }
        Some(sample)
    }
}

impl<S: rodio::Source> rodio::Source for Boundary<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(
        &mut self,
        position: Duration,
    ) -> std::result::Result<(), rodio::source::SeekError> {
        self.inner.try_seek(position)
    }
}

/// Fetching a seek target can pull a few megabytes of read-ahead; retry
/// requests beyond this window are issued as the decoder needs them.
const RANGE_CHUNK: u64 = 4 * 1024 * 1024;

async fn download_sequential(mut response: reqwest::Response, spool: Arc<Spool>, mut writer: File) {
    if spool.total().is_none()
        && let Some(length) = response.content_length()
    {
        spool.set_total(length);
    }
    let mut offset = 0u64;
    loop {
        if spool.is_cancelled() {
            return;
        }
        match response.chunk().await {
            Ok(Some(chunk)) => match spool.write_at(&mut writer, offset, &chunk) {
                Ok(true) => offset += chunk.len() as u64,
                Ok(false) => return,
                Err(error) => {
                    spool.fail(format!("failed to write the stream spool: {error}"));
                    return;
                }
            },
            Ok(None) => {
                spool.set_total(offset);
                spool.mark_complete();
                return;
            }
            Err(error) => {
                spool.fail(format!("failed to read the stream: {error}"));
                return;
            }
        }
    }
}

async fn fetch_range(
    client: &reqwest::Client,
    url: &str,
    target: u64,
    spool: &Spool,
    writer: &mut File,
) {
    tracing::debug!(target, "fetching byte range");
    let response = client
        .get(url)
        .header(reqwest::header::REFERER, REFERER)
        .header(
            reqwest::header::RANGE,
            format!("bytes={target}-{}", target + RANGE_CHUNK - 1),
        )
        .send()
        .await;
    let mut response = match response {
        Ok(response) => response,
        Err(error) => {
            spool.fail(format!("failed to fetch a byte range: {error}"));
            return;
        }
    };

    match response.status().as_u16() {
        206 => {
            if let Some(total) = content_range_total(&response) {
                spool.set_total(total);
            }
        }
        200 => {
            // The server ignored `Range`; the sequential downloader will reach
            // the target eventually, so let the reader wait for it.
            tracing::warn!("stream server ignored Range; falling back to sequential");
            return;
        }
        status => {
            spool.fail(format!(
                "the stream server returned {status} for a range request"
            ));
            return;
        }
    }

    let mut offset = target;
    loop {
        if spool.is_cancelled() || offset - target >= RANGE_CHUNK {
            return;
        }
        match response.chunk().await {
            Ok(Some(chunk)) => match spool.write_at(writer, offset, &chunk) {
                Ok(true) => offset += chunk.len() as u64,
                Ok(false) => return,
                Err(error) => {
                    spool.fail(format!("failed to write a byte range: {error}"));
                    return;
                }
            },
            Ok(None) => {
                // The range ended at EOF.
                spool.set_total(offset);
                return;
            }
            Err(error) => {
                spool.fail(format!("failed to read a byte range: {error}"));
                return;
            }
        }
    }
}

fn content_range_total(response: &reqwest::Response) -> Option<u64> {
    let value = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    value.rsplit('/').next()?.trim().parse().ok()
}

fn open_device(name: Option<&str>) -> std::result::Result<rodio::MixerDeviceSink, String> {
    match name {
        None => DeviceSinkBuilder::open_default_sink()
            .map_err(|error| format!("no audio output device: {error}")),
        Some(name) => {
            let host = cpal::default_host();
            let device = host
                .output_devices()
                .map_err(|error| format!("failed to enumerate audio devices: {error}"))?
                .find(|device| device.to_string() == name)
                .ok_or_else(|| format!("output device `{name}` was not found"))?;
            DeviceSinkBuilder::from_device(device)
                .and_then(DeviceSinkBuilder::open_stream)
                .map_err(|error| format!("failed to open `{name}`: {error}"))
        }
    }
}

fn audio_thread(
    receiver: Receiver<Command>,
    shared: Arc<Shared>,
    ready: &std::sync::mpsc::SyncSender<std::result::Result<(), String>>,
    device_name: Option<String>,
) {
    let device = match open_device(device_name.as_deref()) {
        Ok(device) => device,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let player = Player::connect_new(device.mixer());
    player.set_volume(shared.volume());
    let _ = ready.send(Ok(()));

    loop {
        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(Command::Load { bytes, ack }) => {
                player.stop();
                *shared
                    .boundary
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = None;
                let len = bytes.len();
                let byte_len = bytes.len() as u64;
                let decoder = Decoder::builder()
                    .with_data(Cursor::new(bytes))
                    .with_byte_len(byte_len)
                    .with_seekable(true)
                    .build();
                match decoder {
                    Ok(decoder) => {
                        shared.loaded.store(true, Ordering::Relaxed);
                        shared.finished.store(false, Ordering::Relaxed);
                        shared.position_ms.store(0, Ordering::Relaxed);
                        shared.wave.reset();
                        player.append(Metered::new(decoder, Arc::clone(&shared.wave)));
                        player.play();
                        tracing::debug!(bytes = len, "audio loaded");
                        let _ = ack.send(Ok(()));
                    }
                    Err(error) => {
                        shared.loaded.store(false, Ordering::Relaxed);
                        let _ = ack.send(Err(format!("audio format not supported: {error}")));
                    }
                }
            }
            Ok(Command::LoadStream {
                reader,
                total,
                spool,
                ack,
            }) => {
                player.stop();
                *shared
                    .boundary
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = None;
                let mut builder = Decoder::builder()
                    .with_data(GrowingFile::new(reader, spool))
                    .with_seekable(true);
                if let Some(total) = total {
                    builder = builder.with_byte_len(total);
                }
                match builder.build() {
                    Ok(decoder) => {
                        shared.loaded.store(true, Ordering::Relaxed);
                        shared.finished.store(false, Ordering::Relaxed);
                        shared.position_ms.store(0, Ordering::Relaxed);
                        shared.wave.reset();
                        // Decode ahead so range fetches never block the device.
                        let source = PrefetchSource::new(decoder);
                        player.append(Metered::new(source, Arc::clone(&shared.wave)));
                        player.play();
                        tracing::debug!("stream loaded from spool");
                        let _ = ack.send(Ok(()));
                    }
                    Err(error) => {
                        shared.loaded.store(false, Ordering::Relaxed);
                        let _ =
                            ack.send(Err(format!("streamed audio format not supported: {error}")));
                    }
                }
            }
            Ok(Command::QueueNext { track, bytes, ack }) => {
                let byte_len = bytes.len() as u64;
                let decoder = Decoder::builder()
                    .with_data(Cursor::new(bytes))
                    .with_byte_len(byte_len)
                    .with_seekable(true)
                    .build();
                match decoder {
                    Ok(decoder) => {
                        let source = Boundary {
                            inner: Metered::new(decoder, Arc::clone(&shared.wave)),
                            track,
                            shared: Arc::clone(&shared),
                        };
                        player.append(source);
                        let _ = ack.send(Ok(()));
                    }
                    Err(error) => {
                        let _ = ack.send(Err(format!("next audio format not supported: {error}")));
                    }
                }
            }
            Ok(Command::Pause) => player.pause(),
            Ok(Command::Resume) => player.play(),
            Ok(Command::Stop) => {
                player.clear();
                *shared
                    .boundary
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = None;
                shared.loaded.store(false, Ordering::Relaxed);
                shared.finished.store(false, Ordering::Relaxed);
                shared.position_ms.store(0, Ordering::Relaxed);
                shared.wave.reset();
            }
            Ok(Command::Seek(position_ms)) => {
                if let Err(error) = player.try_seek(Duration::from_millis(position_ms)) {
                    tracing::warn!(?error, "seek failed");
                }
            }
            Ok(Command::Volume(gain)) => player.set_volume(gain),
            Ok(Command::Shutdown) => break,
            Err(RecvTimeoutError::Timeout) => {
                if shared.loaded.load(Ordering::Relaxed) {
                    let position = player.get_pos();
                    shared
                        .position_ms
                        .store(position.as_millis() as u64, Ordering::Relaxed);
                    if player.empty() {
                        shared.loaded.store(false, Ordering::Relaxed);
                        shared.finished.store(true, Ordering::Relaxed);
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    tracing::debug!("audio thread stopped");
}

/// Maps downloaded byte ranges onto track time. Full coverage returns no
/// ranges, because a fully buffered rail should look like an ordinary empty
/// progress bar rather than a played one.
fn map_buffered_ranges(
    ranges: &[Range<u64>],
    total_bytes: u64,
    duration_ms: u64,
) -> Vec<BufferedRange> {
    if total_bytes == 0 || duration_ms == 0 || ranges.is_empty() {
        return Vec::new();
    }
    if ranges
        .first()
        .is_some_and(|range| range.start == 0 && range.end >= total_bytes)
    {
        return Vec::new();
    }

    let mut mapped: Vec<BufferedRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        let start = u128::from(range.start.min(total_bytes)) * u128::from(duration_ms)
            / u128::from(total_bytes);
        let end = u128::from(range.end.min(total_bytes)) * u128::from(duration_ms)
            / u128::from(total_bytes);
        let start_ms = u64::try_from(start).unwrap_or(u64::MAX);
        let end_ms = u64::try_from(end).unwrap_or(u64::MAX);
        if end_ms <= start_ms {
            continue;
        }
        match mapped.last_mut() {
            Some(previous) if start_ms <= previous.end_ms => {
                previous.end_ms = previous.end_ms.max(end_ms);
            }
            _ => mapped.push(BufferedRange { start_ms, end_ms }),
        }
    }
    if mapped.len() == 1 && mapped[0].start_ms == 0 && mapped[0].end_ms >= duration_ms {
        return Vec::new();
    }
    mapped
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read as _, Seek as _, SeekFrom, Write as _},
        net::TcpListener,
        sync::Arc,
    };

    use tempfile::NamedTempFile;

    use super::{Boundary, Shared, fetch_range, map_buffered_ranges};
    use crate::spool::{GrowingFile, Spool};

    #[test]
    fn maps_buffered_bytes_to_track_time() {
        let ranges = vec![0..25, 50..75];
        let mapped = map_buffered_ranges(&ranges, 100, 1_000);
        assert_eq!(mapped.len(), 2);
        assert_eq!((mapped[0].start_ms, mapped[0].end_ms), (0, 250));
        assert_eq!((mapped[1].start_ms, mapped[1].end_ms), (500, 750));
    }

    #[test]
    fn full_coverage_needs_no_shading() {
        let complete = 0..100;
        let split = vec![0..40, 40..100];
        let tiny = 0..10;
        assert!(map_buffered_ranges(std::slice::from_ref(&complete), 100, 1_000).is_empty());
        assert!(map_buffered_ranges(&split, 100, 1_000).is_empty());
        assert!(map_buffered_ranges(&[], 100, 1_000).is_empty());
        assert!(map_buffered_ranges(std::slice::from_ref(&tiny), 0, 1_000).is_empty());
    }

    #[tokio::test]
    async fn range_fetch_fills_a_hole_ahead_of_the_sequential_download() {
        let body: Vec<u8> = (0..64 * 1024).map(|index| (index % 251) as u8).collect();
        let total = body.len();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_body = body.clone();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            loop {
                let read = stream.read(&mut buffer).unwrap_or(0);
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&request).to_ascii_lowercase();
            let start: usize = text
                .split("range: bytes=")
                .nth(1)
                .and_then(|rest| rest.split('-').next())
                .and_then(|value| value.trim().parse().ok())
                .expect("a Range header");
            let slice = &server_body[start..];
            let header = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{total}\r\nConnection: close\r\n\r\n",
                slice.len(),
                total - 1,
            );
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(slice).unwrap();
            stream.flush().unwrap();
        });

        let spool = Spool::new(Some(total as u64));
        let temp = NamedTempFile::new().unwrap();
        let mut writer = temp.reopen().unwrap();
        let client = reqwest::Client::new();
        let target = 32 * 1024u64;
        fetch_range(
            &client,
            &format!("http://{address}/audio"),
            target,
            &spool,
            &mut writer,
        )
        .await;
        server.join().unwrap();

        assert!(spool.is_covered(target));
        assert_eq!(spool.total(), Some(total as u64));
        let mut file = GrowingFile::new(temp.reopen().unwrap(), Arc::clone(&spool));
        file.seek(SeekFrom::Start(target)).unwrap();
        let mut fetched = vec![0u8; 1024];
        file.read_exact(&mut fetched).unwrap();
        assert_eq!(
            &fetched,
            &body[target as usize..target as usize + fetched.len()]
        );
    }

    #[test]
    fn boundary_source_signals_once_on_the_first_sample() {
        let shared = Arc::new(Shared::new(0.0));
        let mut source = Boundary {
            inner: rodio::source::SineWave::new(440.0),
            track: lekima_domain::TrackId(7),
            shared: Arc::clone(&shared),
        };
        assert!(source.next().is_some());
        let boundary = shared
            .boundary
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert_eq!(*boundary, Some(lekima_domain::TrackId(7)));
    }
}
