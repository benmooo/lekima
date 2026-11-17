# Releasing

This machine uses the **rsproxy.cn crates.io mirror** (`~/.cargo/config.toml`
replaces `crates-io` with the sparse index). Every registry write must name the
real registry: `cargo publish`, `cargo yank`, `cargo owner` and `cargo login` all
take `--registry crates-io`. Without it they operate on (or look for credentials
for) the mirror, which is read-only. Newly published versions can also lag
briefly in the mirror index; see §3.

There are two release shapes:

| Shape | Artifact | Pipeline |
| --- | --- | --- |
| **App release** (`lekima`) | GitHub binaries + Homebrew tap | tag → Actions → tap formula |
| **Library release** (`ncmapi`) | crates.io package | publish → tag → dependent pin |

## 0. One-time auth

```console
$ cargo login --registry crates-io   # paste a token from crates.io/settings/tokens
```

Token hygiene: scope tokens to the crates you publish, set an expiry and revoke
unused ones. The current `ncmapi` token is restricted to the `ncmapi` pattern;
publishing the app workspace crates needs a second token scoped to `lekima*`.
`cargo login` writes tokens to `~/.cargo/credentials.toml`.

## 1. App release — `lekima`

### 1.1 Preflight

```console
$ cargo fmt --all --check
$ cargo clippy --workspace --all-targets -- -D warnings
$ cargo test --workspace
```

Also render the TUI examples for the changed surfaces:

```console
$ cargo run -p lekima-tui --example preview -- --width 120 --height 30
```

### 1.2 Version and changelog

- Bump `version` in the workspace `Cargo.toml`, every internal `lekima-*`
  dependency, and `crates/lekima-platform/Cargo.toml`. Every published crate
  shares one version line, so an unchanged crate is re-published at the same
  number instead of keeping a second line.
- Add a `CHANGELOG.md` entry with the date.
- Update the `## Status` paragraph in `README.md` and the scope table in
  `docs/product-design.md`.

### 1.3 Tag and build

```console
$ git add -A
$ git commit -m "release: lekima 0.8.0"
$ git tag -a v0.8.0 -m "lekima 0.8.0"
$ git push origin main
$ git push origin v0.8.0
$ gh run list --branch v0.8.0 --limit 1
$ gh run watch <run-id> --exit-status
```

The workflow builds macOS arm64/x86_64 and Linux arm64/x86_64 tarballs plus
`.sha256` files, then creates the GitHub release.

### 1.4 Homebrew tap

The tap lives at `benmooo/homebrew-tap`; Homebrew keeps it under
`$(brew --repository benmooo/tap)`.

1. Download the published checksums:
   ```console
   $ gh release download v0.8.0 --pattern '*.sha256' --dir /tmp/lekima-shas
   $ for f in /tmp/lekima-shas/*.sha256; do cat "$f"; done
   ```
2. Update `Formula/lekima.rb`: four `url` lines and four `sha256` values.
3. Commit and push:
   ```console
   $ git -C "$(brew --repository benmooo/tap)" add Formula/lekima.rb
   $ git -C "$(brew --repository benmooo/tap)" commit -m "lekima 0.8.0"
   $ git -C "$(brew --repository benmooo/tap)" push origin main
   ```
4. Upgrade and verify:
   ```console
   $ brew upgrade lekima
   $ lekima --version
   $ lekima daemon stop && lekima daemon start
   ```

Restarting the daemon makes the running player use the new binary. A running
TUI keeps its old process until relaunched; say so in the release notes.

## 2. Library release — `ncmapi`

`ncmapi` is a published dependency. When `lekima` needs a new provider
endpoint, change the crate in `../ncmapi-rs`, keep the API additive, and release
a minor:

```console
$ cd ../ncmapi-rs
$ cargo fmt --all --check
$ cargo test
$ cargo publish --dry-run --registry crates-io
$ git tag -a v1.9.0 -m "ncmapi 1.9.0"
$ git push origin main v1.9.0
$ cargo publish --registry crates-io
```

Then bump the pin in `lekima` (`ncmapi = "1.9"`), run
`cargo update -p ncmapi`, and verify the lockfile source:

```console
$ grep -A2 'name = "ncmapi"' Cargo.lock
```

Never commit a `[patch.crates-io]` path entry. The only exception is an explicit
pre-release spike, removed before the merge.

### App crates on crates.io

Every published crate shares the workspace version, so one release publishes
the whole workspace at one number even when a crate did not change. Publishing needs a token scoped to `lekima*` with `publish-new`,
`publish-update` and `yank` (`cargo login --registry crates-io`). Publish in
dependency order:

```
lekima-domain → lekima-platform → lekima-config → lekima-ipc → lekima-core →
lekima-ncm → lekima-audio → lekima-tui → lekima
```

Verify with `cargo publish --dry-run -p <crate> --registry crates-io` first.

## 3. Mirror lag

After publishing, the rsproxy index can lag a few minutes behind crates.io.
Symptom: `cargo check` says *“candidate versions found which didn't match”* and
names the older version.

Options:

1. **Wait** and retry. This is the normal fix; nothing else is wrong.
2. Verify the index directly:
   ```console
   $ curl -s https://rsproxy.cn/index/<first2>/<next2>/<crate> | tail -1
   ```
   (for `ncmapi`: `nc/ma/ncmapi`). Compare with
   `https://crates.io/api/v1/crates/ncmapi`.
3. Build against the official index once, by temporarily moving the mirror
   config aside (restored automatically by the `trap`):
   ```console
   $ mv ~/.cargo/config.toml ~/.cargo/config.toml.bak
   $ cargo check -p <crate>
   $ mv ~/.cargo/config.toml.bak ~/.cargo/config.toml
   ```
4. If rsproxy is the lagging one, point this workspace at another mirror that
   has already synced, via a **gitignored** `.cargo/config.toml` (already
   ignored by the repo):
   ```toml
   [source.crates-io]
   replace-with = "ustc"

   [source.ustc]
   registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"
   ```
   Check any mirror with
   `curl -s <index>/<first2>/<next2>/<crate> | tail -1`.

## 4. Yanking

Only for broken releases; users with a lockfile keep working.

```console
$ cargo yank --version 1.9.0 --registry crates-io
$ cargo yank --version 1.9.0 --undo --registry crates-io   # unyank
```

## 5. Checklist

- [ ] `fmt`, `clippy`, `test`, preview renders
- [ ] version + changelog + README status + product scope
- [ ] commit, tag, push `main` and the tag
- [ ] GitHub Actions release is green and all four tarballs exist
- [ ] tap formula URLs and four checksums updated
- [ ] `brew upgrade lekima` reports the new version
- [ ] daemon restarted on the new binary; TUI relaunch noted
- [ ] dependent crates pinned to the published library version (no path patch)
- [ ] crates.io app-crate publication handled or explicitly deferred
