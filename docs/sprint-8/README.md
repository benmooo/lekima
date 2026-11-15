# Sprint 8 — Offline cache and comments

Status: **Complete** (unreleased)

## Offline cache

- Default capacity **256 MiB**, stored under `<config>/cache/streams/` with
  `0700`/`0600` permissions.
- Fully downloaded streams up to `MAX_MEMORY_BYTES` are cached by
  `<track-id>-<quality>`; large/unknown streams keep the ephemeral spool path.
- LRU eviction by file mtime; atomic writes; oversized entries are refused.
- Cache hits skip `stream_url` resolution, so a cached track plays without the
  provider.
- `PlaybackState.cached` drives a `CACHE` badge; the CLI status shows `[CACHE]`.
- Commands: `lekima cache status|clear|limit <MB>|on|off`; changes restart the
  daemon. `doctor` and `cache status --json` report file count/bytes/limit/path.

## Comments

- Provider: public `comments` endpoint with typed models in the domain
  (`Comment`, `CommentReply`, `Comments`).
- CLI: `lekima comments [--id ID] [--limit N] [--offset N] [--json]`, human
  output separates hot and regular comments.
- MCP: `comments` tool.
- TUI: `C` opens a scrollable comments pane (`j/k`, `Ctrl-d/u`, `r` refresh,
  `C`/`Esc` close), with loading, empty and error states; follows track changes.
- Text is sanitized before display.

## Acceptance

- A second playback of the same track/quality is served from cache and reports
  `cached: true`; the file is `0600` and the directory is `0700`.
- Filling beyond the limit evicts the least-recently-used file; `cache clear`
  removes only `*.audio` files.
- `cache status --json` defaults to `limit_bytes = 268435456`.
- Comments decode hot/regular entries, preserve reply quotes and survive a
  missing comment author.
- `cargo test --workspace`, `clippy -D warnings`, `fmt --check` are green.

Evidence: 164 workspace tests, including cache round-trip/eviction/permission,
comment mapping and TUI comment render/pagination tests. Live verification: a
cached preview reported `cached: true`, and `lekima comments --id 186016`
returned hot and regular comments from the provider.
