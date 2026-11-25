# lekima — Product Design

> NetEase Cloud Music in your terminal. Vim-first, agent-native.

Status: **Living document** — shipped through 0.12.0 · Owner: @benmooo · Rewrite
of the archived Go project [`benmooo/lekima`](https://github.com/benmooo/lekima)

---

## 1. Why rewrite

The original lekima (Go, 2018–2022, archived) proved the idea: a terminal
NetEase Cloud Music client that feels like a vim buffer. It also accumulated the
usual debt of a 1.6k-line single-package program:

| Original constraint | Consequence today |
| --- | --- |
| `termui`/`termbox`, no layout engine | No resizing story, no real widgets, one theme |
| Global mutable `Lekima` struct, `panic` on errors | Any API hiccup kills the player; untestable |
| `faiface/beep` + full-body MP3 download | Long delay before playback, no seeking strategy |
| Phone/password login only | Passwords on disk, no QR, no cookie reuse |
| No machine interface | Nothing can drive it except a human at a keyboard |
| Go API client (`benmooo/ncmapi`) | Not reusable from the Rust ecosystem |

The rewrite keeps the soul (keyboard-driven, fast, quiet) and fixes the
foundation: a layered Rust workspace, a long-lived player daemon, and a first
class machine interface.

## 2. Positioning

**One-liner:** the music player for people who already live in a terminal — and
for the agents working beside them.

**Not** a GUI music app, not a downloader, not a streaming aggregator. One
provider, done well, with a UI that never gets in the way.

### Target users

1. **Terminal-native developers** — vim/neovim/helix users, tmux dwellers, tiling
   WM users. They want to search, queue, and skip without leaving the keyboard.
2. **Code agents** — Claude Code, OpenCode, Codex CLI, scripts. They need a
   stable, documented, non-interactive command surface and JSON output.
3. **The author** — a maintainer of `ncmapi-rs` who wants the crate exercised by
   a real, demanding consumer.

### Jobs to be done

- “Play something I like without opening a browser or a 300 MB Electron app.”
- “Keep playing while I work; let me skip with one key.”
- “While I code, have the agent put on my daily playlist and turn it down.”
- “I’m SSH’d into a box; can I still control the music on this machine?”
- “Script it: play my liked playlist at 09:00 on weekdays.”

## 3. Product principles

1. **Keyboard is the first-class input.** Every action has a key. No feature is
   mouse-only. Keys follow vim semantics (`j/k`, `g/G`, `/`, `:` , counts).
2. **Fast by default.** First playback in under 30 s from a cold install; key
   latency under 16 ms; the TUI never blocks on the network.
3. **Quiet and legible.** No splash screens, no ads, no notification spam.
   Errors are one line plus an actionable hint.
4. **Composable.** A daemon owns playback; the TUI and the CLI are equal clients.
   Anything the TUI can do, a shell can do.
5. **Honest about state.** “Paused”, “buffering”, “no stream available (VIP)” are
   distinct states, not a generic error.
6. **Privacy and safety.** Credentials never appear in logs or process args;
   untrusted catalogue text is sanitized before it reaches a terminal.
7. **Provider respect.** No DRM circumvention, no bulk downloading. Use the
   provider’s official play URLs and honour its session model.

## 4. Scope

### v0.1 — “it plays” (shipped in 0.1.0)

| Area | Included |
| --- | --- |
| Session | Assisted web login (`lekima login --qr`, falls back to a terminal QR) or a pasted browser cookie (`--cookie-file -`), `whoami`, `logout` |
| Catalogue | Search tracks, playlists and artists; track metadata; playlist listing, paging and editing (create, rename, describe, add/remove tracks, delete); lyrics |
| Playback | Play/pause/resume/stop, next/previous, seek, volume, mute, repeat off/all/one, autoplay next |
| Queue | View, enqueue (append or “play next”), clear, jump |
| TUI | Now-playing bar with progress, track table, sidebar (playlists), search prompt, help overlay, vim keybindings, resize-safe layout |
| CLI | Every action above as a subcommand, `--json`, stable exit codes, daemon autostart |
| Ops | `doctor`, `daemon start/stop/status`, log file, `LEKIMA_CONFIG_DIR` for hermetic use |

### v0.2 — “it feels finished” (shipped in 0.2.0)

> Detailed specification: [`sprint-2/product-design.md`](./sprint-2/product-design.md)
> and [`sprint-2/system-design.md`](./sprint-2/system-design.md).

Synced lyrics pane, daily recommendations, personal FM, liked playlist,
like/unlike, playlist search, mouse-optional scroll, themes, `NO_COLOR`,
`lekima mcp` stdio server, push events from the daemon instead of polling.

### v0.3 — “platform integration” (shipped in 0.3.0)

> Detailed specification: [`sprint-5/`](./sprint-5/).

MPRIS on Linux, Now Playing and media keys on macOS, output device selection,
range seeks for streamed tracks, and gapless playback for matching formats.

### v0.4 — “TUI polish” (shipped in 0.4.0)

> Detailed specification: [`sprint-6/`](./sprint-6/) and
> [`tui-backlog.md`](./tui-backlog.md).

Responsive HUD and tables, queue confirmation and movement, buffered-range
shading, history, settings, themes and terminal-text sanitization.

### v0.5 — “offline and comments” (shipped in 0.5.0)

> Detailed specification: [`sprint-8/`](./sprint-8/).

A 256 MiB LRU stream cache with `lekima cache`, cached-playback fallback and a
`CACHE` badge; public track comments in the CLI, MCP and a scrollable TUI pane.

### v0.6 — “assisted login” (shipped in 0.6.0)

Browser-assisted sign-in and a persisted device id, so sessions are created from
the terminal without pasting cookies.

### v0.7 — “playback hardening + control” (shipped in 0.7.0/0.7.1)

> Detailed specification: [`sprint-7/`](./sprint-7/).

Preview-duration truth, queue-end recovery, persistence and restart UX, plus
token-guarded HTTP control and the phone web remote. 0.7.1 fixes null comment
fields through `ncmapi 1.8.1`.

### v0.8 — “console surfaces” (shipped in 0.8.0)

The comments pane becomes a wrapped feed with likes, dates and replies; notices
and errors become bordered alert panels; an empty library shows the `LEKIMA`
wordmark with key hints.

### v0.9 — “catalogue depth” (shipped in 0.9.0, hardened in 0.9.1)

> Detailed specification: [`sprint-9/`](./sprint-9/).

Album and artist search, artist pages with albums, album pages with tracks, and
album play/queue across the TUI, CLI, MCP and HTTP.

### v0.10 — “daily-driver loop” (shipped in 0.10.0)

> Detailed specification: [`sprint-10/`](./sprint-10/).

Desktop notifications when the track changes, and a durable listening history:
`lekima history`, an MCP tool, `GET /v1/history`, and an `H` pane that replays
what you select. The log is bounded by `[history] capacity` and records each
play once, including repeat-one loops.

### v0.11 — “extensions” (shipped in 0.11.0, hardened in 0.11.1)

> Detailed specification: [`sprint-11/`](./sprint-11/).

Plugin hooks: a user process the daemon starts, with sequenced events on stdin
and a declared set of commands back on stdout. Hooks stay out of process
(ADR-015) and are guarded by capabilities (ADR-016).

The 0.11.1 patch makes buffered-range shading follow the decoder's own
byte-to-time curve, so variable-bitrate streams shade the bytes that are really
local instead of a constant-bitrate guess.

### v0.13 — “playlist editing” (unreleased)

> Detailed specification: [`sprint-12/`](./sprint-12/).

The first provider writes: create a playlist, rename or describe it, add and
remove tracks, delete it — from the CLI, MCP, HTTP, the TUI playlist pane and
`:playlist` commands, backed by `ncmapi 1.10`.

### v0.12 — “design system” (shipped in 0.12.0)

> Proposal and slices:
> [`proposals/2026-11-25-ux-design-system.md`](proposals/2026-11-25-ux-design-system.md).

One visual contract for every surface: twelve semantic colour roles with WCAG
floors, five type levels and one spacing scale; `lekima ui` for theme, glyphs,
density, motion and lyric offset; one list grammar across tracks, queue,
history and catalogue; unified empty states and alerts; explicit `NO_COLOR`,
`LEKIMA_COLOR` and reduced-motion behavior; and 36 text fixtures for review.

The HUD keeps its original single-row order (title, rail and clock, volume,
queue, modes, optional facts) and shortcut hints spell keys instead of reusing
concept glyphs.

### Closed candidates

The roadmap bucket is now empty. Closed by product decision:

- **Scrobbling / cloud listening history** — the history is a personal log on
  this machine; nothing is uploaded to an external account (ADR-014).
- **Windows** — macOS and Linux are the supported platforms (ADR-013).
- **WASM / embedded agent tooling** — no in-process plugin runtime (ADR-015).

### Explicit non-goals

- GUI/TUI parity with the web player’s social features.
- Uploading listening history anywhere: no scrobbling, no Last.fm, no cloud
  profile.
- Windows support, native or through WSL.
- An in-process plugin runtime or a WASM sandbox; extensions are external
  processes.
- Downloading copyrighted tracks for redistribution.
- Bypassing regional or VIP restrictions.
- Reimplementing the NetEase recommendation stack.

## 5. Experience

### 5.1 Screen model

> Visual language: [`sprint-2/tui-visual-design.md`](./sprint-2/tui-visual-design.md)
> (console/HUD, neon-on-dark, no boxed panels). The wireframe below describes
> information architecture; the styling lives in that document.


```
 LEKIMA  ┃ ▶ Bohemian Rhapsody · Queen  01:42/05:55  ████░░░░░░░░  R:OFF  VOL ████░░  Q:12
──────────────────────────────────────────────────────────────────────────────────────────
// LIBRARY 12          │ // QUEUE · 12 ▌
▌01 Late night         │   #     TITLE                            ARTIST              TIME
 02 Focus              │ ▌ 001 ▶ Bohemian Rhapsody                Queen               5:55
 03 Daily              │   002   Mota                             Russian Circles     6:32
                       │   003   Teardrop                         Massive Attack     5:30
[NORMAL] j/k nav · ⏎ play · space pause · / search · : cmd · Q queue · ? keys
```

- **Header / now-playing**: title, artist, album, progress, volume, repeat,
  shuffle, and the resolved bitrate. Updates at 4 Hz while playing.
- **Sidebar**: sections — My playlists, Daily recommendations, Personal FM,
  Search results. Collapsible, navigable.
- **Main pane**: a table for tracks (index, title, artist, album, duration,
  state marker) or a list for playlists.
- **Status bar**: mode (NORMAL / SEARCH / HELP), contextual hints, queue length,
  transient messages (“queued 3 tracks”, errors in red for 4 s).
- **Overlays**: search prompt (`/`), command line (`:`), help (`?`), confirm
  dialogs for destructive actions.

### 5.2 Keymap (v0.1)

Global — `q`/`Ctrl-c` quit, `?` help, `/` search, `:` command line,
`Tab` cycle focus, `Esc` close overlay/cancel.

| Key | Action |
| --- | --- |
| `j` / `k` | Move down / up |
| `Ctrl-d` / `Ctrl-u` | Half-page down / up |
| `g` / `G` | Top / bottom |
| `5j`, `12G` (v0.2) | Count prefix |
| `Enter` | Play selection (or open playlist/section) |
| `Space` | Pause / resume |
| `n` / `p` | Next / previous track |
| `+` / `-` | Volume up / down 5% |
| `m` | Mute toggle |
| `r` | Cycle repeat off → all → one |
| `s` | Toggle shuffle (v0.2) |
| `a` | Append selection to queue |
| `A` | Play next (insert after current) |
| `x` | Remove selected queued track (with confirm) |
| `l` / `h` | Seek forward / back 10 s (also `→`/`←`) |
| `Ctrl-w 1`…`4` | Focus sidebar / main / queue / lyrics (digits stay counts) |
| `Q` | Show queue pane |
| `C` | Show track comments |
| `H` | Show listening history |
| `y` | Yank track URL/ID to clipboard (v0.2) |
| `P` | Open / close the playlist pane |

> Search categories are glyphs on the prompt line itself — `♪` tracks, `▣`
> albums, `☺` artists, `≡` playlists — with the active one bracketed and `ALL`
> meaning no bracket. `Tab`/`Ctrl-1..5` cycle them while typing, and
> `Ctrl-1..5` re-runs the current query in the results view, where the header
> names the category. The same glyphs label the sidebar, the queue, history,
> lyrics and playlists headers.
>
> Compatibility note: the archived app used `o`/`Enter` to expand the playlist
> tree. We keep `o` as “open” alias for muscle memory.
>
> Queue mode is a peer view, not a modal: `Space`, `n/p`, `h/l`, `m`, `r`, `s`,
> `+/-` and `L` keep working while it is open, and `Enter` leaves it open. The
> selection follows the playing entry until `j/k` takes over.

### 5.3 Interaction states

- **Empty**: “Nothing queued. Press `/` to search.”
- **Loading**: the old track goes silent immediately; the header shows a
  loading glyph until the next stream starts. Keys stay responsive.
- **No stream**: “This track has no playable stream (VIP or region).” The queue
  skips automatically after 2 s unless `--strict`.
- **Preview stream**: when the decoded stream is shorter than catalogue metadata,
  show `PREVIEW` and progress against the real duration; advance once at its end.
- **Offline / provider down**: banner “offline — showing cached library”;
  playback of already-resolved URLs continues.
- **Unauthenticated**: browsing works where the provider allows it; play shows
  “login required” with the exact command to run.

### 5.4 Accessibility & terminal hygiene

- Every control is reachable by keyboard; no key requires a modifier combo that
  terminals commonly swallow.
- Colour is never the only signal (icons + text).
- `NO_COLOR`, `LEKIMA_THEME=light|dark|auto|<palette>` (twelve palettes,
  including a light-terminal theme), and a monochrome-safe theme.
- `LEKIMA_COLOR=truecolor|256|16|mono` downgrades the palette; `LEKIMA_MOTION`
  and `[ui] motion` freeze animation; selection is reverse video in mono.
- Alternate screen is configurable; on exit the terminal is always restored
  (panic-safe guard).
- Catalogue text is sanitized: C0/C1 control characters, ANSI/OSC sequences,
  bidi overrides and zero-width characters are stripped before rendering,
  preventing terminal escape injection from playlist or track names.

## 6. Agent contract

This is a product feature, not an afterthought. Rules:

1. **Non-interactive by default.** No prompts, no pager, no alternate screen for
   non-TUI commands.
2. **Machine output.** `--json` on every command. JSON goes to stdout; logs and
   diagnostics go to stderr. One JSON object per invocation.
3. **Stable vocabulary.** Command names, error codes and JSON fields are
   versioned; breaking changes require a major bump and a deprecation cycle.
4. **Bounded time.** Every command completes or fails within 30 s unless the
   user asks for `--wait`.
5. **Discoverable.** `lekima capabilities --json` returns the command catalogue,
   argument schema and examples; `lekima schema` returns the JSON schema.
6. **Daemon transparent.** Read commands work without a daemon; playback
   commands autostart one (unless `--no-daemon`) and report the PID.
7. **No secrets in argv.** `lekima login` reads the cookie from stdin or a file,
   not `--cookie=<secret>` on the command line (the flag accepts `-` for stdin).

### Command surface (v0.1; `lekima capabilities --json` has the current list)

```
lekima tui                                  open the interactive player
lekima play [QUERY] [--id ID]... [--playlist ID] [--shuffle] [--json]
lekima pause | resume | toggle | stop
lekima next | prev | seek <1:23|83s|+10s|-10s>
lekima volume [LEVEL|+5|-5] [--mute|--unmute] [--json]
lekima queue [list] [--json]
lekima queue add <QUERY|--id ID>... [--next]
lekima queue clear
lekima search <QUERY> [--kind track|album|artist|playlist] [--limit N] [--json]
lekima playlist list [--json]
lekima playlist show <ID> [--json]
lekima playlist create <NAME> [--description TEXT] [--private] [--json]
lekima playlist rename <ID> <NAME>
lekima playlist describe <ID> <TEXT>
lekima playlist add <ID> --ids <ID,ID>
lekima playlist remove <ID> --ids <ID,ID>
lekima playlist delete <ID> --yes
lekima lyrics [--id ID] [--json]
lekima daily [--play] | fm start|next|trash|stop | toplist list|show
lekima cloud [--limit N] [--play] | liked [--play] | like [--id] [--unlike]
lekima watch [--events status,queue,like] [--since SEQ]
lekima mcp                                  MCP server over stdio
lekima status [--json]
lekima login [--qr [--no-browser] | --cookie-file PATH|-]
lekima logout
lekima whoami [--json]
lekima serve --http [--bind ADDR]          HTTP control for phones/LAN

lekima ui [--theme NAME] [--glyphs SET] [--density MODE]
          [--motion auto|full|reduced] [--lyrics-offset MS]
          [--translation on|off] [--json]
lekima themes [--json]
lekima history [--limit N] [--json]
lekima daemon start|stop|status|logs
lekima doctor [--json]
lekima capabilities [--json]
lekima completions <shell>
```

### JSON envelope

```json
{ "ok": true, "data": { "status": "playing", "track": { "id": 123, "name": "…" } } }
```

```json
{ "ok": false, "error": { "code": "unauthorized", "message": "run `lekima login`" } }
```

Error codes are the single source of truth and map 1:1 to the daemon’s IPC
`ErrorCode`: `bad_request`, `unauthorized`, `not_found`, `unsupported`,
`backend`, `playback`, `daemon`, `internal`.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Unexpected internal error |
| 2 | Invalid usage (clap) |
| 3 | `unauthorized` |
| 4 | `not_found` |
| 5 | `backend` (provider/network) |
| 6 | `playback` (audio device) |
| 7 | `daemon` (unreachable and could not autostart) |
| 8 | `unsupported` |

### Example agent transcripts

```console
$ lekima play "mota" --json
{"ok":true,"data":{"status":"playing","track":{"id":…,"name":"mota"},"queue_length":12}}

$ lekima volume +10 --json
{"ok":true,"data":{"volume":0.8,"muted":false}}

$ lekima status --json | jq -r .data.status
playing
```

Roadmap: `lekima mcp` exposes `play`, `pause`, `next`, `search`, `status` as MCP
tools for agents that prefer tool calls over shell. `lekima serve --http` opens
the same command surface to phones and other machines, guarded by a bearer
token.

## 7. Success criteria (v0.1)

- Cold install → first sound in < 30 s, documented in the README.
- TUI stays at ≥ 30 FPS while a search is in flight.
- 100% of TUI actions have a CLI equivalent (checked by a table-driven test).
- `lekima play … --json` works headless (no TTY) with the daemon running.
- Zero panics on malformed provider payloads (fuzz/property tests on mapping).
- Clean `cargo clippy -D warnings`, no `unsafe` (`#![forbid(unsafe_code)]`).
- Runs on macOS (arm64) and Linux (x86_64, Wayland/X11). Those are the only
  supported platforms; Windows is not supported.

## 8. Milestones

| Milestone | Contents | Exit criteria |
| --- | --- | --- |
| **M0 Skeleton** (done) | Workspace, domain, config, IPC, core player rules | `cargo test` green |
| **M1 Playback** (done) | ncm adapter, rodio engine, daemon, CLI play/status | Sound from the CLI |
| **M2 TUI** (done) | ratatui shell, keymap, search, queue, help | Daily-drivable |
| **M3 Agent surface** (done) | JSON everywhere, capabilities, exit codes, doctor | Scripted demo passes |
| **M4 Provider completeness** (done) | QR login, playlist detail, lyrics, daily/FM/like/toplist | TUI feature parity with old app |
| **M5 Release** (done, 0.1.0) | Packaging, docs, CI matrix, tag | Installable from crates.io + release binaries |
| **M6 Playback depth** (done, 0.2.0) | Streaming spool, buffering, large files, sleep timer, quality switching | 0.2.0 shipped |
| **M7 Platform** (done, 0.3.0) | Device picker, range seeks, gapless, MPRIS/Now Playing | 0.3.0 shipped |
| **M8 TUI polish** (done, 0.4.0) | Responsive HUD/tables, queue UX, buffered shading, sanitization | 0.4.0 shipped |
| **M9 Offline cache + comments** (done, 0.5.0) | 256 MiB stream cache, comments CLI/MCP/TUI | 0.5.0 shipped |
| **M10 Assisted login** (done, 0.6.0) | Browser-assisted sign-in, persisted device id | 0.6.0 shipped |
| **M11 Control + hardening** (done, 0.7.x) | HTTP control, phone remote, preview truth, restart UX, null-safe comments | 0.7.1 shipped |
| **M12 Console surfaces** (done, 0.8.0) | Comments feed, alert panels, empty-state wordmark | 0.8.0 shipped |
| **M13 Catalogue depth** (done, 0.9.0/0.9.1) | Album/artist search, artist and album pages, album play/queue, error/search/history hardening | 0.9.1 released |
| **M14 Daily-driver loop** (done, 0.10.0) | Desktop notifications, durable listening history across CLI/MCP/HTTP/TUI | 0.10.0 released |
| **M15 Extensions** (done, 0.11.0) | Out-of-process plugin hooks, capability-checked commands, `doctor` reporting, VBR-accurate buffered ranges | 0.11.1 released |
| **M16 Design system** (done, 0.12.0) | `lekima ui`, measurable theme contract, one list grammar, unified surfaces, colour downgrade, reduced motion, text fixtures | 0.12.0 released |
| **M17 Playlist editing** (unreleased) | Create, rename, describe, add/remove tracks and delete, with `ncmapi 1.10` write routes | 0.13.0 |

## 9. Risks & mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| NetEase changes signing/anti-bot | Playback breaks | Keep transport isolated in `ncmapi`; contract tests; fast releases |
| VIP/region restrictions on stream URLs | “No URL” for many tracks | Distinguish `unsupported` from `backend`; skip-with-notice; document |
| Audio backend differences (CoreAudio, ALSA, Pulse) | Silence/crash | rodio + device enumeration in `doctor`; `--audio-device`; integration test on CI |
| Terminal escape injection via metadata | Security | Sanitize all external strings (unit-tested) |
| Daemon/CLI version skew | Weird behaviour | Protocol version field; client refuses unknown major |
| Cookie theft | Account compromise | `0600` session file, no logging, permission check on startup |
| Scope creep (social features) | Never ships | Non-goals above; milestones |

## 10. Decisions and open questions

1. **Default UI density — decided.** `compact` stays the default; `comfortable`
   is opt-in, persisted with `[ui] density`, and both are settable from
   `lekima ui`. The sidebar stays visible at 64 columns and wider because it is
   the library and playlist switcher, not decoration.
2. **Visual direction — decided.** The twelve palettes are frozen; the default
   `cyber` is polished rather than replaced, and every palette meets the
   documented contrast floors. See
   [`proposals/2026-11-25-ux-design-system.md`](proposals/2026-11-25-ux-design-system.md).
3. **Extensions.** Desktop notifications and listening history shipped in
   0.10.0, plugin hooks in 0.11.0, and the TUI design system in 0.12.0.
   Scrobbling, Windows and in-process sandboxing stay closed (ADRs 013–015).
   A future theme editor or new palette would need a new proposal and
   screenshots.
