#![forbid(unsafe_code)]
//! Audio output over rodio.
//!
//! # Why a dedicated thread
//!
//! `cpal` streams are tied to the thread that created them on several
//! platforms, and rodio's device handle is not guaranteed to be `Send`. The
//! handle returned to the application is therefore a thin, `Send + Sync`
//! facade that forwards commands to a single owning thread. That thread also
//! polls the sink, which is how `is_finished` is derived without callbacks
//! mutating application state.

mod engine;
mod prefetch;
mod spool;
mod wave;

pub use engine::{DeviceInfo, MAX_MEMORY_BYTES, RodioAudioEngine, devices};
pub use prefetch::PrefetchSource;
pub use spool::{GrowingFile, Spool};
pub use wave::{SPECTRUM_BANDS, Waveform};
