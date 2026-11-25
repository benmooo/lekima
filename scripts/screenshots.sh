#!/usr/bin/env bash
# Renders the review fixtures in docs/screenshots from the preview harness.
#
# Every surface is captured at three sizes in the two reference palettes.
# Regenerate after a visual change and commit the diff with the code:
#
#   scripts/screenshots.sh
#
# The files are plain text (trimmed rows, no ANSI), so `git diff` is the review.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/docs/screenshots"
cd "$root"

cargo build -q -p lekima-tui --example preview
binary="target/debug/examples/preview"

mkdir -p "$out"
find "$out" -name '*.txt' -delete

screens=(hud catalogue queue history comments settings)

for theme in cyber mono; do
  for size in 80x24 120x30 160x40; do
    width="${size%x*}"
    height="${size#*x}"
    for screen in "${screens[@]}"; do
      mode="$screen"
      case "$screen" in
        hud) mode="normal" ;;
        catalogue) mode="results" ;;
      esac
      "$binary" \
        --width "$width" \
        --height "$height" \
        --theme "$theme" \
        --screen "$mode" \
        > "$out/$theme-$size-$screen.txt"
    done
  done
done

printf 'rendered %s fixtures in %s\n' "$(find "$out" -name '*.txt' | wc -l | tr -d ' ')" "$out"
