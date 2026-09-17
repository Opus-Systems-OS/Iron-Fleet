# worker/sdk — the rig-gpu worker, on Anthropic's SDK

Stage 2 of the build order, done the way the self-hosted-sandboxes docs
say to: Anthropic's `EnvironmentWorker` (Python SDK ≥ 1.6.0) running in a
CUDA container on the rig. It polls the `rig-gpu` environment's work
queue, serves each session's tool calls locally, and posts results back.
The agent loop stays on Anthropic's side; this box only executes tools.

Why this and not `worker/src` (the Rust crate): the real protocol is not a
"claim → bundle of tool calls → post results" queue. It is a work-queue
*lease* plus a live attachment to the session's **event stream**, with
reconnect/reconcile, duplicate suppression, `always_ask` confirmation
gating, an idle watchdog, lease heartbeats with optimistic-concurrency
(`412` = lease lost), skills download, and memory-store sync. That is
~2 000 lines the SDK maintains and there is no Rust SDK. CLAUDE.md: if a
task starts to look like re-implementing a sandbox lifecycle, stop. So
we don't. The Rust crate stays in-tree for `exec.rs`/`gpu.rs` reference
until the fleet has run on this for a while; see `worker/README.md`.

## The protocol, confirmed 2026-09-17

Against the live API from the rig, no spend (fake key → `401
authentication_error`, i.e. routing and headers are right):

| Step | Call | Auth |
|---|---|---|
| Poll | `GET /v1/environments/{env}/work/poll?block_ms=999` (+ `Anthropic-Worker-ID`) → a `work` object or nothing | `Authorization: Bearer <environment key>` |
| Claim | `POST …/work/{work_id}/ack` — `queued` → `starting` | same |
| Lease | `POST …/work/{work_id}/heartbeat?expected_last_heartbeat=…` every `ttl/2` (≤ 30 s); `state: stopping` or `lease_extended: false` ends the run; `412` = someone else holds it | sessions token from the work item's `secret`, else the env key |
| Serve | `GET /v1/sessions/{id}/events` (SSE) + `GET …/events?limit=1000` to reconcile; run every `agent.tool_use` (`bash`, `read`, `write`, `edit`, `glob`, `grep`); `POST …/events` with `user.tool_result {tool_use_id, content, is_error}` | same |
| Done | `session.status_idle` with `stop_reason.type == end_turn` for `max_idle` (60 s), or `session.status_terminated` → `POST …/work/{work_id}/stop {force: true}` | same |

The work item: `{type: "work", id: "work_…", data: {type: "session", id:
"session_…"}, environment_id, state, secret, …}`. `secret` is a base64
JSON payload carrying a per-session `sessions_token`; the SDK prefers it
over the environment key for everything after the poll. Never log it.

The environment key is `sk-ant-oat01-…`, **generated in the Console**
(Workspace → Environments → rig-gpu → Generate environment key), shown
once. The environments API never returns one, so the control plane's
"printed once at create" path (`registry/sync.rs`) is dead code for this
purpose — it logs the warning instead. The key lives in `worker/sdk/.env`
on the rig and nowhere else.

## Files

| File | What |
|---|---|
| `Dockerfile` | `nvidia/cuda:12.6.0-runtime-ubuntu22.04` + python3.10 + `anthropic==1.6.0`, non-root `worker`, `/workspace` and `/mnt/memory` writable. |
| `worker.py` | The always-on entrypoint from the docs: `EnvironmentWorker.run()`, SIGTERM/SIGINT → task cancel so in-flight work is failed cleanly and the item force-stopped. Env-only config, listed in its docstring. |
| `docker-compose.yml` | `gpus: all`, `env_file: .env`, named `workspace` volume, `host.docker.internal` mapped, 40 s stop grace. |
| `env.example` | The two required values. Copy to `.env` (gitignored). |

## Run

On the rig, Docker Desktop running (WSL2 backend; the GPU passes through
with the host driver alone — no toolkit install on Windows):

```powershell
cd worker\sdk
copy env.example .env      # paste the Console key
docker compose up -d --build
docker compose logs -f     # "poller starting … idle; polling … for work"
```

Stop with `docker compose down` (sends SIGTERM, waits up to 40 s).

## Checks done 2026-09-17 (no key needed)

- Image builds on the rig; `anthropic 1.6.0`, `EnvironmentWorker` and
  `beta_agent_toolset_20260401` import; `/bin/bash` present; runs as
  `uid=1000(worker)`.
- `nvidia-smi` inside the container: `NVIDIA GeForce RTX 5070, 616.92,
  12227 MiB`.
- **Ollama from a session:** both `http://host.docker.internal:11434` and
  `http://100.79.233.8:11434` (the tailnet address) answer `/api/tags`
  with 200 from inside the container. Docker Desktop's host proxy
  connects to the host's loopback, so Ollama's `127.0.0.1` bind is fine
  and `gpu-compute`'s prompt (`host.docker.internal:11434`) is right.
  Closes the question left in `docs/centralization-plan.md`.
- With a fake key the worker starts, hits `GET …/work/poll`, gets `401
  authentication_error: OAuth access token is invalid`, exits. Routing,
  beta header (`managed-agents-2026-04-01`) and worker id header confirmed.

## First live run — done 2026-09-17

Key generated in the Console on rig-gpu, `docker compose up -d --build`,
`idle; polling` with `200` on `/work/poll`. Then, through the control
plane (`fleet.opustower.dev`):

- `POST /sessions` `{agent_slug: "gpu-compute", task: "Run nvidia-smi
  …"}` → claim → ack → stream → heartbeat → `executing tool tool=bash`
  → result posted, ~2 s from claim to tool. Agent answered with the
  5070's free VRAM. **Stage 2 exit.**
- `POST /sessions/{id}/events` asking it to `curl`
  `host.docker.internal:11434/api/chat` on `qwen3:8b` → 41.7 s tool
  step (17.8 s model load + 15.9 s prompt eval + 7.9 s generation),
  `"content":"391"`. **Phase 6 exit clause 2.**
- Session total 8 ¢, visible in `/usage`. After `end_turn` + 60 s the
  runner stopped the work item and went back to polling.
- Note: the work id is the session id (`work_id=sesn_…`).

Evidence and log excerpts in `docs/centralization-plan.md` "Stage 2".

## Not in scope

- Sandbox-per-session (`--on-work` + a fresh container per work item).
  One worker process, one `/workspace` volume, sessions serialised — fine
  for a single-user fleet; revisit if two `gpu-compute` sessions need
  isolation from each other.
- Memory stores are enabled by default (`WORKER_MEMORY_SYNC_SECONDS=15`);
  no agent attaches one yet.
- Skills: downloaded into `/workspace/skills/<name>/` automatically if
  the agent has any; `gpu-compute` has none.
