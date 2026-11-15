# lekima — System Design

Status: **Living document** — up to date through 0.5.0 · Companion to
[`product-design.md`](./product-design.md)

---

## 1. Goals and constraints

| Goal | Design consequence |
| --- | --- |
| Vim-fast UI | TUI never performs network I/O; it talks to a local daemon |
| Agent-drivable | Stable JSON CLI + Unix-socket protocol; playback survives process exit |
| Provider changes cheaply | All NetEase protocol knowledge behind one port + `ncmapi` |
| Testable rules | Queue/state machine is pure, in `lekima-domain`; no I/O in tests |
| Small blast radius | One writer of playback state; adapters are replaceable |
| Safe by construction | `unsafe` forbidden in first-party crates; secrets off argv/logs; strings sanitized |

Hard constraints: Rust 2024 edition, MSRV 1.98, macOS + Linux first-class,
single static binary, no mandatory system audio libraries beyond what rodio
needs (CoreAudio on macOS; ALSA/Pulse on Linux).

## 2. Context

```mermaid
flowchart LR
  user([Human]) -->|keys| tui[lekima TUI]
  agent([Code agent / script]) -->|argv| cli[lekima CLI]
  tui -->|JSON-lines over UDS| daemon[(lekima daemon)]
  cli -->|JSON-lines over UDS| daemon
  daemon -->|HTTPS| ncm[(ncmapi-rs)]
  ncm --> netease[(NetEase Cloud Music)]
  daemon -->|rodio| audio[(Audio device)]
  daemon -->|cookies 0600| disk[(~/.config/lekima)]
  cli -->|spawn on demand| daemon
```

Deployment is a single container in the logical sense but two processes at
runtime. The daemon is optional for read-only commands (`search`, `playlist
show`, `lyrics`, `capabilities`, `doctor` talk to the provider directly).

## 3. Containers and crates

```mermaid
flowchart TD
  cli[lekima<br/>binary: lekima] --> tui[lekima-tui]
  cli --> ipc[lekima-ipc]
  cli --> core[lekima-core]
  cli --> ncm[lekima-ncm]
  cli --> audio[lekima-audio]
  cli --> config[lekima-config]
  tui --> ipc
  tui --> domain[lekima-domain]
  core --> domain
  ncm --> domain
  audio --> domain
  config --> domain
  ipc --> domain
```

Dependency rule: **arrows point inward**. `lekima-domain` depends on nothing
but `serde`, `thiserror`, `async-trait`. `lekima-core` may depend on domain
only. Adapters depend inward. The CLI is the composition root — the only crate
allowed to know every concrete type.

| Crate | Responsibility | Must not |
| --- | --- | --- |
| `lekima-domain` | Entities, queue rules, playback state, ports | Do I/O, know HTTP/audio/terminal |
| `lekima-core` | `Player`: queue + transport orchestration, auto-advance | Know about IPC or CLI |
| `lekima-config` | Paths, `config.toml`, cookie file (0600) | Parse provider payloads |
| `lekima-ipc` | Protocol types, UDS client/server framing | Contain business rules |
| `lekima-ncm` | `MusicService` over `ncmapi` | Expose raw provider JSON |
| `lekima-audio` | `AudioEngine` over rodio | Download catalogue metadata |
| `lekima-tui` | Rendering, keymap, client-side state machine | Block on the network or own playback |
| `lekima` | Command parsing, daemon lifecycle, JSON rendering | Duplicate queue logic |

Workspace layout:

```
lekima/
├── Cargo.toml                # workspace, resolver 3, shared versions
├── docs/
│   ├── product-design.md
│   ├── system-design.md
│   └── adr/                  # architecture decision records
├── crates/
│   ├── lekima-domain/
│   ├── lekima-core/
│   ├── lekima-config/
│   ├── lekima-ipc/
│   ├── lekima-ncm/
│   ├── lekima-audio/
│   ├── lekima-tui/
│   └── lekima/           # [[bin]] name = "lekima"
└── tests/                    # workspace-level integration tests
```

## 4. Domain model

Newtypes instead of raw integers (`TrackId`, `PlaylistId`, `UserId`,
`AlbumId`, `ArtistId`). Prevents id mix-ups at compile time and gives a single
place for validation. Serde is `transparent`, so the wire format stays `123`.

```mermaid
classDiagram
  class Track { TrackId id; String name; Vec~Artist~ artists; Option~Album~ album; u64 duration_ms }
  class Artist { ArtistId id; String name }
  class Album { AlbumId id; String name; Option~String~ cover_url }
  class Playlist { PlaylistId id; String name; u64 track_count; Option~String~ owner }
  class UserProfile { UserId id; String nickname; Option~String~ avatar_url }
  class Queue { Vec~Track~ tracks; Option~usize~ current }
  class PlaybackState { PlaybackStatus status; Option~Track~ track; u64 position_ms; f32 volume; bool muted; RepeatMode repeat; bool shuffle; Vec~BufferedRange~ buffered; u64 revision; Option~u64~ stream_duration_ms }
  Queue --> Track
  PlaybackState --> Queue
```

Queue invariants (unit-tested in `lekima-domain`):

- `current` is `None` iff `tracks` is empty; otherwise it is a valid index.
- `advance(Off)` returns `None` at the tail and leaves the cursor unchanged;
  `advance(All)` wraps; `advance(One)` repeats.
- `rewind` wraps only under `All`.
- `remove_current` clamps the cursor; removing the last track empties the queue.

## 5. Playback state machine

```mermaid
stateDiagram-v2
  [*] --> Stopped
  Stopped --> Playing: play(tracks) / URL resolved
  Playing --> Paused: pause()
  Paused --> Playing: resume()
  Playing --> Playing: track ends (advance)
  Playing --> Stopped: track ends, no next
  Playing --> Stopped: stop()
  Paused --> Stopped: stop()
  Stopped --> Playing: resume() with non-empty queue
  Playing --> Playing: seek()/volume()
```

`Player` is the **only** writer. Guarantees:

- Transitions are serialized by a `tokio::sync::Mutex` around the inner state;
  the audio engine is called inside the critical section so observers never see
  “playing” before the sink is actually playing.
- Starting a track stops the device **before** resolving or downloading the
  stream (`loading = true`), so the previous track never overlaps the next
  one's network latency. A generation counter cancels superseded loads when the
  user skips quickly.
- A failed URL resolution never leaves the machine in `Playing`.
- `tick()` (called by the daemon every 500 ms) is the single place that reacts
  to end-of-track. No audio callback mutates application state.

## 6. Runtime architecture

### 6.1 Processes

| Process | Lifetime | Responsibility | Concurrency |
| --- | --- | --- | --- |
| Daemon (`lekima daemon run`, internal) | Long | Owns `Player`, provider session, socket listener | tokio multi-thread; one task per connection; one monitor task |
| CLI | Short | Parse args, call provider or daemon, print | `#[tokio::main]` |
| TUI | Session | Render, keymaps, call daemon | crossterm event thread + tokio client task |

The TUI is **not** the player. Closing it does not stop the music; that is the
whole point for agents and long coding sessions.

### 6.2 Daemon lifecycle

```mermaid
sequenceDiagram
  participant C as lekima CLI
  participant L as lock file
  participant D as daemon
  C->>C: resolve socket path
  C->>D: connect + ping
  alt not running and autostart enabled
    C->>L: try lock (O_EXCL, 5 s budget)
    C->>D: spawn(binary, "daemon run", --foreground=false)
    D->>L: lock held by parent? wait for socket up (100 ms poll, 5 s)
    C->>D: ping
  end
  C->>D: command
  D-->>C: reply
```

- Socket: `$XDG_RUNTIME_DIR/lekima.sock`, else `<config>/cache/lekima.sock`;
  overridable with `LEKIMA_SOCKET`.
- The socket directory is `0700`; the socket is `0600`.
- Stale-socket handling: connection refused + existing file ⇒ unlink and retry
  once, after taking the startup lock.
- Shutdown: `Shutdown` command or SIGTERM/SIGINT ⇒ stop audio, flush session,
  unlink socket. A `SIGKILL` leaves a stale socket that the next start reaps.
- Single instance per socket path enforced with an advisory lock on
  `lekima.lock`.
- Daemon logs to `<config>/lekima.log` via `tracing` (daily-free, size-capped,
  one rotated previous file) and to stderr when run with `--foreground`.

### 6.3 TUI event loop

```mermaid
sequenceDiagram
  participant K as keyboard thread
  participant U as UI task
  participant I as IPC client task
  K->>U: Key(code, modifiers)
  U->>U: reduce(state, key) → Effect
  U->>I: Effect::Call(Command)
  I-->>U: Reply (or error toast)
  U->>U: draw(frame)

  loop every 500 ms while visible
    U->>I: Status (coalesced, one in flight)
    I-->>U: PlaybackState
  end
```

- Rendering is pure: `view(&App) -> Frame`. The `App` reducer is unit-testable
  without a terminal using `ratatui::backend::TestBackend`.
- Network calls are fire-and-forget with request coalescing: a second `Status`
  request is skipped if one is in flight; key handling never awaits.
- Terminal restore is guarded by a drop type + panic hook (raw mode, alternate
  screen, cursor visibility).

### 6.4 Audio pipeline

```
stream URL ── reqwest GET (UA + Referer, cookie) ──► bytes / spooled file
   │                                                        │
   │  ≤ 64 MiB: decode from memory                          │
   │  larger/unknown: sparse spool + range worker          │
   └──────────────► Decoder (symphonia) ──► Sink ──► OutputStream
                                                  │
                              volume / pause / try_seek / empty() poll
```

Decisions:

- Streams with a known length up to `LEKIMA_MAX_TRACK_BYTES` (64 MiB) are read
  into memory before decoding. Larger or unknown-length streams spool to an
  anonymous temp file and start once `SPOOL_THRESHOLD` (512 KiB) is buffered,
  falling back to `buffering` if the network stalls.
- `PrefetchSource` runs the decoder on a background thread so the audio callback
  never performs I/O. A sparse spool tracks which byte ranges have arrived.
- Seeking within the buffered region is immediate; a seek beyond it asks the
  range worker for an HTTP `Range` fetch of the target and surfaces `buffering`
  until the decoder can continue.
- The spool exposes its merged byte ranges; the audio adapter maps them to track
  time (`buffered_ranges`) and the core publishes them as
  `PlaybackState.buffered`, letting frontends shade what is already local.
- The decoder's real duration is published as `stream_duration_ms` when it is
  shorter than catalogue metadata; frontends label the track `PREVIEW` and draw
  progress against the actual stream length.
- Fully downloaded streams up to `MAX_MEMORY_BYTES` are cached under
  `<config>/cache/streams/<id>-<quality>.audio` (0600, LRU, default 256 MiB).
  A cache hit skips `stream_url` resolution and sets `PlaybackState.cached`;
  larger/unknown streams keep using the ephemeral spool.
- Gapless: when the next queued track fits the preload budget and matches the
  sample rate, its source is prepared and appended to the same rodio queue; a
  boundary source advances core state on the first sample. Only one preload is
  held at a time, keyed by track id; a second request for the same position is
  a no-op, so volume/shuffle/repeat commands cannot enqueue the next track
  twice. A stale boundary (queue edit or manual jump) stops the audio and
  continues with the queue's next entry instead of replaying the track that
  just ended, so lyrics and audio cannot diverge.
- The boundary payload is the preloaded **track id**. A boundary for the
  current track is ignored (manual `n` racing the rollover), a boundary for the
  queue's next track advances, and anything else is stale. FM advances with
  `RepeatMode::Off` regardless of the stored repeat mode, so a finite station
  batch cannot wrap repeat-all back to old tracks.
- Loads and stops clear the audio boundary flag before the next source is
  appended; otherwise a boundary fired just before a manual jump could skip a
  queue entry.
- Device errors (`NoDevice`, `DeviceBusy`) map to `ErrorCode::Playback` with a
  hint; `lekima doctor` enumerates devices.
- The output stream is kept alive for the daemon’s lifetime; one sink per track,
  replaced on load (rodio mixes old/new, so we `stop` + drop before loading).

## 7. IPC protocol

Line-delimited JSON over a Unix stream socket. Requests are connection-per-call;
protocol v2 adds a `hello` handshake, `subscribe` and sequenced event frames on a
dedicated connection.

```jsonc
// client → daemon
{"id":1,"command":{"type":"play","target":{"type":"query","query":"mota"}}}
// daemon → client
{"id":1,"result":{"type":"status","state":{"status":"playing","track":{...},"position_ms":0}}}
// daemon → client on failure
{"id":1,"error":{"code":"unauthorized","message":"run `lekima login`"}}
```

Properties:

- `MAX_FRAME_BYTES = 1 MiB`; oversized frames get one error and the connection
  closes (desynchronised streams are not recoverable).
- Unknown fields are ignored by serde for forward compatibility; unknown
  command types produce `bad_request`.
- `Response.id` echoes the request id.
- **v2 (implemented)**: `hello` returns `{protocol, seq, capabilities}`;
  `subscribe {kinds, since}` acks and then streams `{seq, type, …}` frames.
  The daemon keeps a 256-frame replay ring, coalesces nothing at the socket and
  sends a `resync` frame when a subscriber lags. `lekima watch` prints NDJSON;
  the TUI subscribes and keeps a 20 Hz status poll for the spectrum.
- Background changes publish events too: FM prefetch emits a `queue` frame when
  it appends a batch, and gapless/auto advance emits both `status` and `queue`.
- Status replies carry the daemon’s current event revision. Clients that also
  subscribe to events discard a poll whose revision predates the newest event,
  so a slow reply cannot move progress/lyrics back to the previous track.
- **MCP**: `lekima mcp` speaks JSON-RPC over stdio and maps its tools onto the
  same IPC commands; it owns no state and never reads the session file.

Security:

- Socket permissions `0600`, parent directory `0700`.
- On Linux, verify `SO_PEERCRED` uid matches the daemon owner; on macOS
  `getpeereid`. Reject otherwise. (Defence in depth; the socket mode already
  restricts access.)
- No authentication tokens on the wire; the session stays inside the daemon.

## 8. Provider adapter (`lekima-ncm`)

- `NcmMusicService` implements `MusicService` using `ncmapi::NcmClient`.
- Constructed once per daemon; cookie path = `<config>/session.cookies`
  (written by `lekima login`, `0600`).
- Mapping layer owns all provider quirks: nested artists, missing album, `fee`
  flags, duration in ms, and `code != 200` handling. Mapping functions are pure
  and property-tested against fixtures.
- Errors: HTTP/timeout ⇒ `Unavailable`; `code == 301` ⇒ `AuthenticationRequired`;
  empty result ⇒ `NotFound`; missing endpoint support ⇒ `Unsupported`.

### 8.1 Extending `ncmapi-rs`

`ncmapi` 1.7 is published. 1.1 added playlist detail/paging, lyrics and QR
login; 1.2 added daily recommendations, personal FM, likes and toplists.
`ncmapi` resolves catalogue data and stream URLs; byte-range fetching for
seekable spools lives in `lekima-audio`, not the provider crate. Endpoint calls
that mutate state or whose answer changes between identical requests (QR
polling, FM batches, likes, trashes) are marked `.uncacheable()` so the
transport's request-hash cache cannot replay stale answers.

| Endpoint | Route | Used by |
| --- | --- | --- |
| QR login start/poll | `/eapi/login/qrcode/unikey`, `/eapi/login/qrcode/client/login` (`type: 3`) | `lekima login --qr` — **done in 1.3** |
| Playlist detail with tracks | `/api/v6/playlist/detail` + `/api/v3/song/detail` | `playlist show`, sidebar — **done in 1.1** |
| Lyrics | `/api/song/lyric` | `lekima lyrics` — **done in 1.1** |
| Public comments | `/api/v1/resource/comments/R_SO_4_{id}` | `lekima comments`, TUI `C` — **done in 1.7** |
| Daily recommendations | `/api/v3/discovery/recommend/songs` | Home / `lekima daily` — **done in 1.2** |
| Personal FM | `/weapi/v1/radio/get`, `/weapi/radio/trash/add` | `lekima fm` — **done in 1.2** |
| Likes | `/api/radio/like`, `/weapi/song/like/get` | `lekima like` / liked playlist — **done in 1.2** |
| Toplists | `/api/toplist` | Home sidebar — **done in 1.2** |

Note: `/api/playlist/track/all` is no longer served (404). Playlist detail
returns the full ordered `trackIds` list; pages are sliced locally and hydrated
through `/api/v3/song/detail` in batches of 500.

Process: developed on a branch, contract tests per route, deterministic suite
green, `cargo publish --dry-run`, tag `v1.1.0`, publish, then pin
`ncmapi = "1.1"` in this workspace (no path patch). Future additions follow the
same flow as `1.2.0`. Public API stays additive; the existing
`NcmClient`/service shape is preserved.

## 9. Configuration

`<config>/config.toml`, parsed strictly (`deny_unknown_fields`) so typos fail
loudly:

```toml
[provider]
cache = true
cache_ttl_secs = 180

[playback]
volume = 0.7
quality = "exhigh"   # standard | higher | exhigh | lossless
autoplay = true

[cache]
enabled = true
limit_mb = 256

[ui]
theme = "cyber"          # cyber | amber | mono | light | … (LEKIMA_THEME wins)
alternate_screen = true
show_help_on_start = false

[daemon]
autostart = true
log_level = "info"
```

Precedence: defaults → file → environment (`LEKIMA_CONFIG_DIR`,
`LEKIMA_SOCKET`, `LEKIMA_LOG`, `LEKIMA_THEME`, `NO_COLOR`). Flags win over all.
`LEKIMA_THEME` accepts a palette name, `light`, `dark` or `auto`; `NO_COLOR`
forces mono and wins over both.

Files: `config.toml`, `session.cookies` (0600), `lekima.log`, `cache/`
(response cache, future offline metadata, and `playback.json`). Nothing outside
the root is written.

`cache/playback.json` is a 0600 snapshot of the queue, current index,
repeat/shuffle, volume, quality and position. The daemon rewrites it every two
seconds and on shutdown; on start it restores the queue in a stopped state, and
the next `resume`/`play` seeks to the saved position. Streams themselves are not
persisted.

## 10. Error taxonomy and mapping

| Domain error | IPC code | Exit | UX |
| --- | --- | --- | --- |
| `InvalidInput` | `bad_request` | 2 | Inline usage hint |
| `AuthenticationRequired` | `unauthorized` | 3 | “run `lekima login`” |
| `NotFound` | `not_found` | 4 | “no matches for …” |
| `Unsupported` | `unsupported` | 8 | “not available for this track” |
| `Unavailable` | `backend` | 5 | Retry hint, provider status |
| `Playback` | `playback` | 6 | Device hint + `doctor` |
| IPC unreachable | `daemon` | 7 | Autostart hint |
| Any other | `internal` | 1 | “this is a bug”, log path |

Rules: adapters never construct user-facing prose with format-string injection
from provider data; messages are structured and rendered by the frontend.

## 11. Observability

- `tracing` spans: `ipc.request`, `player.play`, `ncm.request` (URL path only,
  never query payloads with cookies), `audio.load`.
- Log levels via `RUST_LOG`; default `info`, `debug` adds state transitions,
  `trace` adds protocol frames with secrets redacted.
- `lekima doctor --json` reports: version/build, socket path & reachability,
  daemon pid/uptime, session present (boolean, never the value), audio device,
  provider reachability, config file errors.
- In-memory ring buffer (last 128 IPC calls) exposed by `daemon logs --json` for
  post-mortem without filesystem access.

## 12. Security & privacy

1. Session cookie is a bearer credential: file `0600`, directory `0700`, never
   in argv, never logged, redacted in error payloads, cleared by `logout`.
2. `lekima login` accepts stdin/file only (`--cookie-file -`), warns when a TTY
   is absent and stdin is empty.
3. Terminal injection: `lekima_domain::sanitize_line` strips C0/C1, ANSI
   escape sequences, bidi overrides and zero-width controls. Provider mapping
   applies it on the way in; the TUI and human CLI output also sanitize before
   writing. Unit-tested with hostile fixtures.
4. No telemetry, no analytics, no phone-home. Provider requests go only to
   NetEase hosts.
5. `#![forbid(unsafe_code)]` in every first-party crate.
6. `cargo audit` / `cargo deny` in CI; dependency review before release.
7. Provider downloads are capped and never written to a user-visible library;
   caching is metadata-only.

## 13. Performance budgets

| Metric | Budget | Measurement |
| --- | --- | --- |
| CLI cold start (`status`) | < 100 ms | hyperfine in CI benchmark |
| Key → screen update | < 16 ms | `TestBackend` frame timing |
| Daemon idle RSS | < 40 MB | CI on Linux |
| Daemon RSS while playing | < 80 MB (128 kbps), < 200 MB (lossless, spooled to disk) | manual |
| Provider search | < 800 ms p50 | tracing histogram |
| Socket round-trip | < 2 ms p99 local | tracing |
| Status poll rate | 2 Hz idle, 4 Hz visible, 0 when TUI hidden | code review |

## 14. Testing strategy

| Layer | Technique |
| --- | --- |
| Domain | Pure unit tests for queue/state edge cases; property tests for invariants |
| Core | Fake `MusicService` + fake `AudioEngine`; transition and failure-injection tests |
| NCM adapter | Fixture payloads → domain mapping; `wiremock` for transport errors; no live network in CI |
| IPC | Encode/decode golden tests; server loop tests with `tokio::io::duplex` |
| CLI | `assert_cmd` + `insta` snapshots for human and `--json` output; exit codes |
| TUI | `TestBackend` snapshot + reducer tests (key sequences) |
| E2E | Spawn daemon with fake provider + fake audio, drive CLI, assert JSON |
| Faults | Corrupt cookie, socket removed mid-call, provider 5xx, oversized frame, SIGTERM |
| CI | `fmt --check`, `clippy -D warnings`, tests on macOS + Linux, `cargo deny` |

Live-provider smoke tests are `#[ignore]` and run manually with a dedicated
account.

## 15. Packaging & release

- `cargo-dist` builds `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`.
- Homebrew tap, `cargo install lekima`, GitHub Releases with checksums.
- SemVer: protocol and CLI are public APIs. Breaking CLI/JSON change ⇒ major.
- MSRV policy: latest stable; documented in `Cargo.toml`.
- `ncmapi` releases are decoupled but listed in the changelog when a feature
  depends on a new version.

## 16. Architecture decision records

| ADR | Decision | Why |
| --- | --- | --- |
| 001 | Daemon + thin clients | Playback outlives UIs; agents need stateless invocations |
| 002 | Unix socket, JSON lines | Debuggable with `nc -U`, no schema compiler, forward compatible |
| 003 | ratatui + crossterm | Actively maintained, testable `TestBackend`, standard choice |
| 004 | rodio + symphonia | Pure-Rust decoding for mp3/aac/flac; no GStreamer dependency |
| 005 | Newtype ids | Compile-time safety across five id kinds |
| 006 | `ncmapi-rs` extended, not forked | Single client library, shared maintenance, published releases |
| 007 | Cookie-file login first, QR second | Unblocks v0.1 without new endpoints; QR is the polished path |
| 008 | Strict config parsing | Typos surface immediately instead of silently defaulting |
| 009 | Full-download playback in v0.1 | Fewest moving parts; streaming behind the same port in v0.2 |
| 010 | Poll status in v0.1, push events in v0.2 | Correctness first; protocol already supports unsolicited frames |

## 17. Failure modes

| Scenario | Behaviour |
| --- | --- |
| Daemon dies mid-TUI | TUI shows “daemon lost”, retries connect every 2 s, offers `R` to restart |
| Song URL 404/VIP | Toast, skip after 2 s (`--strict` stops instead) |
| Audio device disappears | Playback error, state → Stopped, hint to run `doctor` |
| Cookie expired | `unauthorized`, browsing still works, login hint |
| QR login returns 8821 | NetEase risk control: explain and point at `--cookie-file`; never retry in a loop |
| Provider 5xx/timeout | `backend`, no state corruption; last track keeps playing |
| Queue empty + `next` | `Ack`, state Stopped, exit 0 (not an error) |
| Two daemons raced | Advisory lock; loser exits 0 with pid of winner |
| TUI killed with SIGKILL | Terminal may be left in raw mode; `lekima doctor --fix` and `reset` documented |
| Malformed provider JSON | Mapping returns `Unavailable`, never panics |

## 17b. Sprint documents

The TUI's visual language is defined in
[`sprint-2/tui-visual-design.md`](./sprint-2/tui-visual-design.md): a HUD line,
a rule, dense lists and a command bar, with no boxed chrome.

Sprints 2–5 are shipped; their designs live in `sprint-2/` through `sprint-5/`.
No next increment is designed yet.

## 18. What exists today (honest status)

All nine crates build and are covered by unit, IPC, TUI render and CLI contract
tests (`cargo test --workspace`).

Working end to end against the live provider:

- search, stream URL resolution, playlist listing and paging, timed lyrics with
  optional translations
- daily recommendations, personal FM (prefetch/trash), likes, toplists and the
  music cloud
- playback (rodio/CoreAudio verified), pause/resume/stop/next/prev/seek/volume,
  repeat cycle, queue add/remove/move/jump/clear, auto-advance on track end
- streaming spool for large tracks with `loading`/`buffering` states, range
  seeks, gapless preload, sleep timer, live quality switching and output device
  selection
- OS media integration: MPRIS on Linux, Now Playing and media keys on macOS
- daemon lifecycle (autostart, socket permissions, SIGTERM, log file, autostart
  race handling), CLI JSON contract and exit codes, `doctor`, `capabilities`,
  completions, `login --qr`, `login --cookie-file -`, `logout`
- TUI: HUD, real spectrum, track table, sidebar, queue and lyrics panes, history,
  settings palette, twelve themes (including a light terminal palette), search
  prompt, `:` command line, help overlay, vim keys
- sequenced push events (`lekima watch`) and an MCP stdio server (`lekima mcp`)

The TUI polls status at 20 Hz while playing for the spectrum and also consumes
events for immediate track/queue/like updates.
