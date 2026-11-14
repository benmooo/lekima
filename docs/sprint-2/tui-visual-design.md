# TUI Visual Design — “console, not widget boxes”

Status: **Adopted** · Supersedes the boxed-panel layout inherited from the
archived Go player.

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

Alternates: `amber` (VT220 phosphor) and `mono` (`NO_COLOR`, no RGB at all).
Configure with `[ui] theme = "cyber" | "amber" | "mono"`; `NO_COLOR` forces mono.

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

- Brand chip: black on accent, bold, ` LEKIMA `.
- State glyph colour: accent (playing), warn (paused), muted (stopped).
- Title bold `text`; artist `muted`; separators `dim`.
- Position `accent_alt`, duration `muted`, progress meter 12 cells.
- `R:OFF|ALL|ONE`; `S` only when shuffle is on (hot).
- `VOL` + 6-cell meter; `Q:n`.
- Overflow is clipped from the right; the queue count and volume are the first
  things to disappear on narrow terminals — acceptable, they are also in the
  footer/queue pane.

### Lists and tables

- No surrounding block. A section label line sits above the list.
- Table header: `#`, blank marker column, `TITLE`, `ARTIST`, `TIME` in `panel`.
- Row: dim index, marker column (only the playing track has one), title `text`,
  artist `muted`, duration `dim`.
- Selection: `▌` symbol in accent + `selection_bg`/`selection_fg`, bold.
- Empty state: one muted line, never a centred illustration.

### Queue

- Same list grammar as tracks, with `▶` on the current entry.
- Title `// QUEUE · n`.

### Footer

- Mode badge: black on accent, bold, `[NORMAL]`, `[SEARCH]`, `[CMD]`, `[QUEUE]`,
  `[HELP]`.
- Normal: key hints in `dim`, separated by `·`.
- Search/command: `>` or `:` in accent, input in `text`, blinking `▌`.
- The footer never carries application messages: it is input and state, not a
  log.

### Signal visualiser

- 24 block bars in the HUD (`SIG ▁▂▃▄▅▆▇█`) fed by the audio thread's real RMS
  envelope (50 ms buckets, 20 Hz), not an animation. The daemon samples the
  envelope and ships it in `PlaybackState.wave`; the TUI polls at 10 Hz while
  playing.
- The most recent 24 buckets are drawn; every fourth bar uses the secondary
  accent. Frozen and dim when paused or stopped.
- On narrow terminals the signal area is the first HUD segment to clip.

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
  identical glyphs.
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
- [x] Signal bars animate at 20 fps; paused/stopped are visually distinct.
- [x] Messages are bottom-right toasts, not command-bar text.
- [x] Liked `♥` marker, cloud/chart/daily sections, lyrics pane in the same
      grammar (labels `//`, double-border overlay only for transient panels).
