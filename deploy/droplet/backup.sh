#!/usr/bin/env bash
# Nightly backup of everything on the droplet that isn't reproducible from
# the repo: control-plane.db (the only record of which Anthropic agents,
# skills, environments and vaults are ours), the two env files, and Caddy's
# certificate/ACME state. One age-encrypted tarball to Cloudflare R2.
#
# Runs on the host as root from iron-fleet-backup.timer (07:00 UTC daily);
# `systemctl start iron-fleet-backup.service` runs it by hand. Reads
# backup.env (see backup.env.example) — never the containers' .env for
# config, though that file is one of the things backed up.
set -euo pipefail

HERE=/opt/iron-fleet/deploy/droplet
DB=/var/lib/docker/volumes/droplet_cp_data/_data/control-plane.db
# The API's keys database (Opus-Systems-OS-API). Optional until that service
# has booted once; a bundle without it restores a fleet with no client keys.
API_DB=/var/lib/docker/volumes/droplet_api_data/_data/opus-api.db
CADDY_DATA=/var/lib/docker/volumes/droplet_caddy_data/_data
LOCAL=/var/backups/iron-fleet
KEEP_LOCAL=3

# shellcheck disable=SC1091
set -a; . "$HERE/backup.env"; set +a
: "${AGE_RECIPIENT:?backup.env: AGE_RECIPIENT is empty}"
: "${R2_BUCKET:?backup.env: R2_BUCKET is empty}"
: "${RCLONE_CONFIG_R2_ACCESS_KEY_ID:?backup.env: R2 credentials are empty}"
RETENTION_DAYS="${RETENTION_DAYS:-30}"

STAMP=$(date -u +%Y%m%dT%H%MZ)
NAME="iron-fleet-$STAMP.tar.age"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$LOCAL"
chmod 700 "$LOCAL"

# --- database ------------------------------------------------------------
# The online backup API copies a consistent snapshot while the control plane
# keeps writing (it runs in WAL mode). A plain `cp` would not.
sqlite3 "$DB" ".backup '$TMP/control-plane.db'"
check=$(sqlite3 "$TMP/control-plane.db" 'PRAGMA integrity_check;')
if [[ "$check" != "ok" ]]; then
  echo "backup: integrity_check failed on the snapshot: $check" >&2
  exit 1
fi
agents=$(sqlite3 "$TMP/control-plane.db" 'SELECT count(*) FROM agents;')
api_keys=absent
if [[ -f "$API_DB" ]]; then
  sqlite3 "$API_DB" ".backup '$TMP/opus-api.db'"
  check=$(sqlite3 "$TMP/opus-api.db" 'PRAGMA integrity_check;')
  if [[ "$check" != "ok" ]]; then
    echo "backup: integrity_check failed on the API snapshot: $check" >&2
    exit 1
  fi
  api_keys=$(sqlite3 "$TMP/opus-api.db" 'SELECT count(*) FROM api_keys;')
fi

# --- secrets and TLS state -----------------------------------------------
cp "$HERE/.env" "$TMP/env"
cp "$HERE/backup.env" "$TMP/backup.env"
# The volume also holds Caddy's access logs (MBs, rolling) — only certs/ACME state is worth keeping.
tar -C "$CADDY_DATA" --exclude='access-*.log' -cf "$TMP/caddy_data.tar" .
printf 'stamp=%s\nagents=%s\ndb_bytes=%s\napi_keys=%s\n' "$STAMP" "$agents" "$(stat -c %s "$TMP/control-plane.db")" "$api_keys" > "$TMP/MANIFEST"

# --- encrypt + upload ----------------------------------------------------
tar -C "$TMP" -cf - control-plane.db env backup.env caddy_data.tar MANIFEST \
    $([[ -f "$TMP/opus-api.db" ]] && echo opus-api.db) \
  | age -r "$AGE_RECIPIENT" -o "$LOCAL/$NAME"
chmod 600 "$LOCAL/$NAME"

# --s3-no-head: Ubuntu's rclone (1.60) re-HEADs the object after upload with
# a request R2 answers 501 to; the upload itself is fine, but rclone logs an
# ERROR and retries once. Skipping the read-back avoids the false alarm.
rclone copy --s3-no-check-bucket --s3-no-head "$LOCAL/$NAME" "r2:$R2_BUCKET/"
rclone delete "r2:$R2_BUCKET" --min-age "${RETENTION_DAYS}d" --include 'iron-fleet-*.tar.age'

# Keep the newest KEEP_LOCAL on disk for a restore that doesn't need R2.
ls -1t "$LOCAL"/iron-fleet-*.tar.age | tail -n +$((KEEP_LOCAL + 1)) | xargs -r rm -f

echo "backup: uploaded $NAME ($(stat -c %s "$LOCAL/$NAME") bytes, $agents agents) to r2:$R2_BUCKET; retention ${RETENTION_DAYS}d"
