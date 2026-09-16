<!-- Approved implementation plan for Phase 6 of docs/centralization-plan.md.
     Written on the Mac 2026-09-16 and committed so the Claude Code session on
     the rig can execute it. When the phase is done, fold the outcome into
     centralization-plan.md "Resume here" and delete this file. -->

# Phase 6 — Local inference on the rig (Ollama)

## Context

Phases 0–5 are done. Phase 6 (`docs/centralization-plan.md` §Phase 6) puts
Ollama on the Windows rig's RTX 5070 behind the control plane, so anything
holding a bearer token can get a local chat completion or embedding without
knowing where the rig is — and a `gpu-compute` session can hit it directly
as a tool step. It is **not** an agent loop, has **no** failover in either
direction (rig off → `503`, full stop), is never exposed to jarvis via
`mcp-fleet`, and never touches the droplet's RAM or the public internet.

Decisions taken 2026-09-16 (to be recorded in `CLAUDE.md` "Open decisions"):

| Decision | Choice |
|---|---|
| Private link | **Tailscale** on the droplet host and the rig. The rig keeps Ollama on `127.0.0.1` and publishes it to the tailnet with `tailscale serve --tcp=11434`; control-plane proxies to `http://<rig tailnet IP>:11434`. Nothing opened at home. |
| Models | `qwen3:8b` (~5 GB Q4) + `nomic-embed-text` (~270 MB), in `deploy/rig/models.txt`. |
| GPU sharing | `OLLAMA_MAX_LOADED_MODELS=1`, `OLLAMA_KEEP_ALIVE=5m`; revisit only on a real session OOM. |
| App | Fleet tab: one "Rig" line — online + model tags, or offline; hidden when the deployment has no `INFERENCE_URL`. No chat UI. |
| Rig hands | A Claude Code session on the rig follows `deploy/rig/README.md`; I write it from the Mac. |

Rig today: Ollama installed, no Tailscale, **the worker has never run live**
(`worker/README.md` still says "wire protocol unconfirmed"). That makes the
plan's middle exit clause — a `gpu-compute` session calling the local model
in a tool step — depend on Stage 2's first live run, which is its own
weekend. **This plan delivers everything else and leaves that clause
explicitly open**; the agent prompt change (build step 4) still ships so
the session has the address the day the worker runs.

## Part A — control plane (`control-plane/`)

### `src/config.rs`
- `inference_url: Option<String>` from `INFERENCE_URL` (trailing `/`
  trimmed like `anthropic_base_url`). Unset = no inference routes at all.

### `src/inference.rs` (new)
Small client, same shape as `anthropic/mod.rs` but talking Ollama's native
API (not its OpenAI layer — one shape to maintain):

- `pub struct Inference { base_url: String, http: reqwest::Client }` —
  `connect_timeout(5s)`, `read_timeout(180s)` (a cold model load emits
  nothing for tens of seconds), **no total timeout** (chat streams).
- `models()` → `GET /api/tags`, 10 s total timeout via `.timeout()` on the
  request, returns the JSON as-is.
- `chat(body: Value)` → `POST /api/chat`, returns the `reqwest::Response`
  so the handler can stream it.
- `embeddings(body: Value)` → `POST /api/embed`, buffered JSON.
- `fn classify(e: reqwest::Error) -> Error`: connect/timeout/request
  errors → `Error::RigOffline(msg)`; anything else `UpstreamTransport`.
- Non-2xx from Ollama → `Error::Inference { status, message }` where
  `message` is Ollama's `{"error": "..."}` string (e.g. `model 'x' not
  found`).

### `src/error.rs`
- `RigOffline(String)` → **503**, kind `rig_offline`, message names the
  URL host and the cause. This is the whole "no fallback" story: a 503
  is the answer, nothing retries elsewhere.
- `Inference { status: u16, message: String }` → Ollama 4xx passes through
  with the same status (bad model, bad body); 5xx → 502. Kind `inference`.

### `src/http/inference.rs` (new)
Routes are registered in `http/mod.rs` **only when `state.inference` is
`Some`** — otherwise axum's default 404, so a deployment without a rig is
byte-for-byte unchanged. All three sit inside the bearer-protected group,
with `DefaultBodyLimit::max(1 MiB)` layered on them.

- `GET /inference/models` → Ollama's `/api/tags` JSON unchanged.
- `POST /inference/chat` → body must be a JSON object with string `model`
  and array `messages` (`InvalidRequest` otherwise); `stream` defaults to
  Ollama's `true`. Streaming: `Body::from_stream(resp.bytes_stream())`
  with `content-type: application/x-ndjson` and the same
  `x-accel-buffering: no` as `sessions::stream`. Non-streaming: buffered
  JSON. Everything else in the body (`options`, `keep_alive`, `format`,
  `tools`) passes through untouched.
- `POST /inference/embeddings` → same validation (`model`, `input`), proxies
  `/api/embed`, buffered.
- Unit tests: body validation; `classify` mapping for a connect-refused
  error against a closed local port; error → status mapping.

### `src/main.rs` / `http/mod.rs`
- `AppState.inference: Arc<Option<Inference>>`; log at boot
  `inference backend configured url=…` or `no INFERENCE_URL — /inference/* disabled`.

### `README.md`
Endpoint table rows for the three routes, the 503/404 semantics, and a
curl example with `"stream": false`.

## Part B — rig runbook (`deploy/rig/`, new)

Written for the Claude Code session on the Windows box. Same shape as
`deploy/droplet/README.md`: a file table, the ordered first-run steps,
checks, and a "record these back" list.

| File | What |
|---|---|
| `README.md` | Runbook: prerequisites (Ollama ≥ 0.6 for Blackwell/CUDA 12.8 — check `ollama --version`, and `ollama ps` shows `100% GPU` after a first prompt), Tailscale install + `tailscale up`, run `setup.ps1`, checks, and what to paste back to the Mac side (tailnet IP, `/api/tags` output). |
| `models.txt` | `qwen3:8b`, `nomic-embed-text` — one tag per line, comments allowed. The committed, diffable model list. |
| `setup.ps1` | Idempotent, run as the logged-in user: set user env vars `OLLAMA_HOST=127.0.0.1:11434`, `OLLAMA_KEEP_ALIVE=5m`, `OLLAMA_MAX_LOADED_MODELS=1`; restart Ollama so they take; `ollama pull` each tag in `models.txt` not already in `ollama list`; `tailscale serve --bg --tcp=11434 tcp://127.0.0.1:11434`; print `tailscale ip -4` and `tailscale serve status`. |
| `check.ps1` | `ollama list`, `ollama ps`, `nvidia-smi --query-gpu=memory.used,memory.total --format=csv`, `Invoke-RestMethod http://127.0.0.1:11434/api/tags`, `tailscale status`. The rig session runs this and pastes the output. |

Ollama on Windows already starts at login as the tray app; the runbook
keeps that (the rig auto-logs-in for the worker anyway) rather than adding
NSSM. If the rig session finds it doesn't, the README's fallback is a
Task Scheduler "At log on" task running `ollama serve`.

## Part C — droplet side (`deploy/droplet/`)

- `bootstrap.sh`: new idempotent section — install Tailscale from its apt
  repo (`https://tailscale.com/install.sh` pinned as the documented
  command, not curl-piped inside the script), `tailscale up` **only if
  not already logged in**, printing the auth URL for the user to click.
  `ufw` unchanged: Tailscale's WireGuard traffic is outbound UDP; nothing
  new inbound. Also `sysctl` nothing — no subnet routing.
- `env.example`: `INFERENCE_URL=http://<rig tailnet IP>:11434` with the
  comment that unset disables the routes.
- `README.md` "Ops": a "Rig link" subsection — how to check the tailnet
  (`tailscale status`), the container-reachability check
  (`docker run --rm --network droplet_default curlimages/curl -s http://<rig>:11434/api/tags`
  — the control-plane image has no curl), and what a 503 from
  `/inference/*` means (rig off or Tailscale down; do nothing else).

Docker bridge containers reach tailnet IPs through the host's routing
table, so no compose change is needed; the reachability check above is
what proves it on the first run.

## Part D — agent + app

- `agents/gpu-compute.json` `system`: add a sentence — Ollama listens on
  `http://127.0.0.1:11434` on the rig host (`http://host.docker.internal:11434`
  from a container), with `qwen3:8b` and `nomic-embed-text` pulled; use its
  `/api/chat` and `/api/embed` from tool steps, do not pull other models,
  check `ollama ps` before assuming the GPU is free (it shares the 12 GB).
  Boot sync rolls `gpu-compute` to the next version, as usual.
- App (`app/src-tauri/src/commands.rs`, `app/src/main.ts`, `index.html`):
  Tauri command `get_inference_models` → `GET /inference/models`; it maps
  404 → `{configured: false}`, 503 → `{configured: true, online: false, reason}`,
  200 → `{configured: true, online: true, models: [names]}` so the
  frontend never parses error strings. Fleet tab, above the agents
  table: `Rig · online · qwen3:8b, nomic-embed-text` / `Rig · offline` /
  nothing. Fetched **outside** the `Promise.all` in `refresh()` so a 5 s
  connect timeout on the rig never delays agents and sessions.
  Windows build is untouched (no platform code).

## Docs

- `CLAUDE.md` "Open decisions": the four Phase 6 decisions, one bullet.
- `docs/centralization-plan.md`: "Resume here" Phase 6 block with the
  tailnet IPs, the exit evidence, and the **open clause** (gpu-compute tool
  step — waits on the worker's first live run); Phase 6 section gets a
  "Met" line for the parts that are.
- `worker/README.md`: no change — its "unconfirmed" status is still true.

## Order of work

1. Part A + Part C files + Part D + docs → one PR, `cargo test`, local
   `serve` against a throwaway local Ollama? — **no**: the Mac has no
   Ollama and shouldn't need one; unit-test the client against a closed
   port (→ `RigOffline`) and a tiny axum stub that answers `/api/tags`
   and streams two NDJSON lines for `/api/chat`.
2. Merge, `deploy.sh` (no `INFERENCE_URL` yet → routes 404, unchanged
   behaviour; verify).
3. Rig session runs `deploy/rig/README.md`; pastes back tailnet IP + tags.
4. Droplet: `bootstrap.sh` (Tailscale), you click the auth URL, set
   `INFERENCE_URL` in `.env`, `deploy.sh`.
5. Exit checks below; docs PR with the evidence.

## Verification

- `cargo test -p control-plane` — new tests green; existing 53 untouched.
- Deployed **without** `INFERENCE_URL`: `GET /inference/models` → 404,
  `/agents` and `/usage` unchanged.
- Tailnet: `tailscale status` on the droplet lists the rig; the
  `curlimages/curl` check returns the two model tags from inside the
  compose network.
- **Exit 1:** from the droplet (the Mac's network FortiGuard-blocks
  `opustower.dev`; hotspot or run the curl on the box):
  `POST https://fleet.opustower.dev/inference/chat` with `qwen3:8b`,
  `"stream": false` → a completion; `ollama ps` on the rig shows the model
  loaded 100% GPU at that moment; `nvidia-smi` memory jumps. Then a
  streaming call shows NDJSON frames arriving incrementally through Caddy.
  `POST /inference/embeddings` with `nomic-embed-text` → a 768-float vector.
- **Exit 3:** rig session runs `tailscale down` (or stops Ollama):
  `/inference/chat` → `503 rig_offline` within ~5 s; `/agents`, `/sessions`,
  `/usage`, a jarvis session — all unchanged. `tailscale up` → works again
  with no restart on the droplet.
- **Exit 2 (open):** `gpu-compute` session calls Ollama in a tool step —
  recorded as pending the worker's first live run, not claimed.
- App: Mac rebuild, Fleet tab shows `Rig · online · qwen3:8b, nomic-embed-text`;
  with the rig down, `Rig · offline` and the tables still refresh on time.
