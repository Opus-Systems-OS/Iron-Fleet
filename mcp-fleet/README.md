# mcp-fleet

Stage 5, last in the build order — "once the surface it wraps has stopped
moving." Passed 2026-09-17: all five tools driven live through a jarvis
session that dispatched to `gpu-compute` on the rig (evidence in
`docs/centralization-plan.md` "Stage 5"). Wraps `control-plane`'s API as an MCP server over the Streamable
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
`GET /sessions/{id}` + `GET /sessions/{id}/events?order=desc&types=agent.message&limit=1`,
`POST /sessions/{id}/events`, `POST /sessions/{id}/interrupt`). There is
no generic passthrough and no sixth tool. Adding one is a CLAUDE.md
decision, not a code change to make casually — and
`server::tests::exactly_the_five_jarvis_tools` fails the build if the
router grows.

`get_session_status` is the one tool that shapes its answer rather than
passing the control-plane body through (`session_view` in `server.rs`):
status, `spent_cents` against `cap_cents`, timestamps, and `last_reply` —
the session's most recent `agent.message` — so jarvis can relay what a
session it started actually said. The raw session object was ~2 KB per
call, half of it the embedded agent definition (system prompt included)
plus `vault_ids`, none of which jarvis has a use for. Found on the stage 5
pass (2026-09-17): jarvis offered to "check back for the answer" and had
no way to.

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

## Wiring it to jarvis

The first attempt at this guessed a shape modeled on Anthropic's public
Messages API MCP connector — `authorization_token` inline on the
`mcp_servers` entry — and the live Managed Agents API rejected it outright:
`400 invalid_request_error: Failed to parse request body: unknown field
"authorization_token"`. The real shape, confirmed from
`platform.claude.com/docs/en/managed-agents/mcp-connector` and
`.../vaults`, splits declaration from authentication across two different
places:

**Agent creation** (`agents/jarvis.json`) only declares *which* server, by
name and URL — no credential:

```json
"tools": [
  { "type": "agent_toolset_20260401" },
  { "type": "mcp_toolset", "mcp_server_name": "fleet" }
],
"mcp_servers": [
  { "type": "url", "name": "fleet", "url": "${MCP_FLEET_URL}" }
]
```

Both the `mcp_servers` entry and the matching `mcp_toolset` tools entry are
required together — Anthropic rejects an agent with either one alone
("unreferenced servers or dangling toolsets").

**Session creation** supplies the credential, via a vault referenced by
`vault_ids` — never embedded in the agent definition. `control-plane`
handles this in `src/mcp_fleet.rs`: `ensure_vault` provisions (once, and
only when both `MCP_FLEET_URL` and `MCP_FLEET_TOKEN` are set) a vault and a
`static_bearer` credential keyed to `MCP_FLEET_URL`, and every
`POST /sessions` attaches that vault's id. Anthropic matches the credential
to the server by URL at runtime — the `mcp_server_name`/`name` fields never
enter into authentication at all.

`registry::substitute_env_vars` (`control-plane/src/registry/mod.rs`)
expands `${MCP_FLEET_URL}` in the committed `agents/jarvis.json`, so the
real URL is never committed either.

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
| `CONTROL_PLANE_URL` | yes | On the droplet, `http://control-plane:8080` over the compose network. |
| `CONTROL_PLANE_TOKEN` | yes | |
| `MCP_FLEET_TOKEN` | yes | Must match `agents/jarvis.json`'s `${MCP_FLEET_TOKEN}` substitution. |
| `PORT` | no | Default `8090`. |
| `ALLOWED_HOSTS` | yes, when deployed | Comma-separated `Host` header values `/mcp` accepts. rmcp's Streamable HTTP server has DNS-rebinding protection that defaults to `localhost,127.0.0.1,::1` and answers **403** to anything else — invisible in local testing, fatal behind a real hostname (found 2026-09-14 on the droplet: Anthropic's sessions reached `mcp.opustower.dev` and every call was rejected). Set to the public hostname, e.g. `mcp.opustower.dev`. Unset keeps the local default so `cargo run` works. |
| `RUST_LOG` | no | Default `info`. |

## Deployment

Live at `https://mcp.opustower.dev`, a container beside `control-plane` on
the `opustower.dev` droplet — see `deploy/droplet/README.md` for the compose
file, Caddyfile and `.env` names. `CONTROL_PLANE_TOKEN` and
`MCP_FLEET_TOKEN` match `control-plane`'s own values; `control-plane`'s
`MCP_FLEET_URL` is `https://mcp.opustower.dev/mcp`, which is the URL
`agents/jarvis.json` resolves to and the key `ensure_vault` stores the
credential under.
