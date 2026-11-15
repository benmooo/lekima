# Sprint 7 — Playback hardening

Status: **Complete**

Goal: make playback transitions truthful and recoverable before adding another
feature. Scope is derived from the 0.4.x transition bugs:

| Item | Status | Notes |
| --- | --- | --- |
| One-shot gapless boundaries | done (0.4.1) | A boundary source fires once per source; it cannot reset the position repeatedly. |
| Monotonic track position | done (0.4.1) | A load resets the core position before the URL resolves; a new track cannot inherit the previous track's end. |
| Stale status polls | done (0.4.1) | Status replies carry a daemon revision; the TUI ignores a poll older than the newest event. |
| Queue end keeps the last track | done | `Queue::advance(Off)` returns `None` at the tail without clearing the cursor, so `resume` reloads the last track instead of failing. |
| Preview/stream duration | done | The audio adapter publishes the decoder's real duration; the HUD/CLI show a `PREVIEW` state and progress to the real end. |
| Queue persistence | done | `SavedPlayback`/`PlaybackStore` rewrite `<config>/cache/playback.json` (0600) every 2 s and on shutdown; startup restores a stopped queue and saved position. |
| Transition harness | done | Deterministic preview/A→B/C, FM repeat/prefetch, stale/duplicate boundary, queue end/resume and manual-skip-at-boundary tests. |
| Daemon restart UX | done | TUI polls while offline, reports `daemon restarted`/`queue restored - space to resume`, and a restored queue resumes at the saved position. |

## Acceptance criteria

- A 30 s preview of a 2 min track shows `PREVIEW 00:00/00:30`, never jumps back
  to the previous track's position, and advances once at the end.
- Pressing `n` exactly at a gapless rollover never skips two entries.
- A queue that reaches its end keeps the last track visible; `space`/`resume`
  reloads it, and `status --json` reports `stopped` with a valid `queue_index`.
- FM prefetch appends a new batch, publishes a queue event and the open queue
  pane shows the new songs.
- `cargo test --workspace`, `clippy -D warnings` and `fmt --check` stay green.

Evidence: 153 workspace tests, including `restored_queue_resumes_at_the_saved_position`,
`manual_next_at_a_boundary_does_not_skip_twice`,
`preview_streams_use_their_real_duration`,
`queue_end_keeps_the_last_track_and_can_resume`,
`daemon_restart_is_announced_and_queue_restored`, and the config store
round-trip/permission tests. Live verification: daemon stop/start restored the
queue stopped, and `resume` continued the preview at its saved 0:18.
