# Proposal — TUI density and theme direction

Status: **Draft for review** · 2026-11-25 · Linear AKA-151, AKA-153

Two decisions are still open from the product-design open questions. Both are
cheap to implement and expensive to reverse once screenshots, docs and muscle
memory depend on them, so this note puts the options and a recommendation on the
table rather than picking silently.

## 1. What exists today

**Density** (`[ui] density`, `crates/lekima-config/src/config.rs:77`)

- `compact` is the default (`config.rs:90`); `comfortable` inserts one blank row
  between entries in the track list, the queue and comments
  (`crates/lekima-tui/src/ui.rs:629`, `:1621`, `:2242`).
- It is changed from the TUI settings overlay (`S`) and persisted through
  `save_ui`. There is **no CLI command** that sets density or theme, so the
  project's own rule — "every TUI action has a CLI equivalent" — does not hold
  for these two settings.
- The HUD, sidebar and footer are identical in both modes; only list rhythm
  changes.

**Themes** (`crates/lekima-tui/src/theme.rs:13`)

Twelve palettes ship today: `cyber` (default), `amber`, `mono`, `dracula`,
`gruvbox`, `nord`, `synthwave`, `matrix`, `solarized`, `tokyonight`,
`catppuccin`, `light`. Resolution order is `NO_COLOR` → `LEKIMA_THEME` →
`[ui] theme` → `cyber`, with `auto`/`light` handling from `COLORFGBG`
(`theme.rs:506-540`). `lekima themes` lists them; nothing else drives them.

The original open question was "what is the next theme", written when the set
was much smaller. With twelve palettes the question is no longer "which one
next" but "is the default still the identity we want, and is the set
maintainable".

## 2. Decision A — the default density

| Option | What it means | Trade-off |
| --- | --- | --- |
| **A1. Keep `compact` (recommended)** | Fresh installs start dense; `comfortable` stays an opt-in | Matches the console identity and shows the most rows on 80×24; needs a visible hint so the option is discoverable |
| A2. Default `comfortable` | More air between rows by default | Roughly halves visible tracks in the same terminal; a dense TUI that starts airy feels unlike the screenshots and the vim-first promise |
| A3. Adaptive by terminal height | `compact` below ~30 rows, `comfortable` above, until the user chooses | Rational, but a layout that changes with window size makes screenshots and tests less reproducible, and "why did it change?" is a support cost |

Recommendation: **A1**, plus two small changes that make it a real decision
instead of a non-decision:

1. Show the density in the settings overlay with a one-line explanation
   ("compact fits the most rows; comfortable adds a blank line"), and mention
   `density` in the `?` help.
2. Add `lekima ui --density compact|comfortable` (see §4) so scripts and agents
   can set it, closing the CLI-parity gap.

## 3. Decision B — the visual direction

| Option | What it means | Trade-off |
| --- | --- | --- |
| **B1. Freeze the set, polish `cyber` (recommended)** | No new palette names; tune the default for contrast and for the newer surfaces (hook badges, alerts, history pane), and document the palette contract | Incremental, keeps every existing config and screenshot valid; does not give the project a new "look" |
| B2. A signature theme becomes the default | e.g. a phosphor-green or amber CRT console as `default`, `cyber` demoted | Strongest identity change; invalidates screenshots and muscle memory, and a novel default is a risk on projectors and low-contrast terminals |
| B3. Curate down to ~6 | Keep the best palettes, retire the rest with a migration note | Less surface to maintain and test; removes choices users already set |
| B4. Light-first default | Default to `light` | Terminal users overwhelmingly run dark backgrounds; the `light` palette already exists for those who need it |

Recommendation: **B1**. Concretely:

- Define what a theme must provide: accent, accent-alt, hot, warn, text, muted,
  dim, selection pair, panel, meter pair, glyph set — and a contrast floor for
  `text` and `muted` on its own background.
- Audit the default against the 16-colour and 256-colour cases; name the
  intended fallback for terminals that cannot show truecolor.
- Add a short "authoring a theme" section to `docs/terminal-setup.md` so the set
  stays consistent when someone contributes one.
- Leave `cyber` as the default. If a signature theme is wanted later, it is a
  separate, screenshot-led change with the preview tool.

## 4. Cross-cutting: settings need a CLI

Regardless of A and B, add one command:

```console
$ lekima ui --density compact --theme cyber --glyphs unicode --json
{"ok":true,"data":{"theme":"cyber","glyphs":"unicode","density":"compact","lyrics_translation":false}}
```

- Writes through the existing `save_ui`, so the TUI and the CLI cannot drift.
- `--json` returns the effective set; no daemon is required.
- Makes the density/theme decision scriptable and testable, and satisfies the
  CLI-parity rule.

## 5. Cost

| Choice | Code | Docs | Tests |
| --- | --- | --- | --- |
| A1 + hint + `lekima ui` | Settings copy, one CLI command, `save_ui` reuse | `first-run.md`, `agents.md`, product keymap | CLI contract test, settings render test |
| A2 | One default in config | Same | Update snapshots/preview fixtures |
| A3 | Height-aware default + override | Explain the rule | Layout tests at two heights |
| B1 | Palette audit, authoring notes | `terminal-setup.md` | Theme contrast test |
| B2 | New palette, default flip | Screenshots, README, docs | Preview renders at three sizes |

## 6. What I need from you

1. **Density:** A1 (keep compact) — yes/no? If no, A2 or A3?
2. **Theme:** B1 (freeze and polish) — yes/no? If you want a different identity,
   say "B2" and name the mood (e.g. amber CRT, monochrome terminal, high-contrast).
3. **`lekima ui`:** include it in the same slice?
4. Anything to add or remove from the palette list?

If you accept the recommendations, the slice is: `lekima ui` command + settings
hint + palette contrast audit + docs, with no change to the default density or
the default theme.
