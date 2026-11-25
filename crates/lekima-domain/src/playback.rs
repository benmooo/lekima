//! Queue rules and observable playback state.
//!
//! Queue semantics live here rather than in the daemon so they are testable
//! without a runtime, a socket or an audio device.

use serde::{Deserialize, Serialize};

use crate::model::{Quality, Track};

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

/// Where the current queue came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueKind {
    #[default]
    Manual,
    Daily,
    Fm,
    Toplist,
    Cloud,
    Liked,
    Playlist,
    Album,
}

impl QueueKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Daily => "daily",
            Self::Fm => "fm",
            Self::Toplist => "toplist",
            Self::Cloud => "cloud",
            Self::Liked => "liked",
            Self::Playlist => "playlist",
            Self::Album => "album",
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

/// A byte range of the current track that is already available locally,
/// expressed in milliseconds from the start of the track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferedRange {
    pub start_ms: u64,
    pub end_ms: u64,
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
    #[serde(default)]
    pub queue_kind: QueueKind,
    /// Whether the current track is in the account's liked set.
    #[serde(default)]
    pub liked: bool,
    /// True between starting a load and the first sample reaching the device.
    #[serde(default)]
    pub loading: bool,
    /// True while a streaming track is waiting for more network data.
    #[serde(default)]
    pub buffering: bool,
    /// True when the current audio came from the persistent stream cache.
    #[serde(default)]
    pub cached: bool,
    #[serde(default)]
    pub quality: Quality,
    /// Active sleep timer, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sleep_remaining_ms: Option<u64>,
    /// Recent amplitude envelope from the audio device (oldest first, 0..=1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wave: Vec<f32>,
    /// Actual decoded stream duration when the engine knows it. Shorter than
    /// the metadata duration for a provider preview, and the scale frontends
    /// should use for progress shading and labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_duration_ms: Option<u64>,
    /// Locally buffered ranges of the current track, for progress shading.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buffered: Vec<BufferedRange>,
    /// Monotonic daemon revision; clients use it to ignore stale status polls.
    #[serde(default)]
    pub revision: u64,
    /// Bumped whenever a new stream is loaded for the current track, including
    /// repeat-one loops and replaying the same track. Clients that need to tell
    /// two plays of one track apart key on `(track, generation)`.
    #[serde(default)]
    pub generation: u64,
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
            queue_kind: QueueKind::Manual,
            liked: false,
            loading: false,
            buffering: false,
            cached: false,
            quality: Quality::Exhigh,
            sleep_remaining_ms: None,
            wave: Vec::new(),
            stream_duration_ms: None,
            buffered: Vec::new(),
            revision: 0,
            generation: 0,
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
    /// `Off` stops at the end by returning `None`, leaving the cursor on the
    /// final track so the UI can still show what stopped.
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
        if let Some(next) = next {
            self.current = Some(next);
            self.current()
        } else {
            None
        }
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

    /// Removes a queued track by index, keeping the cursor on the same
    /// position (or the new tail).
    pub fn remove_at(&mut self, index: usize) -> Option<Track> {
        if index >= self.tracks.len() {
            return None;
        }
        let removed = self.tracks.remove(index);
        self.current = match self.current {
            None => None,
            Some(_) if self.tracks.is_empty() => None,
            Some(current) => Some(match index.cmp(&current) {
                std::cmp::Ordering::Less => current - 1,
                std::cmp::Ordering::Equal => index.min(self.tracks.len() - 1),
                std::cmp::Ordering::Greater => current,
            }),
        };
        Some(removed)
    }

    /// Pops the current track, keeping the cursor on the same position.
    pub fn remove_current(&mut self) -> Option<Track> {
        let index = self.current?;
        self.remove_at(index)
    }

    /// Moves a track to a new index, preserving which track is current.
    ///
    /// Returns `false` for out-of-range indices or a no-op move.
    pub fn move_item(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tracks.len() || to >= self.tracks.len() || from == to {
            return false;
        }
        let track = self.tracks.remove(from);
        self.tracks.insert(to, track);
        if let Some(current) = self.current {
            if from == current {
                self.current = Some(to);
                return true;
            }
            // Removing before the cursor shifts it left; inserting at or before
            // the (post-removal) cursor shifts it right.
            let mut cursor = current;
            if from < cursor {
                cursor -= 1;
            }
            if to <= cursor {
                cursor += 1;
            }
            self.current = Some(cursor.min(self.tracks.len() - 1));
        }
        true
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
        assert_eq!(queue.current().unwrap().id.get(), 2);
        assert_eq!(queue.current_index(), Some(1));
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
    fn remove_at_keeps_the_cursor_on_the_same_track() {
        let mut queue = Queue::new(vec![track(1), track(2), track(3)]);
        queue.jump(1);
        assert_eq!(queue.remove_at(0).unwrap().id.get(), 1);
        assert_eq!(queue.current().unwrap().id.get(), 2);
        assert_eq!(queue.current_index(), Some(0));

        assert_eq!(queue.remove_at(1).unwrap().id.get(), 3);
        assert_eq!(queue.current().unwrap().id.get(), 2);

        queue.remove_at(0);
        assert!(queue.is_empty());
        assert_eq!(queue.current_index(), None);
    }

    #[test]
    fn moving_tracks_preserves_the_current_identity() {
        let mut queue = Queue::new(vec![track(1), track(2), track(3)]);
        queue.jump(1); // current 2
        assert!(queue.move_item(1, 2));
        assert_eq!(queue.tracks()[2].id.get(), 2);
        assert_eq!(queue.current().unwrap().id.get(), 2);

        // Move another item across the current one: cursor shifts, identity stays.
        assert!(queue.move_item(0, 2));
        assert_eq!(queue.current().unwrap().id.get(), 2);
        assert_eq!(queue.current_index(), Some(1));

        // Moving the tail above the current one also shifts the cursor.
        assert!(queue.move_item(2, 0));
        assert_eq!(queue.current().unwrap().id.get(), 2);

        assert!(!queue.move_item(0, 0));
        assert!(!queue.move_item(0, 9));
    }

    #[test]
    fn removing_the_last_track_clears_the_cursor() {
        let mut queue = Queue::new(vec![track(1)]);
        assert!(queue.remove_current().is_some());
        assert!(queue.is_empty());
        assert_eq!(queue.current_index(), None);
    }
}
