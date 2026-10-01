#!/usr/bin/env bash
# Every check the game repo's CI runs, in the same order, from the repo root:
# format, lint, types, build. Exit 0 means the PR will be green.
#
#   bash <skill dir>/scripts/check.sh            # from the game repo root
set -euo pipefail
export PATH="$HOME/.local/bin:$PATH"

[ -f default.project.json ] || { echo "run from the game repo root (no default.project.json)" >&2; exit 2; }

echo "== stylua";   stylua --check src
echo "== selene";   selene src
# luau-lsp needs Roblox's global types and a sourcemap of the Rojo tree. The
# definitions file is committed (types/globalTypes.d.luau) so this works
# offline and pins the API surface the code is checked against.
echo "== luau-lsp"
rojo sourcemap default.project.json -o sourcemap.json
luau-lsp analyze \
  --platform=roblox \
  --sourcemap=sourcemap.json \
  --definitions=@roblox=types/globalTypes.d.luau \
  --base-luaurc=.luaurc \
  src
echo "== rojo build"
mkdir -p build
rojo build default.project.json -o build/game.rbxl
ls -l build/game.rbxl
echo "all checks passed"
