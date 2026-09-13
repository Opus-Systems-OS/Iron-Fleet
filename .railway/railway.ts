// Railway Infrastructure as Code for the Iron-Fleet control plane.
//
// Railway's IaC (`.railway/railway.ts`) is in beta and replaces the deprecated
// railway.toml/railway.json (hard cutoff 2026-12-01). The stable contract this
// file expresses — and what to click in the dashboard if `railway apply` ever
// disagrees with the beta reference — is:
//
//   service "control-plane"
//     source:     this GitHub repo, root directory = repo root
//     build:      Dockerfile at control-plane/Dockerfile, context = repo root
//     healthcheck /healthz
//     volume      mounted at /data  (SQLite: registry, policy, usage rollups)
//     variables   DATABASE_PATH=/data/control-plane.db plus the three secrets
//     domain      "Generate Domain" → the public HTTPS URL for the webhook
//
// Secrets are never written here. Set them once:
//   railway variables set ANTHROPIC_API_KEY=sk-ant-... \
//                         ANTHROPIC_WEBHOOK_SIGNING_KEY=whsec_... \
//                         CONTROL_PLANE_TOKEN=$(openssl rand -hex 32)
// `preserve()` keeps whatever Railway already holds for those keys.

import { defineRailway, github, service, volume, preserve, project } from "railway/iac";

export default defineRailway(() => {
  const data = volume("control-plane-data", { sizeMB: 512 });

  const controlPlane = service("control-plane", {
    source: github("Opus1247/Iron-Fleet", { rootDirectory: "." }),
    build: "docker build -f control-plane/Dockerfile .",
    start: "control-plane serve",
    healthcheck: "/healthz",
    volumeMounts: { "/data": data },
    env: {
      DATABASE_PATH: "/data/control-plane.db",
      AGENTS_DIR: "/app/agents",
      RUST_LOG: "info,tower_http=info",
      SYNC_ON_BOOT: "true",
      // Only needed if the API key is not in the org's Default workspace; it
      // is used solely to build the Console trace URL in POST /sessions responses.
      ANTHROPIC_WORKSPACE: "default",
      // Secrets — set out of band, never committed.
      ANTHROPIC_API_KEY: preserve(),
      ANTHROPIC_WEBHOOK_SIGNING_KEY: preserve(),
      CONTROL_PLANE_TOKEN: preserve(),
    },
  });

  return project("iron-fleet", { resources: [controlPlane, data] });
});
