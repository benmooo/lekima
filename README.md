# lekima

> NetEase Cloud Music in your terminal. Vim-first, agent-native.

`lekima` is a rewrite of the archived [benmooo/lekima](https://github.com/benmooo/lekima)
Go player as a layered Rust workspace. A small daemon owns playback; the TUI and
every CLI command are clients of the same local protocol, so music keeps playing
when you close the UI — and a code agent can drive it with one command.

```bash
brew install benmooo/tap/lekima      # or: cargo install lekima
lekima login --qr                    # opens the web player, then saves the session
lekima tui
```

```console
$ lekima play "bohemian rhapsody"
▶ Bohemian Rhapsody · Queen · A Night at the Opera   0:00 / 5:55

$ lekima status --json
{"ok":true,"data":{"status":"playing","track":{…},"volume":0.7}}
```

## Design documents

- [`docs/product-design.md`](docs/product-design.md) — users, principles, feature
  scope, keymap, agent contract, success criteria, milestones, risks.
- [`docs/system-design.md`](docs/system-design.md) — crate graph, playback state
  machine, daemon lifecycle, IPC protocol, provider adapter, security, testing,
  ADRs.
- [`docs/sprint-2/`](docs/sprint-2/) — **Provider depth** (shipped in 0.2.0):
  daily/FM/likes/toplists/lyrics pane, push events, `lekima watch` and MCP.
- [`docs/sprint-2/tui-visual-design.md`](docs/sprint-2/tui-visual-design.md) —
  the console/HUD visual language (`cyber`, `amber`, `mono`, `light` and eight
  more palettes).
- [`docs/terminal-setup.md`](docs/terminal-setup.md) — font families, sizes and
  profiles for iTerm2, Ghostty, kitty, Alacritty and WezTerm.
- [`docs/sprint-3/`](docs/sprint-3/) — **Ship 0.1**: distribution, CI,
  integration harness, queue editing, lyric translation.
- [`docs/sprint-4/`](docs/sprint-4/) — **Playback depth**: streaming spool,
  buffering, large files, sleep timer, quality switching.
- [`docs/sprint-5/`](docs/sprint-5/) — **Platform integration**: MPRIS/Now
  Playing, device picker, range seeks, gapless.
- [`docs/tui-backlog.md`](docs/tui-backlog.md) — TUI rendering, interaction and
  accessibility gaps against the visual design, ordered by priority.
- [`docs/sprint-6/`](docs/sprint-6/) — **TUI correctness** (shipped in 0.4.0):
  responsiveness, accessibility, queue confirmation, sanitization.
- [`docs/sprint-7/`](docs/sprint-7/) — **Playback hardening** (complete):
  preview duration, queue-end recovery, persistence and restart UX.
- [`docs/sprint-8/`](docs/sprint-8/) — **Offline cache and comments** (0.5.0):
  256 MiB stream cache, `lekima cache`, comments CLI/MCP/TUI.
- [`docs/releasing.md`](docs/releasing.md) — release flow, including
  `--registry crates-io` on this USTC-mirrored machine.
- [`docs/sessions/`](docs/sessions/) — archived OpenCode sessions (this rewrite,
  in full) and the exporter script.

## Status

`0.6.0` adds the assisted web login: `lekima login --qr` opens the web player
in a dedicated Chrome profile, opens the login dialog, captures the session and
closes the browser; `--no-browser` keeps the terminal QR. Provider requests
reuse one device id and send browser-like headers through `ncmapi` 1.8.

`0.5.2` fixes the first-run `lekima login --qr` failure on a fresh install by
creating the state directories before the provider writes `session.cookies`.

`0.5.1` moves track comments onto `ncmapi` 1.7's public comments endpoint and
drops the adapter's direct HTTP dependency.

`0.5.0` adds a 256 MiB offline stream cache (`lekima cache`) with `CACHE`
playback status and offline fallback, plus public track comments in the CLI, MCP
and a scrollable TUI pane (`C`).

`0.4.1` fixes gapless previews and track transitions: boundary sources fire
once, a new track starts at zero, and stale status polls can no longer flip the
track, progress or lyrics back to the previous song.

`0.4.0` is a TUI and playback-reliability release: a width-aware HUD with
buffered-range shading, a light palette and `LEKIMA_THEME`, count and pane keys,
a searchable/editable prompt with history, queue transport controls and a
live-updating queue with remaining time. Gapless preloads now carry track ids,
and FM prefetch publishes queue events. Earlier releases added platform
integration, streaming, the agent surface and the first public packages.
`0.1.1` was the first public release: crates.io, GitHub release binaries and a
Homebrew tap. See [install](docs/install.md) and [first run](docs/first-run.md).

Working: search, playlists, timed lyrics, QR/cookie login, daily mix, personal
FM, likes, charts, the music cloud, playback through a background daemon, a
vim-driven TUI (responsive HUD, real spectrum, buffered shading, lyrics pane,
history, UI settings, twelve themes), device selection, gapless playback,
MPRIS/Now Playing, push events (`lekima watch`) and an MCP stdio server
(`lekima mcp`). `ncmapi` 1.8 is published on crates.io.

```console
$ lekima login --qr          # opens the web player in Chrome
$ lekima playlist list
$ lekima playlist show 3778678 | head
$ lekima tui
```

The design documents mark what is **now**, **next** and **later** — treat them
as the plan of record.

## Workspace

| Crate | Purpose |
| --- | --- |
| `lekima-domain` | Entities, queue rules, playback state, ports. No I/O. |
| `lekima-core` | `Player`: orchestration and transitions. |
| `lekima-config` | Paths, `config.toml`, `0600` session storage. |
| `lekima-ipc` | JSON-lines protocol over a Unix socket. |
| `lekima-ncm` | `MusicService` adapter over [`ncmapi`](https://crates.io/crates/ncmapi). |
| `lekima-audio` | `AudioEngine` adapter over rodio. |
| `lekima-platform` | OS media integration: MPRIS (Linux), Now Playing (macOS). |
| `lekima-tui` | ratatui frontend. |
| `lekima` | `lekima` binary and daemon. |

## Development

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Hermetic runs (no writes to your real config):

```bash
LEKIMA_CONFIG_DIR=$(mktemp -d) cargo run -p lekima -- status --json
```

## License

MIT. `lekima` is an unofficial client and is not affiliated with NetEase.
