#!/usr/bin/env bash
# Render models/<name>/preview.png from the scene blender-run.sh just built.
#
#   bash <skill dir>/scripts/render-preview.sh models/coin
set -euo pipefail
dir=${1:?usage: render-preview.sh models/<name>}
here=$(cd "$(dirname "$0")" && pwd)
test -f "/tmp/$(basename "$dir").blend" || { echo "run blender-run.sh $dir first" >&2; exit 2; }
blender -b -P "$here/blender/preview.py" -- "$dir" 2>&1 | grep -E '^(PREVIEW|Error|ERROR|Traceback)' || true
test -f "$dir/preview.png" || { echo "render failed" >&2; exit 1; }
