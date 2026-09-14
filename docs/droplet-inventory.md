# Droplet inventory — 2026-09-14 (read-only recon, Phase 0)

| | |
|---|---|
| Provider / region | DigitalOcean, **nyc1** (not SFO3 as earlier notes said) |
| Hostname | `opus-os-s-1vcpu-1gb-nyc1` |
| IPv4 / IPv6 | `198.199.66.109` / `2604:a880:400:d1:0:4:f807:7001` |
| OS | Ubuntu 24.04.4 LTS, kernel 6.8.0-124 |
| Size | 1 vCPU, 961 MB RAM, 24 GB disk (2.2 GB used), **no swap** |
| Access | `root` on port 22, key auth only (the Mac's `~/.ssh/id_ed25519`) |

## What is running

Nothing beyond the base image. Listening ports: 22 (sshd) and the local
resolver only. No Docker, no Caddy/nginx, **no MQTT broker** (the
"already on the droplet" claim in the outside proposal was wrong), no
services in `/opt`, `/srv`, or `/root`. `ufw` is installed but inactive.
`unattended-upgrades` is enabled.

"Opus Tower OS" is the droplet's name, not software on it. There is
nothing to coexist with — Phase 1 starts from a clean box.

## Implications for the plan

- Docker + compose must be installed (Phase 1, first step).
- Add a 2 GB swapfile before any image build on the box, or build on the
  Mac and ship images. 961 MB with no swap will OOM a Rust release build.
- `ufw`: enable with 22/80/443 before exposing anything. No 1883 rule —
  there is no MQTT to expose.
- Domain: **unconfirmed.** Nothing on the box hints at one. Needed before
  Caddy can issue certificates.
- Mosquitto is dropped from the target diagram until something actually
  needs it.
