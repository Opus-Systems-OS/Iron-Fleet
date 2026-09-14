#!/usr/bin/env bash
#
# Scaffold a BlueWeb customer site from the skill's template.
#
#   new-site.sh --name "Kenn's Plumbing" --slug kenns-plumbing \
#               --domain kennsplumbing.com [--dir ~/Documents/BlueWeb/customers]
#
# Leaves a git repo with one commit and a resolved lockfile. Creating the
# GitHub repo and the Cloudflare project is deliberately
# NOT done here — those are one-way and belong in the skill's workflow, where
# a human is watching.
set -euo pipefail

TEMPLATE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../assets/template" && pwd)"
NAME="" SLUG="" DOMAIN="" PARENT="$HOME/Documents/BlueWeb/customers"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --name)   NAME="$2";   shift 2 ;;
    --slug)   SLUG="$2";   shift 2 ;;
    --domain) DOMAIN="$2"; shift 2 ;;
    --dir)    PARENT="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

die() { echo "new-site: $*" >&2; exit 1; }

[[ -n "$NAME"   ]] || die "--name is required"
[[ -n "$SLUG"   ]] || die "--slug is required"
[[ -n "$DOMAIN" ]] || die "--domain is required (use the domain they will buy, even if they have not bought it yet)"

# The slug becomes the directory, the npm package name and the GitHub repo
# name, so it has to be valid in all three.
[[ "$SLUG" =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$ ]] \
  || die "--slug must be lowercase letters, digits and hyphens: got '$SLUG'"
[[ "$DOMAIN" =~ ^[a-z0-9.-]+\.[a-z]{2,}$ ]] \
  || die "--domain should be a bare hostname, no scheme and no trailing slash: got '$DOMAIN'"

DEST="$PARENT/$SLUG"
[[ -e "$DEST" ]] && die "$DEST already exists — pick another slug or move the old one aside"

mkdir -p "$PARENT"
cp -R "$TEMPLATE" "$DEST"

# A half-scaffolded directory is worse than none: the next run would refuse to
# start because the path exists. Clean up unless we reach the end.
cleanup() { [[ -n "${DONE:-}" ]] || rm -rf "$DEST"; }
trap cleanup EXIT

# Substitute the three tokens that appear in file bodies. Everything else the
# site says about the business is filled in afterwards, by editing
# src/data/business.js against the intake notes.
#
# The escaping is not paranoia: a name like "Kenn's Plumbing" lands inside a
# single-quoted JS string in business.js and inside a JSON string in
# package.json, and a bare apostrophe breaks both.
python3 - "$DEST" "$NAME" "$SLUG" "$DOMAIN" <<'PYEOF'
import json, pathlib, sys

dest, name, slug, domain = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
tokens = {"__BUSINESS_NAME__": name, "__SLUG__": slug, "__DOMAIN__": domain}

def escape(value, suffix):
    if suffix == ".json":
        return json.dumps(value)[1:-1]
    if suffix in (".js", ".mjs", ".astro"):
        # Single-quoted JS string literals, which is what the template uses.
        return value.replace("\\", "\\\\").replace("'", "\\'")
    return value                      # markdown, svg, plain text

changed = 0
for path in pathlib.Path(dest).rglob("*"):
    if not path.is_file():
        continue
    try:
        text = path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        continue                      # a logo or photo dropped in by hand
    new = text
    for token, value in tokens.items():
        new = new.replace(token, escape(value, path.suffix))
    if new != text:
        path.write_text(new, encoding="utf-8")
        changed += 1
print(f"  substituted tokens in {changed} files")
PYEOF

# Placeholder favicon: the business's initial on the template's accent color.
# Reads as deliberate in a browser tab, and is meant to be replaced by a real
# mark before launch.
INITIAL="$(printf '%s' "$NAME" | LC_ALL=C tr -cd '[:alnum:]' | cut -c1 | tr '[:lower:]' '[:upper:]')"
cat > "$DEST/public/favicon.svg" <<SVGEOF
<!-- Placeholder. Replace with the customer's real mark before launch; if they
     have no logo, this is honest enough to ship with. The color is a copy of
     business.brand.accent and does not follow it automatically. -->
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" role="img" aria-label="${INITIAL}">
  <rect width="64" height="64" rx="14" fill="#0b5cad"/>
  <text x="32" y="43" text-anchor="middle" font-family="system-ui, sans-serif"
        font-size="34" font-weight="700" fill="#ffffff">${INITIAL}</text>
</svg>
SVGEOF

cd "$DEST"

# npm install, not ci: there is no lockfile yet and generating it is the point.
# This is the only moment in the site's life where `install` is the right verb.
echo "  resolving dependencies (this writes the lockfile)…"
npm install --silent

git init --quiet --initial-branch=main
git add -A
git commit --quiet -m "Scaffold $NAME site

Astro static site from the BlueWeb customer template: strict CSP served from
public/_headers, LocalBusiness structured data pinned by hash, and a
same-origin Pages Function for the contact form.

Business facts are still placeholders — src/data/business.js is the next edit."

DONE=1
echo
echo "  ✓ $DEST"
echo
echo "  Next: fill in src/data/business.js from the intake notes, then"
echo "        npm run dev"
