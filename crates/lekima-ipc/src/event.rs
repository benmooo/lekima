//! Sequenced push events.
//!
//! The daemon is the only publisher. Frames carry a monotonic `seq` so clients
//! can detect gaps and resync, and a `type` tag so they can filter without
//! understanding the whole schema.

use lekima_domain::{PlaybackState, Track, TrackId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Status,
    Queue,
    Like,
    Resync,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Status {
        state: PlaybackState,
    },
    Queue {
        tracks: Vec<Track>,
        index: Option<usize>,
    },
    Like {
        id: TrackId,
        liked: bool,
    },
    /// The subscriber lagged; fetch `Status`/`Queue` to resync.
    Resync,
}

impl Event {
    #[must_use]
    pub const fn kind(&self) -> EventKind {
        match self {
            Self::Status { .. } => EventKind::Status,
            Self::Queue { .. } => EventKind::Queue,
            Self::Like { .. } => EventKind::Like,
            Self::Resync => EventKind::Resync,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFrame {
    pub seq: u64,
    #[serde(flatten)]
    pub event: Event,
}

#[cfg(test)]
mod tests {
    use super::{Event, EventFrame, EventKind};

    #[test]
    fn frames_are_flat_json_objects() {
        let frame = EventFrame {
            seq: 7,
            event: Event::Like {
                id: 3.into(),
                liked: true,
            },
        };
        let encoded = serde_json::to_string(&frame).unwrap();
        assert!(encoded.contains("\"seq\":7"));
        assert!(encoded.contains("\"type\":\"like\""));
        assert_eq!(frame.event.kind(), EventKind::Like);
    }
}
