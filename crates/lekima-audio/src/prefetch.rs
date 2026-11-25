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
        Self::new_with_progress(decoder, Arc::new(crate::spool::Progress::default()))
    }

    /// Starts a decode-ahead thread that also learns the stream's
    /// byte-to-time curve.
    pub(crate) fn new_with_progress<D>(decoder: D, progress: Arc<crate::spool::Progress>) -> Self
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
            .spawn(move || produce(decoder, &producer, progress))
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

fn produce<D>(mut decoder: D, shared: &Shared, progress: Arc<crate::spool::Progress>)
where
    D: Source<Item = f32>,
{
    let sample_rate = u64::from(decoder.sample_rate().get());
    let channels = u64::from(decoder.channels().get()).max(1);
    // Samples per second, kept exact: truncating to samples-per-millisecond
    // would make 44.1 kHz stereo drift ~0.23 % (about 0.7 s over five minutes).
    let samples_per_second = (sample_rate * channels).max(1);
    // Audio time decoded so far in this segment, plus the time the last seek
    // landed on. `Progress` needs a monotonic time axis for its anchors.
    let mut decoded: u64 = 0;
    let mut base_ms: u64 = 0;

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
            // Anchors describe the file, not the playback order, so a seek
            // keeps them: a backward move simply stops adding points until the
            // reader passes the old frontier again.
            base_ms = u64::try_from(position.as_millis()).unwrap_or(u64::MAX);
            decoded = 0;
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

        decoded = decoded.saturating_add(block.len() as u64);
        let elapsed = u64::try_from(u128::from(decoded) * 1_000 / u128::from(samples_per_second))
            .unwrap_or(u64::MAX);
        progress.record(base_ms.saturating_add(elapsed));

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

    /// An endless tone that can seek, so the producer never reaches EOF before
    /// the test asks it to reposition.
    struct SeekingTone {
        position: usize,
    }

    impl Iterator for SeekingTone {
        type Item = f32;

        fn next(&mut self) -> Option<Self::Item> {
            self.position += 1;
            Some(self.position as f32 / 4410.0)
        }
    }

    impl Source for SeekingTone {
        fn current_span_len(&self) -> Option<usize> {
            None
        }

        fn channels(&self) -> rodio::ChannelCount {
            rodio::ChannelCount::new(1).expect("mono")
        }

        fn sample_rate(&self) -> rodio::SampleRate {
            rodio::SampleRate::new(44_100).expect("sample rate")
        }

        fn total_duration(&self) -> Option<Duration> {
            None
        }

        fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
            self.position = (position.as_secs_f64() * 44_100.0) as usize;
            Ok(())
        }
    }

    fn drain(source: &mut PrefetchSource, cap: usize) -> Vec<f32> {
        let mut samples = Vec::new();
        for _ in 0..cap {
            match source.next() {
                Some(sample) => samples.push(sample),
                None => break,
            }
            if samples.len().is_multiple_of(4096) {
                std::thread::yield_now();
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        samples
    }

    #[test]
    fn plays_every_sample_across_the_thread_boundary() {
        let mut source = PrefetchSource::new(tone());
        std::thread::sleep(Duration::from_millis(20));
        let samples = drain(&mut source, 1_000_000);
        let audible: Vec<f32> = samples
            .into_iter()
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
        let mut source = PrefetchSource::new(SeekingTone { position: 0 });
        std::thread::sleep(Duration::from_millis(20));
        source.try_seek(Duration::from_millis(50)).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        // Silence is emitted while the producer repositions.
        let samples: Vec<f32> = source.by_ref().take(20_000).collect();
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
