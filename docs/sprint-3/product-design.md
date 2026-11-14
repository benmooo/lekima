# Sprint 3 — Product Design

> Theme: **Ship 0.1** — turn a working local player into an installable product.
> Status: **Approved** · Builds on sprint 1/2 · Repo: `benmooo/lekima`
> (the archived Go project becomes `benmooo/lekima-go`)

---

## 1. Why this sprint

The player works: playback, library, discovery, cloud, lyrics, events and MCP.
But it has never left this machine — no remote, no CI, no LICENSE, no release.
A product nobody can install is still a prototype. This sprint makes `lekima`
real for three audiences:

| Audience | Job | Today |
| --- | --- | --- |
| New user | “Install it and play music in under a minute.” | Clone + build + cargo mirror workarounds |
| Maintainer | “Cut a release without manual archaeology.” | No CI, no changelog, no tag process |
| Agent host | “Add the MCP server from a README snippet.” | Works, but undiscoverable |

## 2. Goals

1. **One-command install** on macOS and Linux: Homebrew tap, `cargo install`,
   GitHub release binaries with checksums.
2. **Proof it works**: CI on macOS + Linux running fmt, clippy, tests,
   dependency audit; a no-network integration suite.
3. **A release process** documented and repeatable: version, changelog, tag,
   publish, binaries, tap bump.
4. **Two small product gaps** so 0.1 feels complete: queue editing and lyric
   translation.
5. **Trust documents**: LICENSE, SECURITY, CONTRIBUTING, install/first-run/agent
   docs, issue templates.

## 3. Non-goals (next sprints)

- Progressive streaming / large lossless playback.
- Album/artist pages, playlist create/edit, downloads/offline, comments.
- MPRIS/Now Playing, notifications, scrobbling, Windows.
- Social features, listen-together, podcasts/videos.

## 4. Scope

### 4.1 Distribution

| Channel | Contents |
| --- | --- |
| GitHub | `benmooo/lekima` (Rust), archived `benmooo/lekima-go` (old Go history), tag `v0.1.0`, release notes |
| Binaries | `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `.sha256` |
| crates.io | `lekima 0.1.0` (name verified free), depends on published `ncmapi 1.5` |
| Homebrew | `benmooo/homebrew-tap` formula `lekima` (bottle-less, binary download) |
| Docs | README quickstart, `docs/install.md`, `docs/first-run.md`, `docs/agents.md` |

Release notes must state: unsigned macOS binaries (Gatekeeper `xattr` note),
Linux ALSA dependency, proxy caveat for NetEase risk control.

### 4.2 CI and quality gates

- Matrix: macOS 14 (arm64), Ubuntu 24.04 (x86_64).
- Jobs: `fmt --check`, `clippy --all-targets -D warnings`, `test --workspace`,
  MSRV (1.98) build, `cargo deny check`, no-network test suite.
- Linux job installs `libasound2-dev`, `pkg-config`.
- Cache cargo registry/target; concurrency cancels superseded runs.
- Release workflow on `v*` tags builds the four targets and attaches binaries.

### 4.3 Reliability

- **Integration harness** (`crates/lekima/tests/e2e.rs`):
  - temp `LEKIMA_CONFIG_DIR` and socket;
  - fake `MusicService` + fake `AudioEngine` compiled into a test daemon binary
    or selected via an env-gated backend;
  - drive every CLI command, assert JSON envelopes and exit codes;
  - events: subscribe, mutate, assert sequence order;
  - MCP: initialize, tools/list, successful and failing tools/call.
- `doctor` additions: binary path/version, config parse errors, socket perms,
  session presence, audio device, provider reachability, log path, disk cache
  size; `--json` stays stable.
- Log rotation: cap `lekima.log` at 5 MB with one `.1` backup.
- Session file permission check on startup (warn if not `0600`).

### 4.4 Product gaps

#### Queue editing

- Queue pane: `x` removes the selected item, `J`/`K` move it down/up, `Enter`
  jumps, `A` still plays next.
- Removing the current track stops it, selects the following item and loads it;
  removing the last item stops playback.
- Moving the current track keeps it current (the cursor follows the track).
- CLI: `lekima queue remove <INDEX>`, `lekima queue move <FROM> <TO>`,
  `lekima queue next <ID>` (insert after current) — all `--json`.
- MCP: `queue_remove`, `queue_move`.
- Undo is **not** in this sprint; the action is immediate but visible.

#### Lyric translation

- Provider already returns `tlyric`; parse and merge by timestamp.
- `LyricLine { at_ms, text, translation: Option<String> }`.
- `t` in the TUI toggles translation for the session; the setting persists as
  `[ui] lyrics_translation` (default `false`).
- When on and available, the translated line renders beneath the original in
  `dim`; missing translation falls back to the original only.
- CLI `lekima lyrics --translate` prints `original` / `translation` pairs;
  `--json` includes both.
- Offset (`[`/`]`) applies to both.

### 4.5 Documentation

- `docs/install.md`: Homebrew, cargo, release tarball, Gatekeeper/ALSA notes.
- `docs/first-run.md`: `login --qr`, cookie fallback (HttpOnly explanation),
  `doctor`, first play, themes/settings.
- `docs/agents.md`: `lekima watch`, `lekima mcp`, host config snippets, tool
  table, exit codes, JSON envelope.
- README: 60-second quickstart at the top; links to the above.
- CONTRIBUTING: dev setup, test/CI commands, architecture links, release rule.
- SECURITY: supported versions, reporting, credential handling.

## 5. UX details

### Queue keys

| Key | Action |
| --- | --- |
| `x` | Remove selected queued track |
| `J` / `K` | Move selected track down / up |
| `Enter` | Jump to selected |
| `A` | Insert selection after current (existing) |
| `Esc` | Close queue |

### Translation toggle

`t` is context-sensitive: inside the lyrics pane it toggles translation; outside
it does nothing (avoids clobbering charts). The pane footer shows `T:ON`/`T:OFF`
when the track has a translation.

## 6. Success criteria

- Fresh macOS: `brew install benmooo/tap/lekima && lekima doctor` passes.
- Fresh Linux container: `cargo install lekima` then `lekima --version`.
- CI green on both OSes, no network in tests, < 3 minutes.
- `v0.1.0` release has 4 binaries + checksums; crates.io page renders README.
- Every CLI command has a contract test; every MCP tool has a call test.
- Queue editing and translation covered by unit + integration tests.
- No `unwrap`/`expect` in production paths introduced this sprint.

## 7. Risks

| Risk | Mitigation |
| --- | --- |
| Linux build needs ALSA headers | Document + CI install step; `doctor` gives a clear error |
| macOS unsigned binary blocked | README first-run note; optional `xattr -d com.apple.quarantine` |
| crates.io token scoped to `ncmapi` | Create a second token scoped to `lekima` (or widen deliberately) |
| Repo rename breaks old links | GitHub redirects; README of `lekima-go` points to the new repo |
| Hook/build time in CI | Cache, split jobs, `cargo nextest` optional |
| Feature creep into streaming | Explicit non-goals above |

## 8. Milestones

| # | Milestone | Exit criteria |
| --- | --- | --- |
| M1 | Design approved | These docs + sprint system design |
| M2 | Queue + translation | Tests green, TUI verified |
| M3 | Repo & CI | Rename done, project pushed, CI green |
| M4 | Release 0.1.0 | Tag, binaries, crates.io, Homebrew |
| M5 | Docs | Install/first-run/agents docs published |

## 9. Open questions

1. Homebrew tap name: `benmooo/tap` (short) vs `benmooo/homebrew-tap`.
2. Do we notarize macOS binaries now, or document the quarantine workaround for
   0.1? Proposal: document, defer notarization.
3. Should `lekima update` exist in 0.1? Proposal: no; Homebrew handles updates.
4. MSRV policy: pin 1.98 for 0.1 and revisit after the release.
