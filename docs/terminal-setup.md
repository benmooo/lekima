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
theme = "cyber"          # cyber | amber | mono | … (see the theme table)
glyphs = "unicode"       # unicode | ascii
density = "compact"      # compact | comfortable
motion = "auto"          # auto | full | reduced
```

- `lekima ui --json` prints the effective set; the same flags
  (`--theme`, `--glyphs`, `--density`, `--motion`, `--translation`) write it
  through the same config the TUI saves, so scripts and agents can set a
  preference without editing TOML.
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

## Themes

`lekima themes` lists them; `S` cycles them live; `:theme <name>` jumps
directly. All themes keep the terminal background untouched.

| Theme | Accent | Secondary | Hot |
| --- | --- | --- | --- |
| `cyber` (default) | `#00FF9C` | `#00E5FF` | `#FF2E88` |
| `amber` | `#FFB000` | `#FFD666` | `#FF5A1F` |
| `mono` | white | gray | white |
| `dracula` | `#50FA7B` | `#8BE9FD` | `#FF79C6` |
| `gruvbox` | `#B8BB26` | `#83A598` | `#FB4934` |
| `nord` | `#A3BE8C` | `#88C0D0` | `#BF616A` |
| `synthwave` | `#00F0FF` | `#FF2E88` | `#FF2E88` |
| `matrix` | `#00FF41` | `#00CC33` | `#FF5555` |
| `solarized` | `#B58900` | `#2AA198` | `#DC322F` |
| `tokyonight` | `#9ECE6A` | `#7AA2F7` | `#F7768E` |
| `catppuccin` | `#A6E3A1` | `#89B4FA` | `#F38BA8` |

Terminal profiles above use the `cyber` palette; for other themes keep the
terminal background/cursor neutral and let the TUI carry the accents.
