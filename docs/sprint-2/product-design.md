# Sprint 2 — Product Design

> Theme: **Provider depth, live UI, agent-native control**
> Status: **Proposed** · Depends on: lekima 0.1 (sprint 1), ncmapi 1.1 (shipped)

Sprint 1 delivered the skeleton that matters: a daemon-owned player, a
keyboard-first TUI, a stable JSON CLI, and provider coverage for search,
playlists and lyrics. Sprint 2 closes the remaining feature gap with the
archived Go player and makes the experience feel finished rather than
functional.

---

## 1. Sprint goal

**A user can live in lekima for a full day without touching another player, and
an agent can drive every new action through the same stable contract.**

Concretely, at the end of this sprint:

- `lekima tui` opens on a **Home** view with Daily recommendations, Personal FM,
  Toplists and the liked playlist one key away.
- The now-playing bar updates **without polling** (sub-50 ms after a track
  changes, including changes initiated by another shell or agent).
- Lyrics are a **pane, not a command**: synced highlighting, translation toggle,
  offset adjustment, no layout breakage at small sizes.
- `lekima mcp` exposes the player as MCP tools so an agent can call `play`,
  `pause`, `next`, `search`, `status`, `lyrics` directly.
- Every new capability exists in three places with identical semantics: TUI key,
  CLI subcommand, JSON/event payload.

## 2. Users and jobs

| User | Job this sprint | Current pain |
| --- | --- | --- |
| Terminal developer | “Put on my daily mix and keep it going while I code.” | Daily/FM missing; must search manually |
| Long-session listener | “Show lyrics while a track plays, without leaving the queue.” | `lekima lyrics` replaces the screen and blocks |
| Agent operator | “React to track changes; keep a queue fed.” | The agent has to poll `status` every 500 ms |
| MCP-native agent | “Call the player as tools, not shell commands.” | No MCP server; shell quoting is brittle |
| Curator | “Like this track, and later find everything I liked.” | No like action, no liked list |

## 3. Principles carried forward

1. Keyboard first; every new feature gets a key and a hint.
2. One writer: only the daemon mutates state; UIs render events.
3. Stable agent contract: additive changes only within 0.x; error codes and exit
   codes are frozen.
4. Nothing blocks the UI thread; loading is a state, not a freeze.
5. Respect the provider: likes/FM/trash are user-initiated actions only, no
   background automation.

## 4. Scope

### 4.1 In scope

| # | Feature | Why now |
| --- | --- | --- |
| S2-1 | Daily recommendations | The single most-used entry point in the old app |
| S2-2 | Personal FM (start, next, trash) | Original feature; also exercises infinite queues |
| S2-3 | Like / unlike + liked playlist | Small surface, high daily value |
| S2-4 | Toplists (list + detail) | Discovery without an account |
| S2-5 | Lyrics pane (synced, translated, offset) | Lyrics exist in the API; the pane is the missing UI |
| S2-6 | Daemon push events + `lekima watch` | Removes polling; enables agent reactions |
| S2-7 | MCP server (`lekima mcp`) | The explicit agent-native ask |
| S2-8 | Polish pass | Themes/`NO_COLOR`, counts, yank, loading states, hints |

### 4.2 Out of scope (sprint 3 candidates)

- Comments pane, cloud disk, downloads/offline library.
- MPRIS / macOS Now Playing, desktop notifications, scrobbling.
- Multi-account, social features, podcast/video.
- Windows named pipes (WSL + TCP localhost remain the story).

## 5. Feature specifications

### S2-1 Daily recommendations

**Story:** As a listener, I press `d` and get today's personalised list, already
playing.

- Source: `GET/POST /api/v3/discover/recommend` (weapi). Returns `recommend`
  (playlists), `dailySongs` and `recommendReasons`.
- Home view shows a **Daily** row with the first 5 tracks and “+N more”.
- `Enter` plays from the selected track to the end of the list.
- Cached for 30 minutes per account; `Ctrl-r` forces a refresh.
- Unauthenticated users see “Daily needs a session — `lekima login --qr`”
  instead of an error toast.
- CLI: `lekima daily [--limit N] [--json]`, `lekima play --daily`.

**Acceptance:** pressing `d` twice within 30 min does not refetch; the list is
stable across daemon restarts within the TTL; the JSON payload contains
`source: "daily"` so agents can tell queues apart.

### S2-2 Personal FM

**Story:** As a listener, I press `F` and get an endless, personalised station I
can steer with next/trash.

- Source: `/api/v1/radio/get` returns a batch (typically 3 tracks); `/api/radio/trash`
  removes the current track from the station.
- FM is a **queue mode**, not a playlist: when fewer than 2 tracks remain, the
  daemon prefetches the next batch in the background.
- `n` (next) skips without trashing. `x` in the FM pane **trashes** the current
  track and advances (confirm dialog, session-scoped undo).
- Leaving FM (playing anything else) ends the mode; the batch cache is kept for
  the session.
- Playback failure during FM skips after one retry and logs a structured event.
- CLI: `lekima fm start|next|trash|stop`, `lekima play --fm`.

**Acceptance:** 20 consecutive “next” presses never empty the queue while the
network is healthy; trash removes the track for the session and is never
re-added from the same batch; `lekima status` reports `queue.kind = "fm"`.

### S2-3 Like / unlike

**Story:** As a listener, `f` likes the current track; `f` again unlikes it.

- Source: `/api/radio/like?alg=itembased&trackId=<id>&like=<true|false>&time=3`.
- The track list shows a `♥` marker for liked tracks in Daily, FM, playlists and
  search results.
- State is **set-state**, not toggle, in the API: `lekima like --id 123 --value
  true`; the TUI toggles based on cached state.
- Optimistic UI: the marker flips immediately; on failure it rolls back and
  shows “couldn’t update like”.
- The liked set is cached per session (`/api/likelist?uid=…`) and invalidated on
  every successful change.
- `lekima liked [--json]` plays/lists the liked playlist.
- Requires a session; otherwise `unauthorized` with the login hint.

**Acceptance:** like → restart daemon → the marker is still correct; concurrent
likes from two clients converge (last write wins, no phantom toggles); the API
never receives a like action the user did not initiate.

### S2-4 Toplists

**Story:** As a listener, `t` opens the charts and `Enter` plays one.

- Source: `/api/toplist` (list) and existing playlist detail/paging for entries.
- Home sidebar section “Toplists” with the 8–12 official charts.
- Cached 15 minutes; no session required.
- CLI: `lekima toplist [list]`, `lekima toplist show <id> [--play]`.

**Acceptance:** works logged out; chart order matches the provider; opening a
chart never blocks the TUI (loading row shown).

### S2-5 Lyrics pane

**Story:** As a listener, `L` opens lyrics beside the queue and the current line
lights up.

- LRC parsing moves into `lekima-domain` as a pure value type:
  `Lyrics { lines: Vec<LyricLine { at: Duration, text: String }> }` plus optional
  translation per line.
- Pane layout: right column (40% width) on wide terminals; full-screen overlay on
  narrow ones (`< 100` cols).
- Current line is highlighted, auto-scrolled, with a 1-line look-ahead.
- `[` / `]` shift the offset by ±250 ms, persisted per track for the session.
- `t` inside the pane toggles translation; `w` wraps long lines.
- No lyrics ⇒ “No lyrics for this track” placeholder, pane stays open.
- Cached in memory (LRU, 64 tracks) and on disk under `cache/lyrics/` for 30
  days (plain text only, no account data).
- CLI: `lekima lyrics [--id ID] [--offset ±ms] [--translate] [--json]` returns both
  raw LRC and parsed lines.

**Acceptance:** parser handles metadata tags, multiple timestamps per line,
blank lines, translation-only lines; highlighting never drifts more than 250 ms
from the audio position over a 5-minute track; the pane does not reflow the
track table at 120×40.

### S2-6 Push events + `lekima watch`

**Story:** As an agent, I subscribe once and react to track changes without
polling.

- Protocol v2 adds `hello` (version negotiation), `subscribe` (`status`,
  `queue`, `track`, `likes`), `unsubscribe`, and unsolicited `event` frames.
- Coalescing: status events at most 4 Hz; queue/like events are immediate;
  every event carries a monotonically increasing `seq` and the last `seq` is
  replayable via `since`.
- Slow subscribers: status events are dropped (latest wins); state-bearing
  events are never dropped.
- TUI switches to `subscribe`; it falls back to 500 ms polling against a v1
  daemon (detected via `hello`).
- `lekima watch [--events status,queue] [--json]` prints NDJSON frames; exits on
  SIGINT. `--since <seq>` replays from the daemon’s ring buffer (last 256).
- Event payloads reuse the domain types (`PlaybackState`, `Track`) — no parallel
  schema.

**Acceptance:** a track change initiated by `lekima next` in another shell
reaches a watching agent in < 50 ms p99 locally; a stalled reader does not grow
daemon memory (ring buffer bounded); v1 clients keep working unchanged.

### S2-7 MCP server

**Story:** As an MCP-capable agent, I add `lekima mcp` and call player tools.

- `lekima mcp` speaks MCP over stdio (JSON-RPC 2.0), starts no daemon of its own
  but connects to the existing socket (autostart allowed unless disabled).
- Tools: `player_status`, `search`, `play`, `pause`, `resume`, `next`, `previous`,
  `set_volume`, `queue_add`, `queue_list`, `lyrics`, `daily`, `fm`.
- Every tool returns the same JSON `data` payload as the CLI subcommand, plus a
  one-line `content` summary for chat UIs.
- Errors map to MCP `isError` with the stable error code in `structuredContent`.
- Read-only tools are advertised separately so hosts can auto-approve them.
- No secrets cross the MCP boundary; the server holds no session state.

**Acceptance:** `npx @modelcontextprotocol/inspector` lists the tools; calling
`play` with a query produces audible playback and a JSON result; the server
survives a daemon restart (reconnects on the next call).

### S2-8 Polish

- Theme config: `dark` (default), `light`, `mono`, `NO_COLOR`.
- Vim counts: `5j`, `3G`; `gg`/`G`; `Ctrl-o`/`Ctrl-i` jump list (stretch).
- `y` yanks `title — artist  https://music.163.com/song?id=<id>` (ID when no URL).
- Loading/empty/error states for every pane; spinner in the header only.
- Hint bar becomes contextual per pane and mode.
- `lekima doctor` gains provider endpoint checks and cache sizes.

## 6. UX details

### 6.1 Home view

```
 LEKIMA  ┃ ▶ Cigarettes and lighters · LoMo  00:42/01:49  ████░░░░░░  R:ALL  VOL ████░  Q:28
──────────────────────────────────────────────────────────────────────────────────────────
// LIBRARY 12          │ // DAILY · 28 TRACKS ▌
 01 Late night         │ 001 ▶ Cigarettes and lighters   LoMo          1:49  ♥
 02 Focus              │ 002   Mota                      Russian...    6:32  ♥
 03 Daily              │ 003   Teardrop                  Massive...    5:30
 04 Toplists           │
[NORMAL] j/k nav · ⏎ play · d daily · F fm · t toplists · f like · L lyrics
```

### 6.2 Keymap additions

| Key | Context | Action |
| --- | --- | --- |
| `d` | Normal | Open Daily |
| `F` | Normal | Start Personal FM |
| `t` | Normal | Open Toplists |
| `f` | Normal (track) | Like / unlike current or selected |
| `L` | Normal | Toggle lyrics pane |
| `[` / `]` | Lyrics | Offset −250 / +250 ms |
| `t` | Lyrics | Toggle translation (else open charts) |
| `w` | Lyrics | Toggle wrap |
| `x` | FM | Trash current track (confirm) |
| `u` | FM | Undo last trash (session) |
| `5j`, `3G` | Normal | Count prefix |
| `y` | Normal | Yank track label + URL/ID |
| `Ctrl-r` | Normal | Refresh current section (bypass cache) |
| `Ctrl-o`/`Ctrl-i` | Normal | Jump back / forward (stretch) |

`:` commands: `daily`, `fm`, `like`, `liked`, `toplist [id]`, `lyrics`,
`watch`, `theme <name>`.

### 6.3 Pane routing

| Terminal width | Lyrics pane |
| --- | --- |
| ≥ 120 cols | Right column, 40%, queue stays visible |
| 100–119 | Right column, 32% |
| < 100 | Full-screen overlay (`L` toggles, `Esc` closes) |

## 7. Agent contract additions

New commands (all support `--json`):

```
lekima daily [--limit N]
lekima fm start|next|trash|stop
lekima like [--id ID | --current] [--value true|false]
lekima liked
lekima toplist [list] | lexima toplist show <ID> [--play]
lekima lyrics [--id ID] [--offset ±ms] [--translate]
lekima watch [--events status,queue,track,likes] [--since SEQ]
lekima mcp
```

Stable additions to JSON:

```jsonc
// status
{"ok":true,"data":{"status":"playing","queue":{"kind":"daily","length":28}, ...}}

// like
{"ok":true,"data":{"track_id":123,"liked":true}}

// watch (NDJSON, one frame per line)
{"seq":42,"event":{"type":"track_changed","track":{...}}}
```

Rules: `like` is idempotent set-state; `fm trash` is idempotent per
track+batch; `watch` exit code is 0 on SIGINT; new error code
`not_subscribed` is **not** introduced (unknown events are ignored).

## 8. Metrics and acceptance

| Metric | Target |
| --- | --- |
| Event latency (daemon → subscriber, local) | p99 < 50 ms |
| FM “next” time to first audio | p95 < 800 ms (prefetched batch) |
| Lyrics highlight drift | < 250 ms over 5 min |
| TUI frame time with lyrics open | < 8 ms at 120×40 |
| Daily cache hit rate in a session | > 90% |
| Agent demo (MCP): play → status → next → volume | passes in CI harness |
| New JSON commands covered by contract tests | 100% |

## 9. Risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Provider endpoints change (daily/FM/like) | Features break | All routes isolated in ncmapi 1.2 + contract tests with fixtures; graceful `unsupported` |
| FM batch repeats | Feels broken | Dedupe by track id per session; trash-aware filtering |
| Event protocol drift | Agents break | Versioned `hello`; additive event types; golden protocol tests |
| Lyrics timing varies by source | Highlight drift | User offset; clamp; unit tests with real fixtures |
| MCP host quirks | Integration pain | Inspector-based integration test; minimal dependency surface |
| Scope creep (comments/downloads) | Slip | Explicitly out of scope |

## 10. Rollout

1. ncmapi 1.2 (daily, FM, like, toplist) published and pinned.
2. Daemon protocol v2 behind negotiation; v1 clients untouched.
3. TUI features behind keymap; no flag needed (additive).
4. `lekima watch` and `lekima mcp` documented with copy-paste examples.
5. Release 0.2.0 with a migration note: none required, all additive.

## 11. Open questions

1. Should `f` like the **playing** track or the **selected** track? Proposal:
   selected when the track list has focus, playing otherwise; show which in the
   hint bar.
2. Is `x`-to-trash on FM discoverable enough, or should trash live behind `:fm
   trash` only? Proposal: keep `x` with a confirm dialog and undo.
3. Lyrics translation: show side-by-side or replace the original? Proposal:
   replace with `t`, keep original in `data` for agents.
4. Should `lekima watch` be NDJSON by default even without `--json`? Proposal:
   yes — it is inherently machine output; humans read it through `jq`.
5. MCP tool naming: prefix with `lekima_`? Proposal: no prefix; the server is
   already namespaced by the host.
