# Sprint 5 — Platform integration + playback engine

| Document | Contents |
| --- | --- |
| [`product-design.md`](./product-design.md) | MPRIS/Now Playing, device picker, range seeks, gapless, phasing |
| [`system-design.md`](./system-design.md) | Platform crate, MPRIS mapping, run-loop, sparse spool, gapless queue |

Status: **In progress — 5.1 device picker, 5.2 MPRIS, 5.3 macOS Now Playing,
5.4 range seeks and 5.5 gapless landed.**

- `lekima devices [--json]` lists outputs; `lekima audio-device <name|default>`
  persists `[audio] device` and restarts the daemon.
- Gapless: the daemon preloads the next small track, the audio thread appends it
  to the same rodio queue, and a boundary source advances core state on the
  first sample. Large tracks fall back to the normal two-step load.
- Range seeks: a sparse spool plus a `Range:` worker; the decoder runs on a
  background thread (`PrefetchSource`), so a fetch never blocks the audio
  callback. Seeking a 1 GB WAV to 12:30 lands exactly and keeps playing.
- Platform integration: `lekima-platform` exposes `org.mpris.MediaPlayer2.lekima`
  on Linux (compile-checked and unit-tested here), and on macOS runs a
  `lekima media-host` helper with an accessory `NSApplication` run loop that
  publishes Now Playing info and forwards media keys. Verified live with
  `nowplaying-cli`.
- Remaining: 0.3.0 release (crates, tag, binaries, Homebrew).
