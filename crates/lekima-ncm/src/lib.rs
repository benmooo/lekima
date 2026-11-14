#![forbid(unsafe_code)]
//! NetEase Cloud Music adapter.
//!
//! This crate is the only place that knows about `ncmapi`. It translates
//! provider payloads into the domain vocabulary and provider failures into
//! [`lekima_domain::DomainError`]. Pure mapping functions live in [`map`] so
//! they can be tested against fixtures without a network.

mod map;
mod service;

pub use service::{NcmMusicService, NcmOptions};
