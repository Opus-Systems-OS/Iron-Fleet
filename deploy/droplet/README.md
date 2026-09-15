# deploy/droplet

`control-plane` and `mcp-fleet` on the opustower.dev droplet (DigitalOcean
nyc1, `198.199.66.109` / `2604:a880:400:d1:0:4:f807:7001`, root SSH). Phase 1
of `docs/centralization-plan.md`. The box runs three containers — Caddy plus
the two images GitHub Actions publishes to GHCR — and builds nothing.

| File | What |
|---|---|
| `bootstrap.sh` | One-time, idempotent: swap, Docker, ufw, clone to `/opt/iron-fleet`. |
| `docker-compose.yml` | `caddy`, `control-plane`, `mcp-fleet`. Only Caddy publishes ports. |
| `Caddyfile` | `fleet.opustower.dev` → control-plane, `mcp.opustower.dev` → mcp-fleet. Auto TLS. |
| `env.example` | Every variable both services read. Copy to `.env`, `chmod 600`. |
| `deploy.sh` | `git pull`, `compose pull`, `compose up -d`. The whole deploy. |

Images: `ghcr.io/opus1247/iron-fleet/control-plane` and `…/mcp-fleet`,
built by `.github/workflows/images.yml` on every push to `main` that touches
either crate, `agents/`, or the workspace manifests. Both packages are
public (the repo is), so the droplet pulls without a login. If they ever go
private, `docker login ghcr.io` on the box with a `read:packages` PAT.

## First deploy

The order matters for one reason: **the control plane's SQLite is the only
record of which Anthropic agents, skills, environments and vaults belong to
this fleet.** Boot sync consults nothing else. Starting with an empty
database would create a second `rig-gpu` environment (new key, rig orphaned),
collide on the immutable skill name, and duplicate every agent. Seed the
database from a backup of the live one *before* the first start. (The
original 2026-09-14 seed was a copy of the Railway volume; Railway is gone,
so a rebuild seeds from the droplet's own `control-plane.db` — take it with
`sqlite3 /data/control-plane.db '.backup /root/seed/control-plane.db'`
inside the running container, or from the Phase 5 backups once they
exist.)

```sh
# 0. on the Mac, once — puts the passphrased key in the shared agent
ssh-add --apple-use-keychain ~/.ssh/id_ed25519

# 1. bootstrap the box
ssh root@198.199.66.109 'bash -s' < deploy/droplet/bootstrap.sh

# 2. seed the database from a backup (see above); sanity-check it first
mkdir -p seed
sqlite3 seed/control-plane.db 'PRAGMA wal_checkpoint(TRUNCATE); PRAGMA integrity_check; SELECT slug, agent_id FROM agents;'
scp seed/control-plane.db root@198.199.66.109:/root/seed/
ssh root@198.199.66.109 'cd /opt/iron-fleet/deploy/droplet && docker compose create control-plane >/dev/null && docker run --rm -v droplet_cp_data:/data -v /root/seed:/seed alpine cp /seed/control-plane.db /data/'

# 3. fill .env on the box — every name in env.example, values from the
#    secret store; never through a chat or a file on the laptop
ssh root@198.199.66.109   # edit /opt/iron-fleet/deploy/droplet/.env; MCP_FLEET_URL=https://mcp.opustower.dev/mcp

# 4. start
ssh root@198.199.66.109 /opt/iron-fleet/deploy/droplet/deploy.sh
```

The compose project name is the directory, `droplet`, so the data volume is
`droplet_cp_data`. Caddy fetches certificates on the first request to each
host; the first `curl` may take a few seconds.

## Checks

```sh
curl -sS https://fleet.opustower.dev/healthz; curl -sS https://mcp.opustower.dev/healthz
curl -sS -H "Authorization: Bearer $CONTROL_PLANE_TOKEN" https://fleet.opustower.dev/agents   # the four agent ids from the seed
ssh root@198.199.66.109 'cd /opt/iron-fleet/deploy/droplet && docker compose logs control-plane | grep -E "sync complete|vault"'
# expect: agents_updated: 1 (jarvis, its MCP URL changed), nothing created;
#         an mcp-fleet credential added for the new URL.
```

Then a `POST /sessions` for `jarvis` with a trivial task, and the signed
local webhook from `control-plane/README.md`.

## History

This box replaced the original Railway hosting on 2026-09-15; the
cut-over record is in `docs/centralization-plan.md` ("Resume here" and
Phase 1). Two checks from it worth repeating after any rebuild: a `jarvis`
session whose task calls `list_agents` (`docker compose exec caddy tail
/data/access-mcp.log` shows the call), and a session run to idle whose
`session.status_idled` shows up in `GET /usage`.

## Day-to-day

Deploy: `ssh root@198.199.66.109 /opt/iron-fleet/deploy/droplet/deploy.sh`.
Logs: `docker compose logs -f control-plane` (in `/opt/iron-fleet/deploy/droplet`).
Access logs: `docker compose exec caddy tail -f /data/access-fleet.log`.
