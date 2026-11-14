//! Amplitude envelope sampled from the decoded stream.
//!
//! This is a real RMS measurement of the samples that reach the mixer — not a
//! decorative animation. The ring buffer keeps the last ~2.4 s of 20 Hz buckets
//! so the HUD can draw a moving history.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use rodio::{ChannelCount, SampleRate, Source, source::SeekError};

/// Buckets kept in the ring (20 Hz × 2.4 s).
pub const WAVE_CAPACITY: usize = 48;

pub struct Waveform {
    capacity: usize,
    levels: Mutex<VecDeque<f32>>,
}

impl Waveform {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            levels: Mutex::new(VecDeque::with_capacity(capacity)),
        }
    }

    pub fn push(&self, level: f32) {
        let Ok(mut levels) = self.levels.lock() else {
            return;
        };
        if levels.len() == self.capacity {
            levels.pop_front();
        }
        levels.push_back(level.clamp(0.0, 1.0));
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<f32> {
        self.levels
            .lock()
            .map_or_else(|_| Vec::new(), |levels| levels.iter().copied().collect())
    }

    pub fn reset(&self) {
        if let Ok(mut levels) = self.levels.lock() {
            levels.clear();
        }
    }
}

/// Wraps a source, measuring RMS per ~50 ms bucket without touching the samples.
pub struct Metered<S> {
    inner: S,
    wave: Arc<Waveform>,
    accumulator: f32,
    samples: usize,
    bucket: usize,
}

impl<S> Metered<S>
where
    S: Source,
{
    #[must_use]
    pub fn new(inner: S, wave: Arc<Waveform>) -> Self {
        let frames = inner.sample_rate().get() as usize * inner.channels().get() as usize;
        Self {
            inner,
            wave,
            accumulator: 0.0,
            samples: 0,
            bucket: (frames / 20).max(1),
        }
    }
}

impl<S> Iterator for Metered<S>
where
    S: Source,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        self.accumulator += sample * sample;
        self.samples += 1;
        if self.samples >= self.bucket {
            let rms = (self.accumulator / self.samples as f32).sqrt();
            // Music RMS is typically 0.05–0.3; scale it into a visible meter.
            self.wave.push((rms * 3.0).min(1.0));
            self.accumulator = 0.0;
            self.samples = 0;
        }
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S> Source for Metered<S>
where
    S: Source,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(position)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Metered, WAVE_CAPACITY, Waveform};

    #[test]
    fn ring_keeps_the_most_recent_values() {
        let wave = Waveform::new(3);
        for level in [0.1, 0.2, 0.3, 0.4] {
            wave.push(level);
        }
        assert_eq!(wave.snapshot(), vec![0.2, 0.3, 0.4]);
        wave.reset();
        assert!(wave.snapshot().is_empty());
    }

    #[test]
    fn levels_are_clamped() {
        let wave = Waveform::new(WAVE_CAPACITY);
        wave.push(2.0);
        wave.push(-1.0);
        assert_eq!(wave.snapshot(), vec![1.0, 0.0]);
    }

    #[test]
    fn metered_source_emits_levels_and_passes_samples_through() {
        let wave = Arc::new(Waveform::new(WAVE_CAPACITY));
        let source = rodio::source::SineWave::new(440.0);
        let mut metered = Metered::new(source, Arc::clone(&wave));
        let samples: Vec<f32> = (&mut metered).take(48_000).collect();
        assert_eq!(samples.len(), 48_000);
        let levels = wave.snapshot();
        assert!(!levels.is_empty());
        assert!(levels.iter().all(|level| (0.0..=1.0).contains(level)));
    }
}
