# Sprint 10 · Slice A — Desktop notifications on track change

Status: **Implemented, unreleased** · Linear AKA-144

## 1. Problem

`lekima` runs as a daemon and keeps playing when no terminal is attached. In
that state the listener has no way to know what started: the TUI may be closed,
the terminal may be in another tab, and the OS media widget is the only surface
(look at it to find out).

A short notification per track is the cheapest fix, and it is the first item in
the "daily-driver loop" ordering in AKA-142.

## 2. Behaviour

- Fires **once per track**, when the playing track id changes.
- Fires **only while status is `Playing`**. A track change that happens while
  paused records the id, so resuming does not deliver a stale announcement.
- Starting the daemon with a playing track counts as a change; the first
  notification carries the current track.
- Nothing fires when the queue is empty, when playback stops, or on position
  updates.

Payload:

| Field | Source | Fallback |
| --- | --- | --- |
| title | `Track::name` | `Unknown track` when blank after sanitizing |
| body | `artist_names()` · `album_name()` | artist alone, album alone, or empty |

Both parts pass through `lekima_domain::sanitize_line`, the same helper every
terminal surface uses. Provider metadata is untrusted, and a notification shown
by the OS is still user-visible text: escape sequences and bidi overrides must
not reach it.

## 3. Configuration

```toml
[notifications]
on_track_change = true
```

Default `false`. A notification is an OS-level interruption, so it is opt-in
rather than a surprise after an upgrade. No other knob exists in this slice:
no sound, no duration, no filters, no throttle. Tracks shorter than a
notification would be is a mix problem, not a settings problem.

## 4. Implementation

`lekima-platform::notify` owns a small `Notifier` trait, mirroring
`MediaControls`:

```rust
pub trait Notifier: Send + Sync {
    fn notify(&self, notification: &Notification) -> bool;
    fn backend(&self) -> &'static str;
}
```

| OS | Backend | Helper |
| --- | --- | --- |
| macOS | `MacNotifier` | `osascript` — `display notification <body> with title <title>` |
| Linux | `NotifySend` | `notify-send --app-name=lekima --icon=audio-x-generic` |
| other | `NoopNotifier` | — |

Decisions:

- **No new dependency and no unsafe code.** Both helpers are subprocesses. The
  macOS alternative (`UNUserNotificationCenter`) needs a bundle identifier and
  an `NSApplication`, which is why the Now Playing host already exists as a
  helper process; a notification does not justify a second one.
- **Arguments, not string interpolation.** The AppleScript takes title and body
  from `argv`, so no user text is ever escaped into script source.
- **Detection at startup, not per event.** The daemon resolves the backend once,
  so a missing helper costs nothing per track. Lookup checks `PATH` and then
  `/usr/bin`, `/bin`, `/usr/local/bin`, `/opt/homebrew/bin`, because daemons are
  often started without a login shell.
- **Best effort.** A failed helper logs at debug level and returns `false`. The
  daemon never retries and never propagates a notification failure into a
  command.
- **Wiring point.** The `spawn_platform` loop already snapshots state every
  500 ms for the OS media surface. Notifications hang off the same snapshot
  instead of adding a second timer or new IPC event.

Known limitation: on macOS the notification is attributed to the helper
(`osascript`/Script Editor) rather than to `lekima`, because there is no signed
app bundle. Acceptable for the slice; a signed app bundle is a packaging
question, not a notification one.

## 5. Observability

`lekima doctor` gains a `notify` line and a JSON block:

```json
"notifications": { "backend": "osascript", "on_track_change": true }
```

A user who enables the setting and sees no notification can now tell whether
the backend was found (`none`) or the setting is off.

## 6. Tests

- `lekima-platform`: no-op never reports success; `PATH` lookup skips empty
  entries and returns the first match.
- `lekima-cli`: payload joins artist and album; falls back to artist, then
  album; a blank title becomes `Unknown track`; escape sequences, DEL and bidi
  overrides are removed.
- `lekima-config`: the section is optional, defaults to off, and accepts the
  documented key (`deny_unknown_fields` catches typos).

No test spawns a real notification helper; the OS backends are thin argument
builders around the tested lookup and the shared command runner.

## 7. Acceptance

1. With `on_track_change = false` (default), a full album plays with no
   notification.
2. With it enabled, each track transition shows exactly one notification with
   the sanitized title and `artist · album`.
3. Pausing, resuming and seeking within a track do not re-notify.
4. `lekima doctor --json` reports the backend actually in use.
5. On a host without the helper, the daemon logs once at debug and plays on.
