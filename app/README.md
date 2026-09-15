# app

The Iron-Fleet desktop client (J.A.R.V.I.S.): Tauri 2 shell, system webview
(not Electron — the MacBook Air's battery is a real constraint). `src-tauri/`
is the Rust shell; `src/` is a vanilla-TypeScript frontend with no UI
framework. Two tabs: Fleet (stage 3: agents, sessions, and — stage 4 — session
controls) and Usage (stage 4).

## Why the frontend never calls `fetch` directly

Every request goes through a Tauri command (`invoke(...)`) into
`src-tauri/src/commands.rs`, which does the HTTP call in Rust. Two reasons:

- The control plane sends no CORS headers, so a webview `fetch` to a
  different origin would be blocked; a request made from Rust has no such
  restriction.
- It keeps the bearer token out of the page's JS-visible network layer.

## Session controls (stage 4)

- **New session** — the form at the top of the Fleet tab: pick an agent,
  type a task, `POST /sessions`.
- **Message** — a follow-up `user.message` on a running session
  (`POST /sessions/{id}/events`). Prompts for text with `window.prompt`.
- **Interrupt** — stops a session's in-flight work without ending it
  (`POST /sessions/{id}/interrupt`), behind a `window.confirm`.
- **Console** — opens the session's Anthropic Console trace URL in the
  system browser via `@tauri-apps/plugin-opener`'s `openUrl` (hence
  `opener:default` in `capabilities/default.json`, re-added for this).

There is still no way to raise a session's cap — the control plane's budgets
are create-only, so no route exists for it anywhere, not just in this UI.

`/events` and `/interrupt` carry the same caveat as
`control-plane/README.md`: unconfirmed endpoint paths, no fixture from a
live run yet. `control-plane/dev/mock-managed-agents.py` now implements
both so the full loop can be exercised locally before that first live run.

## Connecting to a control plane

`CONTROL_PLANE_URL` / `CONTROL_PLANE_TOKEN` env vars win when set (handy for
`npm run tauri dev`, pointing at a local instance). Otherwise the app reads
`<OS app config dir>/control-plane.json`, written by the in-app connection
form (gear icon, top right) the first time you type a URL and token. This is
app configuration — which control plane to point at — not fleet state; fleet
state itself is never cached here (CLAUDE.md: "clients hold no fleet state").

## Local run

Against the real, deployed control plane:

```sh
cd app
npm install
CONTROL_PLANE_URL=https://fleet.opustower.dev \
  CONTROL_PLANE_TOKEN=<the deployed CONTROL_PLANE_TOKEN> \
  npm run tauri dev
```

Or leave the env vars unset and use the in-app connection form.

Against a fully local stack, no spend, following `control-plane/README.md`'s
local-run recipe:

```sh
# 1. mock Anthropic
python3 control-plane/dev/mock-managed-agents.py 9999
# 2. real control-plane, pointed at the mock
ANTHROPIC_BASE_URL=http://127.0.0.1:9999 ANTHROPIC_API_KEY=test \
  ANTHROPIC_WEBHOOK_SIGNING_KEY=whsec_test CONTROL_PLANE_TOKEN=dev \
  cargo run -p control-plane -- serve
# 3. the dashboard, pre-connected
cd app && CONTROL_PLANE_URL=http://127.0.0.1:8080 CONTROL_PLANE_TOKEN=dev npm run tauri dev
```

## Not yet

No orb UI or voice loop; CLAUDE.md describes those as a view inside this
same app, wired up once `jarvis`'s own session plumbing exists
(`mcp-fleet/`, stage 5). macOS speech I/O being platform-gated in
`src-tauri` is a note for that stage, not this one.
