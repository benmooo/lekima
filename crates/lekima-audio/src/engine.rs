//! The rodio-backed [`AudioEngine`] implementation.

use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    time::Duration,
};

use async_trait::async_trait;
use lekima_domain::{AudioEngine, DomainError, Result};
use rodio::{Decoder, DeviceSinkBuilder, Player};

use crate::wave::{Metered, WAVE_CAPACITY, Waveform};
use tokio::sync::oneshot;

/// Refuse absurd downloads. NetEase lossless tracks stay well below this.
pub const MAX_TRACK_BYTES: u64 = 64 * 1024 * 1024;

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
            wave: Arc::new(Waveform::new(WAVE_CAPACITY)),
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
}

impl RodioAudioEngine {
    /// Opens the default device and starts the audio thread.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::Playback`] when no output device is available.
    pub fn open(volume: f32) -> Result<Self> {
        let shared = Arc::new(Shared::new(volume));
        let (commands, receiver) = channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let thread_shared = Arc::clone(&shared);

        std::thread::Builder::new()
            .name("lekima-audio".to_owned())
            .spawn(move || audio_thread(receiver, thread_shared, &ready_tx))
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
        })
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
        if response
            .content_length()
            .is_some_and(|len| len > MAX_TRACK_BYTES)
        {
            return Err(DomainError::Playback(format!(
                "refusing to buffer more than {} MiB",
                MAX_TRACK_BYTES / (1024 * 1024)
            )));
        }

        let bytes = response.bytes().await.map_err(|error| {
            DomainError::Playback(format!("failed to read the stream: {error}"))
        })?;
        if bytes.len() as u64 > MAX_TRACK_BYTES {
            return Err(DomainError::Playback(
                "stream exceeds the size limit".to_owned(),
            ));
        }
        if bytes.is_empty() {
            return Err(DomainError::Playback("stream is empty".to_owned()));
        }

        let (ack, ack_rx) = oneshot::channel();
        self.send(Command::Load {
            bytes: bytes.to_vec(),
            ack,
        })?;
        match ack_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(DomainError::Playback(message)),
            Err(_) => Err(DomainError::Playback(
                "the audio thread dropped the load request".to_owned(),
            )),
        }
    }

    fn pause(&self) -> Result<()> {
        self.send(Command::Pause)
    }

    fn resume(&self) -> Result<()> {
        self.send(Command::Resume)
    }

    fn stop(&self) -> Result<()> {
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
}

impl Drop for RodioAudioEngine {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
    }
}

fn audio_thread(
    receiver: Receiver<Command>,
    shared: Arc<Shared>,
    ready: &std::sync::mpsc::SyncSender<std::result::Result<(), String>>,
) {
    let device = match DeviceSinkBuilder::open_default_sink() {
        Ok(device) => device,
        Err(error) => {
            let _ = ready.send(Err(format!("no audio output device: {error}")));
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
