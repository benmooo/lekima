# Changelog

All notable changes to lekima are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/) and semantic versioning.

## 0.10.0 — 2026-11-20

### Added

- Desktop notifications when the track changes. Opt in with
  `[notifications] on_track_change = true`; the daemon uses `osascript` on macOS
  and `notify-send` on Linux, never blocks playback on a slow helper, and
  degrades to a no-op when neither exists. `lekima doctor` reports the backend.
- Listening history: `lekima history [--limit N] [--json]`, an MCP `history`
  tool, `GET /v1/history?limit=N`, and an `H` pane in the TUI where `enter`
  replays a play. Records carry the track, start time, played time and a
  completion flag, and are bounded by `[history] capacity` (default 1000) in
  `<state>/history.json`.
- `lekima doctor` reports the history file, capacity, entry count and any load
  error, so a corrupt file is visible without reading logs.

### Changed

- `status --json` gains `generation`, the player's stream counter. Clients can
  use it to tell two plays of the same track apart; repeat-one and replay now
  produce separate history records because of it.
- A play is recorded when the track stops being current, and only if audio was
  actually heard, so a restored queue no longer invents 0 ms entries.

### Fixed

- A corrupt `history.json` is reported and left untouched instead of being
  rewritten as empty at startup; a newer on-disk format version is rejected
  rather than downgraded.

## 0.9.2 — 2026-11-18

### Added

- MCP gains `queue_album`, so agents can append an album or insert it after the
  current track like the CLI and HTTP surfaces.
- `preview --mode search|artist|album` renders catalogue fixtures for visual
  review without a daemon or account.

## 0.9.1 — 2026-11-18

### Fixed

- Provider `404` responses map to `not_found` (exit 4) instead of `backend`
  (exit 5), so agents can distinguish a bad id from a provider outage.
- Search filters apply to the results on screen; `Esc` restores the applied
  filter, in-flight searches show a loading state, and replies from an older
  query/kind are ignored.
- Artist, album and search pages that left the screen mid-request refetch when
  restored from history instead of showing a permanent spinner.
- Album search results include the artist and description from `ncmapi 1.9.1`.
- Catalogue footers degrade cleanly by width; album errors expose `r retry`.

## 0.9.0 — 2026-11-17

### Added

- Catalogue depth: `lekima search --kind all` merges tracks, albums, artists and
  playlists, and album results are decoded instead of dropped.
- Artist pages: `lekima artist show <id>` and `lekima artist albums <id>` return
  counts, aliases, biography and paginated albums.
- Album pages: `lekima album show <id>` returns metadata plus the ordered track
  list; `lekima album play <id>` and `lekima album queue <id> [--next]` drive the
  queue through the daemon.
- IPC gains `artist`, `artist_albums`, `album`, `queue_album`, `queue_playlist`
  and `PlayTarget::Album`. The additions are backward compatible; the protocol
  version stays 2.
- MCP gains the read-only `artist`, `artist_albums` and `album` tools, and
  `play` accepts an `album` target.
- The TUI gains search filters (`Tab`/`Ctrl-1..5`), grouped result pages, artist
  and album pages with paging and history, `:album`/`:artist` jumps, and album
  or playlist queueing from result rows.
- HTTP control allows the read-only artist and album commands; album playback
  and queueing stay local by default.

### Changed

- The provider pin moves to `ncmapi 1.9`, which exposes artist and album
  endpoints and album search results.
- Every published crate now shares the workspace version: the unchanged
  `lekima-platform` publishes at 0.9.0 instead of keeping its own 0.3.0 line.

## 0.8.0 — 2026-11-17

### Added

- The comments pane is now a console feed: a header with the total and
  `↓ MORE`, `HOT`/`ALL` sections with counts and notes, author-led entries with
  compact likes and UTC dates, width-wrapped bodies, hanging `↳` replies and
  dim `[deleted]` markers for removed content.
- The empty library shows a compact two-tone `LEKIMA` wordmark with actionable
  key hints, collapsing to one line on small panes and using a text wordmark
  in ASCII glyph mode.
- Notices and errors are console alerts: severity-coloured double borders and
  titles, readable body text, wrapped or ellipsized messages, and
  lyrics-aware placement.

## 0.7.1 — 2026-11-16

### Fixed

- Track comments no longer fail with `provider protocol error: JSON
  serialization or deserialization failed` when the provider returns `null`
  for deleted replies or accounts. Requires `ncmapi 1.8.1`, which tolerates
  null comment fields.

## 0.7.0 — 2026-11-16

### Added

- `lekima serve --http [--bind ADDR]` exposes the daemon over HTTP for phones
  and other machines: `GET /v1/status`, `GET /v1/queue`, `GET /v1/health`,
  `GET /v1/capabilities` and `POST /v1/command` with the IPC command shape.
- A built-in mobile web remote at `/` with the TUI's neon console palette:
  transport controls, seek rail, volume, repeat/shuffle and a jumpable queue.
  It asks for the bearer token once and stores it in `localStorage`.
- A bearer token in `<config>/http.token` (`0600`) is generated on first start;
  `LEKIMA_HTTP_TOKEN` overrides it. Requests use constant-time comparison.
- The HTTP surface is default-deny: `login`, `logout`, `shutdown`, `hello` and
  `subscribe` are not exposed, and command bodies are capped at 1 MiB.

## 0.6.0 — 2026-11-16

### Added

- `lekima login --qr` opens the official web player in Chrome, Chromium, Brave
  or Edge with a dedicated profile (`<config>/web-login`), opens the login
  dialog automatically, waits for the user to sign in, captures the session
  cookies over the DevTools protocol and closes the browser. `--no-browser`
  keeps the terminal QR flow.
- A stable anonymous device id in `<config>/device.id`, reused across commands
  instead of generating a new device per process.

### Changed

- Provider requests send a browser `User-Agent`, `Origin` and `Accept-Language`,
  matching a normal web client more closely (`ncmapi` 1.8).
- `lekima logout` also removes the dedicated browser profile.

### Fixed

- `ncmapi` 1.8 creates `session.cookies` and `device.id` with `0600`
  permissions and creates missing parent directories.

## 0.5.2 — 2026-11-16

### Fixed

- A fresh install no longer fails `lekima login --qr` with "session storage
  failed … No such file or directory". The CLI creates `<config>` and
  `<config>/cache` on startup, so the provider can write `session.cookies`
  before the daemon has ever run.

## 0.5.1 — 2026-11-16

### Changed

- Track comments use `ncmapi` 1.7's `Library::comments` instead of a hand-rolled
  HTTP request. `lekima-ncm` no longer depends on `reqwest` or `serde`
  directly, and the workspace pin moves to `ncmapi` 1.7.

## 0.5.0 — 2026-11-15

### Added

- `PlaybackState.stream_duration_ms`: the decoder's real duration when a
  provider preview is shorter than the catalogue metadata. The HUD and human
  status output show a `PREVIEW` badge and progress to the real end.
- Queue-end recovery: the last entry stays selected when playback stops, so
  `resume`/`space` reloads it instead of failing on a missing current track.
- Queue persistence: the daemon writes `cache/playback.json` (0600) every two
  seconds and on shutdown, then restores the queue, index, repeat/shuffle,
  volume, quality and resume position on start.
- TUI restart UX: offline polling continues at 1 Hz, reconnects automatically,
  and announces `daemon restarted - queue restored - space to resume`.
- Offline stream cache: `[cache] enabled/limit_mb` (256 MiB default),
  `<config>/cache/streams`, LRU eviction, `lekima cache status|clear|limit|on|off`,
  cached playback with `PlaybackState.cached` / `CACHE` badge, and offline
  fallback when the provider cannot resolve a URL.
- Comments: `lekima comments`, MCP `comments`, TUI `C` pane with hot/all
  sections, replies, scrolling, refresh and sanitized provider text.

### Changed

- Status event frames box their `PlaybackState` payload so the event enum stays
  small after the new status fields.

## 0.4.1 — 2026-11-15

### Fixed

- Gapless boundary sources now fire once per source. Previously the shared
  boundary slot could be re-armed on every sample after the daemon consumed it,
  resetting `position_ms` to zero roughly twice a second while a preloaded track
  played — the progress and synced lyrics jumped back to the start.
- Switching to a new track resets the playback position before the stream URL
  is resolved. An incomplete preview of A no longer makes B briefly report A's
  last position and then snap to zero.
- Status replies now carry a daemon revision, and the TUI ignores a poll that
  was answered before a newer event. This prevents the track, progress and
  lyrics from flickering back to the previous song at a transition.

## 0.4.0 — 2026-11-15

### Added

- TUI: width-budgeted HUD with `LOADING`/`BUFFERING` and `MUTE` labels, buffered
  range shading on the progress rail, a one-line notice chip, stacked lyrics at
  mid widths, adaptive track columns, list position cues, an inactive selection
  style, a confirmed queue delete, count prefixes (`5j`, `12G`), `Ctrl-w 1..4`
  pane keys and a prompt editor with word delete and per-mode history.
- `PlaybackState.buffered`: daemon-provided buffered ranges, mapped from the
  sparse spool by `AudioEngine::buffered_ranges`.
- TUI: a scrollable, two-column help overlay generated from a single keymap
  table; it fits 80×24 and shows every binding, including queue editing and
  daemon start.
- TUI: `light` palette for light terminals, `LEKIMA_THEME=light|dark|auto|<name>`
  and ASCII glyphs for separators, overlays, queue markers and the footer.
- `lekima-domain::sanitize`: shared terminal-text sanitizer for control
  characters, ANSI/OSC sequences, bidi overrides and zero-width characters,
  applied by the NCM mapping layer, the TUI and human CLI output.
- `preview` example accepts `--width`, `--height`, `--theme`, `--glyphs`,
  `--density`, `--mode` and `--long-title`.

### Fixed

- Queue mode now keeps the transport controls (`Space`, `n/p`, `h/l`, `m`,
  `r`, `s`, `+/-`, `L`) instead of only navigation and editing.
- Gapless preloads are idempotent and validated against the queue: a second
  preload for the same position is ignored, and a boundary from a stale
  preload stops the stale audio and continues with the queue's next entry
  instead of replaying the track that just ended.
- Gapless boundaries carry the **track id** of the preloaded source. A boundary
  for the already-current track is ignored (the `n`/rollover race), a boundary
  for the queue's next track advances, and any other boundary is stale.
- FM ignores repeat modes when advancing: a finite FM batch stops instead of
  wrapping repeat-all back to old tracks, and `fm next --trash` does not skip a
  second entry if the track ended while the trash request was in flight.
- FM prefetch now publishes a queue event when it appends a new batch, so open
  queue views and `lekima watch` clients see the newly fetched songs instead of
  a stale list.
- Starting or stopping a load clears the audio boundary flag, so a boundary
  fired just before a manual jump can no longer skip tracks (the cause of
  lyrics and audio pointing at different songs).
- Queue mode now follows the current entry until the user moves the selection,
  shows the remaining time, and keeps the queue open after `Enter` so playback
  controls stay available.
- The settings `TRANSLATION` row now toggles translation instead of changing
  density.
- The HUD truncates the title instead of clipping the clock, volume, queue and
  mode badges on narrow terminals.
- Sidebar and list truncation now counts terminal cells, so CJK and emoji
  titles keep their alignment; ASCII mode no longer leaks `▶`, `⏎`, `♥`, `…`,
  `·` or heavy separators.
- Playlist numbering stays aligned past 99 entries.
- Help no longer overflows common terminals, and `ctrl-d/u` is a real
  half-page motion.
- MCP `tools/list` now advertises `queue_remove`, `queue_move`, `sleep` and
  `set_quality`; the handlers already existed but were undiscoverable.

## 0.3.0 — 2026-11-14

### Added

- Output device selection: `lekima devices`, `lekima audio-device`, persisted
  `[audio] device`; the daemon opens that device and reports a clear error if it
  disappears.
- Gapless playback for consecutive small tracks: preload + queue append +
  boundary event; large tracks keep the normal load path.
- Range seeks for streamed tracks: the spool tracks downloaded byte ranges, a
  background range worker fetches the seek target, and the decoder runs on its
  own thread so the audio callback never blocks.
- OS media integration (`lekima-platform`): MPRIS on Linux
  (`org.mpris.MediaPlayer2.lekima`) and macOS Now Playing + media keys via a
  `lekima media-host` helper with an accessory AppKit run loop; `doctor` reports
  the backend.

### Fixed

- Auto-advance now fires when the sink empties: the engine clears `has_track`
  before `tick`, so the old gate (`finished && has_track`) never advanced.
  Repeat/all and plain queues both continue.

### Changed

- HUD keeps a single row: playhead progress, `[⟲ ALL]` / `[⇄]` badges, time,
  volume, queue, quality, sleep and a compact 12-column spectrum.
- Real audio visualisation: 20-band FFT spectrum (1024-point, Hann window,
  attack/decay smoothing) replaces the RMS history; TUI polls at 20 Hz.

### Known limitations

- MPRIS is Linux-only; macOS media keys require the bundled `media-host`
  helper process and artwork is not published yet.
- Gapless is guaranteed only for consecutive tracks at the same sample rate
  that fit the preload budget (small tracks).

## 0.2.0 — 2026-11-14

### Added

- Streaming playback: tracks over 64 MiB spool to an anonymous temp file and
  start after a small buffer; the 1 GB cloud WAV now plays with flat memory.
- `buffering` state mid-track; `loading` while the first samples arrive.
- Sleep timer (`lekima sleep 30m`, `:sleep`, MCP) with status/HUD countdown.
- Live quality switching (`lekima quality`, `:quality`, MCP), persisted.
- Seek within the buffered region; explicit error beyond it.

### Known limitations

- Seek beyond the buffered window and HTTP Range restart are not implemented.
- No gapless playback, device picker, MPRIS or Now Playing yet.

## 0.1.1 — 2026-11-14

### Changed

- `ncmapi 1.6` with pure-Rust protocol crypto: no OpenSSL anywhere in the
  dependency graph, byte-identical request encryption. Release binaries are
  portable (no Homebrew OpenSSL linkage) and cross-compiled on macOS.

## 0.1.0 — 2026-11-14

### Added

- Daemon-owned playback with a vim-first TUI and a stable JSON CLI.
- Search, playlists, daily mix, personal FM (prefetch/trash), likes, charts and
  the music cloud.
- Timed lyrics pane with offset and translation toggle.
- Daisy-chained themes (cyber, amber, mono, dracula, gruvbox, nord, synthwave,
  matrix, solarized, tokyonight, catppuccin), glyph and density variants, and
  an in-TUI settings palette that persists on exit.
- Back/forward history in the middle pane (`Ctrl-o` / `Ctrl-i`).
- Queue editing: remove and move tracks from the TUI, CLI and MCP.
- Real RMS waveform in the HUD; loading state and instant stop between tracks.
- Push events (protocol v2) with replay ring; `lekima watch` NDJSON stream.
- MCP stdio server (`lekima mcp`) with 15 tools.
- Cookie and QR session login; `doctor`, completions, capabilities.

### Fixed

- QR login now uses the eapi type=3 flow; non-idempotent requests bypass the
  response cache.
- Provider models tolerate null fields and duplicate chart keys.
- Backward seeks work (seekable rodio decoder).
