#!/usr/bin/env bash
# Install the Roblox toolchain into ~/.local/bin: rojo, selene, stylua,
# luau-lsp — pinned release binaries, each checked against GitHub's published
# SHA-256 before it is unpacked. About two seconds in a cloud sandbox.
#
# Why not the environment's cargo packages: tried 2026-09-23, nothing landed
# on PATH. Why not rokit: it resolves versions through the GitHub API, which
# the sandbox hits unauthenticated and gets rate-limited on.
#
#   bash <skill dir>/scripts/setup.sh        # idempotent; re-run is a no-op
set -euo pipefail

BIN="$HOME/.local/bin"
mkdir -p "$BIN"
export PATH="$BIN:$PATH"

# name  version  url  sha256
TOOLS=(
  "rojo 7.7.0 https://github.com/rojo-rbx/rojo/releases/download/v7.7.0/rojo-7.7.0-linux-x86_64.zip 22503e5839864f9d7c2171c48b536fc229f2cc4d8774c9cc149f60941d864073"
  "selene 0.31.0 https://github.com/Kampfkarren/selene/releases/download/0.31.0/selene-0.31.0-linux.zip dac452422747999ec4919bbb8bb52992b66aae533b60022bf005669de8616671"
  "stylua 2.5.2 https://github.com/JohnnyMorganz/StyLua/releases/download/v2.5.2/stylua-linux-x86_64.zip bcb0d855e91f102f28a370e850f8566b3b44b79e6274d806ea5246837c0fd5ab"
  "luau-lsp 1.70.0 https://github.com/JohnnyMorganz/luau-lsp/releases/download/1.70.0/luau-lsp-linux-x86_64.zip 4ff08890ea0d4b6d9de25fdff1a4c87e0dc9f2e45d782c894e83475e51d55813"
)

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for line in "${TOOLS[@]}"; do
  read -r name version url sha <<<"$line"
  if [ -x "$BIN/$name" ] && "$BIN/$name" --version 2>/dev/null | grep -q "$version"; then
    continue
  fi
  curl -fsSL "$url" -o "$tmp/$name.zip"
  echo "$sha  $tmp/$name.zip" | sha256sum -c --quiet -
  unzip -oq "$tmp/$name.zip" -d "$tmp/$name"
  install -m 0755 "$tmp/$name/$name" "$BIN/$name"
done

for t in rojo selene stylua luau-lsp; do
  printf '%-9s %s\n' "$t" "$("$BIN/$t" --version 2>&1 | head -1)"
done
echo "PATH needs $BIN (export PATH=\"$BIN:\$PATH\")"
