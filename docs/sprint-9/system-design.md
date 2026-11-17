# Sprint 9 — System design: catalogue depth

Status: **Shipped in 0.9.0**

## 1. Layering

No new crate. The dependency rule is unchanged:

```
lekima-domain  ←  lekima-core  ←  lekima-ipc  ←  lekima-cli / lekima-tui
      ↑                 ↑
  lekima-ncm ───────────┘        (implements MusicService)
```

Catalogue reads are provider calls: they stay in `lekima-ncm`. The domain gains
read models and port methods; the core does not gain I/O.

## 2. Provider — `ncmapi 1.9.0`

Everything below was verified against the live public endpoints on 2026-11-17
with plain GET/POST requests and no session.

### 2.1 Search

`POST https://music.163.com/api/cloudsearch/pc` already exists. Add album
results to the model:

```rust
pub struct SearchResults {
    pub songs: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,        // new, #[serde(default)]
    pub album_count: u64,          // new, #[serde(default)]
}
```

`type=10` returns `result.albums` and `result.albumCount`; the existing
`songs`/`artists`/`playlists` fields stay. `type=1018` (all) returns an empty
result on this endpoint, so `All` is not a provider kind — see ADR-001.

### 2.2 Artist detail

`GET https://music.163.com/api/artist/head/info/get?id={id}`

Relevant shape:

```json
{"code":200,"data":{"artist":{
  "id":6452,"name":"周杰伦","avatar":"https://…","cover":"https://…",
  "albumSize":49,"musicSize":592,"briefDesc":"…",
  "alias":["Jay Chou","周董"],"identities":["作曲"]
}}}
```

Add `ArtistDetail` with `id`, `name`, `avatar_url`, `cover_url`, `album_size`,
`music_size`, `brief_desc`, `aliases`, `identities`; every field is
null-tolerant.

### 2.3 Artist albums

`GET https://music.163.com/api/artist/albums/{id}?limit={limit}&offset={offset}`

Relevant shape: `{code, artist, hotAlbums:[{id,name,picUrl,publishTime,size,…}],
more}`. Add `ArtistAlbumsBody { artist: Option<Artist>, hot_albums: Vec<Album>,
more: bool }`. Reuse the `Album` model, extended with optional `publish_time` and
`size`.

### 2.4 Album detail

`GET https://music.163.com/api/v1/album/{id}`

Relevant shape: `{code, album:{id,name,picUrl,publishTime,size,artist,description},
songs:[Track]}`. Add `AlbumDetailBody { album: Album, songs: Vec<Track> }`.

### 2.5 SDK conventions

- Methods live on `Discovery` (`artist`, `artist_albums`, `album`) next to
  `search` and `toplists`.
- Plain `RequestPlanBuilder::get`; no encryption, no cookie requirement; the
  response cache applies.
- Validate `id > 0` and page limits (`Page::new`) before the request, returning
  `Error::InvalidInput`.
- Models use `#[serde(default, deserialize_with = "null_default")]` for every
  scalar/vector the provider has been seen sending as `null`.
- Tests: one fixture per endpoint (including null fields), one URL contract
  test, and a search test that decodes albums. Live smoke is manual.

### 2.6 Release

`1.9.0` (additive): changelog, tag, `cargo publish --registry crates-io`, then
bump `lekima` to `ncmapi = "1.9"`. Never a path patch.

## 3. Domain

### 3.1 Models

Extend `Album` (all fields optional/additive in JSON):

```rust
pub struct Album {
    pub id: AlbumId,
    pub name: String,
    pub cover_url: Option<String>,
    #[serde(default)] pub artist: Option<Artist>,
    #[serde(default)] pub year: Option<u16>,
    #[serde(default)] pub track_count: Option<u32>,
    #[serde(default)] pub description: String,
}
```

New read models:

```rust
pub struct ArtistDetail {
    pub id: ArtistId, pub name: String,
    pub avatar_url: Option<String>, pub cover_url: Option<String>,
    pub album_count: u64, pub track_count: u64,
    pub description: String, pub aliases: Vec<String>,
}

pub struct ArtistAlbums { pub albums: Vec<Album>, pub has_more: bool }

pub struct AlbumDetail { pub album: Album, pub tracks: Vec<Track> }
```

`SearchResults` gains `albums: Vec<Album>` and `SearchTotals` (tracks, albums,
artists, playlists). `total` stays as the sum for existing consumers.

`SearchKind` gains `All` (serialized `"all"`); `as_str` updated.

### 3.2 Port

```rust
async fn artist(&self, id: ArtistId) -> Result<ArtistDetail>;
async fn artist_albums(&self, id: ArtistId, limit: u32, offset: u32)
    -> Result<ArtistAlbums>;
async fn album(&self, id: AlbumId) -> Result<AlbumDetail>;
```

`MusicService::search` keeps its signature; the adapter handles `All` by
fanning out to Track/Album/Artist/Playlist with a small per-kind limit and
merging the results (ADR-001).

### 3.3 Core

Playback rules do not change. `PlayTarget::Album { id }` is resolved by the
daemon player through `MusicService::album`, then the existing
replace-queue-and-play path. Queueing uses the same fetch with append semantics
(ADR-004).

## 4. IPC

Additive commands and replies; no protocol-major bump.

```rust
Command::Artist { id: ArtistId }
Command::ArtistAlbums { id: ArtistId, limit: u32, offset: u32 }
Command::Album { id: AlbumId }
Command::QueueAlbum { id: AlbumId, next: bool }

Reply::Artist { artist: ArtistDetail }
Reply::ArtistAlbums { page: ArtistAlbums }
Reply::Album { detail: AlbumDetail }

PlayTarget::Album { id: AlbumId }
```

- Requests are read-only except `QueueAlbum` and `Play`.
- Errors map through the existing `DomainError` → `ErrorCode` table:
  `not_found`, `backend`, `unsupported`, `unauthorized`.
- Protocol round-trip tests for every variant; `hello` protocol stays 2.
- Older clients that do not know the new replies are unaffected because they
  never send the new commands.

## 5. CLI and agent surfaces

- `lekima search <query> --kind track|album|artist|playlist|all` (default
  `track`; `all` merges).
- `lekima artist show <id>`, `lekima artist albums <id> [--limit] [--offset]`.
- `lekima album show <id>`, `lekima album queue <id> [--next]`,
  `lekima album play <id>`.
- Human output uses the same sanitized table/list helpers as playlists.
- MCP: `artist`, `artist_albums`, `album` tools with `readOnlyHint: true`;
  queue/play actions reuse `queue_add`/`play` semantics rather than new
  mutating tools unless a target is needed.
- HTTP: `/v1/command` default-deny allowlist gains the three read-only commands;
  album play/queue remain local unless explicitly enabled.

## 6. TUI

### 6.1 State

```rust
enum View { Tracks, Toplists, Search, Artist, Album }

struct SearchState {
    query: String,
    filter: SearchFilter,              // All | Tracks | Albums | Artists | Playlists
    results: Option<SearchResults>,
    entries: Vec<SearchEntry>,         // flattened for selection
    selection: usize,
    error: Option<String>,
}

enum SearchEntry {
    Track(Track), Album(Album), Artist(Artist), Playlist(Playlist),
}

struct ArtistState { id: ArtistId, detail: Option<ArtistDetail>,
    albums: Vec<Album>, selection: usize, has_more: bool, error: Option<String> }

struct AlbumState { id: AlbumId, detail: Option<AlbumDetail>,
    selection: usize, error: Option<String> }
```

Requests carry the view’s target id so stale replies (an artist the user has
already left) are ignored, mirroring the lyrics pane’s guard.

### 6.2 Rendering

- `draw_search`: sectioned list (`// TRACKS`, `// ALBUMS`, `// ARTISTS`,
  `// PLAYLISTS`) with the existing track-table grammar for track rows; album
  rows show artist/year/track count; artist rows show album count; playlist
  rows show track count.
- `draw_artist`: header (`// ARTIST · id · name · counts`), a clipped
  biography line, `// ALBUMS` rows and a `↓ MORE ALBUMS` affordance.
- `draw_album`: header (`// ALBUM · id · name · artist · year · tracks`), then
  the shared track table.
- Reuse `wrap_to_width`, `truncate_to_width`, `compact_count` and the existing
  section/selection styles. No boxed chrome.
- Artist/album pages participate in the existing 1-column gutter and sidebar
  breakpoints.

### 6.3 Interaction

- `/` remembers the last filter; `Tab`/`Shift-Tab` cycle while typing;
  `Ctrl-1..5` select directly. The filter is part of the search request.
- `Enter` dispatches by entry kind. Track: play. Album/artist: navigate.
  Playlist: load tracks as today.
- `a`/`A` on an album or artist album row queue/play-next the album; on a track
  they keep the existing meaning.
- `:album` / `:artist` open the current track’s metadata target.
- History snapshots include search query/filter, artist id and album id, so
  `Ctrl-o`/`Ctrl-i` restore the exact page.

### 6.4 Empty and error states

- Search: `no matches for "…" · / to refine`; per-section `no albums` etc.
- Artist: `// NO ALBUMS` with `r retry`; provider error uses the comments-style
  error block with `r retry · esc close`.
- Album: `// NO TRACKS` when the provider returns an empty song list.
- Loading uses the static spinner glyph and the target name; no per-frame
  animation beyond the existing tick.

## 7. Tests

| Layer | Tests |
| --- | --- |
| `ncmapi` | Album search decode; artist detail/albums; album detail; null fields; URL/validation contracts |
| domain | Model serde round-trips; `SearchKind::All`; totals |
| core | Fake service: album play replaces queue; queue album appends; unknown album returns `not_found` |
| IPC | Round-trip every new command/reply; `PlayTarget::Album` |
| CLI | `search --kind all`, `artist show/albums`, `album show/play/queue` JSON + exit codes |
| TUI | Search rendering, filter cycling, navigation (results→artist→album→back), queue actions, empty/error, 80×24 and 120×30 |
| Manual | Live search → artist → album → play on this machine |

## 8. Rollout

1. `ncmapi 1.9.0`: implement, test, publish, tag.
2. `lekima`: bump the pin, add domain/IPC/core, then CLI/MCP/HTTP, then TUI.
3. Update README, product design, sprint docs and changelog.
4. Release `0.9.0` through the existing tag → CI → tap flow.

## 9. ADRs

### ADR-001 — `All` is an adapter fan-out, not a provider kind

The provider’s `type=1018` returns an empty result on the public search
endpoint. The adapter issues Track (1), Album (10), Artist (100) and Playlist
(1000) searches with a small per-kind limit and merges them. This keeps one IPC
reply and one TUI code path, at the cost of up to four cached provider calls.
The limit is capped (for example 10 per kind for ALL, the requested limit for a
specific kind).

### ADR-002 — domain read models, not provider DTOs

`ArtistDetail`, `ArtistAlbums` and `AlbumDetail` live in `lekima-domain` and
carry no serde provider names. Mapping stays in `lekima-ncm::map`; provider
text is sanitized at that boundary.

### ADR-003 — one view state, history snapshots

New pages extend the existing `View` enum and history snapshot rather than
adding a nested navigation stack. Back/forward remains one linear stack and is
capped at 32 entries.

### ADR-004 — album actions are daemon commands

`QueueAlbum` and `PlayTarget::Album` resolve tracks in the daemon, so the CLI,
TUI, MCP and HTTP clients all share one implementation and one error mapping.
Clients do not fetch an album and enqueue the ids themselves.

### ADR-005 — optional metadata renders as `—`

Year, counts and cover art are optional in provider payloads. Missing metadata
is rendered as an em dash, never as a blank column or a panic.
