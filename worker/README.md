# worker

Stage 2 of the build order: `rig-gpu` running a throwaway agent against the
RTX 5070. This binary is the self-hosted side of that environment — CLAUDE.md:
"Anthropic enqueues assigned sessions; the worker in `worker/` claims items,
spawns an execution context, runs tool calls, posts results back. The agent
loop stays on Anthropic's side — only tool execution is local."

It does **not** talk to `control-plane`. It talks directly to the Managed
Agents API, authenticated as the `rig-gpu` environment (not the account
`ANTHROPIC_API_KEY`), because the rig owns that key and the control plane
never sees it after the one-time provisioning message (see below).

## Status: superseded by `worker/sdk/` (2026-09-17)

The wire protocol was checked against the live API and the SDK's own
worker on 2026-09-17, and it is not what this crate assumed. There is no
claim-with-tool-calls / post-results queue: the work item is a **lease on
a session** (`GET …/work/poll` → `ack` → `heartbeat`), and the worker then
attaches to that session's **event stream**, answers each
`agent.tool_use` with a `user.tool_result` event, and force-`stop`s the
item when the session idles. Auth is `Authorization: Bearer <environment
key>` (Console-generated `sk-ant-oat01-…`, never returned by the API — the
"printed once at create" path below never fires), with a per-session
token unpacked from the work item's `secret` for the session calls.

That is the session-tool-runner + lease state machine Anthropic ships in
the Python/TypeScript/Go SDKs (`EnvironmentWorker`), and there is no Rust
SDK. Rather than re-implement it here (CLAUDE.md: don't rebuild a sandbox
lifecycle), the live path is **`worker/sdk/`** — the SDK worker on the
same CUDA image. Full protocol table, checks and the first-live-run
runbook are in `worker/sdk/README.md`.

This crate is kept as-is for now: `exec.rs` (per-session workdir, tool
timeout) and `gpu.rs` (nvidia-smi status) are still the reference for
what the rig does per tool call, and the mock in `dev/` still exercises
the crate's own loop. It is **not** built into the rig image and nothing
depends on it. Delete it once the SDK worker has served real sessions
for a while, or revive it only if a Rust-native worker becomes necessary
(it would need the event-stream runner ported, not just the routes
renamed). The rest of this file describes the crate as written.

Assumed protocol (wrong, kept for the record), all under the
`managed-agents-2026-04-01` beta header and an `x-environment-key` auth
header:

| Route | What it's for |
|---|---|
| `POST /v1/environments/{id}/claims` | Claim the next queued unit of work. `200` with a claim, or `204` if nothing is queued. |
| `POST /v1/environments/{id}/claims/{claim_id}/heartbeat` | Keep a claim's lease alive while a long tool call runs. |
| `POST /v1/environments/{id}/claims/{claim_id}/results` | Post tool results back: `{"results": [{"tool_use_id", "content", "is_error"}]}`. |
| `POST /v1/environments/{id}/claims/{claim_id}/release` | Give an unfinished claim back (e.g. on shutdown) instead of waiting for the lease to expire. |

Tool dispatch (`exec.rs`) implements `bash`/`shell` (`input.command`) and
`read_file`/`write_file` (`input.path`, `input.content`) against a per-session
working directory. Anything else in `agents/gpu-compute.json`'s
`agent_toolset_20260401` toolset comes back as a clearly-labeled
`is_error: true` result rather than silently doing nothing — same reasoning
as the protocol shapes above: better to fail loud and get patched on the
first real run than guess at a tool's input schema.

## Getting the environment key

**Corrected 2026-09-17:** the environments API does not return a key. The
key is generated in the Console (Workspace → Environments → rig-gpu →
Generate environment key), shown once, and goes into `worker/sdk/.env` on
the rig. `rig-gpu` is already provisioned as
`env_01Tz2CrQM3X4EWDVLGWLY6GH`. What follows is the mechanism as originally
written; the `eprintln!` branch never fires, the `tracing::warn!` one did.

`control-plane sync` (or boot, with `SYNC_ON_BOOT`) provisions
`agents/environments/rig-gpu.json` like any other environment. The first time
it creates a `self_hosted` environment, it prints the id and key **once**,
via `eprintln!` — never `tracing`, so it can't end up in an aggregated log
sink — and does not store the key anywhere:

```
=== new self_hosted environment: rig-gpu (env_...) ===
This key is shown once and is not stored anywhere. Copy it onto the rig now:

  RIG_ENVIRONMENT_ID=env_...
  RIG_ENVIRONMENT_KEY=...

It will not be printed again; if it's lost, provisioning must be redone.
```

Copy those two values into the rig's own secret store (its environment, a
local `.env` the process reads — never this repo).

## Configuration (environment only)

| Variable | Required | Notes |
|---|---|---|
| `RIG_ENVIRONMENT_ID` | yes | From the provisioning message above. |
| `RIG_ENVIRONMENT_KEY` | yes | Same. Never logged — `Config`'s `Debug` impl redacts it. |
| `ANTHROPIC_BASE_URL` | no | Default `https://api.anthropic.com`. For pointing at the mock. |
| `WORKER_POLL_SECONDS` | no | Default `5`. Sleep between empty claims. |
| `WORKER_HEARTBEAT_SECONDS` | no | Default `20`. |
| `WORKER_TOOL_TIMEOUT_SECONDS` | no | Default `900`. Per-tool-call kill timeout. |
| `WORKER_WORKDIR` | no | Default `./work`. One subdirectory per `session_id`, reused across claims for that session. |
| `RUST_LOG` | no | Default `info`. |

## Local run

Against the mock (no key, no GPU, no spend):

```sh
python3 worker/dev/mock-rig-gpu.py 9998
# in another shell
ANTHROPIC_BASE_URL=http://127.0.0.1:9998 \
  RIG_ENVIRONMENT_ID=env_mock RIG_ENVIRONMENT_KEY=test \
  cargo run -p worker -- claim-once
```

The mock pre-seeds one claim (`echo hello from rig-gpu`, then an
`nvidia-smi` probe that falls back to a message if there's no GPU) so
`claim-once` has something to run end to end: claim → execute in
`WORKER_WORKDIR/<session_id>` → submit results. `POST /seed` on the mock
queues another claim for a second run, or drop `claim-once` for the real
poll loop.

Against the real rig, once `RIG_ENVIRONMENT_ID`/`RIG_ENVIRONMENT_KEY` exist:

```sh
cargo run -p worker -- run
```

## Not in stage 2

Anything that looks like re-implementing the agent loop or a sandbox
lifecycle — CLAUDE.md is explicit that Claude Managed Agents owns both; if
this worker starts making decisions about *what* to run next rather than
just running what a claim says, stop and ask. Also not here: the CUDA
runtime image being exercised on real hardware (needs the rig), and
anything that reroutes queued `rig-gpu` work to the cloud when the rig is
off — CLAUDE.md says let it queue.
