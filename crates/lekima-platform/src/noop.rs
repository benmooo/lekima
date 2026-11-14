//! No-op backend used where the OS has no media integration, or when the D-Bus
//! service cannot be started.

use crate::{MediaCommand, MediaControls, NowPlaying};

/// Discards updates and never produces commands.
#[derive(Debug, Default)]
pub struct NoopControls;

impl MediaControls for NoopControls {
    fn update(&self, _state: &NowPlaying) {}

    fn poll(&self) -> Vec<MediaCommand> {
        Vec::new()
    }

    fn backend(&self) -> &'static str {
        "none"
    }
}
