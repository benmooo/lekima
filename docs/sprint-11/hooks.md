# Sprint 11 · Plugin hooks

Status: **Implemented, unreleased** · Linear AKA-149 · ADR-015, ADR-016

## 1. Problem

Every integration has to be a client that asks: the CLI sends one command, MCP
maps tool calls, HTTP wraps the same protocol, and `lekima watch` streams
read-only events. Nothing can be *told* what happened, and nothing can add a
verb. The alternative — compiling features into `lekima-core` — is exactly what
the product decisions ruled out for scrobbling, Windows and in-process code.

A hook is the extension point at the edge: a user process the daemon starts and
talks to.

## 2. Contract

Version 1, carried in the hello frame.

```text
daemon → hook (stdin), one JSON object per line

  {"type":"hello","protocol":1,"version":"0.10.1",
   "events":["status"],"capabilities":["pause"]}
  {"seq":42,"type":"status","state":{…}}
  {"seq":42,"type":"resync"}
```

```text
hook → daemon (stdout), one JSON object per line

  {"type":"pause"}
```

- **Events** are the same sequenced `EventFrame`s the socket already publishes
  (`status`, `queue`, `like`), filtered by the configured `events`.
- **Commands** are the same `Command` objects the CLI sends, filtered by the
  configured `capabilities`. An undeclared command is refused and logged; the
  daemon keeps running.
- **Hooks are action-only.** The command stream carries no replies, so a hook
  cannot ask for `status` or `history` over it. It reads state from the events,
  from the public state files (`history.json`, `playback.json`), or by
  connecting to the socket like any other client. `HOOK_CAPABILITIES` therefore
  lists only commands with a player-visible effect.
- **`resync`** means the hook lagged behind the broadcast ring and may have
  missed events; `seq` is the last frame actually written to that hook. It is
  delivered only if the hook's `events` filter includes `resync` (the default,
  "all kinds", does).
- **Versioning** is additive: the hello names the contract version, new fields
  may appear, and a hook that does not understand a frame can ignore it. Bumping
  `protocol` is a breaking change.
- **stdout is reserved** for commands; noise is logged at debug. Hooks log to
  stderr, which the daemon forwards to `lekima.log` through the shared
  sanitizer, one line at a time.

## 3. Configuration

```toml
[hooks.nowplaying]
command = ["/Users/me/bin/nowplaying"]
events = ["status"]            # default: every kind
capabilities = ["pause"]       # default: none, so read-only

[hooks.scrobble-bridge]
command = ["/Users/me/bin/scrobble-bridge"]
events = ["status"]
capabilities = []              # read history.json and post it yourself
```

- The table name is the hook's identity in logs and `doctor`.
- `command` is an executable plus arguments; it is never a shell string.
- `events` accepts `status`, `queue`, `like`, `resync`.
- `capabilities` accepts the wire tags of commands with a player-visible effect
  (`play`, `pause`, `next`, `enqueue`, `set_volume`, `like`, …). Read-only,
  session and lifecycle tags are deliberately **not** grantable: `status`,
  `queue`, `history`, `search`, `lyrics`, `whoami`, `login`, `logout`,
  `shutdown` and `subscribe` are rejected.
- Configuration is read at daemon start; changing it needs a restart
  (`lekima daemon stop`, then any command autostarts it).

## 4. Lifecycle and failure modes

| Situation | Behaviour |
| --- | --- |
| Invalid config (empty command, unknown event, forbidden capability) | Logged at warn, hook skipped, daemon starts; `doctor` shows the error |
| Executable missing or not runnable | Logged at warn, hook skipped |
| Hook exits or crashes | Its tasks end; **no automatic restart**. Restart the daemon to bring it back |
| Hook stops reading stdin | Its writer task blocks; other hooks and playback are unaffected; events for that hook are dropped |
| Hook lags | One `resync` frame per lag episode, warning once then debug |
| Hook sends invalid JSON or non-UTF-8 | Logged, line ignored; the stream keeps working |
| Hook sends a line over 1 MiB | Line dropped, the stream resynchronizes at the next newline; one warning per connection |
| Hook floods commands | The shared 64-command queue drops the excess (a busy hook can crowd others); the daemon stays responsive |
| Hook sends an undeclared command | Warned once per tag, then debug; command dropped |
| Daemon shuts down | Hook stdin closes and the **direct child** is killed. A hook that leaves grandchildren behind must `exec` its final process |

Hooks run as independent tasks on the daemon's runtime, so a slow or wedged hook
does not block the loops that drive playback, Now Playing, media keys,
notifications or the queue. Spawning a hook is a synchronous fork/exec at
startup, so a hook that is slow to start delays daemon start, not playback.

## 5. Security

- A hook runs as the same user, with the same filesystem access. The capability
  list is an API guard, **not** a sandbox; a hook that wants to read
  `session.cookies` can. Only run hooks you wrote or trust — the config file is
  already inside the user's trust boundary.
- Session and lifecycle commands are not grantable, so a mistaken hook cannot
  log the user out or stop the daemon.
- No secrets are passed to a hook: the hello frame carries the version, the
  event kinds and the capability names, nothing else.
- Hook stderr is forwarded line by line and sanitized before it reaches the log,
  so a hook cannot inject escape sequences into `lekima daemon logs` or a
  foreground `daemon run`. Binary bytes are rendered lossily.
- The daemon logs only a hook's program, never its full argument list, because
  arguments can carry secrets. `doctor` echoes the configured command, so keep
  secrets out of hook argv.

## 6. Observability

`lekima doctor` lists every configured hook. The JSON form carries `ok`
(configuration valid) and `found` (the program exists):

```json
"hooks": [
  { "name": "nowplaying", "command": ["/Users/me/bin/nowplaying"],
    "events": ["status"], "capabilities": ["pause"], "found": true, "ok": true },
  { "name": "bogus", "ok": false,
    "error": "`shutdown` is not a hook capability; allowed: …" }
]
```

The human output repeats the same facts per hook, including `invalid: …` for a
bad table and `not found` for a valid table pointing at a missing program.

The daemon log records `hook started`, `hook output closed`, `hook process
exited` and `hook lagged` at info, `hook command denied` once per rejected tag,
and `hook stopped` at debug when the daemon shuts a hook down. Everything a hook
writes to stderr is forwarded at warn, so the volume is the hook's own
responsibility.

## 7. Tests

- `lekima-ipc`: `HOOK_CAPABILITIES` has no duplicates and excludes read-only,
  session and lifecycle tags (`status`, `queue`, `history`, `search`, `login`,
  `logout`, `shutdown`, `subscribe`, `hello`, `ping`); `Command::tag` and
  `EventKind` names round-trip.
- `lekima-cli`: config validation rejects an empty command, unknown events and
  forbidden capabilities; a hook table missing `command` parses (the daemon
  rejects that hook, not the whole config); empty `events` means every kind;
  empty `capabilities` means read-only; declared events and capabilities filter
  both directions; the line reader reassembles split lines, drops an oversized
  line and recovers at the next newline.
- Manual, with a live daemon and a `sh` hook: the hello frame is written with the
  right contract version, `status` frames follow, a declared `set_volume`
  changes the player, an undeclared `set_volume` leaves it alone and logs a
  denial, an invalid hook is skipped and shown by `doctor`, and the child is
  gone after `daemon stop`.

No test asserts a *visual* terminal surface: hooks have none.

## 8. Acceptance

1. `[hooks.x]` with a valid command and `events = ["status"]` receives a hello
   frame and one frame per status change, and nothing else.
2. A hook with `capabilities = ["set_volume"]` can change the volume; the same
   hook without it cannot, and the refusal is logged.
3. An invalid hook does not prevent the daemon from starting.
4. `lekima doctor` reports both valid and invalid hooks, and marks a configured
   program that does not exist.
5. `lekima daemon stop` leaves no direct hook child behind (a hook that spawns
   grandchildren must `exec`).
6. Removing the hook from config and restarting the daemon stops the process.

## 9. Non-goals

- Scrobbling, uploads or any network call from the daemon on a hook's behalf.
- A sandbox, WASM runtime or resource quotas (ADR-015).
- Automatic restart, health checks or a supervision tree.
- A plugin registry, installation command or versioned package format.
- Granting `login`, `logout` or `shutdown` to a hook.
