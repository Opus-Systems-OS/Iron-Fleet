#!/usr/bin/env bash
# Upload one exported model to Roblox as a Model asset (Open Cloud Assets
# API), wait for Roblox to process it, and record the asset id beside it.
#
#   bash <skill dir>/scripts/upload-asset.sh models/coin/coin.fbx "Gold Coin" ["description"]
#
# Run from the game repo root. The creator (your user or group) comes from
# studio.json {"creator": {"userId": "…"} | {"groupId": "…"}}. The key is
# ROBLOX_ASSET_KEY — in the sandbox a placeholder the egress proxy swaps for
# the real key on apis.roblox.com only. An uploaded asset changes nothing in
# any game until code references its id.
#
# Idempotent: if models/<name>/asset.json already records this exact file
# (SHA-256), nothing is uploaded and the recorded id is printed.
set -euo pipefail

file=${1:?usage: upload-asset.sh <model.fbx|.glb> "<display name>" ["description"]}
name=${2:?display name required}
desc=${3:-"Uploaded by the Opus Systems studio"}
[ -f "$file" ] || { echo "no such file: $file" >&2; exit 2; }
[ -f studio.json ] || { echo "no studio.json in $(pwd)" >&2; exit 2; }
[ -n "${ROBLOX_ASSET_KEY:-}" ] || { echo "ROBLOX_ASSET_KEY is not set in this session" >&2; exit 2; }

case "$file" in
  *.fbx) ctype=model/fbx ;;
  *.glb) ctype=model/gltf-binary ;;
  *.gltf) ctype=model/gltf+json ;;
  *) echo "unsupported model format: $file (fbx, glb, gltf)" >&2; exit 2 ;;
esac

record="$(dirname "$file")/asset.json"
sha=$(sha256sum "$file" | cut -d' ' -f1)
if [ -f "$record" ] && [ "$(jq -r '.sha256 // ""' "$record")" = "$sha" ]; then
  echo "unchanged; asset $(jq -r .asset_id "$record")"
  exit 0
fi

creator=$(jq -ec '.creator | if has("userId") then {userId: (.userId|tostring)} elif has("groupId") then {groupId: (.groupId|tostring)} else error("studio.json creator needs userId or groupId") end' studio.json)
request=$(jq -nc --arg n "${name:0:50}" --arg d "${desc:0:1000}" --argjson c "$creator" \
  '{assetType: "Model", displayName: $n, description: $d, creationContext: {creator: $c}}')

resp=$(curl -sS -w '\n%{http_code}' -X POST https://apis.roblox.com/assets/v1/assets \
  -H "x-api-key: ${ROBLOX_ASSET_KEY}" \
  -F "request=${request}" \
  -F "fileContent=@${file};type=${ctype}")
code=$(tail -n1 <<<"$resp")
body=$(sed '$d' <<<"$resp")
[ "$code" = 200 ] || { echo "upload failed: HTTP $code $(head -c 500 <<<"$body")" >&2; exit 1; }
op=$(jq -er '.path // .operationId' <<<"$body")
op=${op#operations/}

asset=""
for _ in $(seq 1 60); do
  sleep 2
  st=$(curl -sS "https://apis.roblox.com/assets/v1/operations/${op}" -H "x-api-key: ${ROBLOX_ASSET_KEY}")
  if [ "$(jq -r '.done // false' <<<"$st")" = true ]; then
    asset=$(jq -r '.response.assetId // empty' <<<"$st")
    [ -n "$asset" ] || { echo "processing failed: $(head -c 500 <<<"$st")" >&2; exit 1; }
    break
  fi
done
[ -n "$asset" ] || { echo "timed out waiting for operation $op" >&2; exit 1; }

jq -n --arg id "$asset" --arg op "$op" --arg sha "$sha" --arg f "$(basename "$file")" \
  '{asset_id: $id, operation: $op, file: $f, sha256: $sha, uploaded_at: (now | todate)}' > "$record"
echo "uploaded ${file} as asset ${asset} (recorded in ${record})"
