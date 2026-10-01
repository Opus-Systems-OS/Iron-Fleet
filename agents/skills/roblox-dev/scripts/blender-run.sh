#!/usr/bin/env bash
# Build a model from its script and export it (Blender, headless).
#
#   bash <skill dir>/scripts/blender-run.sh models/coin
#
# models/coin/build.py → models/coin/coin.fbx + coin.glb, and a STATS line
# (triangles, sizes). Roblox's hard limit is 20,000 triangles per mesh; the
# house budget is far lower — see references/modeling.md.
set -euo pipefail
dir=${1:?usage: blender-run.sh models/<name>}
here=$(cd "$(dirname "$0")" && pwd)
command -v blender >/dev/null || { echo "blender is not installed in this sandbox" >&2; exit 2; }
blender -b --factory-startup -P "$here/blender/export.py" -- "$dir" 2>&1 \
  | grep -E '^(STATS|Error|ERROR|Traceback|  File|[A-Za-z]+Error)' || true
test -f "$dir/$(basename "$dir").fbx" || { echo "export failed (no .fbx)" >&2; exit 1; }
