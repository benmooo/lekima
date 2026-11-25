# Sprint 12 · TUI playlist pane

Status: **Accepted** · 2026-11-25 · Slice E follow-up

## 1. Problem

Playlist editing is available from the CLI, IPC, MCP and HTTP, but the terminal
surface only exposes `:playlist create|add|delete`. Browsing playlists means
leaving the pane and reading the 24-column sidebar, and rename/describe are not
reachable at all. That makes the TUI the least usable surface for the feature.

This slice adds a dedicated playlist pane: browse, create, rename, describe, add
the current track, delete (with confirmation) and open a playlist's tracks. It
is a normal peer view — `Mode::Playlists` — like the queue, comments and history
panes. It adds no protocol command and no dependency.

## 2. Visual contract

The pane follows
[`docs/proposals/2026-11-25-ux-design-system.md`](../proposals/2026-11-25-ux-design-system.md):

- **Roles only.** `text`, `muted`, `dim`, `accent`, `accent_alt`, `label`,
  `warn`, `danger`, `selection_fg/bg`, `badge_fg/bg` and `panel`. No raw colour
  and no palette name is referenced by the pane.
- **Type levels.** T0 is the footer mode badge. T1 is a playlist name. T2 is the
  description, count and hints. T3 is the `//` pane label and table header. T4 is
  the gutter, rule and divider. There is no `faint` text.
- **One gutter.** Selection is `▌` in the same one-column gutter as tracks,
  queue and history; the table columns never move with focus.
- **Words plus glyph.** The label is `// ≡ PLAYLISTS · 3`; ASCII renders
  `// p PLAYLISTS · 3` and the ASCII concept twin is still backed by the word.
  Selection has a gutter marker and a reverse-video row; a colour-only cue is
  never the only signal.
- **No bordered chrome.** Structure is one label row, one table header and rows.
  The main layout has no boxes. The only bordered surfaces remain the help and
  settings overlays and the toast.
- **One list grammar.** Gutter, zero-padded index, name, optional description,
  right-aligned count. Numerals are tabular. Counts use the same `compact_count`
  helper as comments/history (`20.5k`).

## 3. Entry, exit and keymap

`P` opens the pane from Normal mode and closes it again. It was free in the documented keymap
(`Q` queue, `C` comments, `H` history, `L` lyrics, `S` settings, `?` help).
Opening the pane issues `Command::Playlists`; the existing reply already carries
every playlist the sidebar shows, so the pane and sidebar share one source of
truth.

The footer badge is `[PLAYLISTS]`. `esc`, `q` and `P` close the pane and return
to the previous main view without changing its selection.

| Key | Action |
| --- | --- |
| `j` / `↓` | next playlist |
| `k` / `↑` | previous playlist |
| `g` / `G` | first / last playlist |
| `ctrl-d` / `ctrl-u` | half-page down / up (10 rows, clamped) |
| `enter` | open the selected playlist's tracks (existing `playlist_tracks` path), close the pane, source becomes `playlist: <name>` |
| `n` | new playlist: opens the command line prefilled with `playlist create ` |
| `R` | rename: opens the command line prefilled with `playlist rename <id> ` |
| `d` | describe: opens the command line prefilled with `playlist describe <id> ` |
| `a` | add the current track to the selected playlist |
| `x` | ask to delete the selected playlist (`y` confirms, `n`/`esc` cancels) |
| `r` | refresh the playlist list |
| `esc` / `q` / `P` | close the pane |

`R` is used for rename because `r` is the refresh key across the history pane and
the catalogue pages. A rename key that refreshed on a typo would be worse than a
capital letter.

Prompts reuse the command line: mode badge `[CMD]`, the existing cursor,
history, word delete and validation. The pane does not grow a second text
editor. Esc from a prompt returns to Normal; state is preserved, and `P` reopens
the pane.

`x` never writes immediately. The footer replaces the hints with
`delete "Late night"? [y/N]`; only `y` sends `Command::PlaylistDelete`.

## 4. Layout and responsive rules

The pane takes over the main column; the sidebar stays visible at 64 columns and
above, exactly as for queue/history. The pane itself has the standard one-column
gutter.

### 120×30 (sidebar + 95-column pane)

```text
// ≡ PLAYLISTS · 3
      #  NAME                                    DESCRIPTION                TRACKS
      001 Late night                              after 22:00                    42
      002 Focus                                   deep work                      18
      003 Daily                                   morning                        28
```

### 80×24 (sidebar + 55-column pane)

The description column is the first thing to go; the name keeps the remaining
cells and the count stays right-aligned.

```text
// ≡ PLAYLISTS · 3
      #  NAME                                         TRACKS
      001 Late night                                      42
      002 Focus                                          18
      003 Daily                                          28
```

Rules:

- `≥120` columns: name, description (24 cells) and count. `80–119`: drop the
  description. `<80`: the sidebar disappears at 63 columns, so the pane gets the
  full width; the same name/count grammar survives.
- The index is at least three digits and widens with the list (`001`, `1000`).
  The count column is six cells and right-aligned.
- Names and descriptions truncate with `…` (`...` in ASCII), never mid-word.
- The title carries the count and, when the list overflows, the position cue
  `· 4/42`.
- Long lists scroll with the existing table selection; the header stays put.

## 5. States

| State | Render | Non-colour signal |
| --- | --- | --- |
| Loading, no rows | `~ LOADING PLAYLISTS` on the first content row | spinner/`~` + word |
| Empty | `// PLAYLISTS` label, `no playlists yet`, hints `n create`, `P close` | label + sentence + keys |
| Error, no rows | `! PLAYLISTS UNAVAILABLE`, the adapter message, `r retry` | `!` + uppercase title + key |
| Rows | label, header, selected row | `▌` + reverse video |
| Delete confirm | footer `delete "NAME"? [y/N]` | words, `[y/N]`, `warn` |
| Write in flight | no optimistic mutation; the ack triggers `Command::Playlists` | list changes only after the reply |
| Write failed | standard error toast; sync flag cleared; no refresh | `! ERROR` toast |
| Just created | `created playlist <name>` toast, then the refreshed list | toast + new row |

A failed write never shows a success toast and never refreshes the pane into a
stale "success". The existing `playlist_sync` ack pattern is reused; the error
branch clears the flag before any later `Ack` can be mistaken for this write.

## 6. Accessibility

- The gutter marker and row reverse video carry selection independently of hue.
- The count is a number with a `TRACKS` header, not a coloured dot.
- Error and confirmation states use words plus punctuation (`!`, `?`, `[y/N]`).
- `--glyphs ascii` is a supported render: the pane title becomes `// p PLAYLISTS`,
  the marker `>`, the divider `|`, the ellipsis `...`, and no codepoint above
  U+007E is emitted for chrome and fixture content. Real playlist names and
  descriptions are provider text and may contain CJK; they are width-truncated,
  not transliterated. A render test asserts the chrome glyphs are ASCII.
- `NO_COLOR`/mono keeps the reverse selection and all words; no state depends on
  a role that collapses to the same terminal colour.
- All provider text passes through `sanitize_line`/`clean` before rendering.
- The layout does not rely on wide glyphs: every symbol in the pane is one cell.

## 7. Deliberately not in this slice

- Reordering tracks inside a playlist, removing a track from it, multi-select.
- Choosing `private` or an initial description at create time; the pane creates
  a public playlist and the description can be set immediately with `d`.
- Cover upload, sharing links, playlist folders, subscribing to other people's
  playlists.
- Optimistic writes. The list refreshes after the ack.
- Mouse, a settings row for playlists, or a new IPC command.
- A second text editor: prompts stay in the command line.

## 8. Tests and acceptance

Reducer tests (`crates/lekima-tui/src/app.rs`):

- `P` enters `Mode::Playlists`, sets loading and calls `Command::Playlists`.
- `j/k/g/G/ctrl-d/ctrl-u` move and clamp the selection.
- `enter` calls `Command::PlaylistTracks` with the selected id and closes the
  pane.
- `n`, `R`, `d` prefill the command line with the right command; submitting
  `rename`/`describe` emits `Command::PlaylistUpdate`.
- `a` calls `Command::PlaylistAdd` for the current track; with nothing playing it
  returns `Action::None` and does not write.
- `x` does not write; `y` emits `Command::PlaylistDelete`; `n`/`esc` cancel.
- A successful ack calls `Command::Playlists`; a failed write returns
  `Action::None`, clears `playlist_sync`, and a later `Ack` does not refresh.
- `r` refreshes without leaving the pane.

Render tests (`crates/lekima-tui/src/ui.rs`), at 80×24 and 120×30 plus ASCII:

- The title `PLAYLISTS`, a playlist row and its right-aligned count are visible.
- The description appears at 120 and is absent at 80.
- The empty state shows its sentence and `n create`.
- The confirm prompt shows the playlist name and `[y/N]`.
- `--glyphs ascii` output is pure ASCII and still contains `PLAYLISTS`.
- `help_fits_an_eighty_by_twenty_four_terminal` still passes with the pane's
  bindings documented; the help overlay must not overflow.

Acceptance:

1. `P` opens the pane and lists the same playlists as the sidebar.
2. `enter` opens a playlist's tracks through the existing path.
3. `n` creates a playlist and the refreshed pane shows it.
4. `R` and `d` rename/describe and the refreshed pane shows the new values.
5. `a` adds the current track (playing, paused or stopped; it is the selected track) and the ack refreshes the pane.
6. `x` asks for `y`; `n` cancels; `y` deletes and the refreshed pane no longer
   shows the playlist.
7. A rejected write shows an error and never a success toast.
8. The pane renders at 80×24 and 120×30, in Unicode and ASCII, with no colour-only
   state and no bordered chrome in the main layout.
