# Proposal — TUI design system v2

Status: **Accepted; slices 1–6 implemented** · 2026-11-25 · Linear AKA-151, AKA-153

Revision after review: the HUD stays **one row at every width** (the two-row
mid width is withdrawn); shortcut hints spell keys (`enter`, `Q`, `L`) instead
of reusing concept glyphs. The token, palette, list, surface and accessibility
decisions stand.
Extends [`2026-11-25-tui-density-and-theme.md`](./2026-11-25-tui-density-and-theme.md)
and keeps **A1** (`compact` default) and **B1** (freeze twelve palettes, polish
`cyber`, add `lekima ui`). Decisions: twelve semantic roles, five type levels, one
spacing scale, a right-anchored one-row HUD with a title floor, one gutter, words for
state, explicit motion/colour downgrade, and `lekima ui` first. No new palette or
default flip; "vibrant" means sharper signals, not more hues.

## 1. Diagnosis
The console identity works: one HUD line, one rule, no bordered chrome
(`ui.rs:59-97`); a semantic palette with light and mono escapes (`theme.rs:182-306`);
one glyph per kind (`theme.rs:57-70`); ASCII twins; and 19 track rows at 80×24.

| # | Finding | Evidence |
| --- | --- | --- |
| F1 | **The HUD starves the title.** At 80 columns the budget loop leaves 9 cells: progress+time, volume, queue, repeat and shuffle take 58. At 120 the artist is dropped. | `ui.rs:149-216`; counters are appended after identity (`:266-269`), so they slide with the title |
| F2 | **Colour-only signals.** Shuffle on/off changes only hue (`ui.rs:312-321`); buffered and played both draw `━` (`:3277-3287`), so mono users cannot see what is downloaded. | `ui.rs` cited |
| F3 | **Three list implementations.** Tracks/queue use `Table` (`ui.rs:656`), history a `List` (`:2153`), catalogue pages a padded `Paragraph` (`:1165-1288`). Column spacing is 0 and times are unpadded (`:657`, `:882`). | `ui.rs` cited |
| F4 | **`faint` still carries content**: history age/played (`ui.rs:2126`, `:2147`), position cues (`:612`, `:2192`), past queue rows (`:867`). Copy drifts too: hyphens as dashes, `source` uppercased including queries, meta clipped mid-word. | `ui.rs:505-513`, `:594`, `:1155-1161` |
| F5 | **Thresholds and spec drift.** Nine inline breakpoints (40, 44, 64, 72, 76, 80, 90, 104, 108), four overlay sizes (spec says 68%×78%), chips documented as `[d] DAILY` but rendered as `d ☀ DAILY`, and a trailing header tick while rows use a gutter marker. | `ui.rs:29-41`, `:3012`, `:465-487`, `:1123-1128` |
| F6 | **Theme tokens are redundant.** `overlay == hot` in 8/12 palettes, `badge_bg == accent` in 7, `meter_full == accent` in 9. A contributor cannot tell which field is a decision. | `theme.rs:182-498` |
| F7 | **Ambiguous-width and emoji-capable glyphs.** `♪ ▣ ≡ ♥ ▶` are East Asian *Ambiguous*; `☺ ☀ ☁ ♥` are emoji-capable. The 1-cell test uses `unicode-width`, which reports 1 for ambiguous, so it cannot catch a 2-cell render. `≡`/`≣` are near-identical. | `theme.rs:73-151`, test `:615-643` |
| F8 | **ASCII twins collide with keys:** `q` queue/quit, `t` charts, `c` cloud, `H` history, `d` daily, `a` albums, `r` artists, `l` lyrics, `h` history. | `theme.rs:140-150` vs `keymap.rs` |
| F9 | **Mono has no selection background** (`selection_bg = Reset`); focus is bold + `▌`, and bold is not universal. RGB is also emitted to 16-colour terminals with no downgrade or test. | `theme.rs:271-274`, `:214-234` |
| F10 | **Motion disagrees with the docs.** Tick 50 ms, blink phase 10 ticks (1 Hz, doc says 2), spinner 20 Hz, lyric dots 4 Hz, and a 20 fps redraw while idle. | `lib.rs:74-130`, `ui.rs:36`, `:1760` |
| F11 | **Missing surfaces.** Hooks have no TUI surface (only `doctor --json`); settings have no CLI. | `cli.rs:280`, no `hook` in `lekima-tui` |

## 2. Foundations
### 2.1 Colour roles
Twelve roles. `ground` is the palette's design background, painted only with
`[ui] ground = "solid"`; contrast is measured against it, so it is testable
without a terminal. Ratios are WCAG 2.x; the two pairs are fg-on-bg.

| Role | Dark (`cyber`) | Ratio | Light | Ratio | 256 | 16-colour |
| --- | --- | --- | --- | --- | --- | --- |
| `ground` | `#05070A` | — | `#FFFFFF` | — | 232 | Black |
| `text` | `#C8D6E5` | 13.6 | `#1A2027` | 16.4 | 188 | BrightWhite |
| `muted` | `#8494A5` | 6.5 | `#52606D` | 6.5 | 103 | White |
| `faint` | `#2D3740` | 1.7 | `#8A94A0` | 3.1 | 237 | BrightBlack |
| `label` | `#00A98A` | 6.8 | `#00695C` | 6.6 | 36 | Green |
| `accent` | `#00FF9C` | 15.2 | `#00796B` | 5.3 | 49 | BrightGreen |
| `accent_alt` | `#00E5FF` | 13.1 | `#1565C0` | 5.8 | 45 | BrightCyan |
| `warn` | `#FFB000` | 11.0 | `#B34700` | 5.5 | 214 | BrightYellow |
| `danger` | `#FF2E88` | 5.8 | `#C2185B` | 5.9 | 198 | BrightMagenta |
| `selection fg/bg` | `#D2FFF0` / `#003E34` | 11.1 | `#0B1F1B` / `#CCE8E2` | 13.2 | 195/235 | Black on White |
| `badge fg/bg` | `#000000` / `#00FF9C` | 15.8 | `#FFFFFF` / `#00695C` | 6.6 | 0/49 | Black on BrightGreen |
| `meter full/empty` | `#00FF9C` / `#2A363E` | 15.2/1.6 | `#00796B` / `#D7DEE5` | 5.3/1.2 | 49/236 | BrightGreen/BrightBlack |

- `muted` `#6E7D8C` → `#8494A5`: the old value fails AA on `#282828` (3.49:1);
  the new one holds ≥5:1 up to that ground.
- `label` replaces `panel` and brightens (`#007A64` → `#00A98A`) because table
  headers are text at 3.8:1. Light `warn` `#E65100` (3.8:1) → `#B34700` (5.5:1).
- `overlay`, `badge_bg`, `meter_full` become derived defaults (`danger`, `accent`,
  `accent`); `faint` is chrome, never text. `LEKIMA_COLOR=auto|truecolor|256|16|mono`
  resolves from `COLORTERM`/`TERM`/`NO_COLOR`; 16 colours are best-effort.

### 2.2 Typography
Five levels, two modifiers. Bold = current/focused/identity; reverse =
selection/badge; italic and underline are retired (coverage and URL detection).

| Level | Style | Used for | Never |
| --- | --- | --- | --- |
| T0 identity | `text` bold, or badge pair | brand, HUD title, mode badge, wordmark | list rows |
| T1 primary | `text` regular | titles, bodies, active lyric (bold), values, messages | labels |
| T2 secondary | `muted` | artist, album, dates, durations, counts, hints | a fact found nowhere else |
| T3 label | `label`, uppercase | `//` markers, pane scopes, table headers | provider text |
| T4 chrome | `faint` | rules, divider, gutters, empty meters | any text |

- Uppercase is chrome, sentence case is content; provider text and user input are
  never re-cased (fix `ui.rs:594`).
- Numerals are tabular and right-aligned: `m:ss` in lists, `mm:ss` in the HUD,
  `h:mm:ss` past an hour; indices `001`; counts ≥10 000 as `20.5k`; signed offsets.
- Separators: `//` starts a label; `·` joins peers with one space each side; `┃`
  appears once per HUD; `│` is a divider; a hyphen is never a dash. One space
  between columns, two between HUD groups.

### 2.3 Spacing and density

| Token | Size | Use |
| --- | --- | --- |
| `s0` | 0 | rows inside a compact list; no blank line inside a block |
| `s1` | 1 cell | gutter, column gap, panel padding, inline gap |
| `s2` | 2 cells | HUD group gap, comment indent, reply indent |
| `r1` | 1 row | block gap, compact section gap |
| `r2` | 2 rows | comfortable section gap, overlay breathing at ≥24 rows |

`compact` = stride 1, gap 1; `comfortable` = stride 2, gap 2. **Horizontal tokens
never change**, so columns and screenshots survive; panel padding is `s1`.

### 2.4 Glyphs
One concept → one glyph; 1 cell in `unicode-width`; ASCII twins are 1 cell except
concept twins, which become words. Forbidden: emoji, VS16, Nerd Font private-use,
double-width, combining marks, ZWJ, and anything absent from the recommended fonts.

| Concept | Now | Proposed | Class | Where |
| --- | --- | --- | --- | --- |
| playing, paused, stopped, selection, cursor | `▶ ‖ ■ ▌` | same | Ambiguous | HUD, gutter, prompt |
| loading / spinner | `◌` / `◐◓◑◒` | same | Neutral / mixed | HUD badge, row marker |
| rule, divider, HUD sep, peer sep, ellipsis | `─ │ ┃ · …` | same | Ambiguous | chrome |
| enter / reply / more | `⏎ ↳ ↓` | `↵ ↳ ↓` | Neutral | hints, comments |
| meter / progress (full, empty, played, buffered, head) | `█ ░ ━ ━ ●` | `█ ░ ━ ┄ ●` | A / N | meters, rail |
| repeat / shuffle | `⟲ ⇄` | `↻ ⇄` | Neutral | HUD |
| tracks / albums | `♪ ▣` | same | Ambiguous | labels, rail |
| artists | `☺` | `@` | Neutral | labels, rail |
| playlists / queue | `≡ ≣` | `≡ ⋮` | A / N | labels |
| lyrics / history | `♫ ↺` | same | Neutral | pane labels |
| daily / FM / charts / sleep | `☀ ◉ ▲ ☾` | `◷ ◉ ▲ —` | N / A | sidebar, HUD |
| cloud / liked / completed | `☁ ♥ ✓` | `◎ ♥ ✓` | N / A | sidebar, HUD, history |

ASCII twins: `▶ >`, `‖ ||`, `■ []`, `█ #`, `░ -`, `━ =`, `┄ .`, `● >`, `▌ >`,
`♥ *`, `… ...`, `↵ Enter`, `↳ ->`, `↓ v`, `✓ *`; concept glyphs become words.

- **Ambiguous glyphs never enter the HUD**; they may appear only in label position,
  padded by display width. A render test asserts this.
- **ASCII concept twins are words** (`TRACKS`, `ALBUMS`, `ARTISTS`, `PLAYLISTS`,
  `QUEUE`, `LYRICS`, `HISTORY`, `DAILY`, `FM`, `TOP`, `CLOUD`), removing the
  keybinding collision (F8). No glyph is the only signal; ZWJ is forbidden.

### 2.5 Motion

| Element | Rate | Budget | `reduced` |
| --- | --- | --- | --- |
| HUD spectrum (`SIG`) | 20 Hz | 12 cells, 1 row | hidden |
| row loading spinner | 5 Hz | 1 cell | static `◌` |
| prompt cursor | 2 Hz toggle | 1 cell | steady `▌` |
| toast | appear / disappear | ≤4 s, no fade | same |
| everything else | event-driven | — | — |

- Idle = 0 draws; playing ≤20 fps; ≤2 ms CPU/frame at 120×30; a key must show in
  the next frame (<16 ms). Draw only when an animated or changed cell is dirty.
- Not animatable: selection, layout, progress interpolation, colours. Position
  arrives at 4 Hz and is rendered as received. `[ui] motion = auto|full|reduced`,
  `LEKIMA_MOTION`; `auto` picks `reduced` for `TERM=dumb`/`CI`.

## 3. Components
### 3.1 State vocabulary

| State | Non-colour signal | Colour | Where |
| --- | --- | --- | --- |
| playing / current | `▶` + bold title | `accent` | HUD, row marker |
| paused / stopped | `‖` / `■` | `warn` / `muted` | HUD |
| loading / buffering | `◌` or spinner + word | `warn` / `accent_alt` | HUD, marker |
| muted | `MUTE` replaces the meter | `warn` | HUD |
| preview / cached | `PREVIEW` / `CACHE` | `warn` / `accent_alt` | HUD |
| focused | `▌` in the pane gutter | `accent` | label, selection |
| selected, unfocused | `▌` in the gutter | `muted` + faint bg | rows |
| empty | `// LABEL`, sentence, key hint | `muted` | every pane |
| error | `!` + UPPERCASE title + retry | `danger` | pane, toast |

Default is T1; hover is N/A (no mouse); disabled is a muted label with no key hint,
reserved for settings rows that have no value.

### 3.2 Inventory

| Component | Anatomy | Size rule |
| --- | --- | --- |
| HUD | brand, state, title · artist, right-anchored queue/modes/facts/`SIG`/rail/clock | 1 row ≥96; 2 rows 64–95; chip below 24 |
| Rule / divider | `─` or `│` in `faint` | 1 row / 1 col |
| List / table | gutter, index, marker, title, artist, right-aligned time; one header | 1 row; 2 in `comfortable` |
| Sidebar | `// LIBRARY · n`, five `[key]` rows, `// ≡ PLAYLISTS · n`, counts right | 24 cols; hidden <64 |
| Prompt / command bar | mode badge, filter rail, sigil, input, cursor, one hint | 1 row |
| Overlay | double border, `▓ TITLE ▓`, `//` sections, rows | help ≤100×22; settings 58×9 |
| Alert / toast | border, `! ERROR` / `+ NOTICE`, 1–2 body lines | ≤60 cols; hidden <30×6 |
| Badge | uppercase word or bracketed control | 1 row, never wraps |
| Empty state | `// LABEL`, sentence, ≤4 hints; wordmark only in an empty library | collapses ≤44×12 |
| Pager / footer | mode badge, hints, pending count | 1 row |
| Lyrics | `// ♫ LYRICS · n`, rule, body, rule, offset footer | 38 cols side; ≥6 rows stacked |
| Comments | `// COMMENTS · n`, rule, `HOT`/`ALL`, wrapped entries, replies | takeover; wraps, never truncates |
| Catalogue | label with query/filter, sections per kind, shared rows | takeover |
| History | `// ↺ HISTORY · n`, header, age/✓/title/artist/played/kind | takeover |
| Queue | `// ⋮ QUEUE · n · a/b · left`, track grammar, past rows muted | takeover |
| Hook status | `HOOK! n` badge; details stay in `doctor` | 1 badge |

- **HUD drop order:** `SIG` → sleep → `CACHE` → quality → artist → shuffle label →
  volume meter. Never dropped: state, title (≥20), clock, rail at ≥64.
- **Tables** right-align numerics, pad fields, truncate only with `…`; overlays
  never cover the HUD, and below 50×12 settings drops `< >` rather than clipping.
  Toasts stay out of the command bar; below 30×6 an error takes the footer row.
- **Hook status is blocked:** no hook data or IPC field exists. Add
  `status.hooks` first; until then the badge is not drawn.

## 4. Responsive model
### 4.1 Breakpoints

| Name | Columns | Rows | Rule |
| --- | --- | --- | --- |
| `micro` | <24 | <4 | brand chip only |
| `narrow` | 24–39 | ≥4 | no sidebar; title only; no rail/time |
| `compact` | 40–63 | ≥4 | no sidebar; time column; 1-row HUD |
| `split` | 64–79 | ≥4 | sidebar 24; HUD 2 rows; lyrics stack |
| `standard` | 80–103 | ≥4 | two-column help at ≥24 rows; HUD 2 rows |
| `wide` | 104–119 | ≥4 | 1-row HUD; lyrics side; full hints |
| `full` | ≥120 | ≥30 | all segments; `SIG`; quality and cache |

### 4.2 Surface × breakpoint
`F` = full, `P` = reduced, `—` = absent.

| Surface | narrow | compact | split | standard | wide | full |
| --- | --- | --- | --- | --- | --- | --- |
| HUD identity | F | F | F | F | F | F |
| HUD transport | — | P | P (row 2) | P (row 2) | F | F |
| HUD `SIG`/quality/cache | — | — | — | P | P | F |
| Sidebar | — | — | F | F | F | F |
| Table title / artist | F / — | F / — | F / P | F / F | F / F | F / F |
| Table time / position cue | — / — | F / P | F / F | F / F | F / F | F / F |
| Lyrics | — | P | P (stack) | F (stack) | F (side) | F (side) |
| Filter rail / toast | P / — | P / P | P / F | F / F | F / F | F / F |
| Comments dates/likes/replies | P | P | P | F | F | F |
| History kind/played | — | — | P | P | F | F |
| Settings / help | P / P | P / P | F / P | F / F | F / F | F / F |

### 4.3 Degradation order
1. **HUD:** `SIG` → sleep → `CACHE` → quality → shuffle label → volume meter →
   artist → rail below 64. Clock and title stay last.
2. **Table:** position cue → time → artist → title truncation. Index and marker
   never go.
3. **Sidebar:** counts → section blank line → names → whole sidebar at 63.
4. **Lyrics:** side split → stacked (≥12 body rows) → takeover with `[LYRICS]`;
   never silently absent.
5. **Catalogue / comments / history:** count → year/replies/kind → artist → wrap
   or age; titles last, bodies never truncate.

**No-clip rule.** At every size ≥24×4 these render untruncated: the state glyph,
the clock, the focused pane label and one actionable hint. Everything else may
truncate only with `…`, never mid-word. At `micro` only the brand chip is promised.

## 5. Accessibility
- **Floors:** `text`, `muted`, `label`, `accent_alt`, `warn`, `danger` as text
  ≥4.5:1 on `ground`; `accent` as marker ≥3:1; selection/badge ≥4.5:1 fg-on-bg.
  `faint` is exempt but never text. A test computes all pairs for all palettes.
- **Never colour-only:** state glyphs; shuffle bracket presence; `┄` vs `━`; `!`
  vs `+`; `♥` + count; `▌` + reverse. Each is asserted in a mono render diff.
- **NO_COLOR / ASCII:** mono uses a real reverse selection (`White` bg, `Black` fg);
  ASCII has no codepoint above U+007E, uses `|`, `_`, plain overlays and word
  labels, and a render test rejects the Unicode set on every surface.
- **Width:** no ambiguous glyph in the HUD; label glyphs padded by `unicode-width`;
  a CJK fixture asserts no row exceeds its pane.
- **Reading order:** HUD → pane label → rows in selection order → footer; no
  duplicate text on one line; every icon has a word on the same/adjacent line.
- **Checklist:** contrast green · mono diff · ASCII diff · width audit · no faint text · no colour-only signal · idle draws 0 · error visible at 24×4.

## 6. Identity direction
The console stays. Five moves: (1) a stable instrument panel — right-anchored
cluster, clock/rail at the same column, one row so the title keeps ≥20
cells; (2) one gutter for focus, selection and the label tick; (3) words for state,
brackets for controls, so shuffle reads on/off without colour; (4) numbers that
line up; (5) a calmer palette — brighter `muted`/`label`, a real `danger`, `faint`
reduced to chrome.

### 6.1 HUD, 80 and 120 columns
Before (80): 9 title cells, no artist, counters slide with the title.

```
 LEKIMA  ┃ ▶ Cigarett…  ━━━━━●────── 01:42/01:49  VOL ████░░  Q:25  [⟲ ALL]  [⇄]
```

After (80): one row (second row withdrawn), 23 title cells plus artist, queue right-anchored.

```
 LEKIMA ┃ ▶ Cigarettes and lighters · LoMo                                   Q25
 ━━━━━●──────  01:42/01:49   VOL ████░░   [⟲A] [⇄]                              
```

Before (120): 23 title cells, artist dropped, `SIG` visible.

```
 LEKIMA  ┃ ▶ Cigarettes and lighters  ━━━━━●────── 01:42/01:49  VOL ████░░  Q:25  [⟲ ALL]  [⇄]  EXHIGH  SIG ▁▂▃▅▇▅▃▂▁▂▃▄
```

After (120): title + artist, facts before decoration, clock flush right.

```
 LEKIMA ┃ ▶ Teardrop · Massive Attack           Q12  [⟲A] [⇄]  EXHIGH  CACHE  SIG ▁▂▄▆█▆▄▂▁▂▃▄  ━━●───────── 01:42/05:30
```

### 6.2 Catalogue page, 80×24
Before: trailing tick, three list grammars, left-aligned times, meta clipped mid-word.

```
 LEKIMA  ┃ ▶ Cigarett…  ━━━━━●────── 01:42/01:49  VOL ████░░  Q:25  [⟲ ALL]  [⇄]
────────────────────────────────────────────────────────────────────────────────
// LIBRARY 3            │ // SEARCH · "mota" · ALL · 10 RESULTS ▌               
 d ☀ DAILY              │ // TRACKS · 6                                         
 F ◉ FM                 │ ▌   1  Cigarettes and lighters     LoMo · 1:49        
 t ▲ TOPLISTS           │     2  Mota                        Russian Circles · 6
 c ☁ CLOUD              │     3  Hyperballad                 Björk · 5:21       
 ♥  LIKED               │                                                       
                        │ // ALBUMS · 2                                         
// ≡ PLAYLISTS · 3      │     4  Mota                        Aash · 2024 · 1 tra
 01 Late night      42  │     5  Station                     Russian Circles · 2
 02 Focus           18  │                                                       
 03 Daily           28  │ // ARTISTS · 1                                        
                        │    6  Russian Circles                                 
                        │                                                       
                        │ // PLAYLISTS · 1                                      
                        │    7  Post-rock essentials    42 tracks               
                        │                                                       
                        │                                                       
                        │                                                       
                        │                                                       
                        │                                                       
                        │                                                       
[NORMAL] j/k · ⏎ open · a queue · / refine · c-1..5 filter · esc                
```

After: one gutter, `[key]` chips, aligned columns, degenerate meta degraded, not clipped.

```
 LEKIMA ┃ ▶ Cigarettes and lighters · LoMo                                   Q25
 ━━━━━●──────  01:42/01:49   VOL ████░░   [⟲A] [⇄]                              
────────────────────────────────────────────────────────────────────────────────
// LIBRARY · 3          │▌// SEARCH "mota" · ALL · 10                           
                        │ // TRACKS · 6                                         
 [d] ☀ DAILY            │▌001 ▶ Cigarettes and lighters     LoMo           1:49 
 [F] ◉ FM               │ 002   Mota                        Russian Circles 6:32
 [t] ▲ TOPLISTS         │ 003   Hyperballad                 Björk          5:21 
 [c] ☁ CLOUD            │                                                       
 [♥]  LIKED             │ // ALBUMS · 2                                         
                        │ 004   Mota                        Aash           2024 
 // ≡ PLAYLISTS · 3     │ 005   Station                     Russian Circles 2008
 01 Late night      42  │                                                       
 02 Focus           18  │ // ARTISTS · 1                                        
 03 Daily           28  │  006  Russian Circles                                 
                        │                                                       
                        │ // PLAYLISTS · 1                                      
                        │  007  Post-rock essentials                  42 tracks 
                        │                                                       
                        │ // 10 results                                         
                        │                                                       
                        │                                                       
                        │                                                       
[NORMAL] j/k · ↵ open · a queue · / refine · c-1..5 filter · esc                
```

### 6.3 History pane, 120×30
Before: a `List` with no header; artist and time drift with title length; `faint` content.

```
 LEKIMA  ┃ ▶ Cigarettes and lighters  ━━━━━●────── 01:42/01:49  VOL ████░░  Q:25  [⟲ ALL]  [⇄]  EXHIGH  SIG ▁▂▃▅▇▅▃▂▁▂▃▄
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
// LIBRARY · 3          │ // ↺ HISTORY · 10                                                                             
                        │ just now  ✓ Teardrop · Massive Attack  5:30                                                   
 [d] ☀ DAILY            │      22m  ✓ Mota · Russian Circles  6:32                                                      
 [F] ◉ FM               │       3h    Late night · LoMo  1:01                                                           
 [t] ▲ TOPLISTS         │       2d  ✓ Bohemian Rhapsody · Queen  5:55                                                   
 [c] ☁ CLOUD            │       2d  ✓ Hyperballad · Björk  5:21                                                         
 [♥]  LIKED             │       3d  ✓ Windowlicker · Aphex Twin  6:06                                                   
                        │       4d  ✓ Everything In Its Right Place · Radiohead  4:11                                   
 // ≡ PLAYLISTS · 3     │       5d    Station · Russian Circles  2:14                                                   
 01 Late night      42  │       6d  ✓ Teardrop · Massive Attack  5:30                                                   
 02 Focus           18  │       8d  ✓ Mota · Russian Circles  6:32                                                      
 03 Daily           28  │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
[HISTORY] j/k nav · ⏎ replay · r refresh · H/esc close                                                                  
```

After: a header, aligned age/played columns, a kind column, no `faint` content.

```
 LEKIMA ┃ ▶ Teardrop · Massive Attack           Q12  [⟲A] [⇄]  EXHIGH  CACHE  SIG ▁▂▄▆█▆▄▂▁▂▃▄  ━━●───────── 01:42/05:30
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
// LIBRARY · 3          │▌// ↺ HISTORY · 10                                                                             
                        │      AGE    TITLE                                  ARTIST                    PLAYED KIND      
 [d] ☀ DAILY            │ just now  ✓ Teardrop                               Massive Attack              5:30 manual    
 [F] ◉ FM               │▌     22m  ✓ Mota                                   Russian Circles             6:32 daily     
 [t] ▲ TOPLISTS         │       3h    Late night                             LoMo                        1:01 manual    
 [c] ☁ CLOUD            │       2d  ✓ Bohemian Rhapsody                      Queen                       5:55 manual    
 [♥]  LIKED             │       2d  ✓ Hyperballad                            Björk                       5:21 daily     
                        │       3d  ✓ Windowlicker                           Aphex Twin                  6:06 fm        
 // ≡ PLAYLISTS · 3     │       4d  ✓ Everything In Its Right Place          Radiohead                   4:11 manual    
 01 Late night      42  │       5d    Station                                Russian Circles             2:14 daily     
 02 Focus           18  │       6d  ✓ Teardrop                               Massive Attack              5:30 manual    
 03 Daily           28  │       8d  ✓ Mota                                   Russian Circles             6:32 manual    
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
                        │                                                                                               
[HISTORY] j/k nav · ↵ replay · r refresh · H/esc close                                                                  
```

## 7. Adoption plan
Six slices, each with its docs; screenshots regenerate only in slice 6, and slice
1 is invisible and shippable alone.

| # | Slice | Linear title |
| --- | --- | --- |
| 1 | `lekima ui` + theme contract | `TUI: add lekima ui and a tested theme contract` |
| 2 | HUD v2 | `TUI: right-anchored HUD with a title floor` |
| 3 | One list grammar | `TUI: one list grammar for tracks, catalogue, queue and history` |
| 4 | Surface consistency | `TUI: unify empty, alert and overlay surfaces` |
| 5 | Accessibility + colour | `TUI: colour downgrade, mono selection and reduced motion` |
| 6 | Identity + fixtures | `TUI: cyber v2 palette and screenshot fixtures` |

### Slice 1 — `lekima ui` + theme contract
- **Goal / files:** close the settings CLI gap (density/theme proposal §4) and make
  the palette contract enforceable; `lekima-cli/src/cli.rs`, `commands.rs`;
  `lekima-tui/src/theme.rs` (`ground`, derived defaults, contrast helper); CLI tests.
- **Acceptance:** `lekima ui --json` returns effective
  `theme/glyphs/density/translation/motion` and writes through `save_ui`;
  `--density comfortable` round-trips; every role pair passes the floors for all
  12 palettes; `THEMES.len() == 12`.
  Docs: `terminal-setup.md`, `agents.md`, visual doc §2.

### Slice 2 — HUD v2
- **Goal / files:** stable right edge, title floor, one-row HUD at every width,
  non-colour state; `ui.rs` (`draw_hud`, `progress_spans`), `theme.rs`
  (`progress_buffered`, drop the unused sleep glyph), tests, `preview.rs`.
- **Acceptance:** with a 60-character title, ≥20 cells at 80 and ≥32 at 120; the
  queue counter starts at the same column for two titles; shuffle differs without
  colour; buffered uses `┄`; 80×24 shows 18 list rows (19 before).
  Docs: visual doc §5 HUD and §6 motion; product design §5.1 if accepted.

### Slice 3 — One list grammar
- **Goal / files:** one gutter, one table helper, right-aligned numerals;
  `ui/list.rs` or shared functions in `ui.rs`; CJK width tests.
- **Acceptance:** tracks, queue, search, history and catalogue share the gutter and
  index helpers; every time cell is right-aligned; no row exceeds its pane for a
  CJK fixture; unfocused selection remains visible.
  Docs: visual doc §5 lists; `tui-backlog.md` closes TUI-08/11 against it.

### Slice 4 — Surface consistency
- **Goal / files:** one empty-state anatomy, overlay size rule and alert rule;
  `ui.rs` (empty states, comments, settings, toast).
- **Acceptance:** a table-driven test covers every empty state; settings renders at
  40×12 without clipping or panic; an error at 30×6 is visible in the footer; no
  hyphen-as-dash; comment dates/likes share the right edge.
  Docs: visual doc §5 empty/toasts/overlays; `terminal-setup.md` copy.

### Slice 5 — Accessibility and colour downgrade
- **Goal / files:** explicit capability fallbacks and real motion opt-out;
  `theme.rs` (`ColorMode`, mono reverse, glyph audit), `lib.rs` (draw policy),
  `ui.rs` (HUD glyph classes, tiny error banner), CLI flags.
- **Acceptance:** `LEKIMA_COLOR=16` emits indexes 0–15 only; mono focus reverses;
  `motion=reduced` has no spinner/blink and hides `SIG`; `TERM=dumb` selects it;
  idle draws 0 over 1 s; ASCII has no non-ASCII; no ambiguous HUD glyph.
  Docs: visual doc §7; product design §5.4; `terminal-setup.md`.

### Slice 6 — Identity polish and fixtures
- **Goal / files:** adopt tuned `cyber` values and freeze reviewable screenshots;
  `theme.rs` (muted/label/light warn), `preview.rs` (`--screen`, `--color`),
  `docs/screenshots/`.
- **Acceptance:** all 12 palettes pass the contrast matrix; fixtures for HUD,
  catalogue, queue, history, comments and settings at 80×24, 120×30, 160×40 in
  `cyber` and `mono`; review diff attached.
  Docs: visual doc values, `terminal-setup.md` theme table, README at release.

**CLI gaps:** settings → `lekima ui` (slice 1, closes the known gap); lyrics offset
`[`/`]` → `lekima ui --lyrics-offset` (slice 1, flag only); hook status needs
`status.hooks` before the badge (slice 4, blocked); help and pane focus are view
state, not actions, and are exempt.

## 8. Non-goals and risks
**Preserved:** no mouse; no emoji or Nerd Font icons; no bordered chrome in the
main layout; no in-process scripting; no new palette names and no default
density/theme flip; no animated transitions; no new widget crate. Constraints:
ratatui 0.30, 1-cell glyphs, macOS/Linux, no new heavy dependencies,
`#![forbid(unsafe_code)]`.

| Risk | Impact | Mitigation |
| --- | --- | --- |
| HUD rows change at <96 | Muscle memory; one fewer list row | Preview at 60/80/96/120 before merge; release note |
| Gutter/tick change | Focus feels different for a day | Same `▌`; only its position is fixed; PR fixtures |
| Palette retune | Screenshots stale | No config/theme names change; regenerate in slice 6 |
| Ambiguous-width glyphs | HUD shifts on CJK-wide terminals | Neutral-only HUD + width test + ASCII fallback |
| 16-colour mapping | Exact neon is lost | Best effort; glyph distinction is the guarantee |
| Config writers | Last-writer-wins surprise | Same as `:theme` today; document it |
| Hook badge without data | Fabricated state | Ship only after `status.hooks`; `doctor` meanwhile |

The preview harness (`examples/preview.rs … --density`) de-risks this: every slice adds fixtures, so review happens on cells, not opinion.
