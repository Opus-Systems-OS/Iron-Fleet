# mcp-fleet

Stage 5, last in the build order — "once the surface it wraps has stopped
moving." Wraps `control-plane`'s API as an MCP server over the Streamable
HTTP transport (via [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk),
the official Rust SDK — no reason to hand-roll MCP's JSON-RPC framing), so it
can be attached to the `jarvis` agent as a remote `mcp_servers` entry.

## The five tools, and nothing else

CLAUDE.md is explicit about what jarvis may reach through MCP:

> Allowed: `list_agents`, `start_session`, `get_session_status`, `send_event`,
> `interrupt_session`. **Never expose:** agent creation, environment
> creation, budget mutation, or anything that raises a spending limit.
> Jarvis dispatches work; it does not define the fleet or change its own
> constraints.

`src/server.rs` implements exactly those five as MCP tools, each a thin
proxy to the matching `control-plane` route (`GET /agents`, `POST /sessions`,
`GET /sessions/{id}`, `POST /sessions/{id}/events`,
`POST /sessions/{id}/interrupt`). There is no generic passthrough and no
sixth tool. Adding one is a CLAUDE.md decision, not a code change to make
casually.

A tool call that fails (bad session id, control plane down, upstream 404...)
comes back as a normal MCP tool error (`is_error: true`, readable text) —
jarvis sees a message it can react to, not a crashed connection.

## Two credentials, two directions

- `CONTROL_PLANE_TOKEN` — this server's own credential *to* the control
  plane. Same token control-plane's other callers use; scoped by what
  routes this server happens to call (the five above), not narrowed further
  on that side.
- `MCP_FLEET_TOKEN` — the credential a caller (jarvis's Managed Agents
  session) presents *to this server*, checked in `main.rs`'s
  `require_bearer` the same constant-time way `control-plane` checks its
  own token. Deliberately separate: holding this token only proves "I can
  use the five jarvis tools," never "I can call the control plane directly."

## Wiring it to jarvis — and what's unconfirmed

`agents/jarvis.json` references the server via:

```json
"mcp_servers": [
  { "type": "url", "url": "${MCP_FLEET_URL}", "name": "fleet", "authorization_token": "${MCP_FLEET_TOKEN}" }
]
```

`${VAR}` is expanded from `control-plane`'s own environment at sync time
(`registry::substitute_env_vars`) so neither the URL nor the token is ever
committed — set `MCP_FLEET_URL` and `MCP_FLEET_TOKEN` wherever
`control-plane` runs, matching the value this server is booted with.

The `mcp_servers` object shape above mirrors Anthropic's public Messages API
MCP connector (`type: "url"`, `url`, `name`, `authorization_token`), the
closest confirmed real-world precedent — but whether Managed Agents accepts
exactly this shape for a *hosted* agent (as opposed to a single Messages API
call) has not been exercised against the live API. Same caveat as
`worker/`'s protocol and `control-plane`'s `/events` and `/interrupt`
routes: expect a follow-up fix once this runs for real.

## Local run

The five tools were exercised end to end (list, start, status, follow-up,
interrupt, plus a bad-id error case) with a throwaway `rmcp` client before
this shipped — not part of the crate, since it's a server, not a client, but
worth repeating if you touch `server.rs`:

```sh
# 1. mock Anthropic, 2. real control-plane against it — see control-plane/README.md
python3 control-plane/dev/mock-managed-agents.py 9999
ANTHROPIC_BASE_URL=http://127.0.0.1:9999 ANTHROPIC_API_KEY=test \
  ANTHROPIC_WEBHOOK_SIGNING_KEY=whsec_test CONTROL_PLANE_TOKEN=dev \
  MCP_FLEET_URL=http://127.0.0.1:8090/mcp MCP_FLEET_TOKEN=dev-mcp-token \
  cargo run -p control-plane -- serve
# 3. mcp-fleet itself
CONTROL_PLANE_URL=http://127.0.0.1:8080 CONTROL_PLANE_TOKEN=dev \
  MCP_FLEET_TOKEN=dev-mcp-token cargo run -p mcp-fleet

curl localhost:8090/healthz
# /mcp needs a real MCP client (Streamable HTTP) with
# Authorization: Bearer dev-mcp-token — curl alone can't drive the protocol.
```

## Configuration (environment only)

| Variable | Required | Notes |
|---|---|---|
| `CONTROL_PLANE_URL` | yes | e.g. `https://iron-fleet-production.up.railway.app`. |
| `CONTROL_PLANE_TOKEN` | yes | |
| `MCP_FLEET_TOKEN` | yes | Must match `agents/jarvis.json`'s `${MCP_FLEET_TOKEN}` substitution. |
| `PORT` | no | Default `8090`. |
| `RUST_LOG` | no | Default `info`. |

## Not here

Where this actually deploys is an open decision — CLAUDE.md's "Open
decisions" section only records `control-plane`'s Railway hosting so far.
It needs the same thing `control-plane` needed: a public HTTPS endpoint,
since Anthropic's Managed Agents session reaches it as a remote URL, not a
local child process. `Dockerfile` is ready; nothing has provisioned it yet.
