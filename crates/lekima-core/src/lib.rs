#![forbid(unsafe_code)]
//! Application layer.
//!
//! [`Player`] is the only writer of playback state. It coordinates the two
//! ports it is given — a [`MusicService`] and an [`AudioEngine`] — and exposes
//! intents (`play`, `next`, `seek`, …) rather than raw state mutations. Every
//! frontend, including the TUI and the CLI, talks to this layer.

mod player;

pub use player::{PlayRequest, Player};
