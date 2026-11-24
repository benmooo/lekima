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
- **`resync`** means the hook lagged behind the broadcast ring; it should re-read
  what it cares about (`status`, `history`, …) if a command it sends is
  capability-granted. Its `seq` is the last frame written.
- **Versioning** is additive: the hello names the contract version, new fields
  may appear, and a hook that does not understand a frame can ignore it. Bumping
  `protocol` is a breaking change.
- **stdout is reserved** for commands; stdout noise is a protocol error and is
  logged. Hooks log to stderr, which the daemon inherits into `lekima.log`.

## 3. Configuration

```toml
[hooks.nowplaying]
command = ["/Users/me/bin/nowplaying"]
events = ["status"]            # default: every kind
capabilities = ["pause"]       # default: none, so read-only

[hooks.history-sync]
command = ["/Users/me/bin/sync-history", "--to", "my-server"]
events = ["status", "queue"]
capabilities = ["history"]
```

- The table name is the hook's identity in logs and `doctor`.
- `command` is an executable plus arguments; it is never a shell string.
- `events` accepts `status`, `queue`, `like`, `resync`.
- `capabilities` accepts the command tags that appear on the wire and in
  `lekima capabilities`; `login`, `logout`, `shutdown`, `subscribe`, `hello` and
  `ping` are deliberately **not** grantable.
- Configuration is read at daemon start; changing it needs a restart
  (`lekima daemon stop`, then any command autostarts it).

## 4. Lifecycle and failure modes

| Situation | Behaviour |
| --- | --- |
| Invalid config (empty command, unknown event, forbidden capability) | Logged at warn, hook skipped, daemon starts; `doctor` shows the error |
| Executable missing or not runnable | Logged at warn, hook skipped |
| Hook exits or crashes | Its tasks end; **no automatic restart**. Restart the daemon to bring it back |
| Hook stops reading stdin | Its writer task blocks; other hooks and playback are unaffected; events for that hook are dropped |
| Hook lags | A `resync` frame is written with the last sequence number |
| Hook sends invalid JSON | Logged at debug, line ignored |
| Hook sends an undeclared command | Logged at warn, command dropped |
| Daemon shuts down | Hook stdin closes and the child is killed (`kill_on_drop`) |

Hooks run on their own tasks and never share the daemon's runtime loop that
drives playback, so a slow or wedged hook cannot stall Now Playing, media keys,
notifications or the queue.

## 5. Security

- A hook runs as the same user, with the same filesystem access. The capability
  list is an API guard, **not** a sandbox; a hook that wants to read
  `session.cookies` can. Only run hooks you wrote or trust — the config file is
  already inside the user's trust boundary.
- Session and lifecycle commands are not grantable, so a mistaken hook cannot
  log the user out or stop the daemon.
- No secrets are passed to a hook: the hello frame carries the version, the
  event kinds and the capability names, nothing else.
- Hook stderr goes to the daemon log; it is never written to a terminal by
  `lekima` itself.

## 6. Observability

`lekima doctor` lists every configured hook:

```json
"hooks": [
  { "name": "nowplaying", "command": ["/Users/me/bin/nowplaying"],
    "events": ["status"], "capabilities": ["pause"], "ok": true },
  { "name": "bogus", "ok": false,
    "error": "`shutdown` is not a hook capability; allowed: …" }
]
```

The daemon log records `hook started`, `hook command denied`, `hook lagged`,
`hook exited` and `hook stopped`.

## 7. Tests

- `lekima-ipc`: every command tag maps to a capability name; forbidden tags
  (`login`, `logout`, `shutdown`, `subscribe`, `hello`, `ping`) are not in
  `HOOK_CAPABILITIES`; `EventKind` names round-trip.
- `lekima-cli`: config validation rejects an empty command, unknown events and
  forbidden capabilities; empty `events` means every kind; empty `capabilities`
  means read-only; declared events and capabilities filter both directions.
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
4. `lekima doctor` reports both valid and invalid hooks.
5. `lekima daemon stop` leaves no hook process behind.
6. Removing the hook from config and restarting the daemon stops the process.

## 9. Non-goals

- Scrobbling, uploads or any network call from the daemon on a hook's behalf.
- A sandbox, WASM runtime or resource quotas (ADR-015).
- Automatic restart, health checks or a supervision tree.
- A plugin registry, installation command or versioned package format.
- Granting `login`, `logout` or `shutdown` to a hook.
