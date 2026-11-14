//! Queue rules and observable playback state.
//!
//! Queue semantics live here rather than in the daemon so they are testable
//! without a runtime, a socket or an audio device.

use serde::{Deserialize, Serialize};

use crate::model::Track;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackStatus {
    #[default]
    Stopped,
    Playing,
    Paused,
}

impl PlaybackStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Playing => "playing",
            Self::Paused => "paused",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

impl RepeatMode {
    /// Cycles off -> all -> one -> off, matching the `r` key in the TUI.
    #[must_use]
    pub const fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::All => "all",
            Self::One => "one",
        }
    }
}

/// Everything a client needs to render a "now playing" view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackState {
    pub status: PlaybackStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<Track>,
    #[serde(default)]
    pub position_ms: u64,
    #[serde(default = "default_volume", serialize_with = "round_volume")]
    pub volume: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub repeat: RepeatMode,
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default)]
    pub queue_length: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_index: Option<usize>,
}

const fn default_volume() -> f32 {
    0.7
}

/// Volumes are user-facing percentages; serializing 0.6999999 helps nobody.
fn round_volume<S>(volume: &f32, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let rounded = (f64::from(*volume) * 100.0).round() / 100.0;
    serializer.serialize_f64(rounded)
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            track: None,
            position_ms: 0,
            volume: default_volume(),
            muted: false,
            repeat: RepeatMode::Off,
            shuffle: false,
            queue_length: 0,
            queue_index: None,
        }
    }
}

impl PlaybackState {
    /// Effective gain: muted playback is silent regardless of the stored level.
    #[must_use]
    pub fn effective_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume.clamp(0.0, 1.0)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Queue {
    tracks: Vec<Track>,
    current: Option<usize>,
}

impl Queue {
    #[must_use]
    pub fn new(tracks: Vec<Track>) -> Self {
        let current = if tracks.is_empty() { None } else { Some(0) };
        Self { tracks, current }
    }

    #[must_use]
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    #[must_use]
    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    #[must_use]
    pub fn current(&self) -> Option<&Track> {
        self.current.and_then(|index| self.tracks.get(index))
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Track> {
        self.tracks.get(index)
    }

    /// Appends a track. Returns its index.
    pub fn push(&mut self, track: Track) -> usize {
        self.tracks.push(track);
        if self.current.is_none() {
            self.current = Some(0);
        }
        self.tracks.len() - 1
    }

    pub fn extend(&mut self, tracks: impl IntoIterator<Item = Track>) {
        for track in tracks {
            self.push(track);
        }
    }

    /// Inserts after the currently playing track ("play next").
    pub fn insert_next(&mut self, track: Track) {
        match self.current {
            Some(index) => {
                self.tracks.insert(index + 1, track);
            }
            None => {
                self.push(track);
            }
        }
    }

    /// Replaces the whole queue and selects the first track.
    pub fn replace(&mut self, tracks: Vec<Track>) {
        self.tracks = tracks;
        self.current = if self.tracks.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current = None;
    }

    /// Selects an explicit index.
    pub fn jump(&mut self, index: usize) -> Option<&Track> {
        if index < self.tracks.len() {
            self.current = Some(index);
            self.tracks.get(index)
        } else {
            None
        }
    }

    /// Advances to the next track according to the repeat mode.
    ///
    /// `RepeatMode::One` replays the current track; `All` wraps around;
    /// `Off` stops at the end by returning `None`.
    pub fn advance(&mut self, repeat: RepeatMode) -> Option<&Track> {
        let current = self.current?;
        let next = match repeat {
            RepeatMode::One => Some(current),
            RepeatMode::All => Some((current + 1) % self.tracks.len()),
            RepeatMode::Off => {
                if current + 1 < self.tracks.len() {
                    Some(current + 1)
                } else {
                    None
                }
            }
        };
        self.current = next;
        self.current()
    }

    /// Moves backwards, wrapping only when repeating everything.
    pub fn rewind(&mut self, repeat: RepeatMode) -> Option<&Track> {
        let current = self.current?;
        let previous = if current > 0 {
            Some(current - 1)
        } else if repeat == RepeatMode::All {
            Some(self.tracks.len().saturating_sub(1))
        } else {
            None
        };
        self.current = previous;
        self.current()
    }

    /// Pops the current track, keeping the cursor on the same position.
    pub fn remove_current(&mut self) -> Option<Track> {
        let index = self.current?;
        let removed = self.tracks.remove(index);
        self.current = if self.tracks.is_empty() {
            None
        } else {
            Some(index.min(self.tracks.len() - 1))
        };
        Some(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::{Queue, RepeatMode};
    use crate::model::Track;

    fn track(id: u64) -> Track {
        Track {
            id: id.into(),
            name: format!("track-{id}"),
            artists: Vec::new(),
            album: None,
            duration_ms: 0,
        }
    }

    #[test]
    fn advance_stops_at_the_end_when_repeat_is_off() {
        let mut queue = Queue::new(vec![track(1), track(2)]);
        assert_eq!(queue.current().unwrap().id.get(), 1);
        queue.advance(RepeatMode::Off);
        assert_eq!(queue.current().unwrap().id.get(), 2);
        assert!(queue.advance(RepeatMode::Off).is_none());
    }

    #[test]
    fn advance_wraps_when_repeating_everything() {
        let mut queue = Queue::new(vec![track(1), track(2)]);
        queue.jump(1);
        assert_eq!(queue.advance(RepeatMode::All).unwrap().id.get(), 1);
    }

    #[test]
    fn repeat_one_keeps_the_current_track() {
        let mut queue = Queue::new(vec![track(1), track(2)]);
        assert_eq!(queue.advance(RepeatMode::One).unwrap().id.get(), 1);
    }

    #[test]
    fn insert_next_places_track_after_current() {
        let mut queue = Queue::new(vec![track(1), track(3)]);
        queue.insert_next(track(2));
        assert_eq!(queue.tracks()[1].id.get(), 2);
        assert_eq!(queue.tracks()[2].id.get(), 3);
    }

    #[test]
    fn removing_the_last_track_clears_the_cursor() {
        let mut queue = Queue::new(vec![track(1)]);
        assert!(queue.remove_current().is_some());
        assert!(queue.is_empty());
        assert_eq!(queue.current_index(), None);
    }
}
