# Sprint 10 · Slice B — Listening history

Status: **Implemented, unreleased** · Linear AKA-147

## 1. Problem

`lekima` can resume *one* queue (`cache/playback.json`), but it keeps no record
of what was heard. After a daemon restart or a week of listening there is no way
to answer "what did I play yesterday?", "did I finish that album?", or "play that
track again". Every other player answers this; the terminal one should too, and
agents driving the daemon need it for the same reason.

## 2. What counts as a play

The rule lives in `lekima_domain::HistoryLog`, so it is testable with no player,
no clock and no filesystem:

- A **record is appended when a track stops being current**: another track
  starts, the queue is cleared, or the daemon shuts down.
- The track playing **right now is not in the history**. `status` already answers
  that, and a half-played row would be a lie that changes every second.
- `played_ms` accumulates from **position deltas while playing**. Paused time is
  not counted; a seek forward counts as listened, a seek backwards is not
  counted twice.
- `completed` is `played_ms >= 95% of duration`. A track with **no known
  duration is never completed** — guessing would make every preview look
  finished.
- Records are ordered oldest → newest on disk; every surface returns **newest
  first**.

## 3. Record

```json
{
  "track": { "id": 42, "name": "…", "artists": [], "album": null, "duration_ms": 355000 },
  "started_at": 1700000000000,
  "played_ms": 354000,
  "completed": true,
  "kind": "manual"
}
```

- `started_at` is **Unix milliseconds**, so agents sort and bucket without
  parsing prose. Human output renders a relative age (`14m ago`) because that
  needs no timezone database and no new dependency.
- `kind` is the `QueueKind` the track came from (`manual`, `daily`, `fm`, …), so
  history can be filtered later without a migration.
- `track` is the full domain track, not just an id: the provider may forget a
  track, and a log that cannot render its own rows is not a log.

## 4. Storage

`<state>/history.json`, one versioned document, rewritten atomically
(temp + `rename`, mode `0600`) exactly like `playback.json`:

```json
{ "version": 1, "entries": [ … ] }
```

Decisions:

- **Bounded, not append-only.** `[history] capacity` (default `1_000`) drops the
  oldest records. A JSON array of at most 1000 small objects is a few hundred
  KiB; an unbounded journal would need compaction, rotation and a file format
  nobody asked for.
- **Daemon is the only writer.** Same rule as playback persistence: the CLI and
  TUI read through IPC, never off disk.
- **A corrupt file is reported, not deleted.** The daemon logs a warning, starts
  empty, and overwrites on the next play. Silently wiping a user's history would
  hide the bug that produced it.
- **Writes happen on play boundaries** (start/end of a track), not on every
  tick, so a long track does not rewrite the file 3600 times.
- **A crash loses at most the current track**, because it is not written until it
  stops being current. Shutdown closes it explicitly.

## 5. Surface

| Surface | Shape |
| --- | --- |
| CLI | `lekima history [--limit N] [--json]` |
| IPC | `{"type":"history","limit":N}` → `{"type":"history","entries":[…]}` |
| MCP | tool `history` (`limit` 1–500, read-only hint) |
| HTTP | `GET /v1/history?limit=N` |
| TUI | `H` opens `HISTORY`; `j/k/g/G/ctrl-d/u`, `enter` replays, `r` refreshes |

The IPC limit defaults to 50 and is clamped to 500 by the daemon, so a client
cannot ask for an unbounded reply. `history` is advertised in the `Hello`
capabilities list, the HTTP `COMMAND_TYPES`, and `lekima capabilities`.

## 6. Tests

- `lekima-domain`: record on transition, play-time accumulation, pause not
  counted, seek accounting, completion threshold, unknown duration, queue clear,
  finish-once, capacity trimming, `recent` ordering, load trimming.
- `lekima-config`: round-trip, `0600`, missing and empty files, capacity on load,
  malformed file reported.
- `lekima-ipc`: command/reply wire shape and the default limit.
- `lekima-cli`: HTTP route requires the token and is permitted with one; CLI
  exits 7 with a `daemon` error when no daemon is running.
- Manual: a hand-written `history.json` renders in `lekima history` (human and
  JSON) through a live daemon, with completion markers and `--limit`.

No test performs real network playback; the transition rules are covered at the
unit level instead.

## 7. Acceptance

1. Play a track to the end, skip the next one: two entries, newest first, one
   `completed`.
2. Restart the daemon: the log is unchanged and still readable.
3. `lekima history --json` returns `{"ok":true,"data":{"type":"history","entries":[…]}}`.
4. After `capacity` plays the oldest record is gone and the file did not grow
   without bound.
5. `H` in the TUI lists the same plays and `enter` replays the selected track.
6. An empty history prints a hint instead of an empty table.
7. `lekima doctor` still starts the daemon when `history.json` is corrupt, and
   the warning is in the log.

## 8. Non-goals

- Scrobbling (AKA-143) — separate design note and external credentials.
- Statistics, charts, "year in review", or aggregation by artist/album.
- Editing or deleting individual records.
- Syncing history between machines.
