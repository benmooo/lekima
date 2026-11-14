# Sprint 4 — Playback depth

| Document | Contents |
| --- | --- |
| [`product-design.md`](./product-design.md) | Streaming, large files, buffering UX, sleep timer, quality |
| [`system-design.md`](./system-design.md) | Spool design, decoder reader, seek policy, state fields, tests |

Status: **Implemented, release pending 0.2.0.**

- Streaming spool: tracks > 64 MiB play from an anonymous growing temp file;
  the 1 GB cloud WAV starts in ~0.6 s with ~31 MB RSS.
- Buffering state (`loading` / `buffering`) exposed in JSON and the HUD.
- Seeks within the buffered region; beyond it returns a clear playback error
  (HTTP Range restart is phase 2).
- Sleep timer: `lekima sleep 30m`, `:sleep`, MCP `sleep`, shown in status/HUD.
- Quality switching: `lekima quality lossless`, `:quality`, MCP `set_quality`,
  persisted to `config.toml`; applies to the next track.
- Pending: 0.2.0 crates/binaries/tap, HTTP Range seek, gapless, device picker,
  MPRIS/Now Playing (Sprint 5).
