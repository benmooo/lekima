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

- A play **starts only when audio is running**. A restored queue, a queue edit
  and a paused track are current without being plays, so they never produce a
  record (and cannot invent a 0 ms entry).
- A record is **appended when a play ends**: another track starts, the queue is
  cleared, playback is stopped, or the daemon shuts down. `stop` ends the play —
  it resets position and the next resume starts from zero, so it is not treated
  like a pause.
- A play is identified by **`(track id, generation)`**. `PlaybackState.generation`
  is bumped by the player whenever a new stream is loaded, so repeat-one loops
  and replaying the same track are separate plays rather than one inflated
  record.
- `played_ms` accumulates **position deltas while playing**. Paused time is not
  counted. A forward seek counts as listened; listening to a region twice counts
  twice, because this is time spent listening, not unique audio.
- A play with `played_ms == 0` is **dropped**: nothing was heard.
- `completed` is `played_ms >= 95%` of the **effective duration** — the decoded
  stream duration when the engine reports one (previews), otherwise the metadata
  duration. A track with no known duration is never completed.
- Sampling is at **4 Hz**. A track shorter than a poll interval can be missed,
  and a very short track can land just under the completion threshold; both are
  stated limitations, not accidents.
- Records are ordered oldest → newest on disk; every surface returns **newest
  first**.

The track playing right now is **not** in the history. `status` already answers
that, and a row that changes every second would be noise.

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

- **Bounded, not append-only.** `[history] capacity` (default `1_000`, minimum
  `1`) drops the oldest records. A JSON array of at most 1000 small objects is a
  few hundred KiB; an unbounded journal would need compaction, rotation and a
  file format nobody asked for.
- **Daemon is the only writer.** The CLI, TUI, MCP and HTTP read through IPC,
  never off disk; `lekima doctor` is the one diagnostic exception, and it only
  reads. Two paths write (the poller and shutdown): the log revision decides
  whose snapshot is newer, so a stale poller clone cannot overwrite the final
  play.
- **Written only when the log changed.** Starting a play does not touch the
  file, so a daemon that is merely running — or restarting with a restored
  queue — never rewrites history. The one exception is a file that is over
  capacity or contains zero-played rows from an older build: it is rewritten
  once at load, so the on-disk bound matches memory and a later capacity
  increase cannot resurrect records the running build dropped.
- **A corrupt or future-versioned file is reported, not deleted.** The daemon
  logs a warning, starts empty, and leaves the file alone until a real play is
  recorded. `lekima doctor` shows the error. Silently wiping a user's history
  would hide the bug that produced it.
- **A crash can lose the current play**, and — rarely — the play that ended
  within the last 250 ms sampling window. Shutdown closes the active play
  explicitly, so a clean stop never does.

## 5. Surface

| Surface | Shape |
| --- | --- |
| CLI | `lekima history [--limit N] [--json]` |
| IPC | `{"type":"history","limit":N}` → `{"type":"history","entries":[…]}`; default 50, clamped to 1–500 |
| MCP | tool `history` (`limit` 1–500, read-only hint) |
| HTTP | `GET /v1/history?limit=N` |
| TUI | `H` opens `HISTORY`; `j/k/g/G/ctrl-d/u`, `enter` replays, `r` refreshes |

The IPC limit defaults to 50 and is clamped to 1–500 by the daemon, so a client
cannot ask for an unbounded reply and `0` cannot mean "no data" on one surface
and "all data" on another. MCP clamps the same way. `history` is advertised in
the `Hello` capabilities list, the HTTP `COMMAND_TYPES`, and `lekima
capabilities`.

`lekima doctor` reports the file, capacity, entry count and load error, so a
corrupt file is visible without reading logs.

## 6. Tests

- `lekima-domain`: a restored/stopped/paused track is not a play (and a 0 ms
  play is dropped), play-time accumulation, seek accounting, completion
  threshold, previews completing on the stream duration, repeat-one and
  same-track replay producing separate records, queue clear, finish-once,
  capacity trimming, `recent` ordering, load trimming.
- `lekima-config`: round-trip, `0600`, missing and empty files, capacity on load,
  malformed and future-versioned files reported.
- `lekima-ipc`: command/reply wire shape and the default limit.
- `lekima-cli`: HTTP route requires the token, is permitted with one, and a
  malformed `limit` is 401 without a token and a JSON 400 with one; CLI exits 7
  with a `daemon` error when no daemon is running.
- Manual, with a live daemon and a restored queue: start/stop adds no phantom
  record and does not evict a real one; a corrupt `history.json` survives
  start/stop untouched and shows up in `doctor`; `--limit 0` returns one entry
  and `--limit 1000` is capped at 500.

No test performs real network playback; the transition rules are covered at the
unit level instead.

## 7. Acceptance

1. Play a track to the end, skip the next one: two entries, newest first, one
   `completed`.
2. Repeat-one across two loops: two records, not one record with doubled time.
3. Restart the daemon with a restored queue and change nothing: no new record.
4. `lekima history --json` returns `{"ok":true,"data":{"type":"history","entries":[…]}}`.
5. After `capacity` plays the oldest record is gone and the file did not grow
   without bound.
6. `H` in the TUI lists the same plays and `enter` replays the selected track.
7. An empty history prints a hint instead of an empty table.
8. A corrupt `history.json` is not overwritten by a daemon start, and `doctor`
   reports the parse error.

## 8. Non-goals

- Scrobbling (AKA-143) — separate design note and external credentials.
- Statistics, charts, "year in review", or aggregation by artist/album.
- Editing or deleting individual records.
- Syncing history between machines.
