# Sprint 5 — Platform integration + playback engine

| Document | Contents |
| --- | --- |
| [`product-design.md`](./product-design.md) | MPRIS/Now Playing, device picker, range seeks, gapless, phasing |
| [`system-design.md`](./system-design.md) | Platform crate, MPRIS mapping, run-loop, sparse spool, gapless queue |

Status: **In progress — phase 5.1 (device picker) landed.**

- `lekima devices [--json]` lists outputs; `lekima audio-device <name|default>`
  persists `[audio] device` and restarts the daemon.
- Audio engine opens a named device (fails clearly if it disappeared).
- Next: MPRIS (5.2), macOS Now Playing/media keys (5.3), range seeks (5.4),
  gapless (5.5), 0.3.0.
