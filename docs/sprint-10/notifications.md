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

The rule is "announce a track when it becomes audible", not "announce whenever
the queue index moves":

- Fires **once per track**, when a track that was not already announced starts
  producing audio (`Playing`, not `loading`, not `buffering`).
- A new track that is **current but silent** (`Stopped`, loading, or buffering)
  is held as pending and announced when it starts. This covers a restored queue
  after a daemon restart, a queue edit that replaces the current track, and a
  track whose stream fails to load — none of which are announced early, and a
  failed track is never announced at all.
- A track change that happens **while paused** is recorded without notifying:
  the listener paused deliberately, so resuming is not news.
- An **empty queue** clears the memory, so playing the same track again after
  clearing announces it again.
- Position updates, `pause`, `resume` on the same track and `seek` never
  re-announce.

Payload:

| Field | Source | Fallback |
| --- | --- | --- |
| title | `Track::name` | `Unknown track` when blank after sanitizing |
| body | `artist_names()` · `album_name()` | artist alone when there is no album |

`artist_names()` already renders `Unknown artist` for a track with no credited
artist, so the body is never empty for a real track; a provider track without
artists reads `Unknown artist · Album`.

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
  from `argv`, so no user text is ever escaped into script source. `--` ends
  option parsing, so a title that begins with `-` is not mistaken for a flag.
- **Absolute helper path.** Detection resolves the helper once and the backend
  keeps that `PathBuf`; spawning the bare name would fail for exactly the
  no-login-shell case the fallback search exists for.
- **Detection at startup, not per event.** The daemon resolves the backend once,
  so a missing helper costs nothing per track. Lookup checks `PATH` and then
  `/usr/bin`, `/bin`, `/usr/local/bin`, `/opt/homebrew/bin`, because daemons are
  often started without a login shell.
- **Off the async runtime.** Delivery may block and is bounded by a 2 s
  kill-timeout, so a dedicated `lekima-notify` thread owns the helper and the
  daemon hands over through a small bounded queue. Dropping an announcement is
  better than stalling the loop that also publishes Now Playing and drains media
  keys.
- **Best effort.** A failed, timed-out or missing helper logs at debug level.
  The daemon never retries and never propagates a notification failure into a
  command.
- **Wiring point.** The `spawn_platform` loop already snapshots state every
  500 ms for the OS media surface. Notifications hang off the same snapshot
  instead of adding a second timer or new IPC event.

Known limitations:

- On macOS the notification is attributed to the helper (`osascript`/Script
  Editor) rather than to `lekima`, because there is no signed app bundle.
  Acceptable for the slice; a signed app bundle is a packaging question.
- Title and body are subprocess arguments, so other local users can briefly read
  the current track from the process table. That is metadata, not a credential,
  and it is no worse than the OS media widget.
- `config.toml` is read when the daemon starts, so changing
  `notifications.on_track_change` needs a daemon restart (`lekima daemon stop`,
  then any command autostarts it).

## 5. Observability

`lekima doctor` gains a `notify` line and a JSON block:

```json
"notifications": { "backend": "osascript", "on_track_change": true }
```

The backend comes from the same resolver the daemon uses, so it is the backend
this host would use — not a hardcoded string. `doctor` runs in its own process
with its own configuration read, so a running daemon started before a config
edit can differ until it restarts.

A user who enables the setting and sees no notification can now tell whether
the backend was found (`none`) or the setting is off.

## 6. Tests

- `lekima-platform`: no-op never reports success; `PATH` lookup skips empty
  entries and returns the first match.
- `lekima-cli`, payload: joins artist and album; falls back to artist; a blank
  title becomes `Unknown track`; escape sequences, DEL and bidi overrides are
  removed.
- `lekima-cli`, rule: disabled is silent; a new audible track announces exactly
  once; pause/seek/resume do not repeat; a track seen while paused stays silent
  on resume; a track seen while `Stopped` announces when playback begins;
  `loading` and `buffering` delay the announcement until audio starts; a track
  that fails to load is never announced; clearing the queue lets the same track
  announce again.
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
4. A track that fails to load is not announced.
5. After a daemon restart with a restored queue, the notification arrives when
   playback resumes, not at startup.
6. `lekima doctor --json` reports the backend this host would use.
7. On a host without the helper, the daemon plays on and logs one debug line per
   failed delivery, not per tick.
