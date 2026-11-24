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

impl EventKind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 4] = [Self::Status, Self::Queue, Self::Like, Self::Resync];

    /// Wire name, also the name used in a hook's `events` list.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Queue => "queue",
            Self::Like => "like",
            Self::Resync => "resync",
        }
    }

    /// Parses a wire name, returning `None` for an unknown kind.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Status {
        state: Box<PlaybackState>,
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
    fn kind_names_round_trip() {
        for kind in EventKind::ALL {
            assert_eq!(EventKind::from_name(kind.as_str()), Some(kind));
        }
        assert_eq!(EventKind::from_name("track_change"), None);
    }

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
