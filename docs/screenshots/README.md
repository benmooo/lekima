# Screenshot fixtures

Deterministic text renders of the TUI, captured from the preview harness at
three sizes in the two reference palettes. They exist so visual review happens
on cells rather than opinion, and so a palette or layout change shows up as a
diff.

| Theme | Sizes | Surfaces |
| --- | --- | --- |
| `cyber` (default) | 80×24, 120×30, 160×40 | HUD, catalogue, queue, history, comments, settings |
| `mono` (`NO_COLOR`) | 80×24, 120×30, 160×40 | same |

The files are plain text with trailing spaces trimmed; colour is not encoded.
Regenerate after any visual change and commit the diff with the code:

```console
$ scripts/screenshots.sh
$ git diff docs/screenshots
```

The script builds `lekima-tui`'s `preview` example and calls it with
`--screen` and `--width/--height`. Other useful invocations:

```console
$ cargo run -p lekima-tui --example preview -- --width 80 --height 24 --screen history
$ cargo run -p lekima-tui --example preview -- --width 120 --height 30 --color 16
$ cargo run -p lekima-tui --example preview -- --width 60 --height 20 --glyphs ascii
```

`--color` accepts `truecolor`, `256`, `16` and `mono`; `--motion` accepts
`auto`, `full` and `reduced`. Add `--ansi` to print styled cells instead of
plain text, which is how the colour downgrade is reviewed:

```console
$ cargo run -p lekima-tui --example preview -- --width 80 --height 24 --color 16 --ansi
$ cargo run -p lekima-tui --example preview -- --width 80 --height 24 --color 256 --ansi | grep -c '38;2'   # 0
```

The committed fixtures stay plain so `git diff` shows layout, not escape codes.
