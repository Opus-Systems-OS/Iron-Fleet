#!/usr/bin/env bash
# Install the Roblox toolchain into ~/.local/bin: rojo, selene, stylua,
# luau-lsp — pinned release binaries, each checked against GitHub's published
# SHA-256 before it is unpacked. About two seconds in a cloud sandbox.
#
# Why not the environment's cargo packages: tried 2026-09-23, nothing landed
# on PATH. Why not rokit: it resolves versions through the GitHub API, which
# the sandbox hits unauthenticated and gets rate-limited on.
#
#   bash <skill dir>/scripts/setup.sh            # idempotent; re-run is a no-op
#   bash <skill dir>/scripts/setup.sh --blender  # also Blender 4.5 LTS (~380 MB)
#
# --blender: RoForge needs Blender 4.2+, and the environment's apt blender is
# 4.0. The official LTS tarball goes to ~/.local/opt and `blender` in
# ~/.local/bin points at it, so it wins over apt on PATH — blender-run.sh and
# render-preview.sh use it too. Only modeling jobs need it.
set -euo pipefail

BLENDER=0
[ "${1:-}" = "--blender" ] && BLENDER=1

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

if [ "$BLENDER" = 1 ]; then
  bver=4.5.14
  bdir="$HOME/.local/opt/blender-$bver-linux-x64"
  bsha=9ba871ff2ecd36526b77432745980b7e6664ecd0c7ca11c48849073dcfe06da3
  if [ ! -x "$bdir/blender" ]; then
    curl -fsSL "https://download.blender.org/release/Blender4.5/blender-$bver-linux-x64.tar.xz" -o "$tmp/blender.tar.xz"
    echo "$bsha  $tmp/blender.tar.xz" | sha256sum -c --quiet -
    mkdir -p "$HOME/.local/opt"
    tar xJf "$tmp/blender.tar.xz" -C "$HOME/.local/opt"
  fi
  ln -sf "$bdir/blender" "$BIN/blender"
  printf '%-9s %s\n' blender "$("$BIN/blender" --version 2>/dev/null | head -1)"
fi

for t in rojo selene stylua luau-lsp; do
  printf '%-9s %s\n' "$t" "$("$BIN/$t" --version 2>&1 | head -1)"
done
echo "PATH needs $BIN (export PATH=\"$BIN:\$PATH\")"
