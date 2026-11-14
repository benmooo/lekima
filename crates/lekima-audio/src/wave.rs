//! Real spectrum analysis from the decoded stream.
//!
//! A mono mix of the samples is windowed (Hann) and transformed with a 1024
//! point FFT ~43 times per second. Magnitudes are folded into 20 log-spaced
//! bands and smoothed with a fast attack / slow decay, so the HUD shows the
//! actual spectral shape of the music rather than a decorative animation.

use std::{
    f32::consts::PI,
    sync::{Arc, Mutex},
    time::Duration,
};

use rodio::{ChannelCount, SampleRate, Source, source::SeekError};
use rustfft::{Fft, FftPlanner, num_complex::Complex};

/// Number of rendered spectrum bands.
pub const SPECTRUM_BANDS: usize = 20;
const FFT_SIZE: usize = 1024;
const LOW_HZ: f32 = 60.0;
const HIGH_HZ: f32 = 16_000.0;

pub struct Waveform {
    levels: Mutex<[f32; SPECTRUM_BANDS]>,
}

impl Default for Waveform {
    fn default() -> Self {
        Self::new()
    }
}

impl Waveform {
    #[must_use]
    pub fn new() -> Self {
        Self {
            levels: Mutex::new([0.0; SPECTRUM_BANDS]),
        }
    }

    fn update(&self, targets: &[f32; SPECTRUM_BANDS]) {
        let Ok(mut levels) = self.levels.lock() else {
            return;
        };
        for (level, target) in levels.iter_mut().zip(targets.iter().copied()) {
            let factor = if target > *level { 0.55 } else { 0.12 };
            *level += (target - *level) * factor;
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<f32> {
        self.levels
            .lock()
            .map_or_else(|_| Vec::new(), |levels| levels.to_vec())
    }

    pub fn reset(&self) {
        if let Ok(mut levels) = self.levels.lock() {
            *levels = [0.0; SPECTRUM_BANDS];
        }
    }
}

/// Wraps a source, computing an FFT spectrum without altering the samples.
pub struct Metered<S> {
    inner: S,
    wave: Arc<Waveform>,
    channels: usize,
    frame_sum: f32,
    frame_samples: usize,
    window: Vec<f32>,
    buffer: Vec<Complex<f32>>,
    fft: Arc<dyn Fft<f32>>,
    bands: Vec<(usize, usize)>,
    hud: Vec<f32>,
}

impl<S> Metered<S>
where
    S: Source,
{
    #[must_use]
    pub fn new(inner: S, wave: Arc<Waveform>) -> Self {
        let channels = usize::from(inner.channels().get());
        let sample_rate = inner.sample_rate().get() as f32;
        let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);

        // Log-spaced bin ranges, clamped to the FFT's positive half.
        let bands = (0..SPECTRUM_BANDS)
            .map(|band| {
                let low = LOW_HZ * (HIGH_HZ / LOW_HZ).powf(band as f32 / SPECTRUM_BANDS as f32);
                let high =
                    LOW_HZ * (HIGH_HZ / LOW_HZ).powf((band + 1) as f32 / SPECTRUM_BANDS as f32);
                let to_bin = |hz: f32| {
                    ((hz * FFT_SIZE as f32 / sample_rate).round() as usize).clamp(1, FFT_SIZE / 2)
                };
                let start = to_bin(low);
                let end = to_bin(high).max(start + 1).min(FFT_SIZE / 2);
                (start, end)
            })
            .collect();

        Self {
            inner,
            wave,
            channels: channels.max(1),
            frame_sum: 0.0,
            frame_samples: 0,
            window: Vec::with_capacity(FFT_SIZE),
            buffer: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            fft,
            bands,
            hud: vec![0.0; SPECTRUM_BANDS],
        }
    }

    fn analyze(&mut self) {
        for (index, sample) in self.window.iter().enumerate() {
            let hann = 0.5 - 0.5 * (2.0 * PI * index as f32 / (FFT_SIZE - 1) as f32).cos();
            self.buffer[index] = Complex::new(sample * hann, 0.0);
        }
        self.fft.process(&mut self.buffer);

        let scale = FFT_SIZE as f32 * 0.25;
        for (band, (start, end)) in self.bands.iter().copied().enumerate() {
            let peak = (start..end)
                .map(|bin| self.buffer[bin].norm())
                .fold(0.0_f32, f32::max);
            self.hud[band] = (peak / scale).sqrt().min(1.0);
        }
        self.wave.update(&self.hud_array());
        self.window.clear();
    }

    fn hud_array(&self) -> [f32; SPECTRUM_BANDS] {
        let mut array = [0.0; SPECTRUM_BANDS];
        for (slot, value) in array.iter_mut().zip(self.hud.iter()) {
            *slot = *value;
        }
        array
    }
}

impl<S> Iterator for Metered<S>
where
    S: Source,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        self.frame_sum += sample;
        self.frame_samples += 1;
        if self.frame_samples >= self.channels {
            let mono = self.frame_sum / self.channels as f32;
            self.frame_sum = 0.0;
            self.frame_samples = 0;
            self.window.push(mono);
            if self.window.len() >= FFT_SIZE {
                self.analyze();
            }
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
        self.window.clear();
        self.inner.try_seek(position)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Metered, SPECTRUM_BANDS, Waveform};

    #[test]
    fn a_sine_wave_produces_a_non_empty_spectrum() {
        let wave = Arc::new(Waveform::new());
        let source = rodio::source::SineWave::new(440.0);
        let mut metered = Metered::new(source, Arc::clone(&wave));
        let samples: Vec<f32> = (&mut metered).take(44_100).collect();
        assert_eq!(samples.len(), 44_100);
        let levels = wave.snapshot();
        assert_eq!(levels.len(), SPECTRUM_BANDS);
        assert!(levels.iter().any(|level| *level > 0.1), "{levels:?}");
        assert!(levels.iter().all(|level| (0.0..=1.0).contains(level)));
    }

    #[test]
    fn silence_stays_flat() {
        let wave = Arc::new(Waveform::new());
        let source = rodio::buffer::SamplesBuffer::new(
            std::num::NonZero::new(2).unwrap(),
            std::num::NonZero::new(44_100).unwrap(),
            vec![0.0_f32; 44_100 * 2],
        );
        let mut metered = Metered::new(source, Arc::clone(&wave));
        (&mut metered).take(44_100).for_each(drop);
        let levels = wave.snapshot();
        assert!(levels.iter().all(|level| *level < 0.01), "{levels:?}");
    }

    #[test]
    fn reset_clears_all_bands() {
        let wave = Waveform::new();
        wave.update(&[1.0; SPECTRUM_BANDS]);
        wave.reset();
        assert!(wave.snapshot().iter().all(|level| *level == 0.0));
    }
}
