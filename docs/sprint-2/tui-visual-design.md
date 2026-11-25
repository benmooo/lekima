# TUI Visual Design — “console, not widget boxes”

Status: **Adopted** · Supersedes the boxed-panel layout inherited from the
archived Go player.

Known gaps between this specification and the implementation are tracked in
[`../tui-backlog.md`](../tui-backlog.md).

The first TUI draft looked like the old `termui` app: a bordered header, a
bordered sidebar, a bordered table, a bordered footer. It worked, but it felt
like a 2019 dashboard. This document defines a different identity: a **terminal
console** — one HUD line, one rule, a dense list, a command bar. Structure comes
from dim rules and `//` labels, state from neon accents.

---

## 1. Intent

| Reference | What we take |
| --- | --- |
| A synthesizer front panel | data-dense, segmented, monospaced labels |
| `tmux` status line | one line, inverted mode badge, right-side counters |
| A packet capture / log tail | numbered rows, dim indices, no chrome |
| Cyberpunk terminal aesthetics | near-black ground, one or two neon accents, magenta for danger |

We reject: rounded borders, gradients, emoji, centred empty states, nested
boxes, and anything that needs a mouse.

## 2. Palette

Default theme `cyber`, all colours on the terminal’s own background:

| Token | Hex | Usage |
| --- | --- | --- |
| `accent` | `#00FF9C` | playing state, selection marker, meter fill, focus ticks |
| `accent_alt` | `#00E5FF` | counts, time position, key hints in overlays |
| `hot` | `#FF2E88` | errors, likes, destructive actions, overlay border |
| `warn` | `#FFB000` | paused state, daemon offline |
| `text` | `#C8D6E5` | track titles, primary content |
| `muted` | `#8494A5` | artists, labels, hints, numerals |
| `dim` | `#2D3740` | rules, gutters, separators, empty meters (never text) |
| `selection_bg` | `#003E34` | selected row background |
| `selection_fg` | `#D2FFF0` | selected row foreground |
| `panel` | `#00A98A` | `//` section markers, table headers |
| `ground` | `#05070A` | design background used for contrast; not painted |
| `badge_fg/bg` | `#000000` / `#00FF9C` | mode badge and brand chip |

Alternates: `amber` (VT220 phosphor), `light` for light terminals, and `mono`
(`NO_COLOR`, no RGB at all). Configure with `[ui] theme = …`; `LEKIMA_THEME`
accepts a palette name, `light`, `dark` or `auto`, and `NO_COLOR` forces mono.

The palette is a contract, not a mood board: twelve semantic roles measured
against the palette's own `ground` (the design background, which is not painted
unless a solid ground is requested), with WCAG floors for content and marker
text. `lekima ui --json` reports the effective set; the reference palettes
`cyber` and `light` are asserted in `theme.rs` tests, and the remaining palettes
are brought up to the same floors in the identity slice.

Every palette passes the same floors against its own `ground` — text,
secondary text, labels, accents, warnings, errors, selection and badge pairs at
4.5:1, with the pure accent marker at 3:1 — asserted by
`every_palette_meets_the_contrast_floors`.

Colour capabilities are explicit: `LEKIMA_COLOR` (or `NO_COLOR`) downgrades
every RGB role to the 256 cube, the nearest 16 ANSI colours, or mono, where
selection becomes real reverse video instead of a coloured background. Motion
has the same treatment: `[ui] motion = auto|full|reduced` (or `LEKIMA_MOTION`)
freezes spinners and the cursor, hides the spectrum, and stops repainting an
idle player.

Colour is never the only signal:

- playing `▶`, paused `‖`, stopped `■`
- selection `▌` + background
- errors prefixed `!`, success `+`
- liked marker (sprint 2) is `♥` plus colour

## 3. Typography and glyphs

- One cell per glyph; no emoji, no Nerd Font icons.
- Uppercase for labels and modes; sentence case for content.
- Section labels: `// LIBRARY`, `// TRACKS · 23`, `// QUEUE · 12`.
- HUD separators: `┃`; body divider: `│`; rule: `─`.
- Meters: `█` filled, `░` empty.
- Cursor: `▌`, blinking at 2 Hz only while typing.
- Indices zero-padded to three digits: `001`.
- Durations `m:ss`, zero-padded to `mm:ss` in the HUD.

## 4. Layout

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ LEKIMA ┃ ▶ Cigarettes and lighters · LoMo  01:42/01:49 ████░░░░  R:ALL VOL ████░ Q:25 │
├──────────────────────────────────────────────────────────────────────────────┤
│ // LIBRARY 3 ▌   │ // SEARCH · 40 TRACKS ▌                                     │
│ 01 Late night    │ 001  ▶ Cigarettes and lighters   LoMo            01:49      │
│ 02 Focus         │ 002    Mota                      Russian Circles 06:32      │
│ 03 Daily         │ 003    ...                                                │
├──────────────────────────────────────────────────────────────────────────────┤
│ [NORMAL] j/k nav · ⏎ play · space pause · / search · : cmd · ? keys  + queued │
└──────────────────────────────────────────────────────────────────────────────┘
```

(The outer box above is only this document’s frame; the real TUI draws no outer
border.)

Rows:

| Row | Height | Contents |
| --- | --- | --- |
| HUD | 1 | brand chip, state glyph, title · artist, position/duration, progress, repeat/shuffle, volume meter, queue count |
| rule | 1 | `─` across the width, `dim` |
| body | rest | sidebar (≥64 cols) + 1-col divider + main pane |
| footer | 1 | `[MODE]` badge, contextual hints or prompt, transient message |

Breakpoints:

| Width | Behaviour |
| --- | --- |
| < 24 or height < 4 | render `LEKIMA` chip only |
| < 64 | hide sidebar, main pane full width |
| ≥ 64 | sidebar 24 cols, divider, main pane |
| ≥ 120 | main table gains ARTIST column space; lyrics pane later takes 40% |

## 5. Components

### HUD

One row at every width, with a 20-cell title floor. The right side is anchored:
optional facts are dropped in a fixed order (`SIG` → sleep → `CACHE` → quality →
`PREVIEW` → volume → mode badges) so the title, counters and clock never slide
with each other.

**Left** — brand chip (` LEKIMA `), state glyph, bold title, `·`, artist
(`muted`, truncated first). The title is the first thing shortened.

**Right, flush to the edge** — queue count, `[⟲ ALL|1]`, `[⇄]` (bracket
presence, not colour, means shuffle is on; off renders nothing), quality class,
`CACHE`, `PREVIEW`, sleep countdown, `SIG`, then the 12-cell rail with a
distinct playhead (`━━━━●──────`) and `position/duration`. Buffered-but-unplayed
cells are `┄` in `panel`, so they differ from the played `━` even without
colour. `VOL` appears when the width allows and always when muted.

Durations use concise `m:ss`; the clock is never clipped. ASCII mode uses
`>` as the playhead with `=`/`-` rails and `.` for buffered cells.

### Library sidebar

- Header: `// LIBRARY · n` (playlist count) with a focus tick.
- Two sections in one list: **discover** entries carry their key as a chip
  (`[d] DAILY`, `[F] FM`, `[t] TOPLISTS`, `[c] CLOUD`, `[♥] LIKED`), then a
  blank line and `// PLAYLISTS · n`.
- Playlist rows: `NN name` with the track count right-aligned in `dim`.
  Names truncate with `…`; the selected row uses the standard selection style
  and a `▌` gutter marker.
- Statics are always present, even with no session; playlists show a quiet
  “no playlists” line instead of an empty pane.

### Lists and tables

- No surrounding block. A section label line sits above the list.
- Pane keys: `Ctrl-w 1` library, `Ctrl-w 2` list, `Ctrl-w 3` queue,
  `Ctrl-w 4` lyrics. Digits after `Ctrl-w` never become a count prefix.
- Table header: `#`, blank marker column, `TITLE`, `ARTIST`, `TIME` in `panel`.
- Row: `muted` index (`001`), marker column (only the playing track has one),
  title `text`, artist `muted`, duration `muted` and right-aligned. Tracks,
  queue and history share this gutter/index/marker vocabulary; history adds
  right-aligned `AGE` and `PLAYED` columns plus `KIND`.
- Selection: `▌` symbol in accent + `selection_bg`/`selection_fg`, bold; an
  unfocused pane keeps the gutter marker in `muted`.
- Empty state: every pane uses one anatomy — `// LABEL`, one sentence, then up
  to three `▶ key hint` lines. The empty library is the only surface with brand
  art: the compact `LEKIMA` block wordmark, `NETEASE CLOUD MUSIC // CONSOLE`
  and hints for `/`, `Tab`, `:` and `?`; below 44×12 it collapses to one
  actionable line (`// NOTHING LOADED · / search · Tab library`). No centred
  banner.

### Catalogue pages

- Metadata is right-aligned to the pane edge (year, track count, duration), so
  rows read as columns even when titles differ in length; long metadata is
  truncated with `…` at the pane edge, never mid-column.
- Search shows a filter row above the prompt (`ALL TRACKS ALBUMS ARTISTS
  PLAYLISTS`); the active filter is inverted. `Tab`/`Shift-Tab` cycle,
  `Ctrl-1..5` jump.
- Results are grouped by kind with `// TRACKS · n` headers; rows show an index,
  the name and right-aligned metadata (artist/duration, artist/year/tracks).
- Artist page: `// ARTIST · id · name`, counts and aliases, a clipped biography,
  then `// ALBUMS · n` rows with year, name and track count.
- Album page: `// ALBUM · id · name`, a metadata line, then the shared track
  table.
- New pages keep the one-gutter layout, use `esc`/`Ctrl-o` back and `r` refresh,
  and render loading, error (`r retry`) and empty states explicitly.

### Lyrics pane

- Not a box: a header line (`// LYRICS · n lines`, `LRC` tag), a thin rule, the
  lyric body, a rule, and a compact footer (`offset`, position, `[ ]`, `L`).
- The active line carries a `▌` gutter marker in the accent and bold text; sung
  lines are `dim`, upcoming lines `muted`.
- Auto-follows playback, keeping the active line near the vertical centre.
  Before the first timestamp it shows the opening line as upcoming.
- States are first-class: loading (animated dots), empty (“no lyrics for this
  track”), error (`! message`). The pane never disappears silently.
- Refetches automatically whenever the track changes, and ignores replies for a
  track that is no longer playing.
- Toasts shift left of the pane so they never cover its footer.

### Comments pane

- `C` opens the comments takeover: a header (`// COMMENTS · 20.5k · track`,
  with `↓ MORE` when another page exists), a thin rule, then `HOT` and `ALL`
  sections.
- Sections carry counts and a right-aligned annotation (`MOST LIKED`,
  `N LOADED`). Each entry leads with the author in `accent_alt` and
  right-aligns `♥ likes · YYYY-MM-DD` metadata in `dim`.
- Comment bodies wrap to the pane width instead of truncating; replies hang
  under a `↳` connector in `muted`; deleted content renders as `[deleted]`.
- `j/k` and `Ctrl-d/u` scroll, `g` returns to the top, `r` refetches, `C`/`Esc`
  closes. Loading, empty and error states are explicit; errors add `r retry`
  and `C close` hints.
- Comments follow track changes while the pane is open.
- Provider text is sanitized before rendering; timestamps are rendered as UTC
  dates without a clock dependency.

### Settings palette

- `S` opens a centred double-border overlay (`▓ UI ▓`) listing THEME, GLYPHS,
  DENSITY and TRANSLATION as `< value >` rows. Twelve themes: cyber, amber,
  mono, dracula, gruvbox, nord, synthwave, matrix, solarized, tokyonight,
  catppuccin and light.
- `j/k` select, `h/l` or `Enter` cycle, `Esc` closes. Changes apply immediately
  and are written to `config.toml` when the TUI exits. The selected theme shows a
  two-cell accent swatch and the active source (`config.toml`, `LEKIMA_THEME` or
  `NO_COLOR`).
- `:theme`, `:glyphs`, `:density` do the same from the command line; `lekima ui`
  reports and sets the same preferences without a daemon. The overlay is 58×9,
  and on terminals narrower than 52 columns its hint collapses to
  `h/l change · esc close` instead of clipping.

### History

- The middle pane keeps a back/forward stack (`Ctrl-o` / `Ctrl-i`), vim style.
- Navigation pushes the current view (source, tracks or charts, selection)
  before replacing it; back/forward restore it and focus the main pane.
- A new navigation clears the forward stack, the back stack is capped at 32.
- Keys are shown in the help overlay and the footer hint.

### Typography variants

- `glyphs = "unicode" | "ascii"`: ASCII replaces `▌ ▶ ‖ ■ █ ░ ─ │ ♥` with
  `> > || [] # - - | *` and uses `_` as the input cursor.
- `density = "compact" | "comfortable"`: comfortable inserts a blank row
  between track rows for larger fonts.
- Font family and size live in the terminal emulator; see
  [`../terminal-setup.md`](../terminal-setup.md) for profiles (iTerm2, Ghostty,
  kitty, Alacritty, WezTerm) and recommended families.

### Queue

- Same list grammar as tracks, with `▶` on the current entry, dimmed rows for
  entries already played, and the selection gutter on the row being edited.
- Title `// QUEUE · n · 12/25 · 1:02:10 left`, plus a focus tick.
- The selection follows the playing entry until `j/k`/`g/G`/`J/K` move it.
- `Enter` jumps and keeps the queue open; `Esc` closes it. Transport keys
  (`Space`, `n/p`, `h/l`, `m`, `r`, `s`, `+/-`, `L`) work without leaving the
  queue, and `x` asks for `y/N` before removing an entry.

### Footer

- Mode badge: black on accent, bold, `[NORMAL]`, `[SEARCH]`, `[CMD]`, `[QUEUE]`,
  `[HELP]`.
- Normal: key hints spell the key (`enter`, `space`, `Q`, `L`) rather than
  showing a glyph; hints are separated by `·` and stay in `muted`.
- Search/command: `>` or `:` in accent, input in `text`, blinking `▌`.
- The footer never carries application messages: it is input and state, not a
  log.

### Signal visualiser

- 20 bands from a real 1024-point FFT (Hann window) of the mono mix, computed
  ~43×/s in the audio thread and smoothed with fast attack / slow decay.
  `PlaybackState.wave` carries the bands; the TUI polls at 20 Hz while playing.
- The one-row HUD downsamples them to a 12-column strip (max of each group).
  Low bands render in accent, high bands in the secondary accent; frozen when
  paused/stopped, flat and dim without audio.
- The spectrum is the last HUD segment, so it clips first on narrow terminals.

### Toasts

- Application messages appear as a console alert in the **bottom-right**, above
  the command bar — never inline in the search/prompt line.
- Double border in the severity colour with a title bar: `═ ! ERROR` (`hot`),
  `═ + NOTICE` (`accent`), `═ ! OFFLINE` (`warn`); ASCII glyph mode uses a
  plain border and `-` prefix.
- Body text stays in `text` rather than reversed colour; the first line is
  prefixed `!` or `+`. Notices are one line, errors wrap to two; overflow ends
  with `…`.
- 60 columns max, `Clear` behind, wrapped and offset left of the lyrics pane.
- Auto-dismiss after 4 seconds (80 ticks at 50 ms). The daemon-offline toast
  persists until the daemon answers again.
- At 30×6 the alert is still visible; below that it collapses to a single
  severity-coloured footer line (`! message`), so an alarm is never silent.
  The CLI still reports failures in headless runs.

### Overlays

- Only overlays use a border: double-line, `hot` (magenta), title `▓ KEYS ▓`.
- Help is at most 100×22 (two columns at 80×24, scrollable single column below);
  settings is 58×9. Both are centred with `Clear` behind and clamp to the frame.
- Section labels inside use `//`, keys `accent_alt`, descriptions `text`.

## 6. Motion

- The loop ticks at 20 Hz. The spectrum redraws while playing, the loading
  spinner steps at 5 Hz, and the prompt cursor toggles at 1 Hz.
- Nothing else animates: progress is rendered as received, not interpolated.
- `[ui] motion = reduced` (or `LEKIMA_MOTION`, or `TERM=dumb`/CI with `auto`)
  freezes the spinner and cursor, hides `SIG`, and skips repaints when a status
  poll would not change a cell. An idle reduced-motion player draws no frames.

## 7. Accessibility

- `NO_COLOR` or `theme = "mono"` yields white/gray with reversed selection and
  identical glyphs. The `light` palette keeps dark text for light terminals.
- ASCII glyph mode emits no non-ASCII UI glyph: separators become `|`, borders
  `+-|`, concept labels become words, and `Enter` replaces `⏎`. Provider text
  (titles, artists, comments) is passed through and may still be CJK.
- `LEKIMA_COLOR=256|16|mono` downgrades every role; mono selection is reverse
  video, not a coloured background.
- Contrast: every palette meets 4.5:1 for content and 3:1 for the pure accent
  marker against its design `ground`; never neon-on-neon text.
- Information is always double-encoded (glyph + colour, `//` + weight).
- No key requires a mouse; no key chord conflicts with tmux (`Ctrl-b` is not
  bound).
- Sizes down to 20×8 render without panicking.

## 8. Anti-patterns

| Rejected | Why |
| --- | --- |
| Outer borders on every pane | the archived app’s look; wastes 2 rows/cols each |
| Rounded corners | reads as a web dashboard, not a console |
| Emoji icons | inconsistent width, breaks alignment |
| Banner/logo art | the empty-state wordmark is compact and only appears when there is nothing to list |
| More than two neon hues per screen | noise; hot is reserved for danger |
| Centred empty states | slower to scan than a left-aligned hint |

## 9. Implementation map

| Concern | Location |
| --- | --- |
| Palette, theme resolution, `NO_COLOR` | `crates/lekima-tui/src/theme.rs` |
| Layout, HUD, meters, signal bars, toasts, overlays | `crates/lekima-tui/src/ui.rs` |
| Blink/tick state, toast timestamps | `App::frame`, `App::notify`, `Message::is_visible` |
| Theme selection | `[ui] theme` → `lekima_tui::Theme::resolve` |
| Render tests | `ui.rs` tests with `TestBackend` |

## 10. Acceptance checklist

- [x] No bordered main chrome; one HUD line and one rule.
- [x] Sidebar hidden below 64 columns; tiny sizes degrade to the brand chip.
- [x] Playing/paused/stopped distinguished by glyph and colour.
- [x] Meters are block glyphs, not `Gauge` widgets.
- [x] Overlays use the double border and magenta title.
- [x] `mono` and `NO_COLOR` render identically in structure.
- [x] Render tests cover empty, playing, narrow, tiny, search, queue and toast states.
- [x] Signal bars animate at 20 Hz from the real FFT; paused/stopped are distinct.
- [x] Sidebar uses key chips and right-aligned playlist counts.
- [x] Back/forward history on `Ctrl-o` / `Ctrl-i`.
- [x] ASCII glyphs and comfortable density are configurable.
- [x] Messages are bottom-right toasts, not command-bar text.
- [x] Liked `♥` marker, cloud/chart/daily sections, lyrics pane in the same
      grammar (labels `//`, double-border overlay only for transient panels).
- [x] Empty library shows the compact wordmark and key hints; small panes fall
      back to one actionable line.
- [x] Comments render section counts, wrapped bodies, replies, deleted markers
      and UTC dates.
- [x] Alerts are double-bordered console panels with severity-coloured titles.
- [x] Catalogue search, artist and album pages share the section grammar,
      filter row, history and loading/error states.
