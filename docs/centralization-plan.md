# Centralizing on the droplet — plan (draft, no actions taken)

**Status:** planning only. Nothing in this document has been executed. Written
2026-09-13 for the next session to pick up from.

## Context

- Iron-Fleet's build order (stages 1–5, see `CLAUDE.md`) is complete and live:
  `control-plane` and `mcp-fleet` are deployed as separate Railway services in
  the `practical-compassion` project, wired to each other and to the real
  Managed Agents API, with all secrets rotated clean of this session's chat
  transcripts.
- The user mentioned a second machine: a DigitalOcean droplet at
  `198.199.66.109`, described as hosting something called **"Opus Tower OS."**
  Beyond the IP and that name, nothing about it is known to this repo or this
  session — not its OS, what's currently running on it, its resources, or how
  to reach it (no SSH key, user, or port confirmed here).
- The stated goal: "begin centralizing everything" once Iron-Fleet was done —
  i.e. some consolidation of infrastructure onto that droplet. What
  "everything" includes, and what "centralize" means operationally, is not
  yet defined. See Open Questions below before any of this becomes a real
  plan rather than a draft.

## Why this needs answers before it needs work

`CLAUDE.md`'s "Open decisions" section already records **"Control plane
host: Railway (decided 2026-09-13)"** as a settled architectural decision,
with reasoning (`control-plane` needs a public HTTPS endpoint, "so it cannot
live behind home NAT"). Moving `control-plane` (and/or `mcp-fleet`) to a
self-managed droplet **reverses that decision**, not extends it. That's a
legitimate thing to decide — a droplet has a public IP, so the NAT
constraint doesn't block it — but it should be a deliberate re-decision
recorded in `CLAUDE.md`, not something this plan backs into by default.
Nothing here should be executed until that's explicit.

## Open questions (answer these first, next session)

1. **What is Opus Tower OS?** Another project of the user's already running
   on this droplet? A base OS image/setup? Does Iron-Fleet need to coexist
   with it, integrate with it, or is the droplet effectively empty and
   "Opus Tower OS" is just what the box is called?
2. **What does "centralize everything" actually mean?** Candidate
   interpretations, not mutually exclusive:
   - Move `control-plane` and `mcp-fleet` off Railway onto the droplet
     (self-hosted, no more Railway billing/dependency for those two).
   - Use the droplet as `rig-gpu`'s self-hosted worker environment (unlikely
     — `worker/README.md` and `CLAUDE.md` tie `rig-gpu` to the RTX 5070 rig
     specifically, and a DigitalOcean droplet won't have that GPU).
   - Something broader: a single host fronting multiple unrelated projects
     (Opus Tower OS plus Iron-Fleet) behind one reverse proxy / one set of
     DNS records, for operational simplicity rather than a specific
     technical need.
   - Some combination the user has in mind that isn't captured here yet.
3. **Droplet access and state** — needed before touching it at all:
   - SSH access: key location, user (root vs. non-root), port.
   - OS and current installed software (Docker already present? a reverse
     proxy already running? existing services on 80/443 that would
     conflict?).
   - Resources (CPU/RAM/disk) — enough headroom for `control-plane` +
     `mcp-fleet` (both small Rust binaries, SQLite-backed, currently
     comfortable on Railway's smallest tier) alongside whatever Opus Tower
     OS already uses.
   - A domain, or is this staying on the droplet's bare IP? `control-plane`
     needs HTTPS (webhooks require it); `mcp-fleet` does too (Anthropic
     reaches it as a remote URL). Bare-IP HTTPS means self-signed or an
     IP-issued cert path, which is more friction than a domain + Let's
     Encrypt.
4. **Data migration.** `control-plane` has a persisted SQLite database on a
   Railway volume (`agents`, `session_usage`, `environments`,
   `mcp_fleet_vault` tables) — see `control-plane/README.md`. Moving hosts
   means either migrating that file or accepting a fresh, empty database
   (registry re-syncs from `agents/` automatically on boot; the
   `mcp_fleet_vault` row and `session_usage` history would not — the vault
   would reprovision fine per `mcp_fleet::ensure_vault`'s idempotency, but
   historical usage rollups would be lost unless the `.db` file itself is
   copied over).
5. **Cutover mechanics for two things that only work when addresses are
   right:**
   - The webhook endpoint is registered in the Anthropic Console against
     `https://iron-fleet-production.up.railway.app/webhooks/managed-agents`
     specifically (confirmed live and enabled this session). Moving
     `control-plane` means registering a *new* endpoint at the new URL
     (webhook endpoints aren't editable in place the same way secrets
     aren't — same one-way-rotation situation encountered this session) and
     only removing the old one once the new one's confirmed receiving
     deliveries.
   - `mcp-fleet`'s vault credential is keyed by its `mcp_server_url`
     (confirmed this session, `mcp_fleet.rs`). Moving `mcp-fleet` means a URL
     change, which `mcp_fleet::ensure_vault` already handles gracefully (it
     rotates in a new credential for the new URL automatically on
     `control-plane`'s next boot) — this part of the migration is actually
     already built for.
6. **What stays on Railway, if anything?** Partial centralization (e.g. only
   `mcp-fleet` moves, `control-plane` stays) is a valid outcome depending on
   the answer to question 2 — don't assume "everything" is literal.

## Draft approach (once the above is answered)

This is a starting sketch, not a committed sequence — expect it to change
once the open questions have real answers.

1. **Recon, read-only.** SSH in; inventory OS, installed software, open
   ports, existing services, and what "Opus Tower OS" actually is before
   changing anything.
2. **Decide and record.** Update `CLAUDE.md`'s "Open decisions" section with
   the actual decision (what moves, why, and the droplet's role), the same
   way the original Railway decision was recorded — this repo's own
   convention for infrastructure decisions.
3. **Stand up the target environment on the droplet**, most likely Docker
   containers built from the existing `control-plane/Dockerfile` and
   `mcp-fleet/Dockerfile` (both already written and proven — this session
   built and fixed them for Railway, but they're generic multi-stage builds,
   not Railway-specific) behind a reverse proxy (Caddy is the low-effort
   choice for automatic Let's Encrypt HTTPS) — assuming a domain exists per
   open question 3.
4. **Deploy alongside, not instead of, Railway initially.** Get the droplet
   copies healthy and passing the same checks this session used for Railway
   (`/healthz`, a real signed webhook, a real bearer-authenticated API call)
   before touching DNS, webhook registration, or `MCP_FLEET_URL`.
2. **Data:** copy the live `control-plane.db` file over (`railway volume`
   access or a `pg_dump`-style one-time export — SQLite is just a file, so a
   direct copy while briefly pausing writes is simplest) rather than starting
   fresh, to keep `session_usage` history.
5. **Cut over in the right order:** point `MCP_FLEET_URL` at the droplet's
   `mcp-fleet` first (low risk — `ensure_vault` handles it automatically, and
   nothing depends on the old URL except `mcp-fleet` itself), confirm it
   works, *then* register the new webhook endpoint and re-point Anthropic at
   the droplet's `control-plane`, confirm deliveries land, *then* stop the
   Railway services rather than deleting them immediately (a rollback path
   for the first while).
6. **Decommission Railway** once the droplet copies have run clean for a
   comfortable period, and remove the stale Railway config from
   `.railway/railway.ts` / the "Railway deployment" section of
   `control-plane/README.md`.

## Explicitly not part of this plan

- Nothing about `worker/` or `rig-gpu` — that stays tied to the physical RTX
  5070 rig per `CLAUDE.md`; the droplet has no GPU and isn't a candidate for
  it.
- Nothing about `app/` — it's a desktop client, not a hosted service; it has
  no relationship to where `control-plane`/`mcp-fleet` happen to run beyond
  pointing `CONTROL_PLANE_URL` at whichever URL is current.
- No secrets, keys, or droplet credentials belong in this file or anywhere
  else in this repo — same rule as everywhere else in `CLAUDE.md`'s
  Conventions section.
