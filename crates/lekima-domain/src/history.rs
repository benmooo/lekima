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

use crate::model::{Track, TrackId};
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
    /// `(track, generation)` — a new stream for the same track is a new play,
    /// which is how repeat-one and replay stay separate records.
    key: (TrackId, u64),
    track: Track,
    started_at: i64,
    played_ms: u64,
    last_position_ms: u64,
    /// Metadata duration, replaced by the decoded stream duration for previews.
    duration_ms: u64,
    kind: QueueKind,
}

/// A bounded, newest-last list of finished plays.
#[derive(Debug, Clone)]
pub struct HistoryLog {
    entries: VecDeque<PlayRecord>,
    active: Option<Active>,
    capacity: usize,
    /// Counts appended records. Persisters use it to avoid writing a stale
    /// snapshot over a newer one.
    revision: u64,
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
            revision: 0,
        }
    }

    /// Rebuilds a log from stored records, oldest first, dropping the oldest
    /// when they exceed `capacity`.
    ///
    /// Records with no played time are dropped: older builds could persist a
    /// play that never produced audio.
    #[must_use]
    pub fn from_records(records: impl IntoIterator<Item = PlayRecord>, capacity: usize) -> Self {
        let mut log = Self::new(capacity);
        for record in records {
            // Older builds could persist a play that never produced audio.
            if record.played_ms > 0 {
                log.push(record);
            }
        }
        log
    }

    /// Monotonic count of appended records.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
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
    /// Returns `true` only when a finished play was appended — the one change
    /// worth persisting. Starting or extending a play never asks for a write, so
    /// a restored queue cannot rewrite the file and a long track does not
    /// rewrite it every tick.
    ///
    /// A play starts only when audio is actually running. A restored queue, a
    /// queue edit and a paused track are current without being plays, so they
    /// never produce a record.
    ///
    /// Play time accumulates from position deltas, so a seek forward is counted
    /// as listened and a seek backwards is not counted twice.
    pub fn observe(&mut self, state: &PlaybackState, now_ms: i64) -> bool {
        let Some(track) = state.track.as_ref() else {
            return self.close_active();
        };

        let key = (track.id, state.generation);
        if self.active.as_ref().is_some_and(|active| active.key == key) {
            let Some(active) = self.active.as_mut() else {
                return false;
            };
            // Previews are shorter than their metadata claims; believe the
            // decoded stream once the engine reports it.
            if let Some(stream_ms) = state.stream_duration_ms.filter(|duration| *duration > 0) {
                active.duration_ms = stream_ms;
            }
            if state.status == PlaybackStatus::Playing {
                active.played_ms = active
                    .played_ms
                    .saturating_add(state.position_ms.saturating_sub(active.last_position_ms));
            }
            active.last_position_ms = state.position_ms;
            return false;
        }

        let closed = self.close_active();
        if state.status != PlaybackStatus::Playing {
            return closed;
        }
        self.active = Some(Active {
            key,
            track: track.clone(),
            started_at: now_ms,
            played_ms: 0,
            last_position_ms: state.position_ms,
            duration_ms: state
                .stream_duration_ms
                .filter(|duration| *duration > 0)
                .unwrap_or(track.duration_ms),
            kind: state.queue_kind,
        });
        closed
    }

    /// Appends the active play, if anything was actually heard.
    fn close_active(&mut self) -> bool {
        let Some(active) = self.active.take() else {
            return false;
        };
        // A track that was current but never played (a restored queue, a skip
        // inside the polling interval) is not a play.
        if active.played_ms == 0 {
            return false;
        }
        self.push(PlayRecord {
            completed: is_completed(active.played_ms, active.duration_ms),
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
        self.revision = self.revision.wrapping_add(1);
        self.entries.push_back(record);
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{Track, TrackId};
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

    /// A snapshot of `track(id)` at `position_ms`.
    ///
    /// `status` decides whether this is a play; `generation` separates two plays
    /// of the same track.
    fn state(
        id: u64,
        duration_ms: u64,
        position_ms: u64,
        status: PlaybackStatus,
        generation: u64,
    ) -> PlaybackState {
        PlaybackState {
            status,
            track: Some(track(id, duration_ms)),
            position_ms,
            generation,
            ..PlaybackState::default()
        }
    }

    fn playing(id: u64, duration_ms: u64, position_ms: u64, generation: u64) -> PlaybackState {
        state(
            id,
            duration_ms,
            position_ms,
            PlaybackStatus::Playing,
            generation,
        )
    }

    fn paused(id: u64, duration_ms: u64, position_ms: u64) -> PlaybackState {
        state(id, duration_ms, position_ms, PlaybackStatus::Paused, 0)
    }

    fn stopped(id: u64, duration_ms: u64) -> PlaybackState {
        state(id, duration_ms, 0, PlaybackStatus::Stopped, 0)
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

        assert!(
            !log.observe(&playing(1, 100_000, 0, 0), 1_000),
            "starting a play is not a durable change"
        );
        assert_eq!(log.len(), 0, "the current track is not finished yet");

        assert!(
            !log.observe(&playing(1, 100_000, 30_000, 0), 2_000),
            "a position update is not worth persisting"
        );
        assert_eq!(log.len(), 0);

        assert!(log.observe(&playing(2, 100_000, 0, 1), 3_000));
        assert_eq!(log.len(), 1);
        let record = &log.records()[0];
        assert_eq!(record.track.id, TrackId::from(1_u64));
        assert_eq!(record.started_at, 1_000);
        assert_eq!(record.played_ms, 30_000);
        assert!(!record.completed);
    }

    #[test]
    fn a_restored_queue_is_not_a_play() {
        let mut log = HistoryLog::new(10);

        assert!(!log.observe(&stopped(7, 100_000), 0));
        assert!(!log.finish());
        assert!(log.is_empty(), "a queue that never played has no history");
    }

    #[test]
    fn a_stopped_track_is_not_announced_until_it_plays() {
        let mut log = HistoryLog::new(10);

        assert!(!log.observe(&stopped(1, 100_000), 0));
        assert!(!log.observe(&playing(1, 100_000, 0, 1), 1_000));
        assert!(!log.observe(&playing(1, 100_000, 5_000, 1), 2_000));
        assert!(log.observe(&playing(2, 100_000, 0, 2), 3_000));

        assert_eq!(log.len(), 1);
        assert_eq!(log.records()[0].played_ms, 5_000);
    }

    #[test]
    fn a_paused_track_is_not_a_new_play() {
        let mut log = HistoryLog::new(10);
        log.observe(&playing(1, 100_000, 0, 0), 0);
        log.observe(&playing(1, 100_000, 10_000, 0), 10_000);

        assert!(!log.observe(&paused(1, 100_000, 10_000), 11_000));
        assert!(!log.observe(&paused(1, 100_000, 10_000), 20_000));
        assert!(log.observe(&playing(2, 0, 0, 1), 30_000));

        assert_eq!(log.records()[0].played_ms, 10_000);
    }

    #[test]
    fn play_time_counts_position_deltas_across_seeks() {
        let mut log = HistoryLog::new(10);
        log.observe(&playing(1, 100_000, 0, 0), 0);
        log.observe(&playing(1, 100_000, 20_000, 0), 1_000);
        log.observe(&playing(1, 100_000, 5_000, 0), 2_000);
        log.observe(&playing(1, 100_000, 15_000, 0), 3_000);
        log.observe(&playing(2, 0, 0, 1), 4_000);

        // 0→20 s then 5→15 s: thirty seconds were heard, twenty of them new.
        assert_eq!(log.records()[0].played_ms, 30_000);
    }

    #[test]
    fn repeat_one_records_each_loop() {
        let mut log = HistoryLog::new(10);
        log.observe(&playing(1, 100_000, 0, 1), 0);
        log.observe(&playing(1, 100_000, 100_000, 1), 100_000);

        // A repeat reloads the same id with a new generation.
        assert!(log.observe(&playing(1, 100_000, 0, 2), 101_000));
        assert_eq!(log.len(), 1);
        assert_eq!(log.records()[0].played_ms, 100_000);
        assert!(log.records()[0].completed);

        log.observe(&playing(1, 100_000, 50_000, 2), 151_000);
        assert!(log.finish());
        assert_eq!(log.len(), 2);
        assert_eq!(log.records()[1].played_ms, 50_000);
    }

    #[test]
    fn replaying_the_same_track_records_a_new_play() {
        let mut log = HistoryLog::new(10);
        log.observe(&playing(1, 100_000, 0, 1), 0);
        log.observe(&playing(1, 100_000, 100_000, 1), 100_000);

        assert!(log.observe(&playing(1, 100_000, 0, 2), 200_000));
        assert_eq!(log.len(), 1);
        assert_eq!(log.records()[0].started_at, 0);

        log.observe(&playing(1, 100_000, 30_000, 2), 230_000);
        assert!(log.finish());
        assert_eq!(log.len(), 2);
        assert_eq!(log.records()[1].started_at, 200_000);
        assert_eq!(log.records()[1].played_ms, 30_000);
    }

    #[test]
    fn a_track_played_to_the_end_is_completed() {
        let mut log = HistoryLog::new(10);
        log.observe(&playing(1, 100_000, 0, 0), 0);
        log.observe(&playing(1, 100_000, 99_000, 0), 99_000);
        log.observe(&playing(2, 0, 0, 1), 100_000);

        assert!(log.records()[0].completed);
    }

    #[test]
    fn a_preview_completes_on_the_stream_duration() {
        let mut log = HistoryLog::new(10);
        let mut preview = playing(1, 300_000, 0, 0);
        preview.stream_duration_ms = Some(30_000);
        log.observe(&preview, 0);
        preview.position_ms = 29_000;
        log.observe(&preview, 29_000);
        log.observe(&playing(2, 0, 0, 1), 30_000);

        assert!(log.records()[0].completed, "{:?}", log.records()[0]);
    }

    #[test]
    fn clearing_the_queue_finishes_the_current_track() {
        let mut log = HistoryLog::new(10);
        log.observe(&playing(1, 100_000, 0, 0), 0);
        log.observe(&playing(1, 100_000, 5_000, 0), 1_000);

        let cleared = PlaybackState {
            track: None,
            ..PlaybackState::default()
        };

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
        log.observe(&playing(1, 100_000, 0, 0), 0);
        log.observe(&playing(1, 100_000, 4_000, 0), 4_000);

        assert!(log.finish());
        assert_eq!(log.len(), 1);
        assert!(!log.finish());
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn a_play_with_no_audio_is_dropped() {
        let mut log = HistoryLog::new(10);
        // Observed once at position 0 and immediately replaced: nothing was heard.
        log.observe(&playing(1, 100_000, 0, 0), 0);
        assert!(!log.observe(&playing(2, 100_000, 0, 1), 10));
        assert!(log.is_empty());
    }

    #[test]
    fn capacity_drops_the_oldest_records() {
        let mut log = HistoryLog::new(2);
        for id in 1..=4 {
            log.observe(&playing(id, 1_000, 0, id), id as i64);
            log.observe(&playing(id, 1_000, 900, id), id as i64 * 10);
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
            log.observe(&playing(id, 1_000, 0, id), id as i64);
            log.observe(&playing(id, 1_000, 500, id), id as i64 * 10);
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
            played_ms: 1_000,
            completed: true,
            kind: crate::playback::QueueKind::Manual,
        });
        let log = HistoryLog::from_records(records, 2);

        let ids: Vec<u64> = log.records().iter().map(|r| r.track.id.get()).collect();
        assert_eq!(ids, vec![2, 3]);
        assert_eq!(log.revision(), 3, "every append counts, trimmed or not");
    }

    #[test]
    fn stored_plays_with_no_audio_are_dropped_on_load() {
        let stale = PlayRecord {
            track: track(9, 1_000),
            started_at: 0,
            played_ms: 0,
            completed: false,
            kind: crate::playback::QueueKind::Manual,
        };
        assert!(HistoryLog::from_records([stale], 10).is_empty());
    }
}
