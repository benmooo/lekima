# lekima — Product Design

> NetEase Cloud Music in your terminal. Vim-first, agent-native.

Status: **Draft for v0.1** · Owner: @benmooo · Rewrite of the archived Go project
[`benmooo/lekima`](https://github.com/benmooo/lekima)

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

### v0.1 — “it plays” (current milestone)

| Area | Included |
| --- | --- |
| Session | QR login (`lekima login --qr`) or a pasted browser cookie (`--cookie-file -`), `whoami`, `logout` |
| Catalogue | Search tracks, playlists and artists; track metadata; playlist listing and full playlist paging; lyrics |
| Playback | Play/pause/resume/stop, next/previous, seek, volume, mute, repeat off/all/one, autoplay next |
| Queue | View, enqueue (append or “play next”), clear, jump |
| TUI | Now-playing bar with progress, track table, sidebar (playlists), search prompt, help overlay, vim keybindings, resize-safe layout |
| CLI | Every action above as a subcommand, `--json`, stable exit codes, daemon autostart |
| Ops | `doctor`, `daemon start/stop/status`, log file, `LEKIMA_CONFIG_DIR` for hermetic use |

### v0.2 — “it feels finished”

> Detailed specification: [`sprint-2/product-design.md`](./sprint-2/product-design.md)
> and [`sprint-2/system-design.md`](./sprint-2/system-design.md).

Synced lyrics pane, daily recommendations, personal FM, liked playlist,
like/unlike, playlist search, mouse-optional scroll, themes, `NO_COLOR`,
`lekima mcp` stdio server, push events from the daemon instead of polling.

### Later

Comments pane (“hot comments” was in the original), cloud disk, toplists,
downloads for offline cache, MPRIS/D-Bus + macOS Now Playing integration,
desktop notifications, Last.fm scrobbling, multiple profiles, plugin hooks,
`lekima serve --http` for LAN control, WASM/embedded agent tooling.

### Explicit non-goals

- GUI/TUI parity with the web player’s social features.
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
| `1`…`4` | Focus sidebar / main / queue / lyrics (v0.2) |
| `Q` | Show queue pane |
| `y` | Yank track URL/ID to clipboard (v0.2) |
| `P` | Toggle provider panel? (removed — `P` in the original was play mode) |

> Compatibility note: the archived app used `o`/`Enter` to expand the playlist
> tree. We keep `o` as “open” alias for muscle memory.

### 5.3 Interaction states

- **Empty**: “Nothing queued. Press `/` to search.”
- **Loading**: spinner in the header, skeleton rows, keys stay responsive.
- **No stream**: “This track has no playable stream (VIP or region).” The queue
  skips automatically after 2 s unless `--strict`.
- **Offline / provider down**: banner “offline — showing cached library”;
  playback of already-resolved URLs continues.
- **Unauthenticated**: browsing works where the provider allows it; play shows
  “login required” with the exact command to run.

### 5.4 Accessibility & terminal hygiene

- Every control is reachable by keyboard; no key requires a modifier combo that
  terminals commonly swallow.
- Colour is never the only signal (icons + text).
- `NO_COLOR`, `LEKIMA_THEME=light|dark|none`, and a monochrome-safe theme.
- Alternate screen is configurable; on exit the terminal is always restored
  (panic-safe guard).
- Catalogue text is sanitized: C0/C1 control characters and OSC/CSI sequences
  are stripped before rendering, preventing terminal escape injection from
  playlist or track names.

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

### Command surface (v0.1)

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
lekima lyrics [--id ID] [--json]
lekima daily [--play] | fm start|next|trash|stop | toplist list|show
lekima cloud [--limit N] [--play] | liked [--play] | like [--id] [--unlike]
lekima watch [--events status,queue,like] [--since SEQ]
lekima mcp                                  MCP server over stdio
lekima status [--json]
lekima login [--qr | --cookie-file PATH|-]
lekima logout
lekima whoami [--json]
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
tools for agents that prefer tool calls over shell.

## 7. Success criteria (v0.1)

- Cold install → first sound in < 30 s, documented in the README.
- TUI stays at ≥ 30 FPS while a search is in flight.
- 100% of TUI actions have a CLI equivalent (checked by a table-driven test).
- `lekima play … --json` works headless (no TTY) with the daemon running.
- Zero panics on malformed provider payloads (fuzz/property tests on mapping).
- Clean `cargo clippy -D warnings`, no `unsafe` (`#![forbid(unsafe_code)]`).
- Runs on macOS (arm64) and Linux (x86_64, Wayland/X11); Windows is best-effort.

## 8. Milestones

| Milestone | Contents | Exit criteria |
| --- | --- | --- |
| **M0 Skeleton** (done) | Workspace, domain, config, IPC, core player rules | `cargo test` green |
| **M1 Playback** | ncm adapter, rodio engine, daemon, CLI play/status | Sound from the CLI |
| **M2 TUI** | ratatui shell, keymap, search, queue, help | Daily-drivable |
| **M3 Agent surface** | JSON everywhere, capabilities, exit codes, doctor | Scripted demo passes |
| **M4 Provider completeness** | ncmapi-rs 1.1: QR login, playlist detail, lyrics (done); daily/FM/like next | TUI feature parity with old app |
| **M5 Release** | Packaging, docs, CI matrix, 0.1.0 tag | Installable from crates.io + release binaries |

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

## 10. Open questions

1. **Name.** The rewrite is called `lekima` for continuity; the binary is
   `lekima`. Confirm before the first publish (renaming a crate later is a
   breaking change).
2. **Default UI density.** Do we keep the original’s always-visible sidebar, or
   start in a single-pane “just the queue” mode?
3. **Comments pane.** Beloved in the original; expensive to maintain. v0.3?
4. **Lyrics rendering.** Synced highlighting requires a timed LRC parser — worth
   the dependency, or plain text first?
5. **Windows.** WSL only, or native named pipes in v1.0?
6. **Publishing.** `ncmapi` 1.1 is published; `lekima` 0.1 itself still needs its
   first crates.io release.
