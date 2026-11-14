# Sprint 2 — Provider depth, live UI, agent-native control

Design first, code second. Nothing in this folder has been implemented yet.

| Document | Contents |
| --- | --- |
| [`product-design.md`](./product-design.md) | Users, jobs, feature specs, UX, keymap, agent contract, metrics, risks |
| [`system-design.md`](./system-design.md) | Provider routes, domain/core changes, protocol v2, TUI/MCP architecture, testing, rollout, ADRs |
| [`tui-visual-design.md`](./tui-visual-design.md) | Console/HUD visual language, palette, glyphs, themes, anti-patterns |

## Scope

1. Daily recommendations
2. Personal FM (start/next/trash)
3. Like / unlike + liked playlist
4. Toplists
5. Lyrics pane (synced, translated, offset)
6. Push events + `lekima watch`
7. MCP server (`lekima mcp`)
8. Polish (themes, counts, yank, loading states)

Out of scope: comments, cloud disk, downloads, MPRIS, notifications,
scrobbling, Windows named pipes.

## Dependencies

- `ncmapi 1.2.1` — published (crates.io + tags `v1.1.0` … `v1.2.1`)
- `ncmapi 1.2.x` — daily/FM/like/toplist routes: **done**
- Sprint 1 `lekima` 0.1 daemon, IPC v1 and TUI in place

## Definition of ready

- [x] Product scope and non-goals agreed
- [x] UX and keymap specified
- [x] Agent contract additions specified
- [x] Protocol v2 handshake, events and backpressure specified
- [x] Provider routes and error mapping specified
- [x] Testing and rollout plan specified
- [x] `ncmapi 1.1.0` published
- [ ] Open questions in both documents resolved

## Definition of done

- All sprint-2 acceptance criteria in `product-design.md` pass.
- `cargo test --workspace`, `clippy -D warnings`, `fmt --check` green.
- v1 clients still work against a v2 daemon (compat test).
- `lekima watch` and `lekima mcp` documented with runnable examples.
- `lekima 0.2.0` released and `ncmapi 1.2.0` published.

## First implementation task

`ncmapi 1.2.0`: `discovery().daily()`, `radio().fm_batch()`, `radio().trash()`,
`radio().set_like()`, `radio().liked_ids()`, `discovery().toplists()` — with
stub-transport contract tests and a dry-run publish before any lekima code
depends on them.
