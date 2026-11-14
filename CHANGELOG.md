# Changelog

All notable changes to lekima are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/) and semantic versioning.

## 0.1.1 — 2026-11-14

### Changed

- `ncmapi 1.6` with pure-Rust protocol crypto: no OpenSSL anywhere in the
  dependency graph, byte-identical request encryption. Release binaries are
  portable (no Homebrew OpenSSL linkage) and cross-compiled on macOS.

## 0.1.0 — 2026-11-14

### Added

- Daemon-owned playback with a vim-first TUI and a stable JSON CLI.
- Search, playlists, daily mix, personal FM (prefetch/trash), likes, charts and
  the music cloud.
- Timed lyrics pane with offset and translation toggle.
- Daisy-chained themes (cyber, amber, mono, dracula, gruvbox, nord, synthwave,
  matrix, solarized, tokyonight, catppuccin), glyph and density variants, and
  an in-TUI settings palette that persists on exit.
- Back/forward history in the middle pane (`Ctrl-o` / `Ctrl-i`).
- Queue editing: remove and move tracks from the TUI, CLI and MCP.
- Real RMS waveform in the HUD; loading state and instant stop between tracks.
- Push events (protocol v2) with replay ring; `lekima watch` NDJSON stream.
- MCP stdio server (`lekima mcp`) with 15 tools.
- Cookie and QR session login; `doctor`, completions, capabilities.

### Fixed

- QR login now uses the eapi type=3 flow; non-idempotent requests bypass the
  response cache.
- Provider models tolerate null fields and duplicate chart keys.
- Backward seeks work (seekable rodio decoder).
