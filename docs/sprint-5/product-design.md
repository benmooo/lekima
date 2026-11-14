# Sprint 5 — Product Design

> Theme: **Platform integration + real seeking/gapless**
> Status: **Approved for design**

---

## 1. Why

lekima is a daemon with a terminal face; the operating system still treats it as
a stranger. There is no media panel, no media keys, no way to pick an output
device, seeks fail past the stream buffer, and every track change has a gap.
This sprint makes it a first-class OS citizen and finishes the playback engine.

## 2. Goals

1. **Media controls**: Linux MPRIS, macOS Now Playing + media keys. Play/pause,
   next/previous, seek, metadata (title/artist/album/artwork), position.
2. **Device picker**: list output devices and select one persistently.
3. **Range seeks**: seeking a streamed track beyond the buffered region works by
   fetching the target range on demand, with a visible buffering state.
4. **Gapless**: the next queued track is preloaded and starts with no audible
   gap for same-format tracks.
5. **Notifications** (stretch): optional “now playing” notification on macOS and
   Linux.

### Non-goals

- Windows SMTC (Sprint 6).
- Crossfade, replay gain, equaliser.
- Offline downloads.
- Scrobbling / Last.fm.

## 3. Platform behaviour

### 3.1 Linux (MPRIS)

- D-Bus name `org.mpris.MediaPlayer2.lekima`, path `/org/mpris/MediaPlayer2`.
- Interfaces: `MediaPlayer2` (Raise/Quit), `MediaPlayer2.Player` (PlayPause,
  Play, Pause, Stop, Next, Previous, Seek, SetPosition, Volume, Shuffle,
  LoopStatus, Rate), properties `PlaybackStatus`, `Metadata`, `Position`,
  `CanSeek`, `CanGoNext`, `CanGoPrevious`.
- Works with `playerctl`, GNOME/KDE media widgets, Waybar, Polybar.
- `lekima daemon` owns the service; the TUI/CLI remain clients.

### 3.2 macOS (Now Playing)

- `MPNowPlayingInfoCenter` publishes title, artist, album, duration, position,
  artwork, playback rate.
- `MPRemoteCommandCenter` handles media keys and Control Center:
  play/pause/toggle, next/previous, change-playback-position.
- Implemented on a dedicated thread with an `NSRunLoop`; the daemon stays
  headless. Artwork is fetched/decoded lazily and cached by track id.
- If the process is not bundled as an `.app`, media keys may be unavailable;
  ship a tiny helper bundle or accept Control Center-only support. Documented
  fallback: `brew services`/login item wrapper (future).

### 3.3 Notifications (stretch)

- macOS: `UNUserNotificationCenter` requires a bundle; fallback to
  `osascript display notification` (best effort).
- Linux: `org.freedesktop.Notifications`.
- Config `[notify] on_track_change = false` by default.

## 4. Device picker

- `lekima devices [--json]` lists output devices (name, default flag).
- `lekima audio-device <name|default>` writes `[audio] device` and restarts the
  audio engine in the daemon (or the next daemon start).
- Daemon/doctor report the active device and fail with a clear message if the
  configured device disappeared.
- TUI settings palette shows the active device (read-only in 0.3).

## 5. Range seeks

- A spooled stream tracks **downloaded byte ranges**, not just a high-water mark.
- `GrowingFile::seek` asks the downloader for the missing range and waits while
  the decoder is repositioning (`buffering=true`).
- Decoding runs on a background thread (`PrefetchSource`) with a 2 s buffer;
  blocking range fetches and seeks never run on the audio callback, so output
  pauses with silence instead of underrunning and ending the stream.
- The downloader switches from sequential mode to `Range: bytes=start-end`
  requests, writing at absolute offsets into the same anonymous file.
- Time→byte mapping is owned by symphonia's seek logic; the spool only serves
  bytes. Seeks are bounded by the track duration and the HTTP server's range
  support.
- If the server ignores ranges (200 instead of 206), fall back to “wait until the
  sequential download reaches the target”, with a toast.
- Fully downloaded tracks keep the current instant seeks.

## 6. Gapless

- The daemon preloads the **next** queue entry while the current track plays:
  resolves the URL, starts a spool or download, and hands a prepared source to
  the audio engine.
- The engine appends the prepared source to the same rodio `Player` queue, so
  the mixer transition is sample-accurate.
- When the source boundary is crossed, the audio thread notifies the core, which
  advances the queue/state and emits events. No `sink.empty()` gap.
- Preload is skipped when: repeat is off and there is no next track, the next
  format changes sample rate (rodio queue handles switching, but gapless is only
  guaranteed for matching formats), or the user has disabled it
  (`[playback] gapless = true` default).
- Memory: preload reuses the spool mechanism; cap one preload per queue.

## 7. UX

- `lekima status` and MPRIS show the same state; changes from media keys flow
  back through events to the TUI.
- Seeking past the buffer shows the loading glyph and a “buffering target”
  message; the resulting position is exact once resumed.
- Gapless is invisible when it works; `doctor` reports “gapless: enabled”.
- Device selection is silent unless the device is missing; then playback fails
  with a readable error and `doctor` hint.

## 8. Success criteria

- `playerctl -p lekima status/metadata` works on Linux; play/pause/next from
  media keys works on macOS (or documented limitation).
- `lekima devices` lists at least the default device; selecting another device
  persists across daemon restarts.
- Seeking a 1 GB WAV to 70% from a cold start completes in < 3 s on 50 Mbps.
- Gapless gap < 50 ms for consecutive MP3/FLAC tracks at the same sample rate.
- No regression: 0.2.1 playback, events, MCP and tests stay green.

## 9. Phasing

| Phase | Contents |
| --- | --- |
| 5.1 | Device picker + doctor/config (this slice) |
| 5.2 | MPRIS (Linux), tested in CI with a mock bus |
| 5.3 | macOS Now Playing/media keys (manual verification) |
| 5.4 | Range seeks (sparse spool) |
| 5.5 | Gapless preload + boundary events |
| 5.6 | 0.3.0 release |

## 10. Risks

| Risk | Mitigation |
| --- | --- |
| macOS media keys need an app bundle/run loop | dedicated run-loop thread; document Control Center fallback |
| MPRIS D-Bus integration untestable on macOS | Linux CI + `dbus-run-session` mock tests |
| Range servers ignoring `Range` | detect 200, fall back to sequential wait |
| Format probe seeks to end while sparse | fetch range on demand with a timeout; fall back to full download |
| Gapless sample-rate changes produce artefacts | only claim gapless for matching formats; resample boundary otherwise |
| Device hot-unplug | error + `doctor`; auto-fallback optional config |

## 11. Open questions

1. Does macOS media-key support require a bundled `.app`? Prototype before
   committing; if yes, ship a `lekima.app` wrapper in 5.3.
2. MPRIS implementation: `mpris-server` vs raw `zbus`. Proposal: `mpris-server`
   for typed interfaces, pinned and Linux-only.
3. Artwork in Now Playing: download cover art (extra request) or omit? Proposal:
   lazy fetch, disk-cached, opt-out config.
4. Gapless across formats: claim support only for same sample-rate containers;
   document.
5. Notification defaults: off (avoid noise).
