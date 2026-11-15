# Agents

lekima is designed to be driven by code agents. Two surfaces:

## CLI + JSON

Every command supports `--json` and returns one object:

```json
{"ok":true,"data":{…}}
{"ok":false,"error":{"code":"unauthorized","message":"…"}}
```

Exit codes: `0` ok, `2` usage, `3` unauthorized, `4` not found, `5` backend,
`6` playback, `7` daemon, `8` unsupported. Errors never go to stdout.

```bash
lekima play "mota" --json
lekima queue add "teardrop" --next --json
lekima status --json | jq -r .data.status
```

## Events: `lekima watch`

A dedicated connection streams sequenced NDJSON frames:

```json
{"seq":42,"type":"status","state":{…}}
{"seq":43,"type":"queue","tracks":[…],"index":0}
```

```bash
lekima watch --events status,queue
lekima watch --since 128 | jq 'select(.type=="track")'
```

Use `--since` to replay from the daemon's 256-frame ring; a `resync` frame means
you lagged and should fetch `status`/`queue`.

## MCP

```bash
lekima mcp
```

JSON-RPC over stdio. Tools: `player_status`, `search`, `play`, `pause`,
`resume`, `next`, `previous`, `set_volume`, `queue_list`, `queue_add`,
`queue_remove`, `queue_move`, `sleep`, `set_quality`, `lyrics`, `comments`,
`daily`, `fm`.

## Cache

```bash
lekima cache status --json      # files, bytes, limit_bytes, path, enabled
lekima cache limit 256          # MiB; restarts the daemon
lekima cache clear
```

The default limit is 256 MiB. Cached playback reports `"cached": true` in
`status`.

Host config:

```json
{"mcpServers":{"lekima":{"command":"/usr/local/bin/lekima","args":["mcp"]}}}
```

Read-only tools carry `readOnlyHint`; failures return `isError: true` with
`structuredContent.error.code`.
