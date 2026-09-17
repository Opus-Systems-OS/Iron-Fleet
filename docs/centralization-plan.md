# Centralizing on the droplet — plan

**Status (2026-09-17):** Phases 0–6 done — the droplet is the only
deployment, and **Phase 6 is closed**: all three exit clauses met. Build
order **stage 2 (the worker) is live**: `worker/sdk/` on the rig served
its first real `gpu-compute` session (nvidia-smi, then Ollama from a tool
step), 8 ¢ total, in the Usage rollup. Next real work: stage 5,
`mcp-fleet`. Remaining cosmetic: the Mac app rebuild.

## Resume here

Written for whichever machine picks this up next (Claude memory does not
travel between machines; this section does). Everything below is verified
fact as of 2026-09-15 ~05:20 UTC, not plan. Phases 0–5 are complete.

**Mac app repointed** — done 2026-09-15 ~05:25 UTC on the Mac. `url` in
`~/Library/Application Support/com.ironfleet.app/control-plane.json`
changed to `https://fleet.opustower.dev`, token untouched (the Railway
copy sits beside it as `control-plane.json.railway.bak`; delete when
convenient). The installed `/Applications/J.A.R.V.I.S..app` relaunched:
badge reads `https://fleet.opustower.dev`, Fleet tab shows four agents
with `jarvis` at v5 and the droplet's sessions; `GET /usage` with the
app's token returns the droplet rollups with
`sesn_01MQMjRaHFhwvhefFsfpVemh` (jarvis, 5¢) in `recent`. Both desktop
clients now point at the droplet; nothing references Railway.

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

**Railway is stopped, not deleted** (2026-09-15 ~04:50 UTC, from the
Windows rig): `railway down` on `Iron-Fleet` and `mcp-fleet` — both
Railway URLs now 404; services, `iron-fleet-volume` (41 MB, `/data`) and
variables kept, so rollback is `railway up --detach`. The Railway webhook
endpoint is **disabled** in the Console; only the droplet's is enabled.

- (a) ~~Jarvis MCP round-trip~~ — done, `sesn_01EW9VRp6iPHackSvJWwTWJE`.
- (b) ~~Droplet webhook endpoint~~ — done 2026-09-15 04:35 UTC. Endpoint
  registered, new `whsec_` in the droplet `.env`, control-plane restarted;
  Anthropic's real `session.status_idled` for `sesn_01MANKa4WeWib5ECPymUS9Bs`
  arrived from 160.79.106.132 → 204, signature verified, usage row written.

The cut-over list from `deploy/droplet/README.md`, all done:

- (c) ~~Desktop app → `https://fleet.opustower.dev`~~ — done 2026-09-15
  ~04:45 UTC from the Windows rig. Same token, only the URL in
  `%APPDATA%\com.ironfleet.app\control-plane.json` changed; app rebuilt at
  `9c1b39b`. Fleet tab: four agents (jarvis v5), sessions list. Usage tab:
  by-agent spend plus recent activity including step (b)'s
  `sesn_01MANKa4WeWib5ECPymUS9Bs` row — i.e. the droplet's rollups, not
  Railway's. Mac repointed 2026-09-15 ~05:25 UTC (see top of this
  section).
- (d) ~~Disable the Railway webhook endpoint; remove the Railway
  deployments~~ — done 2026-09-15 ~04:50 UTC, see above. Verified after:
  `sesn_01MQMjRaHFhwvhefFsfpVemh` (jarvis, 5¢) started through the droplet,
  its `session.status_idled` landed on the droplet at 04:54:53 UTC and
  wrote the usage row — with Railway serving nothing, the only place it
  could go.

**Phase 2 done 2026-09-15 ~05:15 UTC**, a week ahead of the planned soak
by the user's call: repo cleanup in `68f7ad2`, then `railway delete` of
the whole `practical-compassion` project (it held only the two services
and the volume). No rollback path to Railway exists now; the droplet's
`control-plane.db` was a superset of the Railway one, so nothing was lost.

**Phase 5 done 2026-09-16 ~04:55 UTC** (PR #9 `5e73072` + follow-up
PR #10). control-plane: `GET /usage?since=&until=` (half-open on
`observed_at`, RFC 3339 or `YYYY-MM-DD`) and `GET /usage/export.csv`, the
rollup audit trail; deployed via `deploy.sh`, boot sync unchanged
(4 agents, 1 skill, 3 credentials). Ops on the droplet:

- **Backups:** `iron-fleet-backup.timer` → `backup.sh` daily 07:00 UTC
  (`Persistent=true`, +0–10 min jitter): online `sqlite3 .backup` +
  `integrity_check`, `.env`, `backup.env`, Caddy cert/ACME state →
  `age` → Cloudflare R2 bucket **`iron-fleet-backups`** (token scoped to
  it, in `backup.env` on the box), 30-day retention, newest 3 kept in
  `/var/backups/iron-fleet/`. First object `iron-fleet-20260916T0447Z.tar.age`
  (112 KB). Tools from Ubuntu apt: sqlite3 3.45.1, age 1.1.1, rclone
  1.60.1 — that rclone needs `--s3-no-head` against R2 (PR #10) or it logs
  a false `NotImplemented` and retries.
- **Keys:** age identity on the Mac at `~/.config/iron-fleet/backup.key`
  (0600; recipient `age1dqjde94au3zs3xl98hlleskl424mh3qzry3zq2qkes0qvg26hgmsssjkq7`),
  with a fallback copy in the user's **iCloud Keychain** (added
  2026-09-16). Not on the droplet, not in the repo. The Mac has no R2 token yet —
  `restore.sh latest` there needs one in `./backup.env` or the environment
  (`deploy/droplet/README.md` "Restore"); until then fetch the object on
  the droplet and `restore.sh <file>`.
- **Exit test (restore tested once), 2026-09-16 ~04:52 UTC:** the R2
  object pulled down, decrypted on the Mac with `restore.sh`
  (`integrity_check ok`, 4 agents at the live versions, 14 usage rows,
  ACME state in `caddy_data.tar`), served locally with
  `SYNC_ON_BOOT=false`: `GET /agents` matched the droplet's ids/versions
  and `GET /usage/export.csv` was byte-identical to the live one
  (md5 `a884554877bcc8f20607cda5859bf613`).
- **Monitoring:** UptimeRobot, two HTTPS monitors on the `/healthz` URLs,
  5-min interval, email alerts (set up by the user 2026-09-16).
- **OS:** verified `ufw` = 22/tcp 80/tcp 443/tcp 443/udp only,
  `unattended-upgrades` active, 19 GB free.

**Phase 4 done 2026-09-16 ~04:00 UTC** (PR #7 `ab6263a`, app-only — no
droplet change). Third tab "Jarvis": hold the orb (or Space), talk,
release; the first utterance is `POST /sessions` for `jarvis`, later ones
`POST /sessions/{id}/events`; the reply comes back on the Phase 3 feed
(second watch slot `"voice"`) and is spoken by the webview's
`speechSynthesis`. Speech input is native macOS (`SFSpeechRecognizer` +
`AVAudioEngine` tap via objc2, one owning thread, `app/src-tauri/src/speech/`),
with `Info.plist` usage strings; other platforms get the text box only.
Exit test: `sesn_013qfALTWMA9L54uURbtKWRf` ("Hey Jarvis", 5¢) started from
the orb on the Mac, answer heard, row in Usage.

Gotcha found on the way: on the Mac the repo sat under `~/Documents`,
which is iCloud Drive with "Optimize Mac Storage" on a disk down to 5 GB
free — iCloud evicted build fingerprints, `node_modules`, `.git` objects
and freshly edited source files, and every read of an evicted file blocked
(cargo "hangs"; a release build that takes 8 min took 30+). The Mac
checkout moved to `~/code/Iron-Fleet` (a fresh clone; the old tree is
disposable) and builds go to `target.nosync/` (`.cargo/config.toml`;
iCloud skips `.nosync`). **Free disk space on the Mac before the next
big build** — the pressure is what triggers eviction.

**Phase 3 done 2026-09-16 ~00:12 UTC** (PR #5 `f2442f9`, deployed via
`deploy.sh`; panel placement follow-up PR #6). control-plane gained
`GET /sessions/{id}/events` (history) and `GET /sessions/{id}/stream` (SSE
proxy, byte-for-byte), and `interrupt` now sends a `user.interrupt`
event — the `/v1/sessions/{id}/interrupt` path it used to POST to does
not exist, so Interrupt in the app and mcp-fleet's `interrupt_session`
had been 404ing. The app's Fleet tab got a selected-session transcript
fed by a Rust watcher (`app/src-tauri/src/stream.rs`) that does the
docs' open-stream → list-history → dedupe-on-id dance.

Exit test on `sesn_01822Qk8zFZQf36NrRb5fDbE` (jarvis, started from the
Mac app, $0.11 of $0.50): the transcript panel showed the turns live;
with the stream open through Caddy, a follow-up posted at 00:03:44 UTC
produced `running → user.message → agent.message → session.usage →
status_idle` frames by 00:03:46; an interrupt posted 3s into a
"count to 2000" turn showed `user.interrupt → thread_status_idle →
session.usage → status_idle (end_turn)` one second later with no
`agent.message`. Stream wire format seen live: a `: connected` comment
first, then `event: message` + `data: {event}` frames.

Gotcha found on the way: the Mac's usual network runs a FortiGuard web
filter that returns 403 for `opustower.dev` ("Unrated"). It is not the
droplet — check for the FortiGuard block page before debugging Caddy.
The Mac must be on another network (hotspot) to use the app.

**Phase 6 started 2026-09-16 ~05:15 UTC — decisions made, nothing built.**
The four "decide first" items are settled (Tailscale link; `qwen3:8b` +
`nomic-embed-text`; `OLLAMA_MAX_LOADED_MODELS=1`/`KEEP_ALIVE=5m`; app gets
only a rig online/offline line) and recorded in `CLAUDE.md` "Open
decisions". The approved implementation plan was `docs/phase-6-plan.md`,
deleted once done — everything it specified is recorded below. Rig facts as of the
start: Ollama installed (version unchecked), no Tailscale, the worker has
never run live — so the exit clause "a `gpu-compute` session calls the
local model in a tool step" is explicitly left open until Stage 2's first
live run; everything else in the phase is in scope.

Work moved from the Mac to the rig at the user's request. Mac-side
leftovers that don't block: the Mac has `age`/`rclone` installed and the
age identity at `~/.config/iron-fleet/backup.key`; the Mac has no R2 token.

**Phase 6 build done on the rig 2026-09-16** (branch `phase-6-inference`,
plan steps 1 and 3 of the implementation plan's "Order of work"):

- **Part A** (control-plane): `INFERENCE_URL` → `src/inference.rs`
  (Ollama native API client, 5 s connect / 180 s read / no total timeout),
  `src/http/inference.rs` (`GET /inference/models`, `POST /inference/chat`
  streaming NDJSON or buffered on `"stream": false`, `POST
  /inference/embeddings`; 1 MiB body cap), registered **only** when the URL
  is set. `Error::RigOffline` → `503 rig_offline`, `Error::Inference` →
  Ollama's 4xx passthrough / 5xx → 502. 7 new tests (closed port →
  `RigOffline`; an axum stub for tags / two-frame chat stream / embed /
  `model 'x' not found`), 60 total green.
- **Part B** (`deploy/rig/`): `README.md`, `models.txt`, `setup.ps1`,
  `check.ps1`. Scripts are pure ASCII on purpose — Windows PowerShell 5.1
  reads BOM-less UTF-8 as ANSI and an em dash's trailing byte closes a
  string. Native exes go through an `Invoke-Native` helper because 5.1
  turns redirected stderr into a terminating error under `'Stop'`.
- **Part C** (`deploy/droplet/`): `bootstrap.sh` Tailscale section (apt
  repo, `tailscale up --hostname=opustower`, login URL printed, blocks up
  to 10 min), `env.example` `INFERENCE_URL` (commented), README "Rig link".
- **Part D**: `agents/gpu-compute.json` `system` names Ollama's address
  and the two models (boot sync will roll `gpu-compute` to its next
  version on deploy); app `get_inference_models` command maps 404 / 503 /
  200 to `{configured, online, models, reason}`; Fleet tab `#rig-status`
  line above the agents table, polled outside `refresh()`'s `Promise.all`.
  `tsc` and `cargo check -p app` clean (the one warning, `PARTIAL_EVENT`,
  predates this).

**Rig, verified live 2026-09-16 (this box, Windows 11, hostname `opus`):**
Ollama **0.34.0** (Blackwell-capable), Tailscale **1.102.4** logged in as
`Opus1247@` (GitHub identity), tailnet IPv4 **`100.79.233.8`**, so
`INFERENCE_URL=http://100.79.233.8:11434`. `setup.ps1` set the three
`OLLAMA_*` user env vars (all were unset), restarted the tray app (back
on `127.0.0.1:11434` in seconds), and pulled the models — see the
`check.ps1` output recorded below. Ollama autostarts from
`Ollama.lnk` in the user's Startup folder, not a service. The rig's
`~/.ssh/id_ed25519.pub` is **not** on the droplet yet — step 4 needs it
added from the Mac or the DO console before this box can run
`bootstrap.sh` / `deploy.sh`.

Rig evidence, 2026-09-16 (`setup.ps1` then a first prompt, then
`check.ps1`):

- `ollama pull`: `qwen3:8b` 5.2 GB (`500a1f067a9f`), `nomic-embed-text`
  274 MB (`0a109f422b47`), ~12 min at ~7–11 MB/s.
- `tailscale serve status`: `tcp://100.79.233.8:11434` (and
  `opus.taile70900.ts.net`, IPv6) `--> tcp://127.0.0.1:11434`, tailnet
  only. `GET http://100.79.233.8:11434/api/tags` from the rig itself
  returns both tags.
- First `/api/chat` on `qwen3:8b` (`think: false`, `stream: false`):
  "Hello! How can I assist you today?" — 48.1 s total of which 22.2 s
  model load; `ollama ps` at that moment `5.6 GB 100% GPU`, `nvidia-smi`
  6918 / 12227 MiB.
- `/api/embed` on `nomic-embed-text` → a 768-float vector. Right after,
  `ollama ps` showed **only** `nomic-embed-text` (323 MB, 100% GPU) —
  `OLLAMA_MAX_LOADED_MODELS=1` evicted the chat model as intended;
  `nvidia-smi` back to 1904 MiB.
- User env: `OLLAMA_HOST=127.0.0.1:11434`, `OLLAMA_KEEP_ALIVE=5m`,
  `OLLAMA_MAX_LOADED_MODELS=1`.

**Droplet side done 2026-09-16 ~07:00 UTC** (PR #12 merged as `958e0d3`,
image run `35065724825`; all commands run from the rig over SSH with the
rig's dedicated `~/.ssh/droplet` key, `Host droplet` in its ssh config):

- `bootstrap.sh` (piped through `tr -d '
'` — the Windows checkout is
  CRLF): Tailscale 1.102.4 installed from the apt repo, `tailscale up`
  authorised as `Opus1247`; droplet is **`opustower` = `100.108.133.31`**.
  First attempt hit the `unattended-upgrades` dpkg lock; re-run after it
  freed. `tailscale status` on the droplet lists `opus 100.79.233.8`.
- Reachability from inside the compose network:
  `docker run --rm --network droplet_default curlimages/curl -s
  http://100.79.233.8:11434/api/tags` → both models. No compose change.
- `deploy.sh` **without** `INFERENCE_URL`: boot sync `agents_updated: 1`
  (`gpu-compute` → **v2**), log `no INFERENCE_URL — /inference/* disabled`;
  `GET /inference/models` → **404**, `/agents` and `/usage` → 200.
- `INFERENCE_URL=http://100.79.233.8:11434` appended to `.env`,
  `docker compose up -d control-plane`: log `inference backend
  configured url="http://100.79.233.8:11434"`.
- **Exit 1 met**, via `https://fleet.opustower.dev` from the droplet:
  `/inference/models` → the two tags; `/inference/chat` `stream:false`
  on `qwen3:8b` → a completion in 2.5 s round trip (rig `ollama ps`
  `100% GPU`, `nvidia-smi` 6948 MiB); streaming → HTTP/2 200,
  `content-type: application/x-ndjson`, `x-accel-buffering: no`, 16
  frames arriving incrementally (+0.27 s … +0.42 s); `/inference/embeddings`
  on `nomic-embed-text` → 768 floats; unknown model → `404 inference`
  with `upstream_status: 404`; missing `messages` → `400 invalid_request`.
- **Exit 3 met**: rig `tailscale down` → `/inference/chat` **503
  `rig_offline`** in **5.07 s** (the connect timeout), `Retry-After: 5`;
  `/agents`, `/sessions`, `/usage`, `/healthz` all 200 throughout. Rig
  `tailscale up` → next call answered in 3.4 s, control-plane container
  never restarted. Note: with only `tailscale serve --tcp=11434 off` (rig
  still on the tailnet) the 503 takes ~31 s — tailscaled accepts the TCP
  connection into its netstack and stalls, so hyper reports `SendRequest`
  rather than a connect failure. Real rig-off is the 5 s path.

**Left for the next session** (in this order):

1. ~~`RigOffline` message~~ — done, PR #13. `classify` walks the whole
   `source()` chain, so the 503 body carries the leaf
   (`… : tcp connect error: connection refused`) after the host instead
   of stopping at hyper's `client error (Connect)`. The offline test
   asserts the leaf is present ("refused" on both Linux and Windows);
   `cargo test -p control-plane` 60 passed. Merged as `9437cbf` and
   deployed with `deploy.sh` 2026-09-16 (control-plane container label
   `revision=9437cbf…`). Verified: rig `tailscale down` →
   `GET /inference/models` via `fleet.opustower.dev` → **503** in 5.09 s,
   `Retry-After: 5`, body `rig offline: 100.79.233.8:11434: client error
   (Connect): tcp connect error: deadline has elapsed` (the connect
   timeout is the real rig-off leaf; `connection refused` is the
   localhost case the test covers). `tailscale up` → 200 in 0.39 s.
2. ~~App (Windows)~~ — done on the rig 2026-09-16 ~17:25 local. Rebuilt
   with `npm run tauri build -- --no-bundle`; note the binary lands in
   **`target.nosync/release/app.exe`** (the committed `.cargo/config.toml`
   sets `target-dir` for the Mac's iCloud problem — it applies on Windows
   too; a stale `target/release/app.exe` from 2026-09-14 is not the
   build). Launched: Fleet tab shows
   `Rig · online · nomic-embed-text:latest, qwen3:8b` above the agents
   table. With `tailscale down` on the rig it flips to
   `Rig · offline · 503 rig_offline` and the agents/sessions tables keep
   their 5 s cadence throughout (the `updated` clock advanced during the
   outage — `refreshRig` really is outside `refresh()`'s `Promise.all`);
   `tailscale up` → back to online, no app restart. **The Mac rebuild is
   still to do** — free disk space first, see the Phase 4 note.
3. ~~`.gitignore`, delete `docs/phase-6-plan.md`, update `CLAUDE.md`~~ —
   done. `CLAUDE.md` "Open decisions" now carries both tailnet addresses
   and points at `deploy/rig/README.md` plus this section.

**Open, by design:** exit clause 2 (a `gpu-compute` session calling
Ollama in a tool step) waits on the worker's first live run. ~~One thing
to confirm then: `host.docker.internal:11434` vs Ollama's `127.0.0.1`
bind~~ — **confirmed fine 2026-09-17**: from inside a container on the
rig both `http://host.docker.internal:11434/api/tags` and
`http://100.79.233.8:11434/api/tags` return 200 (Docker Desktop's host
proxy connects to the host loopback). No change to Ollama's bind or the
agent prompt needed.

### Stage 2: the worker (prepared 2026-09-17, rig, no spend)

The protocol `worker/src` assumed was wrong, and not by a route rename.
Checked against the live API docs, the Python SDK's own
`EnvironmentWorker` source, and a fake-key run from the rig:

- A work item is a **lease on a session** — `GET
  /v1/environments/{env}/work/poll` → `POST …/work/{id}/ack` → heartbeat
  every ttl/2 with optimistic concurrency (`412` = lease lost) → `POST
  …/work/{id}/stop {force:true}` when done. No tool calls in it.
- The tool calls come from the **session's event stream**: attach to
  `GET /v1/sessions/{id}/events` (SSE), reconcile against the list
  endpoint, run each `agent.tool_use` (`bash`/`read`/`write`/`edit`/
  `glob`/`grep`), post `user.tool_result` events, stop after
  `session.status_idle` `end_turn` + 60 s idle.
- Auth is `Authorization: Bearer <environment key>`; the key is
  `sk-ant-oat01-…`, **generated in the Console only**. The environments
  API never returns one, so the control plane's "printed once" path is
  dead; its warn line now says where to get the key.

That is the SDK's session-tool-runner + lease machine (Python/TS/Go), no
Rust SDK exists, and CLAUDE.md says don't rebuild a sandbox lifecycle.
So the live path is **`worker/sdk/`** — Anthropic's `EnvironmentWorker`
in a `nvidia/cuda:12.6.0-runtime` container on the rig (Dockerfile,
`worker.py`, compose, `env.example`, README with the protocol table and
runbook). The Rust crate stays in-tree, unbuilt, as reference; delete
after the SDK worker has served real sessions for a while.

Verified on the rig 2026-09-17 without a key: image builds; `anthropic
1.6.0` imports the worker; `nvidia-smi` in the container sees the
`RTX 5070, 616.92, 12227 MiB`; runs as uid 1000; with a fake key the
worker hits `GET …/work/poll` and fails with `401 authentication_error:
OAuth access token is invalid` — routing and headers right. Environment
`rig-gpu` = `env_01Tz2CrQM3X4EWDVLGWLY6GH` already exists on Anthropic's
side (control-plane sync created it; `workers_polling` will show once
the worker runs).

**First live run — done 2026-09-17 13:49–13:55 UTC, from the rig:**

1. Console key generated on `rig-gpu` → `worker/sdk/.env`;
   `docker compose up -d --build` → `idle; polling
   environment_id=env_01Tz… for work` (`200` on `/work/poll`).
2. **Stage 2 exit:** `POST /sessions` `{agent_slug: "gpu-compute", task:
   "Run nvidia-smi …"}` → `sesn_013jsPgwZW8Zsq4AAbZXPh5L`. Worker log:
   `claimed work` → `ack` → `GET /sessions/{id}` → `events/stream` →
   `heartbeat expected_last_heartbeat=NO_HEARTBEAT` → `events?limit=1000`
   → `executing tool tool=bash` (2 s after claim) → `POST …/events`
   (`user.tool_result`). Agent: "single NVIDIA GeForce RTX 5070 with
   12227 MiB … about 11086 MiB free". Cost `list_cost 6` ¢.
3. **Phase 6 exit clause 2:** second message asked it to `curl`
   `http://host.docker.internal:11434/api/chat` with `qwen3:8b`. Tool
   step ran 41.7 s (Ollama's log: `POST /api/chat 200 41.63 s` from
   `127.0.0.1` — Docker Desktop's host proxy), returned `"content":"391"`
   plus qwen3's `thinking`. Agent reported the breakdown: 17.8 s model
   load, 15.9 s prompt eval, 7.9 s generating 864 tokens. Session total
   `list_cost 8` ¢; `/usage` `recent` shows the row (`gpu-compute`,
   `rig-gpu`, `list_cost_cents: 8`, `budget_reached: false`).
4. After `end_turn` + 60 s the runner logged `session idle … stopping`,
   `POST …/work/{id}/stop` 200, back to polling. The work id **is the
   session id** (`work_id=sesn_…`), not a separate `work_…` — the docs'
   example is illustrative.

Observed for later: the SDK `bash` tool's 120 s cap would have caught a
cold Ollama call only ~3× slower than this one; `OLLAMA_KEEP_ALIVE=5m`
means back-to-back calls are warm, a session that pauses >5 min pays the
17.8 s load again. Fine for now.

**Next:** stage 5 — `mcp-fleet` gets its pass now that the control-plane
surface has settled (CLAUDE.md build order). The worker container is
`restart: unless-stopped` on the rig; it survives reboots as long as
Docker Desktop starts with Windows.

**Mac app rebuild** is still the one cosmetic leftover from Phase 6.

**A new machine needs** (on the Mac the checkout is `~/code/Iron-Fleet` —
never under `~/Documents`, which is iCloud Drive; see the Phase 4 note):

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
| `control-plane` | Droplet, `fleet.opustower.dev` (Railway stopped 2026-09-15) | Live. Registry, `POST /sessions`, session proxy routes, `/events`, `/interrupt`, `/usage`, signed webhooks. SQLite on a `/data` volume. |
| `mcp-fleet` | Droplet, `mcp.opustower.dev` (Railway stopped 2026-09-15) | Live. The five jarvis tools, nothing else. |
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
One exception: Phase 6 depends on Phase 2 only and may run beside 3–5.

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
6. ~~Point the desktop app at the new URL/token (in-app connection form).~~
   Done 2026-09-15 (Windows); Mac pending, see "Resume here".
7. ~~Disable — don't delete — the Railway webhook endpoint and stop the
   Railway services.~~ Done 2026-09-15. Rollback path stays for a week
   or two.

**Exit:** ~~all traffic (app, Anthropic webhooks, jarvis MCP callbacks) hits
the droplet; Railway stopped.~~ Met 2026-09-15 ~05:00 UTC (Mac app
repoint pending, see "Resume here").

### Phase 2 — Decommission Railway

Done 2026-09-15 (`68f7ad2` + `railway delete`), without the soak.

- ~~After the soak period: delete the Railway services and volume.~~ The
  whole project was deleted; it held nothing else.
- ~~Remove `.railway/`, the "Railway deployment" section of
  `control-plane/README.md`, the Railway paragraph in `mcp-fleet/README.md`.
  Replace with pointers to `deploy/droplet/`.~~
- ~~`PORT`/`DATABASE_PATH` defaults that mention Railway env vars in
  `control-plane` config: leave the code, update the comments.~~ The
  `RAILWAY_VOLUME_MOUNT_PATH` fallback was removed rather than kept — dead
  code once the compose file sets `DATABASE_PATH`, and leaving it would
  have failed this phase's own exit criterion.

**Exit:** ~~no Railway references outside git history~~ — met; what
remains is this plan, `docs/droplet-inventory.md`, and a history note in
`deploy/droplet/README.md`.

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

Verified against platform.claude.com on 2026-09-15 (events-and-streaming,
session-operations), then live from the droplet:

- Stream is `GET /v1/sessions/{id}/events/stream` with
  `accept: text/event-stream`; frames are `data: {event}` with the persisted
  event unchanged (`type`, `id: sevt_…`, `processed_at`, …). First frame is a
  `: connected` comment. No keepalive documented, no `Last-Event-ID`. Only
  events emitted after the stream opens are delivered — the reconnect pattern
  is open stream → `GET …/events` history → dedupe on `id`.
- Optional `event_deltas[]=agent.message|agent.thinking` adds token-level
  `event_start`/`event_delta` previews (no `id` of their own).
- **There is no interrupt route.** Interrupt is `{"type":"user.interrupt"}`
  on `POST …/events`; the turn ends with an ordinary `session.status_idle`.
  The control plane's `interrupt_session` posted to a nonexistent path until
  this phase.
- The docs' curl examples append `?beta=true` to every URL; it is not
  required (SDKs don't send it; verified live without it).

**Exit:** app shows a session's agent messages appearing live. **Met
2026-09-16** — see "Resume here".

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
in Usage. **Met 2026-09-16** — see "Resume here".

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

**Exit:** a restore from backup has been tested once. **Met 2026-09-16** —
see "Resume here".

### Phase 6 — Local inference on the rig (Ollama)

Added 2026-09-15 at the user's request. Depends on Phase 2 only — it needs
one control-plane URL to wire against and nothing from Phases 3–5 — so it
may run beside them. Numbered 6 because it's the newest, not the last.

**What it is.** Ollama on the Windows rig, on the RTX 5070, as a local
inference backend the fleet reaches two ways:

1. **From `rig-gpu` sessions, directly.** The worker runs tool calls as
   shell commands on the rig (`worker/README.md`, `exec.rs`), so a
   `gpu-compute` session can call Ollama's HTTP API as soon as it's
   running. Nothing to build beyond telling the agent it's there. The
   agent loop stays on Anthropic; the local model's output is a tool
   result, like any other CUDA job.
2. **Through `control-plane`** — "the Opus API" in the outside proposal's
   words — so the desktop app and anything else holding a bearer token can
   use it without knowing where the rig is. New routes, bearer-authed like
   every other route, proxied to the rig over a private link:
   `POST /inference/chat`, `POST /inference/embeddings`,
   `GET /inference/models`. Rig off → `503` with a clear body. No queue,
   no fallback to Claude.

**What it is not** (constraints 2, 4 and 5, restated for this phase):

- Not an agent loop. No fleet agent runs on a local model. `jarvis`,
  `blueweb-*` and `gpu-compute` stay Claude-model Managed Agents; the
  `model` fields in `agents/` don't change. Local models answer single
  requests — a chat completion, an embedding — and that is the whole
  surface.
- No failover in either direction. Claude unavailable ≠ use Ollama; rig
  off ≠ use Claude. `503` is the answer.
- Not exposed to `jarvis` through `mcp-fleet`. Jarvis dispatches work; if
  a job needs a local model it starts a `gpu-compute` session, which has it.
- Not on the droplet. 1 vCPU / 961 MB runs no model. The "network" is one
  node — the rig — until a second GPU exists; a static `INFERENCE_URL` on
  control-plane, not a node registry, until then.
- Not reachable from the internet. Ollama binds `127.0.0.1` on the rig;
  only the droplet reaches it, over the private link.

**Decide first** (record in `CLAUDE.md` "Open decisions", as Phase 0 did):

- **Private link, droplet → rig.** The rig sits behind home NAT and today
  makes only outbound connections (the worker polls). Keep that posture.
  Recommended: Tailscale on both boxes — control-plane proxies to
  `http://<rig tailnet IP>:11434`, nothing opened at home. Alternative with
  no new dependency: a reverse SSH tunnel from the rig to the droplet
  (`ssh -R 11434:127.0.0.1:11434`), at the cost of keeping a tunnel
  service alive on Windows. Not a router port-forward.
- **Models.** Undecided; sizing beats naming. 12 GB of VRAM, shared with
  whatever a `rig-gpu` session is running. Start with one general model
  and one embedding model, ≤ ~8 GB resident at Q4 so a session job still
  fits beside it — candidates in that class: `qwen3:8b`, `llama3.1:8b`,
  `gemma3:12b`; `qwen2.5-coder:7b` if code is the use; `nomic-embed-text`
  for embeddings. Check the installed Ollama build supports Blackwell
  (CUDA ≥ 12.8) before blaming a model. Record the chosen tags in
  `deploy/rig/models.txt` so the rig is reproducible the way `agents/` is.
- **GPU sharing.** Ollama and `rig-gpu` session jobs contend for the same
  12 GB. Start with `OLLAMA_MAX_LOADED_MODELS=1` and a short
  `OLLAMA_KEEP_ALIVE` so an idle model unloads; revisit only if a real
  session OOMs.
- **What the desktop app does with it.** In this phase, only a models list
  and a "rig online / offline" indicator on the Fleet tab. A chat UI on
  local models is a separate ask, and a voice loop on a local model is not
  Jarvis (constraint 4).

**Build** (once decided), in `deploy/rig/` beside `deploy/droplet/`:

1. Ollama on the rig as a Windows service: `OLLAMA_HOST=127.0.0.1`, the
   env vars above, models pulled from `models.txt`. Runbook in
   `deploy/rig/README.md`, same shape as the droplet's.
2. The private link. `curl http://<rig>:11434/api/tags` from the droplet
   is the check.
3. `control-plane`: `INFERENCE_URL` env var — unset means every
   `/inference/*` route is `404`, so a deployment without a rig is
   unchanged. The three routes above; streaming passthrough for chat; a
   request-body size limit; timeouts sized for a cold model load (the
   first request after idle can take tens of seconds). Proxy Ollama's
   native API, not its OpenAI-compat layer — one shape to maintain.
4. `agents/gpu-compute.json`: extend `system` with where Ollama is and
   which models are pulled. The address depends on where the worker runs
   tools — `127.0.0.1` on the host, `host.docker.internal` from the CUDA
   container (`worker/README.md` says "Linux", so confirm on the first
   run). Roll the agent version through boot sync as usual.
5. Desktop app: `GET /inference/models` → Fleet tab shows the rig's
   models or "rig offline".
6. Usage: local inference has no list-price cost, so no `session_usage`
   rows. If counts are wanted later, a small `inference_requests` rollup —
   not a prompt/response log; constraint 1 applies to local models too.

**Exit:** from the Mac, `POST /inference/chat` via `fleet.opustower.dev`
returns a completion generated on the 5070; a `gpu-compute` session calls
the local model in a tool step and its Claude-side cost still shows in
Usage; with the rig off, the same `/inference/chat` returns `503` and
nothing else in the fleet changes.

**Met (2026-09-16):** build steps 1 (as the tray app at logon, not a
service — see `deploy/rig/README.md` "Ollama at login"), 2, 3, 4 and 5
are live (PR #12, deployed ~07:00 UTC). Step 6 (usage counts) is
deliberately not built. Exit clauses 1 and 3 verified through
`fleet.opustower.dev`; exit clause 2 awaits the worker's first live run.
Evidence in "Resume here".

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
- Local ↔ cloud failover — including Claude ↔ Ollama, once Phase 6 exists.
- Fleet agents running on local models. Phase 6 adds local *inference*,
  not local agents.
- mTLS between services. Bearer tokens over Caddy-terminated TLS is the
  current model and sufficient for two services on one box.
- `worker/` and `rig-gpu`. Unchanged through Phase 5; Phase 6 adds Ollama
  beside the worker on the rig and one line to `gpu-compute`'s prompt.
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
deploy/rig/         (Phase 6) Ollama service setup, models.txt, runbook
docs/               this plan, droplet-inventory.md
```

Same repo. A separate repo for "Opus Tower OS" only makes sense once the
droplet hosts something that isn't Iron-Fleet; at that point the host-level
config (Caddy, backups) would move out and this repo would keep
only its own compose fragment.
