# lekima

> NetEase Cloud Music in your terminal. Vim-first, agent-native.

`lekima` is a rewrite of the archived [benmooo/lekima](https://github.com/benmooo/lekima)
Go player as a layered Rust workspace. A small daemon owns playback; the TUI and
every CLI command are clients of the same local protocol, so music keeps playing
when you close the UI — and a code agent can drive it with one command.

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
- [`docs/sprint-2/`](docs/sprint-2/) — **design only**: product and system design
  for daily/FM/likes/toplists/lyrics pane, push events, `lekima watch` and MCP.
- [`docs/sprint-2/tui-visual-design.md`](docs/sprint-2/tui-visual-design.md) —
  the console/HUD visual language (`cyber`, `amber`, `mono` themes).
- [`docs/releasing.md`](docs/releasing.md) — release flow, including
  `--registry crates-io` on this USTC-mirrored machine.
- [`docs/sessions/`](docs/sessions/) — archived OpenCode sessions (this rewrite,
  in full) and the exporter script.

## Status

Working: search, playlists, lyrics, QR login, playback through a background
daemon, the vim-driven TUI, and the JSON CLI contract. `ncmapi` 1.2 is published
on crates.io.

```console
$ lekima login --qr          # scan with the NetEase app
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
| `lekima-tui` | ratatui frontend. |
| `lekima-cli` | `lekima` binary and daemon. |

## Development

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Hermetic runs (no writes to your real config):

```bash
LEKIMA_CONFIG_DIR=$(mktemp -d) cargo run -p lekima-cli -- status --json
```

## License

MIT. `lekima` is an unofficial client and is not affiliated with NetEase.
