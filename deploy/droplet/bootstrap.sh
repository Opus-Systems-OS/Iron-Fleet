#!/usr/bin/env bash
# One-time droplet setup. Idempotent — safe to re-run. Run as root:
#   ssh root@198.199.66.109 'bash -s' < deploy/droplet/bootstrap.sh
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
  echo "run as root" >&2
  exit 1
fi

# --- swap ------------------------------------------------------------------
# 961 MB with two Rust services + Caddy + dockerd wants headroom.
if ! swapon --show | grep -q '^/swapfile'; then
  if [[ ! -f /swapfile ]]; then
    fallocate -l 2G /swapfile
    chmod 600 /swapfile
    mkswap /swapfile
  fi
  swapon /swapfile
  grep -q '^/swapfile' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
  echo "swap: enabled"
else
  echo "swap: already on"
fi
sysctl -q -w vm.swappiness=10
grep -q '^vm.swappiness' /etc/sysctl.d/99-iron-fleet.conf 2>/dev/null \
  || echo 'vm.swappiness=10' > /etc/sysctl.d/99-iron-fleet.conf

# --- docker ----------------------------------------------------------------
if docker compose version >/dev/null 2>&1; then
  echo "docker: already installed ($(docker --version))"
else
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -q
  apt-get install -y -q ca-certificates curl gnupg git
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
  chmod a+r /etc/apt/keyrings/docker.asc
  . /etc/os-release
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu ${VERSION_CODENAME} stable" \
    > /etc/apt/sources.list.d/docker.list
  apt-get update -q
  apt-get install -y -q docker-ce docker-ce-cli containerd.io docker-compose-plugin
  systemctl enable --now docker
  echo "docker: installed ($(docker --version))"
fi

# --- firewall --------------------------------------------------------------
# 22 ssh, 80 ACME + redirect, 443 tcp+udp (HTTP/3). Nothing else listens.
# Docker punches its own iptables rules for published ports, so ufw rules
# here govern the host; the compose file publishes only Caddy's ports.
ufw allow 22/tcp >/dev/null
ufw allow 80/tcp >/dev/null
ufw allow 443/tcp >/dev/null
ufw allow 443/udp >/dev/null
ufw --force enable >/dev/null
echo "ufw: $(ufw status | head -1)"

# --- checkout --------------------------------------------------------------
if [[ ! -d /opt/iron-fleet/.git ]]; then
  git clone -q https://github.com/Opus1247/Iron-Fleet /opt/iron-fleet
  echo "repo: cloned to /opt/iron-fleet"
else
  echo "repo: /opt/iron-fleet already present"
fi
cd /opt/iron-fleet/deploy/droplet
if [[ ! -f .env ]]; then
  cp env.example .env
  echo ".env: created from env.example — fill it in"
fi
chmod 600 .env
chmod +x deploy.sh
mkdir -p /root/seed

cat <<'NEXT'

bootstrap done. Next, in this order (see deploy/droplet/README.md):
  1. seed the database:   /root/seed/control-plane.db  (a backup of the live one)
  2. fill in:             /opt/iron-fleet/deploy/droplet/.env
  3. first start:         /opt/iron-fleet/deploy/droplet/deploy.sh
NEXT
