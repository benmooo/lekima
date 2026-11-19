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
lekima search "jay chou" --kind all --json
lekima artist show 6452 --json
lekima artist albums 6452 --limit 20 --json
lekima album show 242091487 --json
lekima album queue 242091487 --next --json
lekima history --limit 20 --json
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

JSON-RPC over stdio. Tools: `player_status`, `search`, `artist`,
`artist_albums`, `album`, `play`, `pause`, `resume`, `next`, `previous`,
`set_volume`, `queue_list`, `queue_add`, `queue_album`, `queue_remove`, `queue_move`, `sleep`,
`set_quality`, `lyrics`, `comments`, `history`, `daily`, `fm`.

`search` accepts `kind` (`all`, `track`, `album`, `artist`, `playlist`) and
`play` accepts `query`, `ids`, `playlist` or `album`.

## HTTP control

For agents that cannot reach the Unix socket, `lekima serve --http` exposes the
same commands over HTTP:

```bash
lekima serve --http --bind 0.0.0.0:8787
TOKEN=$(cat "$LEKIMA_CONFIG_DIR/http.token")   # path is printed at startup
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8787/v1/status
curl -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"type":"play","target":{"type":"query","query":"mota"}}' \
  http://127.0.0.1:8787/v1/command
```

`GET /v1/health` and `GET /v1/capabilities` are public; everything else needs
the token. `login`, `logout`, `shutdown` and event subscription are not exposed.
See [http.md](http.md).

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
