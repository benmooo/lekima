# Contributing

Thanks for helping. Before opening a PR:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Read `docs/system-design.md` and `docs/sprint-2/tui-visual-design.md`.
- Keep changes additive to the CLI/JSON/MCP contract; breaking changes need a
  major version.
- New provider calls go through `ncmapi` (see `docs/releasing.md`).
- TUI: no external I/O in the draw path; add a reducer or `TestBackend` test.
- Never commit cookies, tokens or `[patch.crates-io]` entries.

By contributing you agree your work is licensed under MIT.
