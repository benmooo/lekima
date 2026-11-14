# Sprint 4 — Product Design

> Theme: **Playback depth** — large files, streaming, buffering, sleep timer,
> quality switching. Platform integration (MPRIS/Now Playing/media keys) is
> Sprint 5.
> Status: **Approved for design**

---

## 1. Why

The player is shipped, but playback still buffers whole tracks in memory and
refuses anything over 64 MiB. Your 1 GB cloud WAV fails outright; lossless
tracks start slowly and spike RSS. This sprint makes the engine behave like a
real player: **start early, keep memory flat, handle big files**.

## 2. Goals

1. **Stream instead of download.** Start playback after a small buffer; keep
   downloading in the background to a temp spool.
2. **No arbitrary size cap.** A 1 GB cloud upload plays. Memory stays bounded.
3. **Honest buffering UX.** Loading/buffering is visible; the old track still
   stops instantly.
4. **Sleep timer.** “Stop in 30 minutes” from CLI/TUI, shown in status.
5. **Quality switching.** `standard|higher|exhigh|lossless` at runtime, persisted.
6. **No regressions** for normal tracks: first-audio latency and seeking on
   fully downloaded tracks stay as they are.

### Non-goals

- Gapless playback, crossfade, replay gain.
- MPRIS/Now Playing/media keys (Sprint 5).
- Audio device picker (Sprint 5 with platform work).
- Offline downloads library.

## 3. Users and jobs

| User | Job | Today |
| --- | --- | --- |
| Cloud user | “Play my 1 GB WAV.” | Refused: > 64 MiB |
| Lossless listener | “Start in under a second.” | Full download first |
| Long session | “Stop after this album.” | No timer |
| Data/VIP user | “Use 320k, not lossless.” | Config edit + restart |

## 4. Behaviour

### 4.1 Streaming

- Tracks up to **64 MiB** keep the current full-download path (fast, seekable,
  no disk I/O).
- Larger or unknown-length streams go to a **spool**:
  - anonymous temp file (auto-deleted), written by a background download;
  - playback starts once **512 KiB or 2 s worth** (whichever the engine can
    determine) is buffered;
  - the decoder reads from the growing file and blocks briefly if it catches
    up with the writer;
  - memory stays flat (no full body in RAM).
- The previous track is stopped before any network work (sprint-3 rule).
- On stop/skip, the download is cancelled and the spool dropped.

### 4.2 Buffering states

- `loading` (existing) covers “before first sample”.
- A new `buffering` flag is set when the decoder is waiting on the network
  mid-track (stall), cleared when data resumes. The HUD shows `◌` and the
  footer says `buffering`; agents see `"buffering": true`.
- When the download fails mid-track, playback stops with a playback error and
  the queue advances (existing behaviour).

### 4.3 Seeking

- Fully downloaded tracks: unchanged (seek anywhere).
- Streaming tracks: seek **within the buffered region** works; a seek beyond it
  fails with `playback` (“still buffering”) until Range-based seeking lands.
  The TUI clamps to the buffered range when known.
- The status exposes `buffered_ms` so the progress bar can render a second,
  dim segment for the buffered portion.

### 4.4 Sleep timer

- `lekima sleep 30m`, `lekima sleep off`; TUI `:sleep 30m`; MCP `sleep` tool.
- Status includes `sleep_remaining_ms`; HUD shows `SLEEP 28:14` when active.
- Timer fires → pause (not stop), preserving the queue. Cancelling is explicit.
- Timer survives track changes, not daemon restarts.

### 4.5 Quality

- `lekima quality [standard|higher|exhigh|lossless]` sets the live preference and
  persists `[playback] quality`.
- Changing quality applies to the **next** track (never interrupts the current
  stream).
- Status/HUD shows the resolved bitrate class (`320K`, `LOSSLESS`).

## 5. Keymap / commands

| Surface | Action |
| --- | --- |
| CLI | `lekima sleep 30m`, `lekima sleep off`, `lekima quality lossless` |
| TUI | `:sleep 30m`, `:quality exhigh`, HUD badges |
| MCP | `sleep {minutes}`, `set_quality {quality}` |
| JSON | `sleep_remaining_ms`, `buffering`, `buffered_ms`, `quality` |

## 6. Success criteria

- 1 GB WAV starts playing in **< 5 s** on a 50 Mbps link (vs. refused today).
- Peak RSS while streaming a 1 GB track < **150 MB**.
- First-audio latency for a normal 4 MB MP3 does not regress.
- Seeking a fully downloaded track unchanged; streaming seek within buffer works.
- Sleep timer accuracy ±1 s over 30 min; survives track changes.
- Quality change affects the next track and persists across restarts.
- No temp files left after quit/skip/crash (anonymous spool).

## 7. Risks

| Risk | Mitigation |
| --- | --- |
| Decoder seeks to end during probe (MP3/MP4) and blocks | Large files only use the spool; test MP3/FLAC/WAV; cap spool path to formats that probe sequentially; fall back to full download on probe failure |
| Audio underrun when network stalls | 512 KiB pre-buffer, blocking reader, `buffering` flag; retry once |
| Disk usage for huge files | Anonymous temp files, dropped on stop; optional cache-quota check |
| Seek semantics surprise users | Clamp + explicit “still buffering” error; document |
| Sleep timer drift while paused/loading | Tick-based remaining, pause only on fire |

## 8. Milestones

| # | Milestone | Exit |
| --- | --- | --- |
| M1 | Design approved | these docs |
| M2 | Spool + threshold playback | 1 GB WAV plays, RSS flat |
| M3 | Buffering status + seek clamp | HUD/agents see state |
| M4 | Sleep timer | CLI/TUI/MCP tested |
| M5 | Quality switching | persisted, next-track |
| M6 | 0.2.0 release | crates + binaries + tap |
