# Sprint 9 — Catalogue depth

Status: **Shipped in 0.9.0** · Started 2026-11-17

The player can search tracks and play them, but the catalogue is still
one-dimensional: artists and albums are only strings in track metadata, album
results cannot be decoded, and there is no way to open an artist, browse their
albums or play an album. Sprint 9 closes that gap.

## Goal

Make discovery work without the web player:

1. Search tracks, albums, artists and playlists from the TUI and the CLI.
2. Open an artist page with their albums.
3. Open an album page with its track list and play or queue the album.
4. Recognise an artist or album from a track and jump to it.
5. Keep every new surface available to agents through JSON, MCP and HTTP.

## Slices

| Slice | Contents | Exit criteria |
| --- | --- | --- |
| A. Provider | `ncmapi 1.9.0`: album search results, artist detail/albums, album detail/tracks | Fixture tests, live smoke against the public endpoints, additive release |
| B. Domain + IPC | Album/artist models, `MusicService` methods, `SearchKind::All`, IPC commands/replies, `PlayTarget::Album` | Round-trip protocol tests; fake-port core tests |
| C. CLI + agent | `lekima search --kind`, `lekima artist`, `lekima album`, MCP tools, HTTP read-only routes | Contract tests for JSON fields and exit codes |
| D. TUI | Search filters, grouped results, artist and album views, history, queue actions | Render/navigation tests at 80×24 and 120×30 |
| E. Release | docs, changelog, 0.9.0 binaries and tap | Green workspace, published release |

## Design documents

- [`product-design.md`](./product-design.md) — users, flows, keymap, states,
  agent contract, success criteria and non-goals.
- [`system-design.md`](./system-design.md) — provider routes, models, domain and
  IPC changes, TUI navigation, tests, rollout and ADRs.

## Definition of ready

- Provider endpoints and response shapes verified against the live service
  (done 2026-11-17; see the system design).
- No new playback rules; catalogue reads stay in adapters.
- Every TUI action has a CLI equivalent (the existing acceptance rule).

## Definition of done

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace` green.
- `ncmapi` tests green and a minor release published; `lekima` pins it.
- A live search → artist → album → play path works on this machine.
- README, product/system design, sprint docs and changelog updated.
