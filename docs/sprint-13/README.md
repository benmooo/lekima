# Sprint 13 — Live output device switching & hot-swap recovery

Status: **Shipped in 0.14.0** · Started 2026-11-28 · Linear: **Sprint 13 · v0.14 — Resilient output** (`AKA-157`–`AKA-162`)

`lekima devices` and `lekima audio-device <name|default>` shipped in v0.3.0, but
changing the output device still restarts the daemon and drops playback state.
Unplugging headphones, disconnecting Bluetooth, or switching a USB DAC while
playing leaves `RodioAudioEngine` bound to a stale sink.

## Goal

Make audio output device changes and hardware disconnects seamless:

1. Switch output devices live (`lekima audio-device`, TUI `:device` / Settings,
   IPC, MCP, HTTP, hooks) without restarting the daemon or losing queue/position.
2. Recover automatically when the active or system-default output device
   disconnects or changes, preserving playback position (or pausing cleanly if
   no output device is available).
3. Expose the live resolved output device in `PlaybackState`, `status --json`,
   and `lekima devices`.

## Slices

| Slice | Contents | Exit criteria |
| --- | --- | --- |
| A. Domain + audio engine | `PlaybackState` device field, `AudioPort` device switch + loss/recovery events, `RodioAudioEngine` live sink reopen + position restore | Unit tests with fake audio port; audio-thread switch/reopen tests |
| B. Core player | `Player::set_audio_device`, `DeviceLost`/`DeviceRecovered` handling, debounce, fallback-to-default policy | Fake-port tests covering live switch, missing device, disconnect/pause, and reconnect |
| C. IPC + daemon + config | `Command::SetAudioDevice` (`set_audio_device`), `EventFrame` status updates, daemon live-switch wiring without restart | Protocol round-trip tests; `lekima audio-device` updates running daemon without restart |
| D. Agent surfaces | MCP `list_devices` + `set_audio_device`, HTTP `COMMAND_TYPES` + `GET /v1/devices`, hook capability `set_audio_device`, `docs/agents.md` | MCP `tools/list` + HTTP tests; contract docs updated |
| E. TUI | `:device` command, Settings overlay device selector/row, inline notice on hot-swap fallback or no-device pause | Reducer + render tests at 80×24 and 120×30 |
| F. Release | Update `docs/product-design.md`, `docs/system-design.md`, `CHANGELOG.md`, release `0.14.0` | `fmt`, `clippy`, `test --workspace` green; live macOS/Linux device-switch verification |

## Design documents

- [`audio-device-hotswap.md`](./audio-device-hotswap.md) — audio engine
  reopen flow, domain/core state, IPC/CLI/MCP/HTTP/TUI surfaces, edge cases,
  and non-goals.

## Definition of ready

- No `ncmapi-rs` changes required (purely local audio/core/IPC/frontend work).
- Wire changes to `PlaybackState` and `Command` are additive so existing
  clients remain compatible.
- Switching to a non-existent device fails cleanly without interrupting current
  playback.

## Definition of done

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  and `cargo test --workspace` green.
- Live test on macOS: switch between built-in speakers and headphones/virtual
  device mid-track without daemon restart; unplugging active device falls back
  or pauses without crashing.
- `docs/product-design.md`, `docs/system-design.md`, `docs/agents.md`, and
  `CHANGELOG.md` updated.

## Non-goals

- Per-application OS audio routing, AirPlay/Cast, or network audio sinks.
- Loudness normalization (EBU R128), equalizer, or DSP effects.
- Windows audio backends (per ADR-013).
