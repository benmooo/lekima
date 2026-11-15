# TUI backlog — rendering, interaction, accessibility

Status: **Living document** · Last reviewed: 2026-11-15

The visual language in [`sprint-2/tui-visual-design.md`](./sprint-2/tui-visual-design.md)
is adopted: a HUD line, a rule, a dense list and a command bar, with no boxed
chrome. This file tracks gaps between that spec and the implementation, ordered
by user impact. It is not a sprint plan; pick items from the top.

Priority:

| Priority | Meaning |
| --- | --- |
| P0 | Visible defect or incorrect behaviour in a normal terminal |
| P1 | Common-terminal usability or keyboard-flow problem |
| P2 | Polish, accessibility or consistency |
| P3 | Speculative, needs a design decision first |

Paths are relative to `crates/lekima-tui/src/` unless noted.

## Status — 2026-11-15

All P0, P1 and P2 items, TUI-00 and the TUI-21 interaction ideas are
implemented. Evidence lives in the workspace tests:

| Item | Status | Evidence |
| --- | --- | --- |
| TUI-00 | done | `examples/preview.rs` flags; render tests at 60×20, 80×24, 100×30 |
| TUI-01 | done | two columns at 80×24; `help_scrolls_on_small_terminals` |
| TUI-02 | done | `settings_translation_row_toggles_without_touching_density` |
| TUI-03 | done | width-budgeted HUD; `hud_keeps_the_clock_and_volume_when_the_title_is_long` |
| TUI-04 | done | inactive selection style; `inactive_selection_remains_visible` |
| TUI-05 | done | inline `y/N`; `queue_remove_can_be_cancelled`, `queue_confirmation_is_visible_in_the_footer` |
| TUI-06 | done | `unicode-width`; `widths_are_counted_in_cells_not_characters` |
| TUI-07 | done | glyph-routed ASCII; `queue_uses_the_track_grammar_and_ascii_glyphs`, `ascii_overlays_use_plain_borders` |
| TUI-08 | done | adaptive columns; `narrow_tables_drop_the_artist_column` |
| TUI-09 | done | `1/200` position cue; `long_lists_show_a_position_cue` |
| TUI-10 | done | `LOADING`/`BUFFERING`/`MUTE`; `hud_labels_loading_buffering_and_mute` |
| TUI-11 | done | shared track table; focus tick; queue grammar test |
| TUI-12 | done | `keymap.rs` feeds the help; `help_fits_an_eighty_by_twenty_four_terminal` |
| TUI-13 | done | cursor editing and history; `prompt_editing_supports_midline_and_word_delete`, `prompt_history_recalls_a_previous_search` |
| TUI-14 | done | stacked lyrics below 80 columns; `lyrics_stack_on_mid_widths_and_split_on_wide_ones` |
| TUI-15 | done | `empty_states_are_actionable` |
| TUI-16 | done | `playlist_numbering_survives_three_digits` |
| TUI-17 | done | one-line chip; existing toast tests |
| TUI-18 | done | indices and durations use `muted`; `dim` is chrome only |
| TUI-19 | done | shuffle uses `accent_alt` |
| TUI-20 | done | `light` palette, `LEKIMA_THEME`, `dark`/`auto`; `environment_and_light_theme_resolve` |
| TUI-21 | done | counts, search history, theme swatch/source line, `Ctrl-w` pane keys and buffered-range shading; `ctrl_w_pane_keys_do_not_steal_count_prefixes`, `buffered_ranges_shade_the_progress_rail` |

The sanitizer hardening is done in `lekima-domain::sanitize`, applied by the NCM
mapping layer, the TUI and the human CLI output, with hostile-input tests.

---

## P0 — correctness

### TUI-01 — Help overlay is clipped on common terminals

**Where:** `ui.rs` `draw_help`.

**Problem:** the popup is 78% of the terminal height, but the content is 35
lines and there is no scrolling. At 80×24 the content area is ~16 rows, so PLAY
is cut and DISCOVER/APP are invisible. Even at 100×30 six lines are lost.

**Proposal:** split the sections into two columns when the width allows (MOVE +
PLAY on the left, DISCOVER + APP on the right) so the full keymap fits at 80×24.
Below that, render one column with `j/k`/`Ctrl-d`/`Ctrl-u` scrolling and a
`12–23/35` indicator in the overlay title.

**Done when:** an 80×24 render test finds `? keys` and `q / ctrl-c`; a 60×20 test
can scroll to the last binding.

### TUI-02 — Settings `TRANSLATION` row changes density

**Where:** `app.rs` `cycle_setting`; `ui.rs` `draw_settings`.

**Problem:** the overlay shows four rows, but `cycle_setting` matches `0`, `1`,
and `_`; the fourth row falls through to the density list. Pressing `h/l` on
`TRANSLATION` silently cycles density and leaves translation unchanged.

**Proposal:** handle the boolean row explicitly (`h/l`/`Enter`/`Space` toggle),
keep density on row 2 only, and drive the row list from a small enum so the
count and the handlers cannot drift.

**Done when:** a unit test toggles `lyrics_translation` without changing
`density`.

### TUI-03 — HUD clips status before the title

**Where:** `ui.rs` `draw_hud`; spec at `sprint-2/tui-visual-design.md` §5 “HUD”.

**Problem:** spans are emitted in a fixed order and Ratatui clips the right
edge. A long title + artist pushes progress, time, repeat, volume, queue,
quality and the spectrum off-screen; the spec says the title should clip first.
At 80 columns a normal title can already cost the volume and queue counters.

**Proposal:** reserve fixed cells for brand/state, progress and time, and the
volume meter. Give the title a flexible slot with an ellipsis. Drop optional
segments by width tier (`quality` → `sleep` → `SIG`) and never drop progress or
time. Consider a two-line HUD below ~72 columns only if the one-line rule cannot
hold.

**Done when:** render tests at 60, 80 and 120 columns with a 60-character title
still show progress and `VOL`; the title is the first thing shortened.

### TUI-04 — Focus change hides the inactive selection

**Where:** `ui.rs` `draw_tracks`, `draw_toplists`, `draw_sidebar`.

**Problem:** unfocused panes pass `with_selected(None)`, so tabbing to the
library removes the track highlight and tabbing back loses the visual sense of
place. The selected index is preserved but invisible.

**Proposal:** keep an “inactive selection” style: a dim `▌` gutter marker and a
muted background instead of no selection. Reserve the bright accent and bold
weight for the focused pane.

**Done when:** a render test finds the selection marker in both focus states.

### TUI-05 — Queue removal has no confirmation or undo

**Where:** `app.rs` `handle_queue_key`; product spec §5.2.

**Problem:** `x` calls `QueueRemove` immediately, while the product design says
“remove selected queued track (with confirm)”. A mistyped key changes the queue
with no recovery.

**Proposal:** prefer an inline footer prompt (`remove 003? [y/N]`) over a modal,
or a double-press `x` with a short window. If the daemon can reinsert, offer a
session-scoped undo in the notice toast (`removed 003 · u to undo`). Add a hint
to the queue footer either way.

**Done when:** `x` does not remove until confirmed; `Esc` leaves the queue
untouched; a test covers both paths.

### TUI-06 — Width is counted in characters, not terminal cells

**Where:** `ui.rs` `truncate_chars`, `draw_sidebar` padding.

**Problem:** `chars().count()` treats CJK and emoji as one cell. Sidebar counts
drift, names overflow the divider and lyrics clip late. `unicode-width` is
already in the dependency tree through Ratatui but is not a direct dependency.

**Proposal:** add `unicode-width` and use display width for truncation and
padding. Add an ASCII-aware ellipsis token to `Glyphs` (`…` vs `...`). Cover a
CJK title, a combining mark and an emoji in tests.

**Done when:** sidebar counts stay right-aligned for CJK playlist names and no
rendered row exceeds its column.

### TUI-07 — ASCII mode still emits Unicode glyphs

**Where:** `ui.rs` `draw_queue` (hard-coded `▶`), `draw_footer` (`⏎`, `♥`).

**Problem:** ASCII mode replaces the documented glyph set, but the queue marker
and footer hints bypass `theme.glyphs`, so a stripped-down terminal still gets
`▶`, `⏎` and `♥`.

**Proposal:** route every symbol through `Glyphs`; add the missing tokens
(enter, ellipsis, separator if needed). Sweep `ui.rs` for literals outside
`theme.rs` and add a smoke test that renders ASCII mode and rejects the Unicode
set.

**Done when:** ASCII renders contain none of `▶ █ ░ ─ │ ♥ ⏎ …`.

---

## P1 — responsiveness and interaction

### TUI-08 — Track table gives the primary column too little room

**Where:** `ui.rs` `draw_tracks` column constraints.

**Problem:** TITLE is 45% and ARTIST 35%, so titles clip while short artist
names leave slack. Below 64 columns the table also keeps columns that no longer
fit.

**Proposal:** make TITLE `Min`, ARTIST shrinkable, and hide ARTIST/TIME at width
tiers (for example < 70 and < 50 columns). Add an ellipsis to the title rather
than a hard clip.

**Done when:** the preview example shows full common titles at 100×30 and no
misaligned columns at 60×20.

### TUI-09 — Long lists have no position cue

**Where:** `ui.rs` `draw_tracks`, `draw_queue`, `draw_toplists`.

**Problem:** section labels show totals but not the visible window; long
playlists and queues give no sense of where `j/k` has moved.

**Proposal:** append a compact window (`· 12–23/845`) when the list overflows,
or draw a one-column scrollbar at the right edge. Keep it out of the row
content.

**Done when:** a 200-row render at 80×24 shows a non-zero offset after `G`.

### TUI-10 — Loading, buffering and mute are not labelled

**Where:** `ui.rs` `draw_hud`; `lekima-domain` `PlaybackState`.

**Problem:** `loading` and `buffering` share one static glyph, and mute only
empties the volume meter because `effective_volume()` returns zero. Both are
honest states that currently look like silence or low volume.

**Proposal:** show `LOADING` or `BUFFERING` in place of the state glyph (or next
to the time), animate it from `app.frame`, and render a `MUTE` badge instead of
an empty meter while muted.

**Done when:** render tests distinguish all three states and the mute badge.

### TUI-11 — Queue pane does not follow the list grammar

**Where:** `ui.rs` `draw_queue`.

**Problem:** the queue has no focus tick, no gutter marker, no duration column,
and uses a different marker path from the track table. The sidebar also loses
its focus tick while the queue is open, so no pane looks focused.

**Proposal:** use the same header, gutter, index, marker and time columns as the
track table; show the focus tick on the queue header. Consider highlighting the
current row with the state glyph rather than a second marker system.

**Done when:** queue and track renders share the same column vocabulary and the
queue header shows `▌` when active.

### TUI-12 — Help and footer drift from the real keymap

**Where:** `ui.rs` `draw_help`, `draw_footer`; `app.rs` `handle_*`.

**Problem:** help omits `x`, `J`/`K`, `R`, the translation toggle and `Esc` to
close the queue; “ctrl-d/u half page” actually moves ten rows; the footer omits
`Q`, `S`, `a`/`A` and `R`.

**Proposal:** define bindings once in a `keymap` table (key, description, mode)
and render both the help overlay and the contextual footer from it. Until then,
fix the omissions and relabel the half-page hint. Add a test that every handled
key appears in help.

**Done when:** no handler is undocumented and the footer fits at 80 columns
with an adaptive hint set.

### TUI-13 — Prompt editing is append-only

**Where:** `app.rs` `handle_prompt_key`.

**Problem:** search and command input support typing and backspace only. There
is no cursor movement, word delete, history or recall; fixing a typo requires
retyping the query.

**Proposal:** track a cursor index and support `←`/`→`, `Home`/`End`,
`Ctrl-w`, `Ctrl-u`, plus `Up`/`Down` for per-mode history. Keep the blink
cursor at the insertion point.

**Done when:** unit tests edit in the middle of a query and recall a previous
search after `Esc`.

### TUI-14 — Lyrics takeover is abrupt

**Where:** `ui.rs` `draw_body`.

**Problem:** with the sidebar, the split only starts when the main pane is 80
columns (105 total). Between 64 and 104 columns, `L` clears and replaces the
track list with no mode badge or hint.

**Proposal:** at mid widths either stack lyrics below the list (for example 40%
height) or hide the sidebar while lyrics are open. If takeover stays, add a
`[LYRICS]` badge and an `L close` hint so the state is explicit.

**Done when:** 90×24 renders both the list and lyrics or clearly signals the
takeover.

### TUI-15 — Empty states are inconsistent

**Where:** `ui.rs` empty branches.

**Problem:** “nothing here — / to search, tab for library”, “queue is empty”,
“no charts” and “no playlists” differ in tone and only some include a next
action. The product design specifies actionable copy for the queue.

**Proposal:** one sentence plus one key hint per state, for example “Nothing
queued. Press / to search or Enter to play the selection.”

**Done when:** a table-driven render test covers every empty state.

### TUI-16 — Playlist numbering breaks at 100 entries

**Where:** `ui.rs` `draw_sidebar`.

**Problem:** the index is `{:02}` and `prefix` is hard-coded to 4, so a
three-digit playlist index pushes the name and count out of alignment.

**Proposal:** compute the index width from the playlist count and use the same
value for padding.

**Done when:** a 120-playlist render keeps counts aligned at 24 columns.

### TUI-17 — Toast weight and placement

**Where:** `ui.rs` `draw_toast`.

**Problem:** a 54×4 double-bordered box covers four content rows and can hide
the bottom of the list. The lyrics offset is a hard-coded `reserved = 44`
against a 38-column pane. Errors wrap into two lines and clip.

**Proposal:** a one-line inverse chip above the footer, expanding to at most
three lines for errors. Derive the offset from the actual lyrics rect. Keep the
“messages never live in the command bar” rule.

**Done when:** a notice never covers more than one list row and never overlaps
the lyrics footer at 100×30.

---

## P2 — visual and accessibility

### TUI-18 — `dim` is too faint for content

**Where:** `theme.rs` palettes; `ui.rs` indices and durations.

**Problem:** indices and durations use `dim` (`#2D3740` on black, ~1.7:1).
Rules and separators can be faint; numbers are content.

**Proposal:** use `muted` for indices/durations and keep `dim` for rules,
gutters and empty meters. Recheck each theme, including mono.

**Done when:** a contrast check or visual review confirms ≥ 3:1 for all
informational text.

### TUI-19 — Shuffle-on uses the danger accent

**Where:** `ui.rs` `draw_hud`.

**Problem:** enabled shuffle uses `hot` (magenta), which the palette reserves
for errors, likes and destructive actions. It reads as a warning.

**Proposal:** use `accent` or `accent_alt` for enabled modes and reserve `hot`
for errors and likes. Keep the glyph as the non-colour signal.

**Done when:** palette semantics are consistent across HUD, help and toasts.

### TUI-20 — No light-terminal or environment theme support

**Where:** `theme.rs` `Theme::resolve`; product design §5.4.

**Problem:** every palette assumes a dark background and light text;
`resolve` only handles `NO_COLOR`. On a light terminal the TUI is unreadable,
and `LEKIMA_THEME=light|dark|none` is documented but not implemented.

**Proposal:** add a light palette (dark text, adjusted accents) and resolve
`LEKIMA_THEME`, with `auto` left to terminal detection or a config flag. At
minimum, document dark-only in the terminal setup guide and fail grammar
loudly on an unknown value instead of silently choosing `cyber`.

**Done when:** `LEKIMA_THEME=light` renders readable text on a light background
and the setting is covered by a unit test.

## P3 — later interaction ideas

### TUI-21 — Interaction ideas (implemented)

- Search history and per-mode recall in the prompt line.
- Count prefixes (`5j`, `12G`) without stealing pane selection: pane focus uses
  `Ctrl-w 1` (library), `Ctrl-w 2` (list), `Ctrl-w 3` (queue) and `Ctrl-w 4`
  (lyrics).
- Buffered-range shading on the HUD progress rail, backed by
  `PlaybackState.buffered` and `AudioEngine::buffered_ranges`.
- Theme swatches and a theme-source line in the settings palette.

---

## Test and tooling tasks

### TUI-00 — Make the preview and render tests cover real sizes

The `examples/preview.rs` canvas is fixed at 126×22. Add `--width`,
`--height`, `--theme` and `--mode` arguments, then snapshot or assert:

| Size | Must show |
| --- | --- |
| 80×24 | HUD progress, `VOL`, help `q` binding, full footer |
| 60×20 | title, progress, no clipped columns |
| 40×12 | usable list, settings/help do not panic and can scroll |
| 100×30 | lyrics split and toast placement |
| 120×40 | full HUD including quality and spectrum |

Snapshot tests should also cover help, settings, queue, empty states and ASCII
mode. Add an ASCII-mode assertion that rejects the Unicode glyph set.

---

## Related hardening (not TUI-only)

`AGENTS.md` requires provider text to be sanitized with a shared helper before
it reaches a terminal, but no such helper exists. Ratatui filters C0/C1 control
characters during buffer writing, which limits raw ANSI injection in the TUI;
the human CLI path still formats provider titles directly
(`crates/lekima-cli/src/output.rs`). Track this as a separate security fix:
strip C0/C1, CSI/OSC, bidi overrides and zero-width controls at the provider
mapping boundary, with fixture tests.

---

## Delivered slices

The first slice (TUI-00, 01, 02, 03, 04, 06, 07) landed together with the
second (TUI-08–12), the remaining P2 polish, TUI-21 and the sanitizer
hardening. The daemon now reports buffered ranges in `PlaybackState`, and the
HUD shades them without changing the progress rail width.

## Non-goals

Mouse support, emoji iconography, bordered chrome in the main layout, animated
transitions beyond the loading spinner, and turning the lyrics pane into a
separate mode. The console/HUD identity stays.
