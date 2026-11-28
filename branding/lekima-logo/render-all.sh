#!/usr/bin/env bash
# Render the full LEKIMA identity set. Requires Blender 4.2+ and ffmpeg.
set -euo pipefail

BLENDER=${BLENDER:-blender}
HERE=$(cd "$(dirname "$0")" && pwd)

run() {
  "$BLENDER" -b --factory-startup --python "$HERE/logo.py" -- "$@"
}

run --scene hero --samples 72
run --scene square --samples 64
run --scene alpha --samples 64
run --scene loop --samples 48

for theme in amber matrix; do
  run --scene hero --theme "$theme" --res 2560x1440 --samples 48
done
