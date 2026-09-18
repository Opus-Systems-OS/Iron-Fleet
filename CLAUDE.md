# Iron-Fleet

Fleet management for a small set of Claude agents. Ships as a desktop app
(J.A.R.V.I.S.) for macOS Tahoe / Apple Silicon and Windows 11.

Built on **Claude Managed Agents** — Anthropic's hosted agent runtime. We do not
build an agent loop, a sandbox lifecycle, or session persistence. If a task in
this repo starts to look like re-implementing one of those, stop and ask.

## Core architecture

```
Mac client ─┐
            ├─> control plane (cloud) ─> Managed Agents (Anthropic) ─┬─> cloud sandbox
Win client ─┘                                                        └─> rig-gpu worker
```

- **Sessions live on Anthropic's infrastructure.** Session state, event history,
  and the agent loop persist server-side. Neither desktop machine hosts
  anything. This is what makes cross-device sync free — do not build a sync
  layer.
- **Clients are thin and interchangeable.** They render state and stream SSE.
  They hold no fleet state. A session started on the Windows rig must be fully
  viewable and steerable from the MacBook, and vice versa.
- **The control plane is the only service we run.** It holds the agent registry,
  budget policy, trigger config, and usage rollups. Everything else it reads
  live from the Managed Agents API. It needs a public HTTPS endpoint for
  webhooks, so it cannot live behind home NAT.

## Environments

Two, chosen per session (not baked into the agent):

| Environment | Type | Use |
|---|---|---|
| `cloud-default` | Anthropic cloud sandbox | Everything by default |
| `rig-gpu` | `self_hosted` | CUDA work only, RTX 5070 |

`rig-gpu` is a work queue. Anthropic enqueues assigned sessions; the worker in
`worker/` claims items, spawns an execution context, runs tool calls, posts
results back. The agent loop stays on Anthropic's side — only tool execution is
local. When the rig is off, GPU sessions queue rather than fail. Do not add
fallback logic that silently reroutes them to the cloud.

## Agents and budgets

`max_list_cost` is a hard ceiling priced at public list rates, set at session
create. Denominated in **whole US cents as a string** — `"500"` is $5.00.
Decimal forms are rejected. The in-flight request when the cap trips still
finishes, so final cost lands slightly over.

| Agent | Per-session cap | Effort | Notes |
|---|---|---|---|
| `jarvis` | `"50"` | low | Short, high-frequency; must never surprise |
| `blueweb-client` | `"1000"` | high | Billable client code work |
| `blueweb-ops` | `"200"` | medium | Contracts, admin; no code tools |
| `gpu-compute` | `"500"` | medium | Runs on `rig-gpu` |

Per-session caps do not stop fifty sessions. Workspace- and agent-level caps
with alerts are configured in the Console as a backstop, not in this repo.

## Jarvis

The voice assistant is a **fleet member, not a wrapper**. The orb UI and speech
I/O are a view inside the Tauri app. The voice loop does **not** call the Claude
API directly — it sends events to a `jarvis` Managed Agents session through the
control plane, like every other agent, so its spend appears in the Usage tab.

`mcp-fleet/` exposes the control plane to the `jarvis` agent as MCP tools:

- Allowed: `list_agents`, `start_session`, `get_session_status`, `send_event`,
  `interrupt_session`
- **Never expose:** agent creation, environment creation, budget mutation, or
  anything that raises a spending limit. Jarvis dispatches work; it does not
  define the fleet or change its own constraints.

macOS speech I/O is platform-gated in `src-tauri`. Windows is text-only for now.

## Layout

```
control-plane/   Rust, hosted. Registry, webhooks, budget policy, usage rollups
app/             Tauri 2. src-tauri/ (Rust) + src/ (frontend)
worker/          rig-gpu worker: sdk/ (Anthropic's EnvironmentWorker on the CUDA
                 image — the live path) + src/ (Rust crate, superseded, reference)
mcp-fleet/       MCP server wrapping the control plane for the jarvis agent
agents/          Agent, environment and skill definitions as versioned config
```

`agents/` is committed config, not Console clicks. The fleet must be
reproducible and diffable.

## Conventions

- Rust workspace at the root; `control-plane`, `worker`, `mcp-fleet`, and
  `app/src-tauri` are members.
- Tauri 2, system webview. Not Electron — the MacBook Air's battery is a real
  constraint.
- Managed Agents API requests need the `managed-agents-2026-04-01` beta header,
  except memory store endpoints, which use `agent-memory-2026-07-22`. The SDK
  sets these automatically; hand-rolled requests must not forget them.
- Agent first, then session. The session's `agent` field is a pinned
  `{type: "agent", id, version}` — or, only when a client adds session-local
  custom tools or a system suffix, `{type: "agent_with_overrides", …}` built
  by the control plane on the agent's *live* definition (an override
  replaces the field in full). Overrides never touch the agent resource;
  `agents/` remains the only place the fleet is defined.
- Secrets live in the host's secret store and the rig's environment key stays on
  the rig. Never commit an API key, environment key, or GitHub token.

## Build order

Do not start a stage before the one above it works end to end.

1. `control-plane/` — start a session from `curl`, receive a webhook back. No UI.
2. `worker/` — `rig-gpu` running a throwaway agent against the 5070. Most likely
   to eat a weekend; do it before any UI depends on it. **Live 2026-09-17**
   (`worker/sdk/` on the rig; first session served, 8 ¢). Runbook and
   evidence in `worker/sdk/README.md`.
3. `app/` — Tauri shell, Fleet Dashboard, read-only.
4. Session controls, then the Usage tab.
5. `mcp-fleet/` — last, once the surface it wraps has stopped moving.
   **Passed 2026-09-17**: all five tools driven live by a jarvis session
   dispatching to the rig; `get_session_status` returns a compact view
   with `last_reply`.

## Open decisions

- **Control plane host: the `opustower.dev` droplet** (decided 2026-09-14,
  reversing the 2026-09-13 Railway decision; plan, phases and **current
  progress — start at its "Resume here" section** — in
  `docs/centralization-plan.md`, box inventory in `docs/droplet-inventory.md`).
  DigitalOcean nyc1, `198.199.66.109` / `2604:a880:400:d1:0:4:f807:7001`.
  `fleet.opustower.dev` → `control-plane`, `mcp.opustower.dev` → `mcp-fleet`,
  both behind Caddy-terminated TLS; deploy config lives in `deploy/droplet/`.
  The droplet hosts the two binaries we already have and nothing else — no
  session store, no agents, no second control plane. Cut over from Railway
  and Railway deleted 2026-09-15; the droplet is the only deployment.
  Droplet secrets live in `.env` on the box (`chmod 600`), never in this
  repo.
- **Local inference (Phase 6, decided 2026-09-16, live, closed 2026-09-17):** Ollama on the
  rig, reached from the droplet over **Tailscale** (rig keeps Ollama on
  `127.0.0.1`, publishes it with `tailscale serve --tcp=11434`; control-plane
  proxies to `INFERENCE_URL`, unset = routes 404). Tailnet: rig `opus` =
  `100.79.233.8`, droplet `opustower` = `100.108.133.31`, so
  `INFERENCE_URL=http://100.79.233.8:11434`. Models `qwen3:8b` +
  `nomic-embed-text` in `deploy/rig/models.txt`;
  `OLLAMA_MAX_LOADED_MODELS=1`, `OLLAMA_KEEP_ALIVE=5m`. App shows only a
  rig online/offline line with model tags. Rig off → `503`, never Claude.
  Runbook `deploy/rig/README.md`; outcome and evidence in
  `docs/centralization-plan.md` "Resume here".
- Multi-agent orchestration, memory stores, and outcomes are beta on the
  Managed Agents side. Keep the control plane able to fall back to direct
  Messages API calls for anything where beta instability would actually hurt.
