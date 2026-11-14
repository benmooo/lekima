# Sprint 5 — System Design

> Companion to [`product-design.md`](./product-design.md).
> Platform integration, device selection, range seeks, gapless.

Status: **Approved for design**

---

## 1. Constraints

| Constraint | Consequence |
| --- | --- |
| Daemon is headless, no window | macOS media APIs need a run-loop thread; no app delegate |
| Audio thread must not block on network | Range fetches happen in background; reader waits on condvars |
| rodio `Player` queues sources | Gapless = append prepared source + boundary notification |
| MPRIS is Linux/D-Bus only | `cfg(target_os = "linux")`, mock-bus tests in CI |
| Cross-platform compile | `lekima-platform` with cfg modules; no Linux deps on macOS |
| Existing protocol stable | Add events/commands, never break 0.2 clients |

## 2. New crate

`lekima-platform` — OS integration adapters.

```rust
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub playing: bool,
    pub artwork: Option<Vec<u8>>,
}

pub trait MediaControls: Send + Sync {
    fn update(&self, state: &NowPlaying);
    fn poll(&self) -> Vec<MediaCommand>;
}

pub enum MediaCommand { Play, Pause, Toggle, Next, Previous, Seek(u64), Stop }
```

- `platform::new()` returns an implementation (or a no-op) per OS.
- The daemon owns the handle, updates on every state event, and forwards
  `poll()` commands into the `Player` (same code path as the TUI).
- Artwork is optional and fetched by the adapter from the album cover URL,
  cached under `cache/artwork/<album-id>`.

Dependency sketch:

| OS | Crates | Notes |
| --- | --- | --- |
| Linux | `mpris-server` (zbus) | `org.mpris.MediaPlayer2.lekima` |
| macOS | `objc2`, `objc2-foundation`, `objc2-media-player` | run-loop thread |
| other | none | no-op adapter |

MPRIS property mapping:

| MPRIS | Source |
| --- | --- |
| `PlaybackStatus` | `PlaybackStatus` (Playing/Paused/Stopped) |
| `Metadata` | title, artist, album, `mpris:length` (µs), `mpris:artUrl` |
| `Position` | `position_ms` (µs) |
| `Volume` | linear gain 0..1 |
| `Shuffle` / `LoopStatus` | shuffle / repeat (None/Track/Playlist) |
| `CanSeek` | true when not loading |

Commands (`PlayPause`, `Next`, `Previous`, `Seek`, `SetPosition`, `Volume`)
are translated into `Command` values and executed by the daemon.

macOS: `MPNowPlayingInfoCenter` updated on the run-loop thread via
`performSelectorOnMainThread` (or a dedicated thread's run loop);
`MPRemoteCommandCenter` handlers push `MediaCommand` into an mpsc channel that
the daemon polls. `NSApplicationLoad()` is called once. If remote commands are
unavailable (no bundle), log a warning and keep Now Playing read-only.

## 3. Device selection

- Config:
  ```toml
  [audio]
  device = "MacBook Pro Speakers"   # omitted = system default
  ```
- `lekima_audio::devices() -> Vec<DeviceInfo { name, is_default }>` using
  rodio's re-exported `cpal` (`default_host().output_devices()`).
- `RodioAudioEngine::open(volume, device: Option<&str>)`:
  resolve the device by exact name, `DeviceSinkBuilder::from_device(device)?`
  then `open_stream()`. Missing device → `Playback("output device … not found")`.
- Daemon passes `config.audio.device`; `doctor` prints active device and the
  device list.
- CLI `lekima devices` prints `* name` for default; `lekima audio-device <name>`
  persists via `save_audio_device` and restarts the audio engine (daemon
  command `ReloadAudio` or a quiet daemon restart in 5.1).

## 4. Range seeks

### 4.1 Sparse spool

Extend `Spool` state:

```rust
struct SpoolState {
    ranges: Vec<Range<u64>>,   // sorted, merged downloaded ranges
    total: Option<u64>,
    complete: bool,
    error: Option<String>,
    cancelled: bool,
    pending_fetch: Option<Range<u64>>,
}
```

- Sequential writer appends and merges `[written_before, written_after)`.
- `GrowingFile::read` waits until `position` is inside a downloaded range.
- `GrowingFile::seek(target)`:
  1. if target inside a range → position it;
  2. if `complete` → EOF/allow;
  3. else set `pending_fetch = target..target+CHUNK`, wake the downloader, and
     wait (with timeout) until covered; `buffering=true`.
- Downloader loop:
  - poll `pending_fetch`; if set, cancel the sequential response and issue
    `GET Range: bytes=start-end`;
  - write at absolute file offsets (`File::seek(Start)`), merge the range;
  - on completion of the range, resume sequential from the highest contiguous
    range? For simplicity, stay in range mode: each read gap triggers another
    fetch. Metadata end-seeks become one extra request.
- Server ignoring Range (HTTP 200): mark `range_unsupported`, fall back to
  reading until the target is available (sequential wait), toast once.

### 4.2 Position semantics

- The audio position remains decoder-relative and correct; the spool only
  supplies bytes. No synthetic offset is needed because the decoder's seek
  repositions to the target time.
- `buffered_ms` is derived from the contiguous range containing the current
  position and the byte rate estimate (total/duration) when both exist.

### 4.3 Failure

- Range 4xx/5xx → `Playback("server refused range seek")`; keep playing from the
  old position.
- 30 s stall → playback error + skip (existing policy).

## 5. Gapless

### 5.1 Preparation

`Player` gains a preload slot:

```rust
struct Preloaded {
    track: Track,
    source: PreparedSource,   // bytes or spool + spool handle
}
```

- After a track starts and the queue has a next entry (and repeat/queue policy
  allows it), the daemon resolves the next URL, downloads/spools it, and calls
  `audio.prepare_next(prepared)`.
- `AudioEngine` port:
  ```rust
  async fn enqueue_next(&self, source: SourceHandle) -> Result<()>;
  ```
  `SourceHandle` is an opaque enum (bytes / spool) owned by the audio crate.

### 5.2 Playback boundary

- Audio thread appends the prepared decoder to the same rodio `Player`
  (queue) instead of replacing the sink.
- A wrapper source emits a `TrackingEvent::Boundary` through an mpsc channel
  exactly when the first source ends; the audio engine exposes
  `take_boundary()` and the daemon ticker advances the core queue/state and
  publishes events.
- If the next source fails to prepare, fall back to the current
  stop-then-load path and log.
- Sample-rate changes: rodio's queue handles per-source rates; we only claim
  gapless for matching rates. A `Crossfade` is out of scope.

### 5.3 State invariants

- Core's current index advances only on the boundary event, not on
  `sink.empty()`.
- `finished` is no longer used for gapless-consecutive tracks; keep it for
  unprepared ends.
- Preload is cancelled on skip/stop/queue edits; one slot only.

## 6. Testing

| Area | Tests |
| --- | --- |
| Devices | `devices()` returns at least one entry in CI? (skip if headless); missing-device error |
| MPRIS | `dbus-run-session` + mock client: properties, PlayPause/Next/Seek mapping |
| macOS | manual checklist; CI only compiles |
| Range | local axum/wiremock server honouring Range; seek to 70%, assert bytes fetched and position |
| Range fallback | server ignoring Range: sequential wait path |
| Gapless | generated 2×1 s WAV fixtures; measure silence gap < 50 ms at boundary |
| Core | boundary advances queue once; cancellation on skip; repeat one reuses current |
| CLI | `devices`, `audio-device`, config persistence |
| Regression | 0.2 suite, events, MCP |

## 7. Work breakdown

| # | Task | Depends |
| --- | --- | --- |
| 1 | `lekima-platform` crate + no-op impl + daemon wiring | — |
| 2 | Device enumeration + config + CLI + doctor | — |
| 3 | MPRIS adapter + mock tests | 1 |
| 4 | macOS Now Playing + media keys prototype | 1 |
| 5 | Sparse spool + range downloader | — |
| 6 | Reader seek integration + buffering/fallback | 5 |
| 7 | Gapless prepare/enqueue + boundary events | — |
| 8 | Tests, docs, 0.3.0 release | all |

## 8. ADRs

| ADR | Decision | Why |
| --- | --- | --- |
| 040 | Separate `lekima-platform` crate | Keeps OS deps out of the core/TUI |
| 041 | MPRIS via `mpris-server` | Typed interfaces, fewer hand-rolled D-Bus strings |
| 042 | Media keys on a run-loop thread | MPNowPlaying needs a run loop; daemon stays async |
| 043 | Sparse spool with fetch-on-seek | Works with the decoder's byte-level seeks |
| 044 | Gapless via rodio queue append | Sample-accurate without a custom mixer |
| 045 | Claim gapless only for matching rates | Honest guarantees, no artefacts |

## 9. Open questions

1. Range chunk size (256 KiB?) and max concurrent fetches (1).
2. Preload budget: bytes vs spool threshold; preload only ≤ 64 MiB? Proposal:
   preload uses the same rules as playback (memory for small, spool for large).
3. MPRIS `Raise`: focus the terminal? No portable answer; no-op.
4. macOS artwork: fetch by default or opt-in? Proposal: opt-in for bandwidth.
5. Device hot-plug: poll every 5 s or rely on errors? Proposal: errors only.
