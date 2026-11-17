# Sprint 9 — Product design: catalogue depth

Status: **Shipped in 0.9.0**

## 1. Problem

`lekima` can play what you already know how to find: a track search, a
playlist, daily, FM or the charts. It cannot answer the next question a
listener asks — *who is this? what else did they make? play the album* — without
leaving the terminal for the web player.

The provider already exposes that data. The TUI only decodes track results, the
domain has no album detail, and the agent surface has no album/artist command.

## 2. Users and jobs

| User | Job | Success |
| --- | --- | --- |
| Listener | “I heard a track; show me the album and queue it.” | Track → album → playing in three actions |
| Explorer | “Search an artist and browse their records.” | Artist page lists albums; Enter opens one |
| Agent | “Find the album for this track and enqueue it.” | Stable JSON + MCP tools, no UI needed |
| Power user | “Keep both hands on the keyboard.” | Everything reachable from `/`, `:`, `j/k`, `Enter` |

## 3. Scope

### In

- Search filters: **ALL**, **TRACKS**, **ALBUMS**, **ARTISTS**, **PLAYLISTS**.
- Grouped result views with section headers and empty sections hidden.
- Artist page: avatar URL (not rendered as an image), counts, aliases/description,
  paginated albums.
- Album page: metadata, cover URL, year, track count and track table.
- Actions: play/open a result; queue an album; play an album; jump from a track
  to its artist or album.
- CLI, MCP and HTTP read-only equivalents.
- Back/forward history for the new views.

### Out

- Recommendations based on listening history.
- Album reviews, comment threads on albums, sharing.
- Downloads or offline album browsing (stream cache still applies per track).
- Artist top tracks unless the provider endpoint proves stable in slice A.
- Lyrics for albums as a whole.

## 4. Experience

### 4.1 Search

`/` opens the prompt with a filter row above the input:

```
[ALL] TRACKS ALBUMS ARTISTS PLAYLISTS
> mota_
```

- `Tab` / `Shift-Tab` cycle the filter; `Ctrl-1..5` select directly.
- The default is **ALL**, which issues one merged search (the adapter fans out to
  the four provider kinds and merges the top results).
- `Enter` searches; `Esc` cancels. The last filter is remembered for the session.

Results are a single scrollable page with sections:

```
// SEARCH · "mota" · 96 RESULTS
// TRACKS · 8
▌ 001 ▶ Mota                       Russian Circles          6:32
  002   Mota (Live)                Russian Circles          6:40
// ALBUMS · 4
  001   Mota                       Russian Circles · 2024   1 track
// ARTISTS · 2
  001   Russian Circles            4 albums
// PLAYLISTS · 3
  001   Post-rock essentials                               42 tracks
```

- `j/k`, `Ctrl-d/u`, `g/G` move; the selection spans sections.
- `Enter`: track plays; album opens; artist opens; playlist loads its tracks.
- `a` queues the selection (a track, or every track of an album/playlist);
  `A` plays it next. `a` on an artist is a no-op with a notice.
- `Ctrl-o` / `Ctrl-i` return to the result page.

Empty section copy: `no albums`, `no artists`, `no playlists` in `dim`, shown
only when the section was requested and returned nothing. A fully empty search
shows one line: `no matches for "…" · / to refine`.

### 4.2 Artist page

```
// ARTIST · 6452 · 周杰伦                        46 ALBUMS · 592 TRACKS
────────────────────────────────────────────────────────────────────────
  Jay Chou · 周董
  华语流行乐男歌手、音乐人…
// ALBUMS · 46
▌ 2026  太阳之子                     12 tracks · 2026-07-15
  2022  最伟大的作品                 12 tracks · 2022-07-15
  2021  Mojito                        1 track  · 2021-06-12
```

- Header: name, counts, aliases, a one-line clipped biography.
- `j/k` selects an album; `Enter` opens it; `a` queues the album; `A` plays it
  next; `Ctrl-o` returns to the previous page.
- `r` refreshes the page; loading/empty/error states are explicit.
- Albums page with `n` when `has_more` is true (the provider pages).

### 4.3 Album page

```
// ALBUM · 242091487 · Mota                    Aash · 2024 · 1 TRACK
────────────────────────────────────────────────────────────────────────
  #     TITLE                                      ARTIST            TIME
▌ 001 ▶ Mota                                       Aash              2:34
```

- The track table reuses the library grammar; `Enter` plays a track, `a` queues
  it, `A` plays it next.
- `:album queue` queues the whole album; `:album play` replaces the queue with
  it. The CLI equivalents are `lekima album queue|play <id>`.
- `Ctrl-o` returns to the artist or search page that opened it.

### 4.4 Jump from a track

`:artist` opens the current track’s artist; `:album` opens its album. These are
also the agent-friendly forms. No new global key is introduced, so no existing
binding changes.

### 4.5 States

| State | Rendering |
| --- | --- |
| Loading | `◐ LOADING …` line with the target name |
| Empty | `// NO ALBUMS` / `// NO TRACKS` plus one actionable hint |
| Error | `! CATALOGUE UNAVAILABLE`, message, `r retry · esc close` |
| Pagination | `↓ MORE ALBUMS · n loads the next page` |
| Missing metadata | `—` for year/counts, never a blank column |

## 5. Keymap additions

| Key | Context | Action |
| --- | --- | --- |
| `Tab` / `Shift-Tab` | Search prompt | cycle result filter |
| `Ctrl-1..5` | Search prompt | select ALL/TRACKS/ALBUMS/ARTISTS/PLAYLISTS |
| `Enter` | Results / artist | open the selection according to its kind |
| `a` / `A` | Album or artist album row | queue / play next |
| `r` | Artist/album page | refetch |
| `Ctrl-o` / `Ctrl-i` | All new views | history back / forward |
| `:album` / `:artist` | Anywhere | open the current track’s album/artist |

## 6. Agent contract

New CLI surface:

```console
$ lekima search "jay chou" --kind album --json
$ lekima artist show 6452 --json
$ lekima artist albums 6452 --limit 20 --offset 0 --json
$ lekima album show 242091487 --json
$ lekima album queue 242091487 --json
$ lekima album play 242091487 --json
```

- Existing `lekima search --kind` gains `all` and `album` results.
- JSON objects follow the existing `{"ok":true,"data":…}` envelope; new fields
  are additive and versioned.
- `album show` returns metadata plus `tracks`; `artist show` returns profile
  metadata; `artist albums` returns a page with `has_more`.
- MCP tools: `album`, `artist`, `artist_albums` (read-only, with
  `readOnlyHint`).
- HTTP: `album` and `artist` are read-only additions to the default-deny
  allowlist; `album play/queue` stay local unless explicitly enabled.
- Exit codes and error codes are unchanged.

## 7. Success criteria

- Search → open album → play is at most four key presses after the query.
- An artist page and an album page render without overflow at 80×24 and 120×30.
- Every new TUI action has a CLI equivalent and a contract test.
- No playback regressions: the existing transition, cache and queue tests stay
  green.
- `ncmapi` 1.9.1 is released and pinned; `lekima` does not reintroduce a path
  patch.
- Provider text (names, biography, album descriptions) is sanitized before it
  reaches the terminal; hostile-input fixtures cover the new fields.

## 8. Risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Provider response drift | Decode failure | Null-tolerant models; fixture tests; fast `ncmapi` patch releases |
| Album tracks restricted by VIP/region | Empty or partial album | Show the provider error/`unsupported` state, do not crash |
| Artist biography contains links/markup | Noisy terminal output | Strip control/bidi, clip to one line, keep URLs as text only |
| Large albums/artist pages | Slow render | Page albums; cap track tables; fetch on demand |
| Search fan-out in ALL | Four provider calls | Small per-kind limits, short cache, one merged reply |
| History explosion | Memory | Existing capped back/forward stack stores only view snapshots |

## 9. Non-goals

- Social features, reviews, album comments.
- Audio downloads or redistribution.
- Bypassing regional/VIP restrictions.
- Reimplementing recommendations.
- Mouse-only interactions.
