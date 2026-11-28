# LEKIMA identity — 3D, generated from the source of truth

The logo is built from the project's own visual language, not from a mood board:

- the wordmark is the shipped `WORDMARK` block art in
  [`crates/lekima-tui/src/ui.rs`](../../crates/lekima-tui/src/ui.rs), extruded
  into segmented console keys;
- the two-tone split (`LEK` mint, `IMA` cyan) is that constant's split;
- every colour is parsed from [`crates/lekima-tui/src/theme.rs`](../../crates/lekima-tui/src/theme.rs),
  so the same model wears any of the twelve palettes;
- the slab carries the HUD vocabulary: inverted brand chip, `// CONSOLE`
  section label, dim rule, 12-cell rail, version silkscreen and focus ticks.

Nothing here is liquid metal, chrome, or a rendered gradient. The slab is matte
console hardware; the only light sources are the emissive keys and the studio.

## Render

```bash
# 4K hero (16:9, dark slab)
blender -b --factory-startup --python branding/lekima-logo/logo.py -- --scene hero

# transparent lockup (RGBA), square monogram, 4 s loop
blender -b --factory-startup --python branding/lekima-logo/logo.py -- --scene alpha
blender -b --factory-startup --python branding/lekima-logo/logo.py -- --scene square
blender -b --factory-startup --python branding/lekima-logo/logo.py -- --scene loop

# a skin: the same model in another palette
blender -b --factory-startup --python branding/lekima-logo/logo.py -- --scene hero --theme amber

# fast look-dev pass
blender -b --factory-startup --python branding/lekima-logo/logo.py -- --scene hero --preview
```

Outputs land in `out/`:

| File | Purpose |
| --- | --- |
| `lekima-logo-hero-<theme>.png` | 3840×2160 product shot (skin renders are 2560×1440) |
| `lekima-logo-alpha-<theme>.png` | 3840×2160 transparent lockup (no slab, no floor) |
| `lekima-logo-square-<theme>.png` | 2048×2048 `L` monogram tile |
| `lekima-logo-loop-<theme>.mp4` | 1920×1080, 4 s loop, H.264, 30 fps |
| `lekima-logo-hero-cyber-1920.png` | web-size hero |
| `lekima-icon-512.png`, `lekima-icon-128.png` | icon derivatives |
| `frames/<theme>/` | rendered PNG sequence for the loop |

`./render-all.sh` renders the whole set; set `BLENDER=/path/to/blender` if it
is not on `PATH`.

## What maps to what

| TUI token / constant | In the render |
| --- | --- |
| `WORDMARK` (`█`, `▀`, `▄`) | one extruded key per inked half-cell |
| `accent` / `accent_alt` | `LEK` keys / `IMA` keys (emissive, brand-exact) |
| `badge_bg` / `badge_fg` | the ` LEKIMA ` chip and the blinking `▌` cursor |
| `panel` / `muted` / `dim` | `//` markers, silkscreen, the rule under the wordmark |
| `meter_empty` / `accent` | the 12-cell rail: dim cells, stepped fill |
| `ground` | slab tone and world |
| cursor `▌`, 2 Hz | blinking block in the top-right, 8 frames at 30 fps |

Motion follows the design system: the camera drifts two degrees for parallax,
the rail steps one cell at a time, and the cursor blinks at the HUD's 2 Hz. No
smooth progress gradients.

## Notes

- **EEVEE is the only practical engine here.** Headless EEVEE works on this
  machine; Cycles on the M1 GPU took 8.5 minutes for a 480×270 test frame.
- **Blender 5.3 in this build has no FFMPEG image format**, so `--scene loop`
  renders PNGs and shells out to `ffmpeg` for the H.264 encode.
- **`mono` cannot be rendered.** It has no RGB roles (it inherits terminal
  colours), so the script refuses it instead of inventing a palette. `light`
  works: the slab tone follows the palette ground.
- The script reads `Cargo.toml` for the version silkscreen, so a release only
  needs a re-render, not a text edit.
