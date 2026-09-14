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

## Status: wire protocol unconfirmed

`control-plane/src/anthropic` has fixtures captured from a real run (its
commit history includes "fix three mismatches found on the first live
Managed Agents run"). `worker/src/protocol.rs` has no equivalent — nothing
here has been run against the live API yet. The endpoint paths and field
names in `protocol.rs` and `client.rs` are a best-effort match to the REST
conventions the rest of Managed Agents already uses (`/v1/<resource>`,
`{"type": "...", ...}` error bodies), not a confirmed spec. Expect a
follow-up fix commit the same way stage 1 needed one, once this runs against
a real `rig-gpu` claim.

Assumed protocol, all under the `managed-agents-2026-04-01` beta header and
an `x-environment-key` auth header:

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

`control-plane sync` (or boot, with `SYNC_ON_BOOT`) now provisions
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
