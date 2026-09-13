// Railway Infrastructure as Code for the Iron-Fleet control plane.
// Imported from the live project with `railway config pull`, then cleaned:
// no commit pin (track main), non-secret values inline, secrets preserved.
//
// Secrets are set once, out of band, and never written here:
//   railway variable set ANTHROPIC_API_KEY=sk-ant-... \
//                        ANTHROPIC_WEBHOOK_SIGNING_KEY=whsec_... \
//                        CONTROL_PLANE_TOKEN=$(openssl rand -hex 32)
//
// Review changes with `railway config plan` before `railway config apply`.

import { defineRailway, github, preserve, project, service, volume } from "railway/iac";

export default defineRailway(() => {
  // SQLite: agent registry, budget policy, usage rollups. Single replica only.
  const data = volume("iron-fleet-volume", {
    region: "sfo",
    sizeMB: 500,
    allowOnlineResize: true,
    alerts: { usage: { "80": {}, "95": {}, "100": {} } },
  });

  const controlPlane = service("Iron-Fleet", {
    source: github("Opus1247/Iron-Fleet", { upstreamUrl: "https://github.com/Opus1247/Iron-Fleet" }),
    // Build context is the repo root: the workspace Cargo.toml and agents/ are needed.
    build: { builder: "DOCKERFILE", dockerfilePath: "control-plane/Dockerfile", buildEnvironment: "V3" },
    healthcheck: "/healthz",
    healthcheckTimeout: 120,
    replicas: { sfo: 1 },
    networking: { privateNetworkEndpoint: "iron-fleet" },
    volumeMounts: { "/data": data },
    env: {
      DATABASE_PATH: "/data/control-plane.db",
      SYNC_ON_BOOT: "true",
      RUST_LOG: "info,tower_http=info",
      // Only used to build the Console trace URL in POST /sessions responses.
      ANTHROPIC_WORKSPACE: "default",
      ANTHROPIC_API_KEY: preserve(),
      ANTHROPIC_WEBHOOK_SIGNING_KEY: preserve(),
      CONTROL_PLANE_TOKEN: preserve(),
    },
  });

  return project("practical-compassion", { resources: [controlPlane, data] });
});
