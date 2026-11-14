# Terminal setup — fonts, sizes and variants

A TUI cannot set your terminal's font family or size; those belong to the
emulator. What lekima can do is adapt its own typography (`glyphs`, `density`,
`theme`) and ship ready-made terminal profiles so the whole surface feels like
one system.

## Recommended fonts

Monospace, unambiguous at small sizes, with good box-drawing and block glyphs:

| Font | Why |
| --- | --- |
| **JetBrains Mono** (14–16 pt) | tall x-height, crisp `▌ ▶ █ ░`, free |
| **Berkeley Mono** | dense, excellent at 13–14 pt, paid |
| **Iosevka Term** | narrow; fits more columns on small screens |
| **IBM Plex Mono** | neutral, wide language coverage |
| **SF Mono / Menlo** | already installed on macOS; safe default |

Suggestions: size **14** for laptops, **16** for external displays; line height
**1.2–1.3**; **disable ligatures** (HUD glyphs should not turn into `->`);
disable font smoothing/thickness changes while testing density.

## Variants inside lekima

```toml
[ui]
theme = "cyber"          # cyber | amber | mono
glyphs = "unicode"       # unicode | ascii
density = "compact"      # compact | comfortable
```

- `glyphs = "ascii"` swaps block glyphs for `# - | > []` — for terminals or
  fonts where `█ ░ ▌ ▶` render unevenly.
- `density = "comfortable"` inserts a blank row between track rows.
- `NO_COLOR=1` keeps the glyph/density choice but removes colour.

## iTerm2

Create `~/Library/Application Support/iTerm2/DynamicProfiles/lekima.json`:

```json
{
  "Profiles": [
    {
      "Name": "lekima",
      "Guid": "lekima-cyber",
      "Normal Font": "JetBrainsMono-Regular 14",
      "Non Ascii Font": "JetBrainsMono-Regular 14",
      "Use Non-ASCII Font": false,
      "Horizontal Spacing": 1,
      "Vertical Spacing": 1.05,
      "Use Ligatures": false,
      "Background Color": { "Red Component": 0.02, "Green Component": 0.03, "Blue Component": 0.04 },
      "Foreground Color": { "Red Component": 0.78, "Green Component": 0.84, "Blue Component": 0.89 },
      "Cursor Color": { "Red Component": 0.0, "Green Component": 1.0, "Blue Component": 0.61 },
      "Cursor Text Color": { "Red Component": 0.0, "Green Component": 0.0, "Blue Component": 0.0 },
      "Blinking Cursor": false
    }
  ]
}
```

Then choose the **lekima** profile. Blinking is handled by the TUI itself.

## Ghostty

```ini
font-family = JetBrains Mono
font-size = 14
adjust-cell-height = 10%
background = #050708
foreground = #c8d6e5
cursor-color = #00ff9c
cursor-text = #000000
```

## kitty

```conf
font_family      JetBrains Mono
font_size        14
adjust_line_height 110%
background       #050708
foreground       #c8d6e5
cursor           #00ff9c
cursor_text_color #000000
disable_ligatures always
```

## Alacritty

```toml
[font]
size = 14

[font.normal]
family = "JetBrains Mono"

[colors.primary]
background = "#050708"
foreground = "#c8d6e5"

[colors.cursor]
text = "#000000"
cursor = "#00ff9c"
```

## WezTerm

```lua
local wezterm = require 'wezterm'
return {
  font = wezterm.font('JetBrains Mono', { harfbuzz_features = { 'calt=0', 'clig=0', 'liga=0' } }),
  font_size = 14,
  line_height = 1.15,
  colors = {
    background = '#050708',
    foreground = '#c8d6e5',
    cursor_bg = '#00ff9c',
    cursor_fg = '#000000',
  },
}
```

The palette matches `theme = "cyber"`: accent `#00FF9C`, secondary `#00E5FF`,
hot `#FF2E88`, warn `#FFB000`.
