# AGENTS.md

Guidance for coding agents working in this repository.

## Read first

1. `docs/product-design.md` — what we are building and why.
2. `docs/system-design.md` — layering, crate boundaries, IPC, ADRs.
3. This file — conventions and commands.

## Non-negotiables

- **Dependency rule:** arrows point inward. `lekima-domain` must not gain I/O
  dependencies. Adapters depend on the domain, never the other way round. The
  CLI is the only composition root.
- **One writer:** only `lekima_core::Player` mutates playback/queue state. Never
  mutate state from a frontend or adapter.
- **No `unsafe`.** Every crate uses `#![forbid(unsafe_code)]`.
- **No secrets on argv or in logs.** Session cookies live in
  `<config>/session.cookies` with mode `0600` and are redacted anywhere they
  could be printed.
- **Sanitize provider text** before it reaches a terminal (C0/C1, CSI, OSC,
  bidi). Use the shared helper, never `println!` raw metadata.
- **Errors are typed.** Map to `DomainError`/`ErrorCode`; do not return strings
  from adapters. Frontends own user-facing prose.
- **Agent contract is public API:** command names, JSON fields, error codes and
  exit codes are versioned. Adding is fine; renaming is a major bump.

## Releasing

This machine mirrors crates.io through USTC. Every registry write must name the
real registry:

```bash
cargo publish --dry-run --registry crates-io
cargo publish --registry crates-io
cargo yank --version <v> --registry crates-io
```

Full checklist: `docs/releasing.md`. Never commit a `[patch.crates-io]` path as a
workaround for mirror lag.

## Commands

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo deny check          # when touching dependencies
LEKIMA_CONFIG_DIR=$(mktemp -d) cargo run -p lekima -- status --json
```

## Code style

- Edition 2024, MSRV 1.98, `rustfmt` defaults; no `unwrap()`/`expect()` outside
  tests and `main` (use typed errors).
- Public items documented; docs explain *why*, tests explain *what*.
- Prefer newtypes over raw ids; prefer enums over booleans for modes.
- Keep pure logic in `lekima-domain`/`lekima-core` and cover it with tests. If a
  rule needs a socket or a device to test, it belongs in a frontend instead.
- Conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`).

## Adding a feature (checklist)

1. Model + rules in `lekima-domain` (unit tests).
2. Intent in `lekima-core::Player` (fake ports tests).
3. Provider call in `lekima-ncm` using an `ncmapi` endpoint; if it is missing,
   add it to `ncmapi-rs` with contract tests, release a minor, then bump.
4. Protocol command/reply in `lekima-ipc`.
5. CLI subcommand + JSON output + exit code; TUI key binding + hint.
6. Update `docs/product-design.md` (surface) and `docs/system-design.md`
   (protocol/ADR) if the contract changed.

## Changing `ncmapi`

`ncmapi` is a published dependency (currently `1.7`). When a feature needs a new
endpoint, change the crate in `../ncmapi-rs`, keep the API additive, run its
deterministic suite (`cargo test`, `cargo publish --dry-run`), publish a minor
version, then bump the pin here. Do not reintroduce a path patch unless you are
also preparing that release.
