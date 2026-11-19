//! Listening history.
//!
//! The daemon feeds playback snapshots into a [`HistoryLog`]; the log is the
//! only place that decides what counts as a play, so the rule is testable
//! without a player, a clock or a filesystem.
//!
//! A record is appended when a track stops being current (another track starts,
//! the queue is cleared, or the daemon shuts down). The track that is playing
//! right now is therefore *not* in the log yet — `status` already answers that
//! question, and a half-played entry is worse than none.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::model::Track;
use crate::playback::{PlaybackState, PlaybackStatus, QueueKind};

/// How much of a track must be heard before it counts as completed.
pub const COMPLETION_PERCENT: u64 = 95;

/// One finished play.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayRecord {
    pub track: Track,
    /// Unix milliseconds when the track became current.
    pub started_at: i64,
    /// Milliseconds of audio actually played.
    pub played_ms: u64,
    /// Whether at least [`COMPLETION_PERCENT`] of the track was heard.
    pub completed: bool,
    /// Where the queue came from, so history can be filtered later.
    #[serde(default)]
    pub kind: QueueKind,
}

impl PlayRecord {
    /// Renders the played duration as `m:ss`, or `--:--` when unknown.
    #[must_use]
    pub fn played_label(&self) -> String {
        crate::model::format_duration_ms(self.played_ms)
    }
}

/// True when `played_ms` covers enough of `duration_ms` to count as completed.
///
/// A track with no known duration can never be completed: guessing would make
/// every preview look finished.
#[must_use]
pub fn is_completed(played_ms: u64, duration_ms: u64) -> bool {
    duration_ms > 0
        && played_ms.saturating_mul(100) >= duration_ms.saturating_mul(COMPLETION_PERCENT)
}

/// The track currently accumulating play time.
#[derive(Debug, Clone)]
struct Active {
    track: Track,
    started_at: i64,
    played_ms: u64,
    last_position_ms: u64,
    kind: QueueKind,
}

/// A bounded, newest-last list of finished plays.
#[derive(Debug, Clone)]
pub struct HistoryLog {
    entries: VecDeque<PlayRecord>,
    active: Option<Active>,
    capacity: usize,
}

impl HistoryLog {
    /// Default number of records kept on disk. About a month of heavy listening.
    pub const DEFAULT_CAPACITY: usize = 1_000;

    /// Creates an empty log that keeps at most `capacity` records.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            active: None,
            // A zero capacity would silently disable the feature.
            capacity: capacity.max(1),
        }
    }

    /// Rebuilds a log from stored records, oldest first, dropping the oldest
    /// when they exceed `capacity`.
    #[must_use]
    pub fn from_records(records: impl IntoIterator<Item = PlayRecord>, capacity: usize) -> Self {
        let mut log = Self::new(capacity);
        for record in records {
            log.push(record);
        }
        log
    }

    /// Finished plays, oldest first.
    #[must_use]
    pub fn records(&self) -> &VecDeque<PlayRecord> {
        &self.entries
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The newest `limit` finished plays, newest first.
    #[must_use]
    pub fn recent(&self, limit: usize) -> Vec<PlayRecord> {
        self.entries.iter().rev().take(limit).cloned().collect()
    }

    /// Feeds one snapshot into the log.
    ///
    /// Returns `true` when a record was appended (or the active play changed),
    /// which is the only thing worth persisting. Position updates mutate the
    /// active play without asking for a write.
    ///
    /// Play time accumulates from position deltas, so a seek forward is counted
    /// as listened and a seek backwards is not counted twice.
    pub fn observe(&mut self, state: &PlaybackState, now_ms: i64) -> bool {
        let Some(track) = state.track.as_ref() else {
            return self.close_active();
        };

        let same_track = self
            .active
            .as_ref()
            .is_some_and(|active| active.track.id == track.id);
        if !same_track {
            self.close_active();
            self.active = Some(Active {
                track: track.clone(),
                started_at: now_ms,
                played_ms: 0,
                last_position_ms: state.position_ms,
                kind: state.queue_kind,
            });
            // A new active play is state worth persisting, even though no
            // record was appended yet.
            return true;
        }

        let Some(active) = self.active.as_mut() else {
            return false;
        };
        if state.status == PlaybackStatus::Playing {
            active.played_ms = active
                .played_ms
                .saturating_add(state.position_ms.saturating_sub(active.last_position_ms));
        }
        active.last_position_ms = state.position_ms;
        false
    }

    /// Appends the active play, if any.
    fn close_active(&mut self) -> bool {
        let Some(active) = self.active.take() else {
            return false;
        };
        self.push(PlayRecord {
            completed: is_completed(active.played_ms, active.track.duration_ms),
            track: active.track,
            started_at: active.started_at,
            played_ms: active.played_ms,
            kind: active.kind,
        });
        true
    }

    /// Ends the active play and returns whether anything was appended. The
    /// daemon calls this on shutdown so the last track is not lost.
    pub fn finish(&mut self) -> bool {
        self.close_active()
    }

    fn push(&mut self, record: PlayRecord) {
        self.entries.push_back(record);
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{Quality, Track, TrackId};
    use crate::playback::{PlaybackState, PlaybackStatus};

    use super::{COMPLETION_PERCENT, HistoryLog, PlayRecord, is_completed};

    fn track(id: u64, duration_ms: u64) -> Track {
        Track {
            id: TrackId::from(id),
            name: format!("Track {id}"),
            artists: Vec::new(),
            album: None,
            duration_ms,
        }
    }

    fn state(id: u64, duration_ms: u64, position_ms: u64, playing: bool) -> PlaybackState {
        PlaybackState {
            status: if playing {
                PlaybackStatus::Playing
            } else {
                PlaybackStatus::Paused
            },
            track: Some(track(id, duration_ms)),
            position_ms,
            volume: 0.7,
            muted: false,
            repeat: crate::playback::RepeatMode::Off,
            shuffle: false,
            queue_length: 1,
            queue_index: Some(0),
            queue_kind: crate::playback::QueueKind::Manual,
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
        }
    }

    #[test]
    fn completion_needs_a_known_duration_and_enough_play_time() {
        assert!(!is_completed(0, 0));
        assert!(!is_completed(100_000, 0));
        assert!(!is_completed(10_000, 100_000));
        assert!(!is_completed(94_000, 100_000));
        assert!(is_completed(95_000, 100_000));
        assert!(is_completed(200_000, 100_000));
        assert_eq!(COMPLETION_PERCENT, 95);
    }

    #[test]
    fn a_track_is_recorded_when_the_next_one_starts() {
        let mut log = HistoryLog::new(10);

        assert!(log.observe(&state(1, 100_000, 0, true), 1_000));
        assert_eq!(log.len(), 0, "the current track is not finished yet");

        assert!(
            !log.observe(&state(1, 100_000, 30_000, true), 2_000),
            "a position update is not worth persisting"
        );
        assert_eq!(log.len(), 0);

        assert!(log.observe(&state(2, 100_000, 0, true), 3_000));
        assert_eq!(log.len(), 1);
        let record = &log.records()[0];
        assert_eq!(record.track.id, TrackId::from(1_u64));
        assert_eq!(record.started_at, 1_000);
        assert_eq!(record.played_ms, 30_000);
        assert!(!record.completed);
    }

    #[test]
    fn play_time_accumulates_only_while_playing() {
        let mut log = HistoryLog::new(10);
        log.observe(&state(1, 100_000, 0, true), 0);
        log.observe(&state(1, 100_000, 10_000, true), 10_000);
        log.observe(&state(1, 100_000, 10_000, false), 11_000);
        log.observe(&state(1, 100_000, 10_000, false), 20_000);
        log.observe(&state(2, 100_000, 0, true), 30_000);

        assert_eq!(log.records()[0].played_ms, 10_000);
    }

    #[test]
    fn a_seek_backwards_does_not_double_count() {
        let mut log = HistoryLog::new(10);
        log.observe(&state(1, 100_000, 0, true), 0);
        log.observe(&state(1, 100_000, 20_000, true), 1_000);
        log.observe(&state(1, 100_000, 5_000, true), 2_000);
        log.observe(&state(1, 100_000, 15_000, true), 3_000);
        log.observe(&state(2, 0, 0, true), 4_000);

        assert_eq!(log.records()[0].played_ms, 30_000);
    }

    #[test]
    fn a_track_played_to_the_end_is_completed() {
        let mut log = HistoryLog::new(10);
        log.observe(&state(1, 100_000, 0, true), 0);
        log.observe(&state(1, 100_000, 99_000, true), 99_000);
        log.observe(&state(2, 0, 0, true), 100_000);

        assert!(log.records()[0].completed);
    }

    #[test]
    fn clearing_the_queue_finishes_the_current_track() {
        let mut log = HistoryLog::new(10);
        log.observe(&state(1, 100_000, 0, true), 0);
        log.observe(&state(1, 100_000, 5_000, true), 1_000);

        let mut cleared = state(1, 100_000, 0, true);
        cleared.track = None;
        cleared.queue_length = 0;
        cleared.queue_index = None;

        assert!(log.observe(&cleared, 6_000));
        assert_eq!(log.len(), 1);
        assert_eq!(log.records()[0].played_ms, 5_000);
        assert!(
            !log.observe(&cleared, 7_000),
            "closing twice appends nothing"
        );
    }

    #[test]
    fn finish_records_the_playing_track_once() {
        let mut log = HistoryLog::new(10);
        log.observe(&state(1, 100_000, 4_000, true), 0);

        assert!(log.finish());
        assert_eq!(log.len(), 1);
        assert!(!log.finish());
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn capacity_drops_the_oldest_records() {
        let mut log = HistoryLog::new(2);
        for id in 1..=4 {
            log.observe(&state(id, 1_000, 0, true), id as i64);
        }
        log.finish();

        let ids: Vec<u64> = log.records().iter().map(|r| r.track.id.get()).collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids, vec![3, 4]);
    }

    #[test]
    fn a_zero_capacity_still_keeps_one() {
        assert_eq!(HistoryLog::new(0).capacity(), 1);
    }

    #[test]
    fn recent_returns_newest_first() {
        let mut log = HistoryLog::new(10);
        for id in 1..=3 {
            log.observe(&state(id, 1_000, 0, true), id as i64);
        }
        log.finish();

        let ids: Vec<u64> = log.recent(2).iter().map(|r| r.track.id.get()).collect();
        assert_eq!(ids, vec![3, 2]);
    }

    #[test]
    fn from_records_keeps_the_newest() {
        let records = (1..=3).map(|id| PlayRecord {
            track: track(id, 1_000),
            started_at: 0,
            played_ms: 0,
            completed: false,
            kind: crate::playback::QueueKind::Manual,
        });
        let log = HistoryLog::from_records(records, 2);

        let ids: Vec<u64> = log.records().iter().map(|r| r.track.id.get()).collect();
        assert_eq!(ids, vec![2, 3]);
    }
}
