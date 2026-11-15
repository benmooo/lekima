# Sprint 7 — Playback hardening

Status: **In progress**

Goal: make playback transitions truthful and recoverable before adding another
feature. Scope is derived from the 0.4.x transition bugs:

| Item | Status | Notes |
| --- | --- | --- |
| One-shot gapless boundaries | done (0.4.1) | A boundary source fires once per source; it cannot reset the position repeatedly. |
| Monotonic track position | done (0.4.1) | A load resets the core position before the URL resolves; a new track cannot inherit the previous track's end. |
| Stale status polls | done (0.4.1) | Status replies carry a daemon revision; the TUI ignores a poll older than the newest event. |
| Queue end keeps the last track | done | `Queue::advance(Off)` returns `None` at the tail without clearing the cursor, so `resume` reloads the last track instead of failing. |
| Preview/stream duration | done | The audio adapter publishes the decoder's real duration; the HUD/CLI show a `PREVIEW` state and progress to the real end. |
| Queue persistence | next | Persist queue, current index, repeat/shuffle and position across daemon restarts. |
| Transition harness | next | Deterministic A→B→C tests for preview streams, FM batches, manual skip during a boundary and queue end/resume. |
| Daemon restart UX | next | Reattach the TUI cleanly and report whether playback was restored or stopped. |

## Acceptance criteria

- A 30 s preview of a 2 min track shows `PREVIEW 00:00/00:30`, never jumps back
  to the previous track's position, and advances once at the end.
- Pressing `n` exactly at a gapless rollover never skips two entries.
- A queue that reaches its end keeps the last track visible; `space`/`resume`
  reloads it, and `status --json` reports `stopped` with a valid `queue_index`.
- FM prefetch appends a new batch, publishes a queue event and the open queue
  pane shows the new songs.
- `cargo test --workspace`, `clippy -D warnings` and `fmt --check` stay green.
