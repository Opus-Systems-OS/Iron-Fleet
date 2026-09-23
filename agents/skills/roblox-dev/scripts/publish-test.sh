#!/usr/bin/env bash
# Build the place and publish it to the game's TEST experience through Roblox
# Open Cloud (Place Publishing API). There is no production target in this
# script, on purpose: publishing the live game is the owner's job.
#
#   bash <skill dir>/scripts/publish-test.sh     # from the game repo root
#
# Ids come from the repo's places.json ({"test": {"universe_id": …,
# "place_id": …}}). The key is ROBLOX_PUBLISH_KEY, which in the sandbox is a
# placeholder the egress proxy swaps for the real key on apis.roblox.com
# only. That key is an Open Cloud `universe-places` write key scoped to the
# test experience alone, so even a wrong id in places.json cannot reach the
# live game: Roblox refuses it (403).
set -euo pipefail
export PATH="$HOME/.local/bin:$PATH"

[ -f places.json ] || { echo "no places.json in $(pwd)" >&2; exit 2; }
[ -n "${ROBLOX_PUBLISH_KEY:-}" ] || { echo "ROBLOX_PUBLISH_KEY is not set in this session" >&2; exit 2; }
universe=$(jq -er '.test.universe_id' places.json)
place=$(jq -er '.test.place_id' places.json)
[[ "$universe" =~ ^[0-9]+$ && "$place" =~ ^[0-9]+$ ]] || { echo "places.json ids must be numbers" >&2; exit 2; }

mkdir -p build
rojo build default.project.json -o build/game.rbxl

resp=$(mktemp)
code=$(curl -sS -o "$resp" -w '%{http_code}' -X POST \
  "https://apis.roblox.com/universes/v1/${universe}/places/${place}/versions?versionType=Published" \
  -H "x-api-key: ${ROBLOX_PUBLISH_KEY}" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @build/game.rbxl)
if [ "$code" != 200 ]; then
  echo "publish failed: HTTP $code $(head -c 500 "$resp")" >&2
  exit 1
fi
version=$(jq -r '.versionNumber' "$resp")
echo "published test place ${place} (universe ${universe}) as version ${version}"
echo "play it: https://www.roblox.com/games/${place}"
