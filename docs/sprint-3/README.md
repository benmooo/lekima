# Sprint 3 — Ship 0.1

| Document | Contents |
| --- | --- |
| [`product-design.md`](./product-design.md) | Audience, distribution channels, CI, queue editing, translation, docs, risks |
| [`system-design.md`](./system-design.md) | Release pipeline, CI, integration harness, queue invariants, lyric merge, ADRs |

Status: **In progress.**

- Design approved.
- Queue editing (TUI/CLI/MCP) and lyric translation implemented and tested.
- Repo renamed: old Go project is `benmooo/lekima-go`; Rust project is
  `benmooo/lekima`. CI is green on macOS + Linux + MSRV.
- Remaining: cargo-dist release workflow, crates.io publishing (the workspace
  crates must be published in dependency order), Homebrew tap, install/
  first-run/agents docs, `v0.1.0` tag.
