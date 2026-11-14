//! Decode-ahead source that keeps blocking I/O off the audio callback.
//!
//! rodio pulls samples on the device callback thread. Wrapping a decoder in a
//! background producer means a network stall or a range fetch pauses playback
//! (the consumer emits silence) instead of underrunning and killing the stream.
//! Seeks are queued to the producer, which performs the blocking decoder seek,
//! so `Source::try_seek` returns immediately.

use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rodio::{ChannelCount, SampleRate, Source, source::SeekError};

/// Samples decoded per burst.
const BLOCK: usize = 4096;
/// How far ahead of the device the producer decodes.
const BUFFER_SECONDS: u32 = 2;
const WAIT: Duration = Duration::from_millis(50);

struct Shared {
    samples: Mutex<VecDeque<f32>>,
    not_full: Condvar,
    seek: Mutex<Option<Duration>>,
    seeking: AtomicBool,
    eof: AtomicBool,
    error: Mutex<Option<String>>,
    cancelled: AtomicBool,
    capacity: usize,
}

impl Shared {
    fn lock_samples(&self) -> std::sync::MutexGuard<'_, VecDeque<f32>> {
        self.samples
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn take_error(&self) -> Option<String> {
        self.error
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

/// Pulls samples from a background decoder thread.
pub struct PrefetchSource {
    shared: Arc<Shared>,
    channels: ChannelCount,
    sample_rate: SampleRate,
    total_duration: Option<Duration>,
    done: bool,
}

impl PrefetchSource {
    /// Starts a decode-ahead thread for `decoder`.
    ///
    /// The thread is detached and stops when the source is dropped (via
    /// [`Shared::cancelled`]) or when the decoder reaches EOF.
    pub fn new<D>(decoder: D) -> Self
    where
        D: Source<Item = f32> + Send + 'static,
    {
        let channels = decoder.channels();
        let sample_rate = decoder.sample_rate();
        let total_duration = decoder.total_duration();
        let capacity =
            sample_rate.get() as usize * channels.get() as usize * BUFFER_SECONDS as usize;
        let shared = Arc::new(Shared {
            samples: Mutex::new(VecDeque::with_capacity(capacity)),
            not_full: Condvar::new(),
            seek: Mutex::new(None),
            seeking: AtomicBool::new(false),
            eof: AtomicBool::new(false),
            error: Mutex::new(None),
            cancelled: AtomicBool::new(false),
            capacity,
        });

        let producer = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("lekima-decode".to_owned())
            .spawn(move || produce(decoder, &producer))
            .expect("failed to spawn the decoder thread");

        Self {
            shared,
            channels,
            sample_rate,
            total_duration,
            done: false,
        }
    }
}

fn produce<D>(mut decoder: D, shared: &Shared)
where
    D: Source<Item = f32>,
{
    struct Finish<'a>(&'a Shared);
    impl Drop for Finish<'_> {
        fn drop(&mut self) {
            // A panicking or cancelled producer must not leave the consumer
            // emitting silence forever.
            self.0.eof.store(true, Ordering::SeqCst);
        }
    }
    let _finish = Finish(shared);

    loop {
        if shared.cancelled.load(Ordering::SeqCst) {
            return;
        }

        let seek = shared
            .seek
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        if let Some(position) = seek {
            let outcome = decoder.try_seek(position);
            let mut samples = shared.lock_samples();
            samples.clear();
            match outcome {
                Ok(()) => {
                    shared.eof.store(false, Ordering::SeqCst);
                }
                Err(error) => {
                    let message = format!("seek failed: {error:?}");
                    *shared
                        .error
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner()) = Some(message);
                    shared.eof.store(true, Ordering::SeqCst);
                }
            }
            shared.seeking.store(false, Ordering::SeqCst);
            drop(samples);
            shared.not_full.notify_all();
            continue;
        }

        {
            let mut samples = shared.lock_samples();
            while samples.len() >= shared.capacity
                && !shared.cancelled.load(Ordering::SeqCst)
                && shared.seek.lock().map_or(true, |slot| slot.is_none())
            {
                let (guard, _) = shared
                    .not_full
                    .wait_timeout(samples, WAIT)
                    .unwrap_or_else(|poison| poison.into_inner());
                samples = guard;
            }
            if shared.cancelled.load(Ordering::SeqCst) {
                return;
            }
        }

        let mut block = Vec::with_capacity(BLOCK);
        let mut ended = false;
        for _ in 0..BLOCK {
            match decoder.next() {
                Some(sample) => block.push(sample),
                None => {
                    ended = true;
                    break;
                }
            }
        }

        let mut samples = shared.lock_samples();
        samples.extend(block);
        if ended {
            shared.eof.store(true, Ordering::SeqCst);
        }
        drop(samples);
    }
}

impl Iterator for PrefetchSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        // While the producer repositions, do not drain the stale buffer.
        if self.shared.seeking.load(Ordering::SeqCst) {
            return Some(0.0);
        }
        if let Some(sample) = self.shared.lock_samples().pop_front() {
            return Some(sample);
        }
        if self.shared.take_error().is_some() || self.shared.eof.load(Ordering::SeqCst) {
            self.done = true;
            return None;
        }
        // The producer is still fetching or decoding: keep the timeline alive
        // with silence instead of stalling the device.
        Some(0.0)
    }
}

impl Source for PrefetchSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }

    fn try_seek(&mut self, position: Duration) -> std::result::Result<(), SeekError> {
        *self
            .shared
            .seek
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(position);
        self.shared.seeking.store(true, Ordering::SeqCst);
        self.shared.not_full.notify_all();
        Ok(())
    }
}

impl Drop for PrefetchSource {
    fn drop(&mut self) {
        self.shared.cancelled.store(true, Ordering::SeqCst);
        self.shared.not_full.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use rodio::{Source, buffer::SamplesBuffer, source::SeekError};

    use super::PrefetchSource;

    fn tone() -> SamplesBuffer {
        let samples: Vec<f32> = (0..4410).map(|index| index as f32 / 4410.0).collect();
        SamplesBuffer::new(
            rodio::ChannelCount::new(1).expect("stereo"),
            rodio::SampleRate::new(44_100).expect("sample rate"),
            samples,
        )
    }

    #[test]
    fn plays_every_sample_across_the_thread_boundary() {
        let mut source = PrefetchSource::new(tone());
        let audible: Vec<f32> = source
            .by_ref()
            .take(20_000)
            .filter(|sample| *sample != 0.0)
            .collect();
        // The producer may pad the start with silence while it fills the
        // buffer, but no audio sample may be lost.
        assert_eq!(audible.len(), 4409);
        let last = audible.last().copied().unwrap_or_default();
        assert!((last - 4409.0 / 4410.0).abs() < 1e-4, "last sample {last}");
    }

    #[test]
    fn seeks_are_forwarded_to_the_producer() {
        let mut source = PrefetchSource::new(tone());
        source.try_seek(Duration::from_millis(50)).unwrap();
        // Silence is emitted while the producer repositions.
        let samples: Vec<f32> = source.by_ref().take(20_000).collect();
        assert!(samples.len() < 20_000, "source did not finish after a seek");
        let first_sound = samples
            .iter()
            .copied()
            .find(|sample| *sample != 0.0)
            .expect("no audio after the seek");
        // The tone is index/4410, so 50 ms in is around 0.5.
        assert!(
            (0.45..0.55).contains(&first_sound),
            "expected the midpoint of the tone, got {first_sound}"
        );
    }

    #[test]
    fn a_blocking_source_does_not_stall_the_consumer() {
        struct Slow {
            calls: Arc<AtomicUsize>,
        }

        impl Iterator for Slow {
            type Item = f32;

            fn next(&mut self) -> Option<f32> {
                if self.calls.fetch_add(1, Ordering::SeqCst) > 8 {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(30));
                Some(0.5)
            }
        }

        impl Source for Slow {
            fn current_span_len(&self) -> Option<usize> {
                None
            }

            fn channels(&self) -> rodio::ChannelCount {
                rodio::ChannelCount::new(1).unwrap()
            }

            fn sample_rate(&self) -> rodio::SampleRate {
                rodio::SampleRate::new(44_100).unwrap()
            }

            fn total_duration(&self) -> Option<Duration> {
                None
            }

            fn try_seek(&mut self, _: Duration) -> std::result::Result<(), SeekError> {
                Ok(())
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let mut source = PrefetchSource::new(Slow {
            calls: Arc::clone(&calls),
        });
        assert!(source.next().is_some());
    }
}
