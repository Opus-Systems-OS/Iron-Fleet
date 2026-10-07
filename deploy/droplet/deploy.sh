#!/usr/bin/env bash
# Pull the latest deploy config and images, restart what changed.
# Run on the droplet as root. Images are built by GitHub Actions; if a build
# is still running, `compose pull` just fetches the previous tag.
set -euo pipefail

cd /opt/iron-fleet
git pull --ff-only
cd deploy/droplet

# jarvis-web's keys are Docker secret files (docker-compose.yml). Once, move
# each from .env into its own 0400 file; then delete the line from .env.
mkdir -p -m 700 secrets
for pair in WEB_API_KEY:web_api_key WEB_POWERS_API_KEY:web_powers_api_key; do
  var=${pair%%:*} file=secrets/${pair#*:}
  if [ ! -s "$file" ]; then
    value=$(grep -E "^${var}=" .env | tail -1 | cut -d= -f2- | tr -d "'\"" || true)
    if [ -z "$value" ]; then
      echo "deploy: $file is missing and .env has no $var; jarvis-web needs it" >&2
      exit 1
    fi
    (umask 077 && printf '%s\n' "$value" > "$file") && chmod 400 "$file"
    echo "deploy: moved $var into $file — now delete its line from .env"
  elif grep -qE "^${var}=" .env; then
    echo "deploy: $var is still in .env; jarvis-web reads $file now — delete the line"
  fi
done

docker compose pull
docker compose up -d --remove-orphans
docker compose ps
