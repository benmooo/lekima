# Sprint 12 · Playlist editing

Status: **Shipped in 0.13.0** · Linear: new issues to be opened from the slice table

## 1. Problem

`lekima` can list playlists, open one, play it and queue it. It cannot create
one, rename it, or put a track into it — the only documented way to change a
playlist is the web player. Agents hit the same wall: MCP exposes
`queue_playlist`, not `playlist_add`.

## 2. Provider routes (`ncmapi 1.10.3`)

All are `WEAPI` POSTs; the transport already appends `csrf_token` from the
session cookie, so no new signing code is needed. The first attempt used a
combined `/api/playlist/update` and `/api/playlist/delete`; both are wrong, and
live testing found the real routes:

| Operation | Route | Payload | Response |
| --- | --- | --- | --- |
| Create | `POST /api/playlist/create` | `name`, `privacy` (`0` public, `10` private) | `{ code, id, playlist? }` |
| Rename | `POST /api/playlist/update/name` | `id`, `name` | `{ code }` |
| Describe | `POST /api/playlist/desc/update` | `id`, `desc` | `{ code }` |
| Manipulate | `POST /api/playlist/manipulate/tracks` | `op` (`add`/`del`), `pid`, `trackIds` (JSON string), `imme` (`true`) | `{ code }` |
| Delete | `POST /api/playlist/remove` | `ids` as a JSON array string | `{ code }` |

A combined `update` route and `playlist/delete` with `id` answer `404 接口未找到`
and `400 请求参数错误` respectively; ncmapi 1.10.1 carries the route fix; 1.10.2 the duplicate/ordering fixes; 1.10.3 narrows the duplicate `502` to its provider message.

- Responses are decoded by the existing `ApiResponse`; a non-200 `code` is a
  provider error, not a transport error.
- `playlist/create` is the only non-idempotent call: the CLI prints the new id
  and never retries on timeout.
- Adding an already-present track answers `502 歌单内歌曲重复`; `ncmapi` treats
  that specific response as success, so adding and retrying are safe. Any other
  `502` still surfaces as a backend error. Removing a track that is not
  there is a provider no-op.
- Deleting a playlist is idempotent **at the provider**: a missing id still
  answers `200`, so `playlist delete` reports success for an id that is not
  there. The client does not pre-check ownership.

## 3. Domain and adapter

`MusicService` gains five methods, with `Unsupported` defaults so existing fakes
and offline implementations keep compiling:

```rust
async fn playlist_create(&self, name: &str, description: &str, private: bool) -> Result<Playlist>;
async fn playlist_update(&self, id: PlaylistId, name: Option<&str>, description: Option<&str>) -> Result<()>;
async fn playlist_add(&self, id: PlaylistId, tracks: &[TrackId]) -> Result<()>;
async fn playlist_remove(&self, id: PlaylistId, tracks: &[TrackId]) -> Result<()>;
async fn playlist_delete(&self, id: PlaylistId) -> Result<()>;
```

`NcmMusicService` maps them to the new `ncmapi` calls and returns a minimal
`Playlist { id, name, track_count: 0, … }` for create (the provider does not
return the full page; a later `playlist show` is authoritative).

Errors follow the existing taxonomy: `not_found` when the provider says the
playlist does not exist, `unauthorized` when the session expired, `invalid_input`
for a rejected name or track, `backend` otherwise. Deleting a missing id is
provider-idempotent and reports success. A missing
`playlist/create` id in a 200 response is `backend`, not a panic.

## 4. IPC, CLI and surfaces

| Command | Payload | Reply |
| --- | --- | --- |
| `playlist_create` | `name`, `description`, `private` | `Playlist { playlist }` |
| `playlist_update` | `id`, `name?`, `description?` | `Ack` |
| `playlist_add` | `id`, `tracks` | `Ack` |
| `playlist_remove` | `id`, `tracks` | `Ack` |
| `playlist_delete` | `id` | `Ack` |

CLI:

```console
$ lekima playlist create "Late night" --description "after 22:00" --private --json
{"ok":true,"data":{"type":"playlist","playlist":{"id":123,"name":"Late night",…}}}

$ lekima playlist rename 123 "Late night 2"
$ lekima playlist describe 123 "quiet hours"
$ lekima playlist add 123 --ids 42,43
$ lekima playlist remove 123 --ids 42
$ lekima playlist delete 123 --yes
```

- `add`/`remove` take `--ids` only. Resolving a search query into ids is a
  second, ambiguous decision; users get ids from `lekima search --json`.
- `delete` requires `--yes`; the TUI asks for confirmation.
- All are capability tags for hooks (`playlist_create`, …), so a hook can
  maintain a playlist with the same declared authority.

MCP tools: `playlist_create`, `playlist_update`, `playlist_add`,
`playlist_remove`, `playlist_delete` (no `readOnlyHint`).

HTTP: the five tags are added to `COMMAND_TYPES` and to the permitted set;
`playlist_delete` is permitted because the endpoint already requires the token
and the CLI has a confirmed path. `GET /v1/hooks` style read routes are not
added: the reply is an acknowledgement, not a resource.

## 5. TUI

Command line only in this slice, because a full playlist editor (modal list,
multi-select, cover) is a design of its own:

| Input | Effect |
| --- | --- |
| `:playlist create NAME` | Creates a playlist; the sidebar refreshes |
| `:playlist add ID` | Adds the current track to playlist `ID` |
| `:playlist delete ID yes` | Deletes; the literal `yes` is the confirmation |

The `?` overlay names `:playlist` under the command-line binding. A dedicated
pane is a follow-up, not a hidden requirement.

## 6. Tests

- `ncmapi`: one fixture per route asserting URL, method and payload shape
  (including `trackIds` as a JSON string and `imme`), plus a non-200 `code`
  mapping and a create response without `id` being a `backend` error.
- `lekima-domain`/`lekima-core`: the default port methods return `Unsupported`,
  so a fake service cannot silently pretend to write.
- `lekima-ipc`: command/reply wire shapes and tags.
- `lekima-cli`: usage errors (`--ids` required, empty list, `delete` without
  `--yes`), exit codes, and JSON envelope.
- `lekima-tui`: `:playlist create/add/delete` reducers and the confirmation.
- Live: create → add → rename → remove → delete against the signed-in account.

## 7. Acceptance

1. `lekima playlist create` returns an id; `playlist show` sees it immediately.
2. Adding and removing a track changes `playlist show` on the next fetch.
3. A private playlist is not listed on the public profile.
4. A missing session yields `unauthorized` (exit 3) without a provider stack
   trace.
5. Deleting a playlist removes it from `playlist list`; the TUI refreshes the
   sidebar on the ack.
6. MCP `playlist_add` and `POST /v1/command` with `{"type":"playlist_add"}` do
   the same thing as the CLI.

## 8. Non-goals

- Reordering, cover upload, sharing, collaborative edits, folders.
- Optimistic UI: the TUI refreshes after the ack, it does not pre-apply.
- Bulk import from a file (a hook or script can loop over `playlist add`).
