# Releasing

This machine uses the **rsproxy.cn crates.io mirror** (`~/.cargo/config.toml`
replaces `crates-io`). Two consequences:

1. **Write commands must name the real registry**: `cargo publish`, `cargo yank`,
   `cargo owner`, `cargo login` all take `--registry crates-io`. Without it they
   operate on (or look for credentials for) the mirror, which is read-only.
2. **Newly published versions may lag** in the mirror index. Reads can fail for a
   short window even though crates.io has the crate. See §5.

## 0. One-time auth

```console
$ cargo login --registry crates-io   # paste a token from crates.io/settings/tokens
```

Token hygiene: scope tokens to the crates you publish (`publish-new`,
`publish-update`, crate-name pattern), set an expiry, and revoke unused ones.
`cargo login` writes them to `~/.cargo/credentials.toml`.

## 1. Preflight

```console
$ cargo fmt --all --check
$ cargo clippy --workspace --all-targets -- -D warnings
$ cargo test --workspace
$ cargo publish --dry-run --registry crates-io
```

`--dry-run` packages and verifies the crate without uploading. Fix anything it
reports before tagging.

## 2. Version and changelog

- Bump `version` in `Cargo.toml` (SemVer; additive endpoints are minor).
- Add a `CHANGELOG.md` entry.
- Update `README.md` examples if the public API changed.

## 3. Tag, then publish

```console
$ git commit -am "release: ncmapi 1.2.0"
$ git tag -a v1.2.0 -m "ncmapi 1.2.0"
$ git push origin main
$ git push origin v1.2.0
$ cargo publish --registry crates-io
```

Cargo waits until the version is visible in the index; the crate is live when
it prints `Published <crate> v<version> at registry crates-io`.

Create the GitHub release from the tag (`gh release create v1.2.0 --notes …`).

## 4. Yanking

Only for broken releases; users with a lockfile keep working.

```console
$ cargo yank --version 1.2.0 --registry crates-io
$ cargo yank --version 1.2.0 --undo --registry crates-io   # unyank
```

## 5. Mirror lag (this machine only)

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
4. Point this workspace at another mirror that has already synced, via a
   **gitignored** `.cargo/config.toml` (already ignored by the repo):
   ```toml
   [source.crates-io]
   replace-with = "ustc"

   [source.ustc]
   registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"
   ```
   Check any mirror with
   `curl -s <index>/<first2>/<next2>/<crate> | tail -1`.

Do not commit a `[patch.crates-io]` path entry to work around lag: it breaks
`cargo install` for everyone else.

## 6. Dependent crates

After publishing, update dependents to the new registry version and confirm the
lockfile source:

```console
$ cargo update -p <crate> --precise <version>
$ grep -A2 'name = "<crate>"' Cargo.lock   # source = registry+https://github.com/rust-lang/crates.io-index
```

## 7. Checklist

- [ ] `fmt`, `clippy`, `test`, `publish --dry-run --registry crates-io`
- [ ] version + changelog + README
- [ ] commit, tag, push `main` and the tag
- [ ] `cargo publish --registry crates-io`
- [ ] GitHub release
- [ ] dependent updated to the registry version (no path patch)
- [ ] mirror has caught up (or the lag workaround is documented in the PR)
