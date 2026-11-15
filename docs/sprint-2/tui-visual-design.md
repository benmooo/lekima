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
| `muted` | `#6E7D8C` | artists, labels, hints |
| `dim` | `#2D3740` | rules, indices, separators, empty meters |
| `selection_bg` | `#003E34` | selected row background |
| `selection_fg` | `#D2FFF0` | selected row foreground |
| `panel` | `#007A64` | `//` section markers, table headers |
| `badge_fg/bg` | `#000000` / `#00FF9C` | mode badge and brand chip |

Alternates: `amber` (VT220 phosphor), `light` for light terminals, and `mono`
(`NO_COLOR`, no RGB at all). Configure with `[ui] theme = …`; `LEKIMA_THEME`
accepts a palette name, `light`, `dark` or `auto`, and `NO_COLOR` forces mono.

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

One row, ordered by importance. Everything after the spectrum may clip on narrow
terminals; the title is clipped first only when the terminal is very small.

- Brand chip (` LEKIMA `), state glyph, bold title, artist.
- A 12-cell progress rail with a distinct playhead (`━━━━●──────`), followed by
  `position/duration`. Locally buffered cells use the `panel` colour so a
  streamed track shows what is available before it is played.
- Mode badges: `[⟲ OFF|ALL|1]` (dim / cyan / accent) and `[⇄]` (hot when on,
  dim when off). Playhead `●`; ASCII mode uses `>` with `=`/`-` rails.
- `VOL` meter, queue count, quality class (`EXHIGH`), a `PREVIEW` badge when
  the decoded stream is shorter than the catalogue duration, sleep countdown,
  then a compact 12-column spectrum strip (`SIG`).

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
- Row: dim index, marker column (only the playing track has one), title `text`,
  artist `muted`, duration `dim`.
- Selection: `▌` symbol in accent + `selection_bg`/`selection_fg`, bold.
- Empty state: one muted line, never a centred illustration.

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

### Settings palette

- `S` opens a centred double-border overlay (`▓ UI ▓`) listing THEME, GLYPHS,
  DENSITY and TRANSLATION as `< value >` rows. Twelve themes: cyber, amber,
  mono, dracula, gruvbox, nord, synthwave, matrix, solarized, tokyonight,
  catppuccin and light.
- `j/k` select, `h/l` or `Enter` cycle, `Esc` closes. Changes apply immediately
  and are written to `config.toml` when the TUI exits. The selected theme shows a
  two-cell accent swatch and the active source (`config.toml`, `LEKIMA_THEME` or
  `NO_COLOR`).
- `:theme`, `:glyphs`, `:density` do the same from the command line.

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
- Normal: key hints in `dim`, separated by `·`.
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

- Application messages appear as a toast in the **bottom-right**, above the
  command bar — never inline in the search/prompt line.
- Double border: `hot` for errors and offline, `panel` for notices.
- Title: ` ! ERROR `, ` + NOTICE `, ` ! OFFLINE `.
- 54 columns max, 4 rows, wrapped text, `Clear` behind.
- Auto-dismiss after 4 seconds (80 ticks at 50 ms). The daemon-offline toast
  persists until the daemon answers again.
- Hidden below 40 columns or 10 rows; the CLI still reports failures there.

### Overlays

- Only overlays use a border: double-line, `hot` (magenta), title `▓ KEYS ▓`.
- Centred 68% × 78%, `Clear` behind.
- Section labels inside use `//`, keys `accent_alt`, descriptions `text`.

## 6. Motion

- Cursor blink on the input only (2 Hz, driven by the 250 ms tick).
- No pulsing, no spinners with frames beyond a static glyph. A terminal player
  should not twitch while you read.
- Progress and meters update on events/ticks; no animation interpolation.

## 7. Accessibility

- `NO_COLOR` or `theme = "mono"` yields white/gray with reversed selection and
  identical glyphs. The `light` palette keeps dark text for light terminals.
- ASCII glyph mode also uses `|` separators, plain overlay titles and `Enter`
  instead of `⏎`; box-drawing borders stay because terminals that need ASCII
  glyphs still render them acceptably.
- Contrast: neon on the terminal background, never neon-on-neon text.
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
| Banner/logo art | wasted space; brand is one chip |
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
