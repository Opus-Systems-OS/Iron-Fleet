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
| `backup.sh` | Nightly: `sqlite3 .backup` of the db + `.env` + `backup.env` + Caddy's cert state → one `age`-encrypted tarball → Cloudflare R2, 30-day retention. Run by the timer. |
| `iron-fleet-backup.service` / `.timer` | systemd units for `backup.sh`, 07:00 UTC daily, `Persistent=true`. Installed by `bootstrap.sh`. |
| `backup.env.example` | R2 token, bucket, age recipient. Copy to `backup.env`, `chmod 600`. Read only by `backup.sh`. |
| `restore.sh` | Runs on the Mac: fetch a bundle (or `latest`), decrypt, `integrity_check`, print the agents and the re-seed commands. Never touches the droplet. |

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
so a rebuild seeds from the nightly backup: `restore.sh latest` on the Mac
produces `restore/<stamp>/control-plane.db`, plus the `.env` and Caddy
state to put back — see "Ops" below. If the old box is still reachable,
`sqlite3 /var/lib/docker/volumes/droplet_cp_data/_data/control-plane.db
'.backup /root/seed/control-plane.db'` on it is fresher.)

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
Access logs: `docker compose exec caddy tail -f /data/access-fleet.log`
(Caddy rolls them itself: 100 MB × 10 files, nothing to configure).

## Ops (Phase 5)

### Backups

Every night at 07:00 UTC (`iron-fleet-backup.timer`) `backup.sh` bundles
`control-plane.db` (online `.backup`, then `PRAGMA integrity_check` must
say `ok`), `.env`, `backup.env`, a tar of the `droplet_caddy_data` volume
(certs + ACME account) and a `MANIFEST`, encrypts the tar to the age
recipient in `backup.env`, uploads it as `iron-fleet-<stamp>.tar.age` to
the R2 bucket, prunes objects older than `RETENTION_DAYS`, and keeps the
newest three bundles in `/var/backups/iron-fleet/` for a restore that
doesn't need the bucket.

Keys, and where they live:

| Thing | Where | Never |
|---|---|---|
| age identity (`AGE-SECRET-KEY-1…`) | Mac: `~/.config/iron-fleet/backup.key` (0600) + the Mac's secret store | on the droplet, in the repo, in a chat |
| age recipient (`age1…`) | `backup.env` on the droplet; public, harmless to expose | — |
| R2 token (scoped to the one bucket, Object Read & Write) | `backup.env` on the droplet; the Mac's secret store for restores | the repo, a chat |

Set-up, once (all of it is idempotent to re-run):

```sh
# Mac: the key pair. The recipient line is what goes in backup.env.
mkdir -p ~/.config/iron-fleet && age-keygen -o ~/.config/iron-fleet/backup.key && chmod 600 ~/.config/iron-fleet/backup.key
age-keygen -y ~/.config/iron-fleet/backup.key

# Cloudflare dashboard → R2: create bucket `iron-fleet-backups`; Manage R2 API
# Tokens → Object Read & Write, scoped to that bucket. Note the S3 endpoint
# (https://<account-id>.r2.cloudflarestorage.com) — no public access.

# Droplet: install tools + units, then fill backup.env and run once by hand.
ssh root@198.199.66.109 'bash -s' < deploy/droplet/bootstrap.sh
ssh root@198.199.66.109        # edit /opt/iron-fleet/deploy/droplet/backup.env
ssh root@198.199.66.109 'systemctl start iron-fleet-backup.service && journalctl -u iron-fleet-backup -n 3 --no-pager'
```

Checking on it:

```sh
ssh root@198.199.66.109 'systemctl list-timers iron-fleet-backup.timer --no-pager; journalctl -u iron-fleet-backup -n 5 --no-pager'
ssh root@198.199.66.109 'cd /opt/iron-fleet/deploy/droplet && set -a && . ./backup.env && set +a && rclone lsl r2:$R2_BUCKET'
```

### Restore

On the Mac (`brew install age rclone`; `sqlite3` ships with macOS), with
the R2 variables from `backup.env.example` in the environment or in a
`./backup.env` (`BACKUP_ENV=` to point elsewhere):

```sh
deploy/droplet/restore.sh latest        # or an object name, or a local .tar.age
```

It writes `restore/<stamp>/{control-plane.db,env,backup.env,caddy_data.tar,MANIFEST}`,
fails unless `integrity_check` is `ok`, prints the agents table, and ends
with the exact commands to serve the restored db locally and to re-seed a
droplet from it ("First deploy" above, with the restored `env` as `.env`
and `caddy_data.tar` untarred into `droplet_caddy_data` so the certs don't
have to be re-issued). Restoring onto the live box is always a manual,
deliberate step — the script never does it.

### Monitoring

UptimeRobot (free tier), two HTTPS monitors at the 5-minute interval, email
alerts to the account address:

- `https://fleet.opustower.dev/healthz`
- `https://mcp.opustower.dev/healthz`

Both return `200` with an empty-ish body; a `403` from the Mac's usual
network is the FortiGuard filter, not the droplet (Phase 3 note in
`docs/centralization-plan.md`). Nothing else watches the box — no
Prometheus, no log shipping; `journalctl -u iron-fleet-backup` and Caddy's
access logs are the whole observability story for two binaries on 1 GB.

### OS

`bootstrap.sh` owns the host config: `ufw` allows exactly 22/tcp, 80/tcp,
443/tcp, 443/udp; `unattended-upgrades` is on from the base image; a 2 GB
swapfile. Check with `ufw status verbose` and
`systemctl is-active unattended-upgrades`. Docker publishes only Caddy's
ports, so the services are unreachable except through Caddy.

### Rig link (Phase 6)

The control plane reaches the rig's Ollama over Tailscale and nothing
else: `bootstrap.sh` installs `tailscale` from its apt repo and runs
`tailscale up` (hostname `opustower`) once, printing a login URL — same
GitHub identity as the rig, so both land on one tailnet. `ufw` is
untouched; WireGuard is outbound UDP from this box. `INFERENCE_URL` in
`.env` is the rig's `100.x` address (`deploy/rig/setup.ps1` prints it);
the routes don't exist while it's unset.

Checks, on the box:

```sh
tailscale status                       # the rig ("opus") listed, not "offline"
# From inside the compose network — the control-plane image has no curl:
docker run --rm --network droplet_default curlimages/curl -s http://<rig tailnet IP>:11434/api/tags
# expect: {"models":[{"name":"qwen3:8b",...},{"name":"nomic-embed-text",...}]}
docker compose logs control-plane | grep -E "inference backend|INFERENCE_URL"
```

Containers on the default bridge reach tailnet addresses through the
host's routing table (`tailscale0`), so nothing in `docker-compose.yml`
changes; the `curlimages/curl` line is what proves it after a rebuild.

A `503 rig_offline` from `/inference/*` means the rig is off, Ollama is
stopped, or one side's Tailscale is down. That is the designed answer —
**do nothing on the droplet**. Everything else (`/agents`, `/sessions`,
`/usage`, jarvis) is unaffected, and the routes come back the moment the
rig does, with no restart here.
