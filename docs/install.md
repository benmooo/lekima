# Install

## Homebrew (macOS / Linux)

```bash
brew install benmooo/tap/lekima
```

## Cargo

```bash
cargo install lekima --locked
```

Requires Rust 1.98+. Linux also needs ALSA headers:

```bash
sudo apt-get install -y libasound2-dev pkg-config
```

## Release tarball

Download the archive for your platform from
[Releases](https://github.com/benmooo/lekima/releases), verify the `.sha256`,
and put `lekima` on your `PATH`.

macOS binaries are unsigned; if Gatekeeper blocks the first run:

```bash
xattr -d com.apple.quarantine "$(which lekima)"
```

## First run

See [`first-run.md`](./first-run.md).
