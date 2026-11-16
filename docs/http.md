# HTTP control

`lekima serve --http` exposes the daemon's command surface over HTTP for phones,
other machines on the same network, and scripts that cannot reach a Unix socket.
The server owns no state: every request is forwarded to the daemon over the
normal IPC protocol.

```console
$ lekima serve --http                          # 127.0.0.1:8787
$ lekima serve --http --bind 0.0.0.0:8787      # reachable on the LAN
```

## Authentication

The first start generates a 64-character bearer token in `<config>/http.token`
with `0600` permissions and prints its path. Every endpoint except
`/v1/health` and `/v1/capabilities` requires:

```console
$ TOKEN=$(cat /path/printed/by/serve/http.token)
$ curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8787/v1/status
```

`LEKIMA_HTTP_TOKEN` overrides the stored token (minimum 16 characters), which is
useful for containers. The token is never accepted on the command line, never
logged, and never returned by an endpoint.

The default bind is loopback. Binding `0.0.0.0` exposes the API to your network;
keep the token private and prefer a VPN or SSH tunnel for the internet. There is
no CORS support yet, so a browser page on another origin cannot call the API.

## Endpoints

| Method | Path | Auth | Body / notes |
| --- | --- | --- | --- |
| `GET` | `/v1/health` | no | liveness only |
| `GET` | `/v1/capabilities` | no | version, auth scheme, endpoints, command tags |
| `GET` | `/v1/status` | yes | same data as `lekima status --json` |
| `GET` | `/v1/queue` | yes | same data as `lekima queue --json` |
| `POST` | `/v1/command` | yes | one IPC `Command` object, max 1 MiB |

Responses use the CLI envelope:

```json
{"ok":true,"data":{...}}
{"ok":false,"error":{"code":"daemon","message":"..."}}
```

### Commands

`POST /v1/command` accepts the tags listed by `/v1/capabilities`:

```console
$ curl -s -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
    -d '{"type":"play","target":{"type":"query","query":"mota"}}' \
    http://127.0.0.1:8787/v1/command

$ curl -s -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
    -d '{"type":"set_volume","level":0.5}' \
    http://127.0.0.1:8787/v1/command

$ curl -s -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
    -d '{"type":"seek","position_ms":30000}' \
    http://127.0.0.1:8787/v1/command
```

`login`, `logout`, `shutdown`, `hello` and `subscribe` are not exposed. Queue
and playback commands, search, library reads, likes, sleep and quality changes
are.

## Status codes

| Code | Meaning |
| --- | --- |
| `200` | command succeeded |
| `400` | malformed JSON or invalid command shape |
| `401` | missing or wrong bearer token |
| `403` | command tag exists but is not exposed over HTTP |
| `404` | provider resource not found |
| `409` | playback device failure |
| `501` | provider operation unsupported |
| `502` | provider/backend failure |
| `503` | daemon not running |
| `504` | daemon did not answer in time |

The daemon is not started by the HTTP server. Start it once with any playback
command (`lekima play …`) or `lekima daemon start`; otherwise requests return
`503`.
