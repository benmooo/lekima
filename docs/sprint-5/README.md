# Sprint 5 — Platform integration + playback engine

| Document | Contents |
| --- | --- |
| [`product-design.md`](./product-design.md) | MPRIS/Now Playing, device picker, range seeks, gapless, phasing |
| [`system-design.md`](./system-design.md) | Platform crate, MPRIS mapping, run-loop, sparse spool, gapless queue |

Status: **In progress — 5.1 device picker, 5.4 range seeks, 5.5 gapless landed.**

- `lekima devices [--json]` lists outputs; `lekima audio-device <name|default>`
  persists `[audio] device` and restarts the daemon.
- Gapless: the daemon preloads the next small track, the audio thread appends it
  to the same rodio queue, and a boundary source advances core state on the
  first sample. Large tracks fall back to the normal two-step load.
- Range seeks: a sparse spool plus a `Range:` worker; the decoder runs on a
  background thread (`PrefetchSource`), so a fetch never blocks the audio
  callback. Seeking a 1 GB WAV to 12:30 lands exactly and keeps playing.
- Next: MPRIS (5.2), macOS Now Playing/media keys (5.3), 0.3.0.
