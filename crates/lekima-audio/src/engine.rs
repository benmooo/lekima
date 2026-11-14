//! The rodio-backed [`AudioEngine`] implementation.

use std::{
    fs::File,
    io::Cursor,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    time::Duration,
};

use async_trait::async_trait;
use lekima_domain::{AudioEngine, DomainError, Result};
use rodio::{Decoder, DeviceSinkBuilder, Player, cpal, cpal::traits::HostTrait};

use crate::{
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
}

impl Shared {
    fn new(volume: f32) -> Self {
        Self {
            position_ms: AtomicU64::new(0),
            loaded: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            volume_bits: AtomicU32::new(volume.clamp(0.0, 1.0).to_bits()),
            wave: Arc::new(Waveform::new()),
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
        let mut writer = temp.reopen().map_err(|error| {
            DomainError::Playback(format!("failed to open the stream spool: {error}"))
        })?;
        let reader = temp.reopen().map_err(|error| {
            DomainError::Playback(format!("failed to open the stream spool: {error}"))
        })?;

        let spool = Spool::new(total);
        if let Ok(mut slot) = self.spool.lock() {
            *slot = Some(Arc::clone(&spool));
        }

        let writer_spool = Arc::clone(&spool);
        tokio::spawn(async move {
            let mut response = response;
            loop {
                if writer_spool.is_cancelled() {
                    break;
                }
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        if let Err(error) = writer_spool.push(&mut writer, &chunk) {
                            writer_spool.fail(format!("failed to write the stream spool: {error}"));
                            break;
                        }
                    }
                    Ok(None) => {
                        writer_spool.finish();
                        break;
                    }
                    Err(error) => {
                        writer_spool.fail(format!("failed to read the stream: {error}"));
                        break;
                    }
                }
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
                        player.append(Metered::new(decoder, Arc::clone(&shared.wave)));
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
            Ok(Command::Pause) => player.pause(),
            Ok(Command::Resume) => player.play(),
            Ok(Command::Stop) => {
                player.clear();
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
