# Centralizing on the droplet — plan

**Status (2026-09-14, late):** Phase 0 done. Phase 1 deployed and mid
cut-over — see "Resume here" just below. Phase 2+ not started.

## Resume here

Written for whichever machine picks this up next (the Mac session that did
the work ends here; its Claude memory does not travel). Everything below
is verified fact as of 2026-09-14 ~04:20 UTC, not plan.

**Live on the droplet** (`/opt/iron-fleet/deploy/droplet`, compose project
`droplet`, images from GHCR, both public):

- `https://fleet.opustower.dev` → control-plane, `https://mcp.opustower.dev`
  → mcp-fleet, Let's Encrypt certs, HTTP→HTTPS redirect. Both `/healthz` 200.
- SQLite seeded from the Railway volume before first boot; `GET /agents`
  returns the same four `agent_id`s as Railway; `/usage` history intact.
- Boot sync ran clean with credits: jarvis rolled to **v5** (its MCP URL is
  now `mcp.opustower.dev`), everything else unchanged, nothing created.
  `ensure_vault` added the new-URL credential to vault `vlt_011Cf2…`
  alongside Railway's, so Railway's jarvis still works.
- Smoke sessions run through the droplet: `sesn_01GqkKoUceeA9ArgNfycBttr`
  (4¢), `sesn_01EW9VRp6iPHackSvJWwTWJE` (6¢, called `list_agents` via
  `mcp.opustower.dev` — 200s in Caddy's `access-mcp.log`).
- Locally-signed webhook → 204, replay deduped, bad signature → 400.
- Bug found and fixed on the way (PR #4, `26d8b35`): rmcp 3.3 rejected every
  non-localhost `Host` with 403; `mcp-fleet` now takes `ALLOWED_HOSTS`.

**Railway is still running** and is still the registered webhook endpoint.
Anthropic's real deliveries go there, not to the droplet, until step (b).

**Next, in order** (the cut-over list in `deploy/droplet/README.md`):

- (b) **Console → Manage → Webhooks → Add endpoint**
  `https://fleet.opustower.dev/webhooks/managed-agents`, events
  `session.status_idled` + `session.budget_reached`. Put the new `whsec_`
  into the droplet's `.env` (`ANTHROPIC_WEBHOOK_SIGNING_KEY`), then
  `docker compose up -d control-plane`. Run any jarvis session to idle and
  confirm `session idled` appears in `docker compose logs control-plane`
  on the droplet and the row shows in `GET /usage`. Keep Railway's endpoint
  enabled until (d).
- (c) Desktop app → in-app connection form → `https://fleet.opustower.dev`,
  same `CONTROL_PLANE_TOKEN`. Fleet + Usage tabs render.
- (d) Console: **disable** (not delete) the Railway webhook endpoint.
  Railway dashboard: remove the active deployment of `Iron-Fleet` and
  `mcp-fleet` (keep services, volume, variables — rollback is a redeploy).
- Then mark Phase 1 complete here and strike the steps.

**A new machine needs:**

1. Its own SSH key on the droplet. Only the Mac's `id_ed25519` is in
   `root@198.199.66.109:~/.ssh/authorized_keys`. Generate one
   (`ssh-keygen -t ed25519`), then append its `.pub` from a machine that
   already has access (Mac Terminal.app:
   `ssh root@198.199.66.109 "echo '<pub line>' >> ~/.ssh/authorized_keys"`)
   or via the DigitalOcean web console.
2. `gh auth login` (PR merges) and `git`.
3. Nothing from Railway — the DB copy and `.env` are done. `railway` CLI
   only matters again for Phase 2 teardown.
4. No secrets: everything the droplet needs is already in its `.env`.
   Read variable *names* only, never `cat .env` into a chat.

**Day-to-day on the droplet:** `ssh root@198.199.66.109
/opt/iron-fleet/deploy/droplet/deploy.sh` after any merge to `main` that
rebuilt an image (`gh run list --workflow images.yml`).

## What exists today (do not rebuild)

Stages 1–5 of `CLAUDE.md`'s build order are complete and live:

| Thing | Where | State |
|---|---|---|
| `control-plane` | Railway, `practical-compassion` / `Iron-Fleet` | Live. Registry, `POST /sessions`, session proxy routes, `/events`, `/interrupt`, `/usage`, signed webhooks. SQLite on a `/data` volume. |
| `mcp-fleet` | Railway, same project, service `mcp-fleet` | Live. The five jarvis tools, nothing else. |
| `worker/` | The RTX 5070 rig, not the droplet | `rig-gpu` self-hosted environment. Out of scope here. |
| `app/` | Desktop (Tauri 2) | Fleet tab + Usage tab + session controls. **No voice loop yet.** |
| `agents/` | Committed JSON | Four agents, two environments, synced on boot. |

The only things Iron-Fleet runs itself are `control-plane` and `mcp-fleet`.
Sessions, their event history, and the agent loop live on Anthropic.

## The droplet

DigitalOcean nyc1, 1 vCPU / 961 MB, Ubuntu 24.04, `198.199.66.109`, named
"Opus Tower OS." Recon done 2026-09-14 — see `docs/droplet-inventory.md`.
Short version: a blank box. Root SSH on 22, nothing else listening, no
Docker, no MQTT broker (the outside proposal's claim was wrong), no swap,
firewall inactive. Domain: `opustower.dev` (Cloudflare DNS, records
DNS-only so Caddy does its own ACME).

## Decision this plan makes

**Move `control-plane` and `mcp-fleet` from Railway to the droplet.** This
reverses `CLAUDE.md`'s "Control plane host: Railway (decided 2026-09-13)".
The reasoning behind that decision — the control plane needs a public HTTPS
endpoint and can't sit behind home NAT — still holds and the droplet
satisfies it. What changes is *who runs the box*. Phase 0 records this in
`CLAUDE.md`'s "Open decisions" before any deploy happens.

The droplet does **not** become a second control plane, an agent host, or a
place where session state is stored. It is a host for the two binaries we
already have, plus whatever non-Iron-Fleet services (MQTT, later projects)
happen to share it.

## Target

```
Mac client ─┐
            ├─> droplet ──────────────────────────┐
Win client ─┘   Caddy (HTTPS, Let's Encrypt)      │
                 ├─ control-plane :8080  ─────────┼─> Managed Agents (Anthropic) ─┬─> cloud sandbox
                 └─ mcp-fleet     :8090  <────────┘   (jarvis sessions call back)  └─> rig-gpu worker (5070 rig)
```

Same diagram as `CLAUDE.md`, with "control plane (cloud)" now meaning the
droplet instead of Railway. Nothing else moves.

## Constraints that bind every phase

From `CLAUDE.md`, restated because the outside proposal violated each one:

1. **No session store.** Session state and history are Anthropic's. We keep
   the registry, budget policy, and usage rollups — that's it. No
   prompt/response history table, no Postgres "agent state."
2. **No agent loop, no sandbox lifecycle.** Agents are never "spawned" on the
   droplet. There are no "local agents." `POST /sessions` starts a Managed
   Agents session; that is the whole spawn story.
3. **Clients are thin.** The desktop app and the voice view render state
   and stream events. They hold nothing.
4. **Jarvis is a fleet member.** The voice loop is a view inside the Tauri
   app that sends events to a `jarvis` session through `control-plane`. It
   is not a separate app and it never calls Claude directly.
5. **No silent rerouting.** `rig-gpu` sessions queue when the rig is off.
   No "if busy, try elsewhere" logic anywhere.
6. **Budgets only go down from committed config.** Nothing on the droplet,
   and nothing reachable by jarvis, can raise a cap.
7. **Secrets stay out of the repo.** Droplet credentials, tokens, keys —
   `.env` on the box, never committed.

## Phases

Hard boundaries: a phase does not start until the one above it is verified.

### Phase 0 — Recon and record

Read-only on the droplet; one edit to this repo.

- ~~SSH in. Inventory.~~ Done 2026-09-14 → `docs/droplet-inventory.md`.
- ~~Confirm or obtain a domain.~~ Done 2026-09-14: `opustower.dev`.
  `fleet.opustower.dev` and `mcp.opustower.dev` each have an A record to
  `198.199.66.109` and an AAAA to `2604:a880:400:d1:0:4:f807:7001`,
  Cloudflare proxy **off** (grey cloud) so Caddy can complete HTTP-01 and
  hold the certificates itself. Bare-IP HTTPS was never an option for the
  Anthropic webhook endpoint or the MCP server URL.
- ~~Update `CLAUDE.md` "Open decisions".~~ Done 2026-09-14.

**Exit:** ~~inventory written~~, ~~domain resolving to the droplet~~,
~~`CLAUDE.md` updated~~ — all done 2026-09-14.

### Phase 1 — Rehost control-plane and mcp-fleet

Everything goes in `deploy/droplet/` in this repo:

- `bootstrap.sh` (run once on the box): 2 GB swapfile, Docker Engine +
  compose plugin from Docker's apt repo, `ufw allow 22,80,443` + enable.
- `docker-compose.yml`: `control-plane` (built from
  `control-plane/Dockerfile`), `mcp-fleet` (from `mcp-fleet/Dockerfile`),
  `caddy`. Named volume for `/data/control-plane.db`. `restart:
  unless-stopped`.
- `Caddyfile`: two sites, automatic TLS, reverse proxy to the two
  containers. Access logs on.
- `env.example` (no leading dot — `.gitignore` drops `.env.*`) listing
  every variable from the two READMEs. Real `.env` on the box only,
  `chmod 600`, filled by piping `railway variables --kv` over SSH so values
  never land in a chat or a Mac-side file.
- `deploy.sh`: `git pull && docker compose pull && docker compose up -d`.
  Manual deploy from the box is fine at this scale; GitHub Actions →
  SSH → deploy.sh is a later nicety, not a requirement.
- `README.md`: the runbook.
- Builds: a Rust release build on 1 vCPU / 961 MB is out. Images are
  built by GitHub Actions (`.github/workflows/images.yml`) on every push to
  `main` and pushed to `ghcr.io/opus1247/iron-fleet/{control-plane,mcp-fleet}`;
  the droplet is run-only (`docker compose pull`). Decided 2026-09-14 over
  building on the Mac — a Rust build under amd64 emulation on Apple Silicon
  is 20–40 min per deploy and ties deploys to the laptop.

Run **alongside** Railway, not instead of it:

1. **Seed the droplet's SQLite from the Railway volume first**
   (`railway volume files download`, checkpoint the WAL locally, drop the
   file into the `cp_data` volume before the first start). The earlier
   draft said "fresh, empty SQLite; sync recreates nothing" — wrong:
   `registry/sync.rs` consults only the local DB, so an empty one would
   create a second `rig-gpu` environment (new key, rig orphaned), collide
   on the immutable skill name, and duplicate every agent and vault. The
   copy also carries `session_usage` history for free.
2. Bring the three containers up. Boot sync should report everything
   unchanged except `jarvis` (one new version, its `${MCP_FLEET_URL}`
   changed) and `ensure_vault` adding a credential for the new MCP URL
   alongside Railway's — Railway's jarvis keeps working during overlap.
3. Same checks used for the Railway deploy: `/healthz` on both, a bearer
   `GET /agents` returning the same agent ids as Railway, a
   `POST /sessions` for `jarvis` with a trivial task, a locally-signed
   webhook to `/webhooks/managed-agents`.

Cut over in this order, each step confirmed before the next:

4. Set `MCP_FLEET_URL` on the droplet's control-plane to
   `https://mcp.opustower.dev/mcp`. `mcp_fleet::ensure_vault` rotates the vault
   credential for the new URL on boot (already built — see
   `control-plane/README.md`). Start a `jarvis` session and confirm a
   `list_agents` tool call round-trips.
5. Register `https://fleet.opustower.dev/webhooks/managed-agents` in Console →
   Webhooks (`session.status_idled`, `session.budget_reached`). Put the
   new `whsec_` in the droplet's `.env`. Run a session to idle and confirm
   the delivery lands on the droplet.
6. Point the desktop app at the new URL/token (in-app connection form).
7. Disable — don't delete — the Railway webhook endpoint and stop the
   Railway services. Rollback path stays for a week or two.

**Exit:** all traffic (app, Anthropic webhooks, jarvis MCP callbacks) hits
the droplet; Railway stopped.

### Phase 2 — Decommission Railway

- After the soak period: delete the Railway services and volume.
- Remove `.railway/`, the "Railway deployment" section of
  `control-plane/README.md`, the Railway paragraph in `mcp-fleet/README.md`.
  Replace with pointers to `deploy/droplet/`.
- `PORT`/`DATABASE_PATH` defaults that mention Railway env vars in
  `control-plane` config: leave the code, update the comments.

**Exit:** no Railway references outside git history.

### Phase 3 — Session event streaming

`CLAUDE.md` says clients "stream SSE," but no such route exists yet. This is
the piece that both the voice view and live dashboard updates need, and it
belongs in `control-plane`, not in a new service.

- `GET /sessions/{id}/stream`: proxy the Managed Agents session event
  stream to the client as SSE, bearer-authenticated like every other route.
  Verify the exact upstream endpoint and event shapes against the Managed
  Agents docs before writing it; use the SDK if it exposes streaming so
  the beta header is never hand-rolled.
- Desktop app: replace polling in the Fleet tab with the stream for the
  selected session.

This replaces the proposal's "WS /events" and "prompt streaming" —
same outcome, delivered by proxying what Anthropic already emits rather
than by inventing an event model.

**Exit:** app shows a session's agent messages appearing live.

### Phase 4 — Jarvis voice view

The stage `app/README.md` calls "Not yet." Builds on Phase 3.

- Orb view in the Tauri app. macOS: speech-to-text and text-to-speech via
  platform APIs, gated in `src-tauri`. Windows: text input, same view.
- Loop: transcript → `POST /sessions` (first turn) or
  `POST /sessions/{id}/events` (follow-ups) on a `jarvis` session →
  Phase 3 stream → speak the agent's text output.
- Spend appears in the Usage tab automatically because it's a normal
  session with `jarvis`'s `"50"` cap.
- Jarvis dispatching work (start a `blueweb-client` session, check on it)
  already works through `mcp-fleet`; nothing new on that side.

**Exit:** speak to the Mac, hear a `jarvis` session answer, see its cost
in Usage.

### Phase 5 — Usage, audit, and ops

What "history and audit" means under constraint 1:

- Usage: `GET /usage` already rolls up per agent. Add `?since=` / `?until=`
  windowing and a CSV export of `session_usage` rows. That is the audit
  trail we own; the transcript of any session is one click away via the
  existing Console link.
- Backups: nightly `sqlite3 .backup` of `control-plane.db` to DO Spaces
  (or Litestream if continuous replication is wanted). Backup the `.env`
  and Caddy state to the same place, encrypted.
- Monitoring: an external uptime check on both `/healthz` URLs (DO's
  monitoring or any free pinger) and Caddy access logs. No Prometheus, no
  ELK — not on 1 GB, and not worth it for two binaries.
- OS: `unattended-upgrades` (already on), `ufw` allowing 22/80/443 only.

**Exit:** a restore from backup has been tested once.

## Explicitly not in this plan

- A new API service (FastAPI/Express), a Postgres database, or any
  "central hub" other than `control-plane`. The proposal's Opus API
  endpoint list maps onto existing routes: `/agents` → `GET /agents`,
  `/prompts` → `POST /sessions` + `/events`, `/agents/{id}/tasks` →
  `GET /sessions?agent_slug=`, `/stats` → `GET /usage`, `/dashboard` and
  `WS /events` → the Fleet tab + Phase 3 stream. `POST /agents/spawn` and
  `DELETE /agents/{id}` have no counterpart on purpose.
- "Local agents" on the droplet. There are none. `rig-gpu` is the 5070 rig.
- A standalone voice app. The voice loop is a view in `app/`.
- Local ↔ cloud failover.
- mTLS between services. Bearer tokens over Caddy-terminated TLS is the
  current model and sufficient for two services on one box.
- `worker/` and `rig-gpu`. Unchanged.
- MQTT/IoT. There is no broker on the box. If one is ever wanted it's a
  separate service that nothing in Iron-Fleet talks to.
- BlueWeb customer sites. Separate concern, separate hosting.

## What the outside proposal got wrong

Recorded so the next reader doesn't relitigate it:

| Proposal | Reality |
|---|---|
| "Opus API" central hub, FastAPI + Postgres | `control-plane` exists, in Rust + SQLite, and is live. Rehost it. |
| "Agent state in PostgreSQL" | Sessions live on Anthropic. We store registry + usage rollups only. |
| "Spawn/shutdown agents from desktop" | Agents are committed JSON synced on boot. Sessions are started, not agents. |
| "Jarvis voice app modified to hit Opus API instead of Claude directly" | Voice loop doesn't exist yet; when built it's a view in `app/` talking to `control-plane`. It was never going to call Claude directly. |
| "Local agents (Opus)" on the droplet | No such thing. The only local execution is `rig-gpu` on the 5070. |
| "Failover: local busy → try Railway" | Forbidden by `CLAUDE.md`. |
| "Prompt/response history DB, conversation replay" | That's the Managed Agents session log. Link to Console; don't copy it. |
| "Railway agents authenticate with Opus API" | Railway hosts nothing after Phase 2. |
| ELK, Prometheus, horizontal scaling | 1 vCPU / 1 GB, single-replica SQLite. Uptime pings and access logs. |
| Week-numbered timeline | Hard phase boundaries with exit criteria instead; see `iron-fleet-working-style`. |

## Repo layout after this plan

```
deploy/droplet/     compose, Caddyfile, .env.example, deploy.sh
docs/               this plan, droplet-inventory.md
```

Same repo. A separate repo for "Opus Tower OS" only makes sense once the
droplet hosts something that isn't Iron-Fleet; at that point the host-level
config (Caddy, backups) would move out and this repo would keep
only its own compose fragment.
