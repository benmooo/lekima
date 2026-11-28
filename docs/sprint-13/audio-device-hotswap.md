# Sprint 13 · Live output device switching & hot-swap recovery

Status: **Shipped in 0.14.0** · Linear: `AKA-157`–`AKA-162` (`Sprint 13 · v0.14 — Resilient output`)

## 1. Problem

Today, `lekima devices` lists output devices and `lekima audio-device <name|default>`
persists `[audio] device` in `config.toml`, then calls `restart_daemon()`
(`crates/lekima-cli/src/commands.rs`). Inside `lekima-audio/src/engine.rs`,
`RodioAudioEngine` opens a single `rodio::MixerDeviceSink` once on audio-thread
startup.

Consequences:

- Switching between speakers and headphones requires restarting the daemon,
  interrupting playback and discarding transient state.
- Unplugging a USB DAC or disconnecting Bluetooth headphones while playing
  leaves the sink broken or silent until the user manually restarts the daemon.
- `lekima devices` reports the configured device from `config.toml`, not the
  device the running daemon is actually using.

## 2. Goals & UX

1. **Live switching:** `lekima audio-device <name|default>` switches the running
   daemon's output device in place (and persists `[audio] device`). If the
   daemon is not running, it persists the setting as before.
2. **Position preservation:** When switching mid-track (playing or paused),
   playback resumes on the new sink at the last known `position_ms` (and keeps
   the current play/pause state). Gapless preload on the old sink is discarded
   and re-armed on the new sink if applicable.
3. **Hot-swap recovery:**
   - **System default mode (`device = None`):** When the OS default output device
     changes or the active sink errors, the engine reopens the new default
     device and resumes at `position_ms`.
   - **Pinned device mode (`device = Some(name)`):** If the pinned device
     disappears, the engine falls back to the system default (emitting a
     transient notice, without overwriting `[audio] device` in `config.toml`).
   - **No output device available:** Playback pauses cleanly (`PlaybackStatus::Paused`)
     with an actionable error/notice; it does not crash the daemon or spin.
4. **Atomic failure on invalid switch:** Requesting a device that does not
   exist returns `ErrorCode::NotFound` and leaves the current sink untouched.

## 3. Domain & Core (`lekima-domain`, `lekima-core`)

### 3.1 `PlaybackState`

Add an optional field (backward-compatible JSON/wire):

```rust
/// Currently active output device name, or `None` when using the system default / stopped.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub output_device: Option<String>,
```

### 3.2 `AudioPort` & events

Extend `AudioPort` (with default/no-op stubs for fakes if needed, or explicit
methods covered in fake-port tests):

- `async fn set_device(&self, name: Option<&str>) -> Result<String>` — switches
  to `name` (or system default when `None`), returning the resolved device name.
- `AudioEvent::DeviceLost { reason: String }` / `AudioEvent::DeviceChanged { name: String }`
  emitted to `Player` when the audio thread detects a sink error or default-device
  change.

### 3.3 `Player` coordination

- `Player::set_audio_device(&self, name: Option<&str>) -> Result<PlaybackState>`:
  1. Snapshot current `status`, `track`, and `position_ms`.
  2. Call `audio.set_device(name)`. On `NotFound` / `Playback` error, return
     the error without mutating state.
  3. Clear any gapless preload handle (`preloaded = None`), update
     `output_device`, and if a track was playing/paused, re-seek/re-attach at
     `position_ms` (preserving paused state if paused).
  4. Emit `Status` event so TUI/CLI/hooks see the new `output_device`.
- On `DeviceLost` / `DeviceChanged`:
  - Debounce rapid OS notifications (e.g., 250 ms window) so Bluetooth profile
    switches do not thrash the sink.
  - Attempt fallback reopen (`None` / system default). If zero devices exist,
    transition to `Paused` and publish an error/notice.

## 4. Audio Engine (`lekima-audio`)

Inside `crates/lekima-audio/src/engine.rs`:

- Add `AudioCommand::SetDevice { name: Option<String>, reply: oneshot::Sender<Result<String>> }`.
- Extract sink creation + `rodio::Player` construction into a helper so
  startup, `SetDevice`, and hot-swap recovery share one code path.
- Keep the current `Spool` / cached source metadata on the audio thread (or
  re-open from the spool/cache at `position_ms`) so switching sinks does not
  require re-downloading the track from NetEase.
- Monitor default-device changes and `cpal` stream errors:
  - When `configured_device` is `None`, periodically compare
    `host.default_output_device().map(|d| d.to_string())` against the open
    sink's name (e.g., every 1 s while playing/paused, or on stream error) and
    trigger reopen when it changes.
  - On `cpal` stream error callback, signal the audio thread to attempt
    recovery rather than silently wedging the `rodio::Player`.

## 5. IPC, CLI, MCP, HTTP & Hooks

| Surface | Addition / Change | Notes |
| --- | --- | --- |
| `lekima-ipc` | `Command::SetAudioDevice { device: Option<String> }` (`set_audio_device`) | Returns `Reply::Status { state }` (or `Ack`) |
| `lekima-cli` | `lekima audio-device <name\|default>` | Persists config, then sends `SetAudioDevice` if daemon is running; no `restart_daemon` |
| `lekima-cli` | `lekima devices [--json]` | Queries running daemon for live `active` device when available; falls back to config |
| `lekima-cli` (MCP) | `list_devices` (read-only), `set_audio_device` | Follows existing MCP tool schema & annotations |
| `lekima-cli` (HTTP) | `GET /v1/devices`, `set_audio_device` in `COMMAND_TYPES` | Token-guarded `POST /v1/command` |
| Hooks | `set_audio_device` added to `HOOK_CAPABILITIES` | `status` event frames carry updated `output_device` |

## 6. TUI (`lekima-tui`)

- **Command line:** `:device <name|default>` switches the output device live
  and shows a confirmation toast (`output: <name>`).
- **Settings overlay:** Add an `OUTPUT` row showing the active device name;
  `h`/`l` or `Enter` cycles available devices (or opens a compact device
  list) and dispatches `SetAudioDevice`.
- **Hot-swap feedback:** If a pinned device disconnects and falls back to
  default, or if no output device is available, show a single-line notice/alert
  consistent with the design system.

## 7. Verification plan

- **Unit tests (`lekima-domain`, `lekima-core`):** fake `AudioPort` tests for
  live switch while playing, live switch while paused, invalid device rollback,
  device-loss fallback to default, and pause when no device exists.
- **Protocol & CLI tests (`lekima-ipc`, `lekima-cli`):** `SetAudioDevice`
  round-trip, `COMMAND_TYPES`, `HOOK_CAPABILITIES`, and MCP `tools/list`.
- **TUI tests (`lekima-tui`):** `:device` dispatch, Settings `OUTPUT` row
  interaction without breaking existing `TRANSLATION`/`DENSITY` rows, and
  render fixtures at 80×24 and 120×30.
- **Live manual check:** switch between macOS built-in speakers and headphones
  mid-track; disconnect an output device mid-playback and verify recovery.
