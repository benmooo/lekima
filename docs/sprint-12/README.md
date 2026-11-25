# Sprint 12 — Playlist editing

Status: **In progress** · Started 2026-11-25

Playlists are read-only today: `lekima playlist list` and `show`, the TUI
sidebar, MCP `queue_playlist` and the HTTP routes can consume a playlist but
nothing can create one, rename it, or change its tracks. Every other music
client writes to playlists; this is the largest remaining gap between lekima and
the web player, and it is the first feature that needs provider **write**
endpoints.

## Goal

Create and maintain played lists from every surface:

1. Create a playlist (name, optional description, public or private).
2. Rename or describe an existing playlist.
3. Add tracks to a playlist, remove tracks from it.
4. Delete a playlist.
5. Keep every new surface available to agents through JSON, MCP and HTTP.

## Slices

| Slice | Contents | Exit criteria |
| --- | --- | --- |
| A. Provider | `ncmapi 1.10`: playlist create/delete/update, add/remove tracks | Fixture tests for each route and payload, additive release |
| B. Domain + adapter | `MusicService` write methods, `NcmMusicService` mapping, domain errors | Fake-port tests; provider 4xx maps to typed errors |
| C. IPC + daemon | Commands, replies, capability tags, `lekima playlist …` | Round-trip protocol tests; CLI contract tests |
| D. Agent surfaces | MCP tools, HTTP command tags, `docs/agents.md` | `tools/list` includes the write tools; HTTP refuses them without the token |
| E. TUI | `:playlist create`, `:playlist add`, `:playlist delete` | Reducer tests; hints updated |
| F. Release | Docs, changelog, 0.13.0 | Green workspace, published release |

## Design documents

- [`playlists.md`](./playlists.md) — provider routes, payloads, models, IPC
  surface, error mapping, TUI flow and non-goals.

## Definition of ready

- NetEase write routes verified against the public API surface (payload names
  and response codes), with no new signing work: the existing WEAPI transport
  already adds `csrf_token`.
- No playback rule changes: playlist edits never touch the queue.
- Every write is idempotent enough to retry: create is not, so the CLI reports
  the created id and never retries automatically.

## Definition of done

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace` green.
- `ncmapi` tests green and a minor release published; `lekima` pins it.
- A live create → add → rename → remove → delete path works on this machine.
- README, product/system design, sprint docs and changelog updated.
- Independent verification pass (subagent) before the release is cut.

## Non-goals

- Reordering tracks inside a playlist, cover upload, sharing links.
- Subscribing to or editing someone else's playlist.
- Collaborative/offline edits, or a local draft that reconciles later.
- Playlist folders.
