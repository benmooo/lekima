# Sprint 3 — Ship 0.1

| Document | Contents |
| --- | --- |
| [`product-design.md`](./product-design.md) | Audience, distribution channels, CI, queue editing, translation, docs, risks |
| [`system-design.md`](./system-design.md) | Release pipeline, CI, integration harness, queue invariants, lyric merge, ADRs |

Status: **Shipped — 0.1.1.**

- Queue editing (TUI/CLI/MCP) and lyric translation implemented.
- Repo renamed: `benmooo/lekima-go` (archived Go project) and
  `benmooo/lekima` (Rust). CI green on macOS + Linux + MSRV.
- `ncmapi 1.6` is pure Rust: no OpenSSL, rustls for TLS; ciphertext fixtures
  unchanged.
- Published: `lekima` + all workspace crates on crates.io; GitHub release
  `v0.1.1` with four platform tarballs + checksums; Homebrew formula in
  `benmooo/homebrew-tap`.
- Verified: `brew install benmooo/tap/lekima`, `cargo install lekima`, binary
  has no OpenSSL linkage, Homebrew test passes.
