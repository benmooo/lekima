# Session exports

Archived OpenCode sessions that produced this repository. They are engineering
records: design discussions, decisions, commands and their output.

| File | Session |
| --- | --- |
| `2026-11-14-lekima-rewrite-rust-daemon-cyberpunk-tui-ncmapi-1-1.{md,json}` | `ses_e28664b7affeuL4iOFcFFhRTAc` — browse setup, product/system design, the Rust rewrite, ncmapi 1.1 release, TUI redesign |

The `.md` is a readable transcript (tool payloads truncated, reasoning
collapsed). The `.json` is the faithful record (`info` + flattened `messages`).

## Re-export

```console
$ python3 scripts/export_session.py ses_<id> [--out-dir docs/sessions]
```

The script calls `opencode session export`, scrubs emails, GitHub/crates.io
tokens and session cookies, then writes both files named
`<created-date>-<title-slug>.{md,json}`.

## Why not `--sanitize`

`opencode session export --sanitize` redacts every message body and tool
payload (`[redacted:text:msg_…]`), which makes the export useless as a record.
The script scrubs only patterns that are actually sensitive.

## Before publishing

Exports contain local paths, track metadata and full tool output. Review a diff
before pushing them to a public repository, and never add a session that
contained credentials or private browsing content.
