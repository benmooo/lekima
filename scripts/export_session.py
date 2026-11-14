#!/usr/bin/env python3
"""Export an OpenCode session into a scrubbed Markdown transcript and JSON.

Why not `opencode session export --sanitize`? That flag redacts every message
and tool payload, leaving a useless export. This script strips only actual
secrets (emails, GitHub/crates.io tokens, session cookies) and truncates tool
payloads for readability, while keeping the full JSON alongside.

Usage:
    python3 scripts/export_session.py <session-id> [--out-dir docs/sessions]
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
SECRETS = [
    (re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"), "<redacted-email>"),
    (re.compile(r"github_pat_[A-Za-z0-9_]{20,}"), "<redacted-github-token>"),
    (re.compile(r"gh[pousr]_[A-Za-z0-9]{20,}"), "<redacted-github-token>"),
    (re.compile(r"cio[A-Za-z0-9]{20,}"), "<redacted-crates-token>"),
    (re.compile(r"(MUSIC_U=)[^;\"\\\s]+"), r"\1<redacted>"),
    (re.compile(r"(__csrf=)[^;\"\\\s]+"), r"\1<redacted>"),
]
TOOL_INPUT_LIMIT = 1600
TOOL_OUTPUT_LIMIT = 2400
REASONING_LIMIT = 1200


def scrub(text: str) -> str:
    for pattern, replacement in SECRETS:
        text = pattern.sub(replacement, text)
    return ANSI.sub("", text)


def truncate(text: str, limit: int) -> str:
    text = text.strip()
    if len(text) <= limit:
        return text
    return text[:limit].rstrip() + f"\n… [truncated, {len(text) - limit} chars omitted]"


def fence(text: str, language: str = "") -> str:
    return f"````{language}\n{text}\n````"


def stamp(millis: int | None) -> str:
    if not millis:
        return "?"
    return datetime.fromtimestamp(millis / 1000, tz=timezone.utc).strftime(
        "%Y-%m-%d %H:%M:%S UTC"
    )


def slugify(title: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
    return slug[:60] or "session"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("session")
    parser.add_argument("--out-dir", default="docs/sessions", type=Path)
    args = parser.parse_args()

    result = subprocess.run(
        ["opencode", "session", "export", args.session],
        check=True,
        capture_output=True,
        text=True,
    )
    raw = result.stdout
    for pattern, replacement in SECRETS:
        raw = pattern.sub(replacement, raw)
    for pattern, _ in SECRETS[:4]:
        if pattern.search(raw):
            print(f"refusing to write: secret survived scrubbing ({pattern.pattern})", file=sys.stderr)
            return 1

    data = json.loads(raw)
    info = data["info"]
    created = datetime.fromtimestamp(info["time"]["created"] / 1000, tz=timezone.utc)
    stem = f"{created:%Y-%m-%d}-{slugify(info['title'])}"
    args.out_dir.mkdir(parents=True, exist_ok=True)
    md_path = args.out_dir / f"{stem}.md"
    json_path = args.out_dir / f"{stem}.json"
    json_path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")

    lines = [
        f"# Session export — {info['title']}",
        "",
        f"- **Session**: `{info['id']}`",
        f"- **Agent / model**: {info.get('agent', '?')} · "
        f"{info['model']['providerID']}/{info['model']['id']} "
        f"({info['model'].get('variant', 'default')})",
        f"- **Created**: {stamp(info['time'].get('created'))}",
        f"- **Last update**: {stamp(info['time'].get('updated'))}",
        f"- **Tokens**: {info['tokens']['input']:,} in · {info['tokens']['output']:,} out · "
        f"{info['tokens'].get('reasoning', 0):,} reasoning · "
        f"{info['tokens'].get('cache', {}).get('read', 0):,} cache read",
        f"- **Cost**: ${info.get('cost', 0)}",
        f"- **Raw export**: [`{json_path.name}`](./{json_path.name})",
        "",
        "> Tool calls are summarised: inputs and outputs are truncated, and the",
        "> transcript is scrubbed of emails, tokens and session cookies. The JSON",
        "> next to this file is the faithful record (also scrubbed).",
        "",
        "---",
        "",
    ]

    for message in data["messages"]:
        kind = message.get("type")
        if kind == "user":
            text = scrub(message.get("text", "")).strip()
            if text:
                lines += ["### User", "", text, "", "---", ""]
            continue
        if kind != "assistant":
            continue

        wrote_header = False
        for part in message.get("content", []):
            ptype = part.get("type")
            if ptype == "text":
                text = scrub(part.get("text", "")).strip()
                if not text:
                    continue
                if not wrote_header:
                    lines += ["### Assistant", ""]
                    wrote_header = True
                lines += [text, ""]
            elif ptype == "reasoning":
                text = truncate(scrub(part.get("text", "")), REASONING_LIMIT)
                if text:
                    lines += [
                        "<details><summary>reasoning</summary>",
                        "",
                        text,
                        "",
                        "</details>",
                        "",
                    ]
            elif ptype == "tool":
                name = part.get("name", "tool")
                state = part.get("state", {}) or {}
                status = state.get("status", "?")
                payload = state.get("input")
                rendered = (
                    truncate(json.dumps(payload, indent=2, ensure_ascii=False), TOOL_INPUT_LIMIT)
                    if payload is not None
                    else ""
                )
                outputs = [
                    chunk.get("text", "")
                    for chunk in state.get("content", []) or []
                    if isinstance(chunk, dict) and chunk.get("type") == "text"
                ]
                output = truncate(scrub("\n".join(outputs)), TOOL_OUTPUT_LIMIT)
                if not wrote_header:
                    lines += ["### Assistant", ""]
                    wrote_header = True
                lines += [
                    f"<details><summary>tool <code>{name}</code> · {status}</summary>",
                    "",
                ]
                if rendered:
                    lines += [fence(rendered, "json"), ""]
                if output:
                    lines += [fence(output), ""]
                lines += ["</details>", ""]
        if wrote_header:
            lines += ["---", ""]

    md_path.write_text("\n".join(lines).rstrip() + "\n")
    print(f"wrote {md_path} ({md_path.stat().st_size:,} bytes)")
    print(f"wrote {json_path} ({json_path.stat().st_size:,} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
