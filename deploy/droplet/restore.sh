#!/usr/bin/env bash
# Fetch, decrypt and check a backup.sh bundle. Runs on the Mac (or any box
# with rclone, age and sqlite3), never on the droplet by itself: it writes
# only under ./restore/<stamp>/ and prints the re-seed commands rather than
# touching the live volume.
#
#   restore.sh latest                       # newest object in the bucket
#   restore.sh iron-fleet-20260916T0700Z.tar.age
#   restore.sh /var/backups/iron-fleet/iron-fleet-20260916T0700Z.tar.age   # local file
#
# Needs the R2 variables from backup.env.example in the environment, or in
# the file named by $BACKUP_ENV (default: ./backup.env if present), and the
# age identity in $AGE_IDENTITY (default ~/.config/iron-fleet/backup.key).
set -euo pipefail

SRC="${1:?usage: restore.sh <latest|object-name|local-file>}"
AGE_IDENTITY="${AGE_IDENTITY:-$HOME/.config/iron-fleet/backup.key}"
BACKUP_ENV="${BACKUP_ENV:-./backup.env}"
OUT=./restore

if [[ -f "$BACKUP_ENV" ]]; then
  # shellcheck disable=SC1090
  set -a; . "$BACKUP_ENV"; set +a
fi
[[ -r "$AGE_IDENTITY" ]] || { echo "restore: age identity not readable: $AGE_IDENTITY" >&2; exit 1; }

# --- fetch ---------------------------------------------------------------
mkdir -p "$OUT"
if [[ -f "$SRC" ]]; then
  BUNDLE="$SRC"
else
  : "${R2_BUCKET:?R2_BUCKET is not set (source backup.env)}"
  if [[ "$SRC" == "latest" ]]; then
    SRC=$(rclone lsf "r2:$R2_BUCKET" --include 'iron-fleet-*.tar.age' | sort | tail -n 1)
    [[ -n "$SRC" ]] || { echo "restore: no iron-fleet-*.tar.age objects in r2:$R2_BUCKET" >&2; exit 1; }
  fi
  rclone copy "r2:$R2_BUCKET/$SRC" "$OUT/"
  BUNDLE="$OUT/$SRC"
fi

STAMP=$(basename "$BUNDLE" .tar.age)
STAMP="${STAMP#iron-fleet-}"
DIR="$OUT/$STAMP"
mkdir -p "$DIR"

# --- decrypt + check -----------------------------------------------------
age -d -i "$AGE_IDENTITY" "$BUNDLE" | tar -C "$DIR" -xf -
chmod 600 "$DIR/env" "$DIR/backup.env"

echo "restore: $BUNDLE -> $DIR"
cat "$DIR/MANIFEST"
check=$(sqlite3 "$DIR/control-plane.db" 'PRAGMA integrity_check;')
echo "integrity_check: $check"
[[ "$check" == "ok" ]]
echo "agents:"
sqlite3 -header -column "$DIR/control-plane.db" 'SELECT slug, anthropic_id, anthropic_version FROM agents ORDER BY slug;'
echo "session_usage rows: $(sqlite3 "$DIR/control-plane.db" 'SELECT count(*) FROM session_usage;')"

cat <<EOF

To prove the restore, serve it locally (no Anthropic calls with SYNC_ON_BOOT=false):
  DATABASE_PATH=$DIR/control-plane.db SYNC_ON_BOOT=false PORT=18080 \\
    ANTHROPIC_API_KEY=x ANTHROPIC_WEBHOOK_SIGNING_KEY=whsec_eA== CONTROL_PLANE_TOKEN=dev \\
    cargo run -p control-plane -- serve
  curl -H 'Authorization: Bearer dev' localhost:18080/agents
  curl -H 'Authorization: Bearer dev' localhost:18080/usage

To re-seed a droplet from it, follow "First deploy" in deploy/droplet/README.md
with $DIR/control-plane.db as the seed, $DIR/env as .env and
$DIR/caddy_data.tar untarred into the droplet_caddy_data volume.
EOF
