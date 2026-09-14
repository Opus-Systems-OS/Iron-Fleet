# control-plane

The one service Iron-Fleet runs. Holds the agent registry, budget policy and
usage rollups in SQLite; everything about sessions is read live from the
Managed Agents API. Started life as stage 1 of the build order (start a
session from `curl`, receive the webhook back); now also carries the session
controls and Usage tab endpoints stage 4's dashboard calls.

## Endpoints

All routes except `/healthz` and `/webhooks/*` require
`Authorization: Bearer $CONTROL_PLANE_TOKEN`.

| Route | What it does |
|---|---|
| `GET /healthz` | Liveness. Railway's healthcheck. |
| `GET /agents` | The registry as synced: slug, Anthropic agent id/version, cap, effort, default environment. |
| `POST /sessions` | `{agent_slug, task, environment?, repositories?}` → creates a Managed Agents session pinned to the synced agent version, with that agent's `max_list_cost` cap and the task as the first `user.message`. `repositories` is extra `https://github.com/<owner>/<repo>` URLs to clone into the sandbox on top of the agent's registry defaults (see "Per-agent vaults and repository mounts"); `400` on an agent with no `github` block. Returns `201 {session_id, status, …, console_url}`. |
| `GET /sessions?agent_slug=&limit=&page=&order=` | Proxies `GET /v1/sessions`; the Anthropic envelope (`data`, `next_page`, `prev_page`) is returned unchanged except each item gains a `console_url`. |
| `GET /sessions/{id}` | Proxies `GET /v1/sessions/{id}`; the session object is returned unchanged except for an added `console_url`. |
| `POST /sessions/{id}/events` | `{task}` → appends one `user.message` to a running session. **Unconfirmed**: unlike the rest of this file, this endpoint path has no fixture from a live run yet — see `anthropic/mod.rs::send_events`'s doc comment. |
| `POST /sessions/{id}/interrupt` | Stops a session's in-flight work without ending it. Same unconfirmed-endpoint caveat as `/events`. |
| `GET /usage` | `{by_agent: [{agent_slug, session_count, total_list_cost_cents, budget_reached_count}], recent: [...session_usage rows]}`. Built entirely from the local `session_usage` rollup — no Anthropic call, so it's only as fresh as the last webhook delivery. |
| `POST /webhooks/managed-agents` | Anthropic → us. Verifies the Standard Webhooks HMAC, dedupes on event id, handles `session.status_idled` (INFO log) and `session.budget_reached` (WARN log), records a usage rollup. |

Errors are always `{"error": {"type": "...", "message": "..."}}`. Upstream
Anthropic errors come back as `502` (or `404`/`503` where that is what they
mean) with `upstream_status` and `request_id` for the support ticket.

## Configuration (environment only)

| Variable | Required | Notes |
|---|---|---|
| `ANTHROPIC_API_KEY` | yes | |
| `ANTHROPIC_WEBHOOK_SIGNING_KEY` | yes | The `whsec_…` value shown once when the endpoint is created in Console → Manage → Webhooks. |
| `CONTROL_PLANE_TOKEN` | yes | Bearer token for the control plane's own API. `openssl rand -hex 32`. |
| `MCP_FLEET_URL` | yes | `agents/jarvis.json` references `${MCP_FLEET_URL}` unconditionally (registry load fails without it) — any reachable-looking URL works if `mcp-fleet` isn't deployed yet, since Anthropic doesn't validate reachability at agent-create time. |
| `MCP_FLEET_TOKEN` | no | Independent of the URL. When also set, provisions (once) a vault + `static_bearer` credential authenticating `mcp-fleet` and attaches it to every session's `vault_ids`. Without it, `mcp-fleet` connections are attempted unauthenticated. See `mcp-fleet/README.md`. |
| `BLUEWEB_GITHUB_TOKEN` | yes* | Classic GitHub PAT (`repo`, `workflow`, `read:org`) for `blueweb-client`: becomes the sandbox's `GH_TOKEN` and the token that clones its repository mounts. *Required only because `agents/blueweb-client.json` names it — sync fails loud if a referenced `from_env` is unset. |
| `BLUEWEB_CLOUDFLARE_API_TOKEN` | yes* | Cloudflare API token (Pages: Read) for `blueweb-client`: the sandbox's `CLOUDFLARE_API_TOKEN`. Same rule. |
| `PORT` | no | Railway injects it. Default `8080`. |
| `DATABASE_PATH` | no | Default `$RAILWAY_VOLUME_MOUNT_PATH/control-plane.db`, else `./control-plane.db`. |
| `AGENTS_DIR` | no | Default `./agents`; `/app/agents` in the image. |
| `SYNC_ON_BOOT` | no | Default `true`. |
| `ANTHROPIC_WORKSPACE` | no | Default `default`. Only used to build the Console trace URL. |
| `ANTHROPIC_BASE_URL` | no | For pointing at a mock. |
| `RUST_LOG` | no | Default `info,tower_http=info`. |

## The registry: `agents/`

`agents/<slug>.json` is the committed, diffable fleet definition:

```jsonc
{
  "slug": "jarvis",
  "default_environment": "cloud-default",
  "policy": { "max_list_cost_cents": "50" },       // whole cents, as a string — a number is a load error
  "agent": { /* verbatim POST /v1/agents body; effort lives in agent.model */ }
}
```

`agents/environments/<slug>.json` holds `{ "slug", "environment": { verbatim POST /v1/environments body } }`.

A file may reference `${VAR_NAME}`, expanded from `control-plane`'s own
process environment before the JSON is parsed — how `agents/jarvis.json`
points at `mcp-fleet`'s URL without committing it. This runs on raw text, so
a substituted value can't itself contain a character that needs JSON
escaping (`"`, backslash, control characters) — fine for tokens and URLs,
not a general templating engine.

`agents/jarvis.json` declares `mcp-fleet` as an `mcp_servers` entry plus a
matching `tools[type=mcp_toolset]` entry — both are required together, or
Anthropic rejects the agent (see `mcp-fleet/README.md`'s "Wiring it to
jarvis" for the exact shape and why the first attempt at this was wrong).
Authentication is a separate, session-time concern handled by
`mcp_fleet::ensure_vault` (`src/mcp_fleet.rs`), not anything in this
registry file.

On boot (and on `control-plane sync`) the service reconciles this directory
with Anthropic: unknown agents are created, changed ones (content hash) are
updated into a new version, unchanged ones are left alone. Environments —
`cloud` and `self_hosted` alike — are created once. `POST /sessions` for an
agent whose environment has not been provisioned yet (i.e. the very first
sync since the environment file was added) returns
`409 environment_not_provisioned`.

### Custom skills: `agents/skills/<name>/`

A directory here is a custom skill — `SKILL.md` plus whatever scripts,
references and templates it ships — uploaded whole to the Skills API
(`POST /v1/skills`, GA, multipart, no beta header). The directory name must
equal the `name:` in `SKILL.md`'s frontmatter, because Anthropic makes that
slug immutable from the first upload. Sync reconciles skills before agents:
unknown → created, content hash changed → a new version (a full snapshot,
never a delta), unchanged → untouched; ids land in the `skills` table.

An agent attaches one with the registry's reference form, resolved at sync:

```jsonc
"skills": [{ "type": "custom", "skill": "blueweb-customer-site" }]
//  -> on the wire: { "type": "custom", "skill_id": "skill_…", "version": "<version id>" }
```

This is the one field in the otherwise-verbatim `agent` body that is not on
the wire (alongside `${VAR}` substitution). It pins the *version id*, not
`"latest"`, so a skill edit changes the agent's definition hash and rolls a
new agent version — sessions pinned to an agent version get the matching
skill snapshot. Pre-built Anthropic skills (`{"type":"anthropic","skill_id":"xlsx"}`)
and literal `skill_id` entries pass through untouched. Skills need the agent's
`read` tool (`agent_toolset_20260401` includes it). Nothing in a skill may
assume the author's `~/.claude/skills` path — it is mounted elsewhere in the
sandbox; the registry test checks for that string.

The skill's own instructions may need credentials the sandbox doesn't have
(`blueweb-customer-site` wants `gh` and `wrangler` logins). Those are a
session-time vault concern, like `mcp-fleet`'s bearer token, not something
the registry provisions.

Provisioning a `self_hosted` environment (`rig-gpu`) returns an
`environment_key` that the rig, not the control plane, needs — the rig owns
its key (CLAUDE.md). Sync surfaces it exactly once via `eprintln!` (never
`tracing`, so it can't land in an aggregated log sink) and never stores it;
see `worker/README.md` for what to do with it.

## Per-agent vaults and repository mounts

Two more registry-level blocks on `agents/<slug>.json`, both outside the
verbatim `agent` body (so neither touches the agent's definition hash):

```jsonc
"credentials": [
  { "type": "environment_variable", "secret_name": "GH_TOKEN",
    "from_env": "BLUEWEB_GITHUB_TOKEN",
    "allowed_hosts": ["api.github.com", "github.com", "uploads.github.com"] },
  { "type": "static_bearer", "mcp_server_url": "https://api.githubcopilot.com/mcp/",
    "from_env": "BLUEWEB_GITHUB_TOKEN" }
],
"github": { "token_env": "BLUEWEB_GITHUB_TOKEN",
            "mount": ["https://github.com/Opus1247/Iron-Fleet"] }
```

**`credentials`** → one vault per agent (`agent_vaults`), one credential per
entry (`agent_credentials`, keyed by `secret_name` or `mcp_server_url`),
attached only to *that agent's* sessions — unlike the mcp-fleet vault below,
which rides on every session. The secret is read from `from_env` at sync
time and sent straight to Anthropic; the registry never holds it, the logs
never print it (`CredentialAuth`'s `Debug` redacts), and SQLite keeps only a
SHA-256 over `(value, allowed_hosts)` so a rotated Railway variable becomes an
in-place credential update on the next sync. Two kinds:

- `environment_variable`: inside the sandbox the variable holds an opaque
  placeholder that Anthropic substitutes at egress, in request headers only,
  on the listed hosts only. Right for CLIs that send the token verbatim —
  `gh`, `wrangler`. **Not** for `git push`: GitHub's git endpoint accepts only
  HTTP Basic auth, which base64-encodes the token, so the placeholder never
  matches (verified 2026-09-14 — Bearer/`token` headers get 401 from GitHub
  even with a real token).
- `static_bearer`: a token for one of the agent's own `mcp_servers`, matched
  by exact URL at runtime; load fails if no server declares that URL. This is
  how `blueweb-client` pushes — through the GitHub MCP server's `push_files`
  / `create_pull_request`, which the skill's `references/preflight.md` walks
  through.

Cloud sandboxes only; `gpu-compute` cannot use these.

**`github`** → every session of the agent mounts the `mount` repos as
`github_repository` resources under `/workspace/<repo>`, authenticated with
the token in `token_env` (read per request, never stored — `agent_github`
keeps only the variable *name*). `POST /sessions … repositories: [...]` adds
per-session repos on top. The token must be a **classic** PAT: fine-grained
ones are scoped to one owner and this one has to reach both the fleet repo
and `BlueWeb-Org/*`. Mounting a repo also loads its root `.claude/skills/`.

If an `agent_credentials` row is lost while the credential still exists,
Anthropic returns 409 on the recreate (`secret_name` is unique per vault);
archive the stale credential in the Console, then sync again.

## The mcp-fleet vault

Separate from registry sync, and gated on `MCP_FLEET_URL`/`MCP_FLEET_TOKEN`
both being set: on every boot, `mcp_fleet::ensure_vault` provisions (once —
the `mcp_fleet_vault` table makes it idempotent across restarts) a vault and
a `static_bearer` credential keyed to `MCP_FLEET_URL`, then that vault's id
rides on `vault_ids` in every `POST /v1/sessions` request. Vault credentials
apply only to agents whose own `mcp_servers` references the matching URL, so
attaching it broadly is harmless for agents that don't use `mcp-fleet`. If
`MCP_FLEET_URL` ever changes, the old credential can't be edited in place
(its key is immutable) — `ensure_vault` logs a warning and keeps the old
vault rather than guess at fixing it; archiving the stale credential and
clearing the `mcp_fleet_vault` row is a manual step.

Two API facts that shape this:

- **Effort only takes effect on the agent.** A per-session `model` override
  silently drops it, so effort is in `agent.model.effort`, applied at sync,
  not at session create.
- **Budgets are create-only on the session** and denominated in whole US cents
  as a string. `Cents` in `src/money.rs` is the only type that can occupy that
  field and the only way to construct one is the validating string parser.

## Local run

```sh
export ANTHROPIC_API_KEY=sk-ant-…
export ANTHROPIC_WEBHOOK_SIGNING_KEY=whsec_…      # any valid whsec_ works locally
export CONTROL_PLANE_TOKEN=dev
export MCP_FLEET_URL=https://mcp-fleet.example.invalid/mcp   # jarvis.json needs this to load; see above
cargo run -p control-plane                          # boot sync, then listen on :8080

curl -H 'Authorization: Bearer dev' localhost:8080/agents
curl -H 'Authorization: Bearer dev' -H 'content-type: application/json' \
     -X POST localhost:8080/sessions \
     -d '{"agent_slug":"jarvis","task":"Say hello and stop."}'
curl -H 'Authorization: Bearer dev' localhost:8080/sessions/<session_id>
```

To exercise the webhook handler without a public URL, sign a body with the dev
helper (it uses `ANTHROPIC_WEBHOOK_SIGNING_KEY`):

```sh
BODY='{"type":"event","id":"whe_local_1","created_at":"2026-09-13T17:00:00Z","data":{"type":"session.status_idled","id":"<session_id>"}}'
H=$(printf '%s' "$BODY" | cargo run -q -p control-plane -- sign-webhook --id whe_local_1)
eval curl -i $H -H "'content-type: application/json'" -X POST localhost:8080/webhooks/managed-agents --data-binary "'$BODY'"
```

`control-plane/dev/mock-managed-agents.py` is a wire-shape mock of the
Managed Agents API (asserts the mandatory headers, echoes bodies) for running
the whole loop with no spend: `ANTHROPIC_BASE_URL=http://127.0.0.1:9999`.

## Railway deployment

Live at `https://iron-fleet-production.up.railway.app` — project
`practical-compassion`, service `Iron-Fleet`, environment `production`,
connected to this GitHub repo so a push to `main` deploys.

`.railway/railway.ts` is the project's Infrastructure-as-Code file, imported
from the live project with `railway config pull` and cleaned. `railway config
plan` should report no changes; review any diff before `railway config apply`.
It needs the authoring package: `cd .railway && npm install railway`
(`node_modules` is gitignored there).

What the service config amounts to, if it ever has to be recreated by hand:

```sh
railway link --project practical-compassion --environment production --service Iron-Fleet
railway environment edit --json <<'JSON'
{"services":{"<service-id>":{"build":{"builder":"DOCKERFILE","dockerfilePath":"control-plane/Dockerfile"},
  "deploy":{"healthcheckPath":"/healthz","healthcheckTimeout":120}}}}
JSON
railway volume add -m /data
railway variable set DATABASE_PATH=/data/control-plane.db SYNC_ON_BOOT=true RUST_LOG=info,tower_http=info --skip-deploys
railway variable set ANTHROPIC_API_KEY=sk-ant-... ANTHROPIC_WEBHOOK_SIGNING_KEY=whsec_... \
                     CONTROL_PLANE_TOKEN=$(openssl rand -hex 32) --skip-deploys
railway domain            # public HTTPS URL for the webhook
railway up --detach       # or push to main
```

Webhook registration is Console-only: **Manage → Webhooks** → add
`https://<domain>/webhooks/managed-agents`, subscribed to
`session.status_idled` and `session.budget_reached`. Copy the `whsec_…` shown
once into `ANTHROPIC_WEBHOOK_SIGNING_KEY`; the variable change redeploys.

The image runs as root: Railway volumes are root-owned and the platform's own
fix for non-root images is `RAILWAY_RUN_UID=0`. Volumes are single-replica; do
not scale this service horizontally (SQLite would not survive it anyway).

## Not in this service

Notification delivery (the webhook logs only), any UI, `mcp-fleet`. Session
state is never stored locally — the `session_usage` table is a cumulative
usage snapshot per session, upserted from webhooks, for the Usage tab to
aggregate later. The `rig-gpu` worker itself lives in `worker/` and does not
talk to this service — see its README.
