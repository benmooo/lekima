# Sprint 10 — Daily-driver loop

Status: **In progress** · Started 2026-11-19 · Linear milestone
`Sprint 10 · v0.10+ — Next`

The player can find and play music (0.9.x). What it cannot do is follow the
listener through a normal day: it never tells you what started playing, it
forgets what you heard, and one machine can only hold one account.

Sprint 10 is the first milestone after 0.9 and the first drawn from the
`Later` bucket in [`../product-design.md`](../product-design.md) §4.

## Scope

The milestone in Linear is a parking list, not a sprint: it holds twelve
uncommitted candidates, and its own epic (AKA-142) requires a design note per
item before implementation. This sprint commits only to the first three, in the
order that epic sets — *strengthen the daily-driver loop first*.

| Slice | Linear | Contents | Exit criteria |
| --- | --- | --- | --- |
| A. Notifications | AKA-144 | Desktop notification on track change, opt-in, per OS backend | Design note, `doctor` reports the backend, unit tests for payload and detection |
| B. Listening history | AKA-147 | Durable play log with timestamps and completion, readable from CLI, MCP, HTTP and TUI | Survives restart, bounded by `[history] capacity`, JSON contract test |
| C. Release | — | Docs, changelog, 0.10.0 binaries | Green workspace, published release |

Dropped: **profiles** (AKA-148) — closed by product decision. lekima serves
one person on one machine, so multi-account isolation solves a problem this
product does not have; `LEKIMA_CONFIG_DIR` already relocates all state for
scripts and tests.

Deferred out of this sprint, with their Linear state unchanged:

- **Scrobbling** (AKA-143) — needs external credentials and a scrobble policy;
  own design note.
- **Hook design** (AKA-149) — plugin hooks are the requirement and WASM
  (AKA-146, now a research spike) is one possible substrate; sequence the spike
  first.
- **Windows** (AKA-145 canceled, AKA-152 open) — decision outstanding.
- The three open design questions (AKA-150, AKA-151, AKA-153).

## Design documents

- [`notifications.md`](./notifications.md) — slice A: payload, backends,
  failure modes, config surface.
- [`history.md`](./history.md) — slice B: record shape, what counts as a play,
  retention, persistence, and the agent surface.

## Definition of done

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace` green.
- Every new user-visible setting is documented and visible in `lekima doctor`.
- No new I/O in `lekima-domain`; no new source of playback or queue mutation
  outside `Player` (a listening log is a separate record, not player state).
- Independent verification pass (subagent) against the slice exit criteria
  before the release is cut.

## Non-goals

- An in-app notification centre. The OS owns the surface.
- Notification actions (skip, like) or a click handler.
- Sound, per-event filters, quiet hours, or a scheduler.
