#!/usr/bin/env bash
# Pull the latest deploy config and images, restart what changed.
# Run on the droplet as root. Images are built by GitHub Actions; if a build
# is still running, `compose pull` just fetches the previous tag.
set -euo pipefail

cd /opt/iron-fleet
git pull --ff-only
cd deploy/droplet
docker compose pull
docker compose up -d --remove-orphans
docker compose ps
