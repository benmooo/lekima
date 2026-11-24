# Sprint 11 — Extensions

Status: **Implemented, unreleased** · Started 2026-11-25

Sprint 10 made the player follow a listener through a normal day: it announces
tracks and remembers what was heard. What it still cannot do is let *someone
else* participate — every integration is a client that polls or shells out, and
nothing can react to playback.

Sprint 11 is the last candidate from the roadmap: plugin hooks (AKA-149), built
as external processes (ADR-015).

## Scope

| Slice | Contents | Exit criteria |
| --- | --- | --- |
| A. Hook contract | Hello frame, event stream, capability-checked commands, config table, versioning | Design note; unit tests for validation, filtering and capability denial |
| B. Supervisor | Spawn, stream, read commands, kill on shutdown, `doctor` reporting | A real script hook changes volume; a hook without the capability cannot |

Out of scope:

- Scrobbling or any cloud upload (ADR-014).
- An in-process runtime or WASM sandbox (ADR-015).
- A plugin registry, package format or scripting language.
- Automatic restart of a crashed hook.

## Design documents

- [`hooks.md`](./hooks.md) — the wire contract, config, failure modes and
  security boundary.

## Definition of done

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace` green.
- A live script hook receives events and changes state through a declared
  capability; an undeclared command is refused and logged.
- `lekima doctor` reports every configured hook and every invalid one.
- Independent verification pass (subagent) before the release is cut.

## Non-goals

- Hooks are not plugins in the "install a package" sense: the user writes the
  process, the daemon only starts it.
- Hooks do not get a second copy of player state; they see events and send
  commands like any other client.
