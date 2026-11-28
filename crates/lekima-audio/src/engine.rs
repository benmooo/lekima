//! The rodio-backed [`AudioEngine`] implementation.

use std::{
    collections::VecDeque,
    fs::File,
    io::{Cursor, Seek as _, SeekFrom},
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    time::Duration,
};

use async_trait::async_trait;
use lekima_domain::{AudioDeviceEvent, AudioEngine, BufferedRange, DomainError, Result, TrackId};
use rodio::{
    Decoder, DeviceSinkBuilder, Player, Source, cpal,
    cpal::traits::{DeviceTrait, HostTrait},
};

use crate::{
    prefetch::PrefetchSource,
    spool::{GrowingFile, Progress, Spool},
    wave::{Metered, Waveform},
};
use tempfile::NamedTempFile;
use tokio::sync::oneshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
}

fn device_label(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|desc| desc.name().to_owned())
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
        .and_then(|device| device_label(&device));
    let devices = host.output_devices().map_err(|error| {
        DomainError::Playback(format!("failed to enumerate audio devices: {error}"))
    })?;
    Ok(devices
        .filter_map(|device| {
            let name = device_label(&device)?;
            Some(DeviceInfo {
                is_default: default.as_deref() == Some(name.as_str()),
                name,
            })
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
        cached: bool,
        ack: oneshot::Sender<std::result::Result<(), String>>,
    },
    LoadStream {
        reader: File,
        total: Option<u64>,
        spool: Arc<Spool>,
        cached: bool,
        ack: oneshot::Sender<std::result::Result<(), String>>,
    },
    QueueNext {
        track: TrackId,
        bytes: Vec<u8>,
        cached: bool,
        ack: oneshot::Sender<std::result::Result<(), String>>,
    },
    Pause,
    Resume,
    Stop,
    Seek(u64),
    Volume(f32),
    SetDevice {
        name: Option<String>,
        ack: std::sync::mpsc::SyncSender<Result<String>>,
    },
    Shutdown,
}

struct Shared {
    position_ms: AtomicU64,
    loaded: AtomicBool,
    finished: AtomicBool,
    volume_bits: AtomicU32,
    wave: Arc<Waveform>,
    boundary: Mutex<Option<TrackId>>,
    boundary_crossed: AtomicBool,
    stream_duration_ms: AtomicU64,
    cached: AtomicBool,
    output_device: Mutex<Option<String>>,
    device_events: Mutex<VecDeque<AudioDeviceEvent>>,
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
            boundary_crossed: AtomicBool::new(false),
            stream_duration_ms: AtomicU64::new(0),
            cached: AtomicBool::new(false),
            output_device: Mutex::new(None),
            device_events: Mutex::new(VecDeque::new()),
        }
    }

    fn set_output_device(&self, name: Option<String>) {
        *self
            .output_device
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = name;
    }

    fn output_device(&self) -> Option<String> {
        self.output_device
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    fn push_device_event(&self, event: AudioDeviceEvent) {
        self.device_events
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push_back(event);
    }

    fn take_device_event(&self) -> Option<AudioDeviceEvent> {
        self.device_events
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .pop_front()
    }

    fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    fn set_volume(&self, volume: f32) {
        self.volume_bits
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    fn set_stream_duration(&self, duration: Option<Duration>) {
        let millis = duration
            .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        self.stream_duration_ms.store(millis, Ordering::Relaxed);
    }

    fn stream_duration(&self) -> Option<u64> {
        match self.stream_duration_ms.load(Ordering::Relaxed) {
            0 => None,
            millis => Some(millis),
        }
    }
}

/// Audio engine backed by the system's default output device.
pub struct RodioAudioEngine {
    commands: Sender<Command>,
    shared: Arc<Shared>,
    client: reqwest::Client,
    spool: Mutex<Option<Arc<Spool>>>,
    cache: crate::cache::StreamCache,
}

impl RodioAudioEngine {
    /// Opens the default device and starts the audio thread.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::Playback`] when no output device is available.
    pub fn open(
        volume: f32,
        device: Option<&str>,
        cache: crate::cache::StreamCache,
    ) -> Result<Self> {
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
            cache,
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
    async fn play_url(&self, url: &str, cache_key: Option<&str>) -> Result<()> {
        self.cancel_spool();
        // Stop the previous track before the network work.
        let _ = self.send(Command::Stop);

        if let Some(key) = cache_key
            && let Some(bytes) = self.cache.get(key)
        {
            if bytes.is_empty() {
                return Err(DomainError::Playback(
                    "the cached stream is empty".to_owned(),
                ));
            }
            let (ack, ack_rx) = oneshot::channel();
            self.send(Command::Load {
                bytes,
                cached: true,
                ack,
            })?;
            return await_load(ack_rx).await;
        }

        if url.is_empty() {
            return Err(DomainError::Playback(
                "no stream URL and no cached copy".to_owned(),
            ));
        }

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
            if let Some(key) = cache_key {
                self.cache.put(key, &bytes);
            }
            let (ack, ack_rx) = oneshot::channel();
            self.send(Command::Load {
                bytes: bytes.to_vec(),
                cached: false,
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
            cached: false,
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

    fn stream_duration_ms(&self) -> Option<u64> {
        self.shared.stream_duration()
    }

    fn has_cached(&self, key: &str) -> bool {
        self.cache.has(key)
    }

    fn last_was_cached(&self) -> bool {
        self.shared.cached.load(Ordering::Relaxed)
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
        map_buffered_ranges(&spool.ranges(), total, duration_ms, &spool.progress())
    }

    async fn queue_next_url(&self, id: TrackId, url: &str, cache_key: Option<&str>) -> Result<()> {
        if let Some(key) = cache_key
            && let Some(bytes) = self.cache.get(key)
        {
            let (ack, ack_rx) = oneshot::channel();
            self.send(Command::QueueNext {
                track: id,
                bytes,
                cached: true,
                ack,
            })?;
            return await_load(ack_rx).await;
        }
        if url.is_empty() {
            return Err(DomainError::Playback(
                "no stream URL and no cached copy".to_owned(),
            ));
        }
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
        if let Some(key) = cache_key {
            self.cache.put(key, &bytes);
        }
        let (ack, ack_rx) = oneshot::channel();
        self.send(Command::QueueNext {
            track: id,
            bytes: bytes.to_vec(),
            cached: false,
            ack,
        })?;
        await_load(ack_rx).await
    }

    fn take_track_boundary(&self) -> Option<TrackId> {
        let boundary = self
            .shared
            .boundary
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        if boundary.is_some() {
            // The next track is the preloaded in-memory one; the finished
            // stream's byte ranges no longer describe what is playing.
            self.cancel_spool();
        }
        boundary
    }

    fn set_device(&self, name: Option<&str>) -> Result<String> {
        let (ack, rx) = std::sync::mpsc::sync_channel(1);
        self.send(Command::SetDevice {
            name: name.map(str::to_owned),
            ack,
        })?;
        match rx.recv_timeout(DEVICE_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Err(DomainError::Playback(
                "timed out while switching the audio output device".to_owned(),
            )),
        }
    }

    fn output_device(&self) -> Option<String> {
        self.shared.output_device()
    }

    fn take_device_event(&self) -> Option<AudioDeviceEvent> {
        self.shared.take_device_event()
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
    duration: Option<Duration>,
    cached: bool,
    shared: Arc<Shared>,
    fired: bool,
}

impl<S: rodio::Source> Iterator for Boundary<S> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        if !self.fired {
            self.fired = true;
            *self
                .shared
                .boundary
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = Some(self.track);
            self.shared.boundary_crossed.store(true, Ordering::Relaxed);
            self.shared.set_stream_duration(self.duration);
            self.shared.cached.store(self.cached, Ordering::Relaxed);
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

const DEVICE_POLL_TICKS: u8 = 10;

enum ActiveSource {
    Memory {
        bytes: Arc<[u8]>,
        cached: bool,
    },
    Stream {
        reader: File,
        total: Option<u64>,
        spool: Arc<Spool>,
        cached: bool,
    },
}

fn open_device_resolved(
    name: Option<&str>,
) -> std::result::Result<(rodio::MixerDeviceSink, String), DomainError> {
    let host = cpal::default_host();
    match name {
        None => {
            let resolved = host
                .default_output_device()
                .and_then(|device| device_label(&device))
                .ok_or_else(|| DomainError::Playback("no audio output device".to_owned()))?;
            let sink = DeviceSinkBuilder::open_default_sink().map_err(|error| {
                DomainError::Playback(format!("no audio output device: {error}"))
            })?;
            Ok((sink, resolved))
        }
        Some(target) => {
            let device = host
                .output_devices()
                .map_err(|error| {
                    DomainError::Playback(format!("failed to enumerate audio devices: {error}"))
                })?
                .find(|device| device_label(device).as_deref() == Some(target))
                .ok_or_else(|| {
                    DomainError::NotFound(format!("output device `{target}` was not found"))
                })?;
            let sink = DeviceSinkBuilder::from_device(device)
                .and_then(DeviceSinkBuilder::open_stream)
                .map_err(|error| {
                    DomainError::Playback(format!("failed to open `{target}`: {error}"))
                })?;
            Ok((sink, target.to_owned()))
        }
    }
}

fn attach_source(
    player: &Player,
    source: &ActiveSource,
    position_ms: u64,
    paused: bool,
    shared: &Shared,
) -> std::result::Result<(), String> {
    match source {
        ActiveSource::Memory { bytes, cached } => {
            let byte_len = bytes.len() as u64;
            let decoder = Decoder::builder()
                .with_data(Cursor::new(Arc::clone(bytes)))
                .with_byte_len(byte_len)
                .with_seekable(true)
                .build()
                .map_err(|error| format!("audio format not supported: {error}"))?;
            shared.set_stream_duration(decoder.total_duration());
            shared.cached.store(*cached, Ordering::Relaxed);
            shared.loaded.store(true, Ordering::Relaxed);
            shared.finished.store(false, Ordering::Relaxed);
            shared.position_ms.store(position_ms, Ordering::Relaxed);
            shared.wave.reset();
            player.append(Metered::new(decoder, Arc::clone(&shared.wave)));
        }
        ActiveSource::Stream {
            reader,
            total,
            spool,
            cached,
        } => {
            let mut cloned = reader
                .try_clone()
                .map_err(|error| format!("failed to reopen stream spool: {error}"))?;
            cloned
                .seek(SeekFrom::Start(0))
                .map_err(|error| format!("failed to rewind stream spool: {error}"))?;
            let progress = spool.progress();
            let mut builder = Decoder::builder()
                .with_data(GrowingFile::new(cloned, Arc::clone(spool)))
                .with_seekable(true);
            if let Some(total) = total {
                builder = builder.with_byte_len(*total);
            }
            let decoder = builder
                .build()
                .map_err(|error| format!("streamed audio format not supported: {error}"))?;
            shared.set_stream_duration(decoder.total_duration());
            shared.cached.store(*cached, Ordering::Relaxed);
            shared.loaded.store(true, Ordering::Relaxed);
            shared.finished.store(false, Ordering::Relaxed);
            shared.position_ms.store(position_ms, Ordering::Relaxed);
            shared.wave.reset();
            let prefetched = PrefetchSource::new_with_progress(decoder, progress);
            player.append(Metered::new(prefetched, Arc::clone(&shared.wave)));
        }
    }
    if position_ms > 0
        && let Err(error) = player.try_seek(Duration::from_millis(position_ms))
    {
        tracing::warn!(?error, position_ms, "seek after device switch failed");
    }
    if paused {
        player.pause();
    } else {
        player.play();
    }
    Ok(())
}

struct AudioState {
    device: Option<rodio::MixerDeviceSink>,
    player: Option<Player>,
    configured_device: Option<String>,
    active_device: Option<String>,
    current_source: Option<ActiveSource>,
    pending_source: Option<ActiveSource>,
    paused: bool,
}

impl AudioState {
    fn promote_boundary_if_crossed(&mut self, shared: &Shared) {
        if shared.boundary_crossed.swap(false, Ordering::Relaxed)
            && let Some(next) = self.pending_source.take()
        {
            self.current_source = Some(next);
            self.paused = false;
        }
    }
    fn current_position_ms(&self, shared: &Shared) -> u64 {
        if !shared.loaded.load(Ordering::Relaxed) {
            return 0;
        }
        self.player
            .as_ref()
            .map(|player| player.get_pos().as_millis() as u64)
            .unwrap_or_else(|| shared.position_ms.load(Ordering::Relaxed))
    }

    fn reopen_on(
        &mut self,
        target: Option<&str>,
        paused_override: Option<bool>,
        shared: &Shared,
    ) -> std::result::Result<String, DomainError> {
        let saved_pos_ms = self.current_position_ms(shared);
        let paused = paused_override.unwrap_or(self.paused);
        let (new_sink, resolved_name) = open_device_resolved(target)?;

        // Drop old player and sink only after the new sink is open.
        self.player = None;
        self.device = None;
        *shared
            .boundary
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
        shared.boundary_crossed.store(false, Ordering::Relaxed);
        self.pending_source = None;

        let new_player = Player::connect_new(new_sink.mixer());
        new_player.set_volume(shared.volume());
        if shared.loaded.load(Ordering::Relaxed)
            && let Some(source) = self.current_source.as_ref()
            && let Err(error) = attach_source(&new_player, source, saved_pos_ms, paused, shared)
        {
            tracing::warn!(%error, "failed to re-attach track after device switch");
        }

        self.device = Some(new_sink);
        self.player = Some(new_player);
        self.paused = paused;
        self.active_device = Some(resolved_name.clone());
        shared.set_output_device(Some(resolved_name.clone()));
        Ok(resolved_name)
    }

    fn drop_sink_and_pause(&mut self, shared: &Shared) {
        let saved_pos_ms = self.current_position_ms(shared);
        if shared.loaded.load(Ordering::Relaxed) {
            shared.position_ms.store(saved_pos_ms, Ordering::Relaxed);
        }
        self.player = None;
        self.device = None;
        self.paused = true;
        self.active_device = None;
        shared.set_output_device(None);
    }

    fn check_health(&mut self, shared: &Shared) {
        let host = cpal::default_host();
        if let Some(pinned) = self.configured_device.clone() {
            let exists = host
                .output_devices()
                .map(|mut devs| devs.any(|d| device_label(&d).as_deref() == Some(pinned.as_str())))
                .unwrap_or(false);
            if exists {
                if self.active_device.as_deref() != Some(pinned.as_str())
                    && let Ok(resolved) = self.reopen_on(Some(&pinned), None, shared)
                {
                    shared.push_device_event(AudioDeviceEvent::Changed { device: resolved });
                }
                return;
            }
            if self.active_device.as_deref() == Some(pinned.as_str()) {
                match self.reopen_on(None, None, shared) {
                    Ok(fallback) => {
                        shared.push_device_event(AudioDeviceEvent::Lost {
                            previous: Some(pinned),
                            fallback: Some(fallback),
                            paused: false,
                        });
                    }
                    Err(_) => {
                        self.drop_sink_and_pause(shared);
                        shared.push_device_event(AudioDeviceEvent::Lost {
                            previous: Some(pinned),
                            fallback: None,
                            paused: true,
                        });
                    }
                }
                return;
            }
            // Pinned device is still absent and we are already on fallback or
            // paused; fall through to monitor the default device's health.
        }

        let default_name = host
            .default_output_device()
            .and_then(|device| device_label(&device));
        match (&self.active_device, default_name) {
            (Some(current), Some(new_default)) if current != &new_default => {
                if let Ok(resolved) = self.reopen_on(None, None, shared) {
                    shared.push_device_event(AudioDeviceEvent::Changed { device: resolved });
                }
            }
            (Some(current), None) => {
                let previous = current.clone();
                self.drop_sink_and_pause(shared);
                shared.push_device_event(AudioDeviceEvent::Lost {
                    previous: Some(previous),
                    fallback: None,
                    paused: true,
                });
            }
            (None, Some(_)) => {
                if let Ok(resolved) = self.reopen_on(None, Some(true), shared) {
                    shared.push_device_event(AudioDeviceEvent::Changed { device: resolved });
                }
            }
            _ => {}
        }
    }
}

fn audio_thread(
    receiver: Receiver<Command>,
    shared: Arc<Shared>,
    ready: &std::sync::mpsc::SyncSender<std::result::Result<(), String>>,
    device_name: Option<String>,
) {
    let (sink, resolved_name) = match open_device_resolved(device_name.as_deref()) {
        Ok(opened) => opened,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    let initial_player = Player::connect_new(sink.mixer());
    initial_player.set_volume(shared.volume());
    shared.set_output_device(Some(resolved_name.clone()));
    let _ = ready.send(Ok(()));

    let mut state = AudioState {
        device: Some(sink),
        player: Some(initial_player),
        configured_device: device_name,
        active_device: Some(resolved_name),
        current_source: None,
        pending_source: None,
        paused: false,
    };
    let mut poll_ticks: u8 = 0;

    loop {
        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(Command::Load { bytes, cached, ack }) => {
                if let Some(player) = state.player.as_ref() {
                    player.stop();
                }
                shared.set_stream_duration(None);
                shared.cached.store(cached, Ordering::Relaxed);
                *shared
                    .boundary
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = None;
                let len = bytes.len();
                let source = ActiveSource::Memory {
                    bytes: Arc::from(bytes.into_boxed_slice()),
                    cached,
                };
                if let Some(player) = state.player.as_ref() {
                    match attach_source(player, &source, 0, false, &shared) {
                        Ok(()) => {
                            state.current_source = Some(source);
                            state.paused = false;
                            tracing::debug!(bytes = len, "audio loaded");
                            let _ = ack.send(Ok(()));
                        }
                        Err(error) => {
                            shared.loaded.store(false, Ordering::Relaxed);
                            state.current_source = None;
                            let _ = ack.send(Err(error));
                        }
                    }
                } else {
                    shared.loaded.store(false, Ordering::Relaxed);
                    let _ = ack.send(Err("no audio output device".to_owned()));
                }
            }
            Ok(Command::LoadStream {
                reader,
                total,
                spool,
                cached,
                ack,
            }) => {
                if let Some(player) = state.player.as_ref() {
                    player.stop();
                }
                shared.set_stream_duration(None);
                shared.cached.store(cached, Ordering::Relaxed);
                *shared
                    .boundary
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = None;
                let source = ActiveSource::Stream {
                    reader,
                    total,
                    spool,
                    cached,
                };
                if let Some(player) = state.player.as_ref() {
                    match attach_source(player, &source, 0, false, &shared) {
                        Ok(()) => {
                            state.current_source = Some(source);
                            state.paused = false;
                            tracing::debug!("stream loaded from spool");
                            let _ = ack.send(Ok(()));
                        }
                        Err(error) => {
                            shared.loaded.store(false, Ordering::Relaxed);
                            state.current_source = None;
                            let _ = ack.send(Err(error));
                        }
                    }
                } else {
                    shared.loaded.store(false, Ordering::Relaxed);
                    let _ = ack.send(Err("no audio output device".to_owned()));
                }
            }
            Ok(Command::QueueNext {
                track,
                bytes,
                cached,
                ack,
            }) => {
                let Some(player) = state.player.as_ref() else {
                    let _ = ack.send(Err("no audio output device".to_owned()));
                    continue;
                };
                let arc_bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
                let byte_len = arc_bytes.len() as u64;
                let decoder = Decoder::builder()
                    .with_data(Cursor::new(Arc::clone(&arc_bytes)))
                    .with_byte_len(byte_len)
                    .with_seekable(true)
                    .build();
                match decoder {
                    Ok(decoder) => {
                        let duration = decoder.total_duration();
                        let source = Boundary {
                            inner: Metered::new(decoder, Arc::clone(&shared.wave)),
                            track,
                            duration,
                            cached,
                            shared: Arc::clone(&shared),
                            fired: false,
                        };
                        player.append(source);
                        state.pending_source = Some(ActiveSource::Memory {
                            bytes: arc_bytes,
                            cached,
                        });
                        let _ = ack.send(Ok(()));
                    }
                    Err(error) => {
                        let _ = ack.send(Err(format!("next audio format not supported: {error}")));
                    }
                }
            }
            Ok(Command::Pause) => {
                state.paused = true;
                if let Some(player) = state.player.as_ref() {
                    player.pause();
                }
            }
            Ok(Command::Resume) => {
                state.paused = false;
                if let Some(player) = state.player.as_ref() {
                    player.play();
                }
            }
            Ok(Command::Stop) => {
                if let Some(player) = state.player.as_ref() {
                    player.clear();
                }
                state.current_source = None;
                state.paused = false;
                *shared
                    .boundary
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = None;
                shared.set_stream_duration(None);
                shared.cached.store(false, Ordering::Relaxed);
                shared.loaded.store(false, Ordering::Relaxed);
                shared.finished.store(false, Ordering::Relaxed);
                shared.position_ms.store(0, Ordering::Relaxed);
                shared.wave.reset();
            }
            Ok(Command::Seek(position_ms)) => {
                if let Some(player) = state.player.as_ref()
                    && let Err(error) = player.try_seek(Duration::from_millis(position_ms))
                {
                    tracing::warn!(?error, "seek failed");
                }
            }
            Ok(Command::Volume(gain)) => {
                if let Some(player) = state.player.as_ref() {
                    player.set_volume(gain);
                }
            }
            Ok(Command::SetDevice { name, ack }) => {
                state.promote_boundary_if_crossed(&shared);
                let result = state.reopen_on(name.as_deref(), None, &shared);
                if result.is_ok() {
                    state.configured_device = name;
                }
                let _ = ack.send(result);
            }
            Ok(Command::Shutdown) => break,
            Err(RecvTimeoutError::Timeout) => {
                state.promote_boundary_if_crossed(&shared);
                if shared.loaded.load(Ordering::Relaxed)
                    && let Some(player) = state.player.as_ref()
                {
                    let position = player.get_pos();
                    shared
                        .position_ms
                        .store(position.as_millis() as u64, Ordering::Relaxed);
                    if player.empty() {
                        shared.loaded.store(false, Ordering::Relaxed);
                        shared.finished.store(true, Ordering::Relaxed);
                        state.current_source = None;
                        state.pending_source = None;
                        state.paused = false;
                    }
                }
                poll_ticks = poll_ticks.wrapping_add(1);
                if poll_ticks >= DEVICE_POLL_TICKS {
                    poll_ticks = 0;
                    state.check_health(&shared);
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
    progress: &Progress,
) -> Vec<BufferedRange> {
    if total_bytes == 0 || duration_ms == 0 || ranges.is_empty() {
        return Vec::new();
    }
    // Full byte coverage needs no shading. Check the merged reach from 0
    // rather than the mapped time: extrapolation clamps at the duration, so a
    // time check could call a partially buffered file complete.
    let mut reach = 0;
    for range in ranges {
        if range.start > reach {
            break;
        }
        reach = reach.max(range.end);
    }
    if reach >= total_bytes {
        return Vec::new();
    }

    // One snapshot for the whole pass: mapping a range against two different
    // anchor lists could invert it or drop it for one poll.
    let curve = progress.curve(total_bytes, duration_ms);
    let mut mapped: Vec<BufferedRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        // Variable-bitrate streams need the decoder's own byte-to-time curve;
        // the average bitrate is only the fallback before anchors exist.
        let start_ms = curve.time_ms(range.start);
        let end_ms = curve.time_ms(range.end);
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
    mapped
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read as _, Seek as _, SeekFrom, Write as _},
        net::TcpListener,
        sync::Arc,
        time::Duration,
    };

    use tempfile::NamedTempFile;

    use super::{Boundary, Shared, fetch_range, map_buffered_ranges};
    use crate::spool::{GrowingFile, Progress, Spool};

    #[test]
    fn maps_buffered_bytes_to_track_time() {
        let ranges = vec![0..25, 50..75];
        let mapped = map_buffered_ranges(&ranges, 100, 1_000, &Progress::default());
        assert_eq!(mapped.len(), 2);
        assert_eq!((mapped[0].start_ms, mapped[0].end_ms), (0, 250));
        assert_eq!((mapped[1].start_ms, mapped[1].end_ms), (500, 750));
    }

    #[test]
    fn variable_bitrate_ranges_follow_the_decoder_curve() {
        // 2 000 bytes / 2 000 ms overall: the first 500 bytes are slow (1 s)
        // and the rest fast (1 s). A linear map would call byte 500 the quarter
        // point (500 ms) when it is really 1 000 ms.
        let progress = Progress::default();
        progress.set_bytes(0);
        progress.record(0);
        progress.set_bytes(500);
        progress.record(1_000);
        progress.set_bytes(2_000);
        progress.record(2_000);

        let range = 500..2_000;
        let mapped = map_buffered_ranges(std::slice::from_ref(&range), 2_000, 2_000, &progress);

        assert_eq!(mapped.len(), 1);
        assert_eq!((mapped[0].start_ms, mapped[0].end_ms), (1_000, 2_000));
    }

    #[test]
    fn anchors_before_the_first_point_interpolate_from_zero() {
        let progress = Progress::default();
        progress.set_bytes(100);
        progress.record(100);
        progress.set_bytes(400);
        progress.record(700);

        // Between (0,0) and (100,100): byte 50 is 50 ms, not the linear 100.
        assert_eq!(progress.curve(400, 800).time_ms(50), 50);
    }

    #[test]
    fn the_curve_extrapolates_past_the_last_anchor_but_never_past_the_end() {
        let progress = Progress::default();
        progress.set_bytes(0);
        progress.record(0);
        progress.set_bytes(1_000);
        progress.record(1_000);

        let curve = progress.curve(2_000, 1_200);
        assert_eq!(curve.time_ms(1_500), 1_200);
        assert_eq!(curve.time_ms(9_999), 1_200);
    }

    #[test]
    fn full_coverage_needs_no_shading() {
        let complete = 0..100;
        let split = vec![0..40, 40..100];
        let tiny = 0..10;
        let progress = Progress::default();
        assert!(
            map_buffered_ranges(std::slice::from_ref(&complete), 100, 1_000, &progress).is_empty()
        );
        assert!(map_buffered_ranges(&split, 100, 1_000, &progress).is_empty());
        assert!(map_buffered_ranges(&[], 100, 1_000, &progress).is_empty());
        assert!(map_buffered_ranges(std::slice::from_ref(&tiny), 0, 1_000, &progress).is_empty());
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
            duration: Some(Duration::from_secs(30)),
            cached: false,
            shared: Arc::clone(&shared),
            fired: false,
        };
        assert!(source.next().is_some());
        assert_eq!(
            shared.stream_duration(),
            Some(30_000),
            "boundary should publish its stream duration"
        );
        let boundary = shared
            .boundary
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert_eq!(*boundary, Some(lekima_domain::TrackId(7)));
        drop(boundary);
        *shared
            .boundary
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
        assert!(source.next().is_some());
        assert!(
            shared
                .boundary
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .is_none()
        );
    }
}
