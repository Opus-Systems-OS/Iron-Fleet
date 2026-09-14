import { defineRailway, github, preserve, project, service, volume } from "railway/iac";

export default defineRailway(() => {
  const ironFleetVolume = volume("iron-fleet-volume", { alerts: { usage: { "100": {}, "80": {}, "95": {} } }, allowOnlineResize: true, region: "sfo", sizeMB: 500 });
  const mcpFleet = service("mcp-fleet", {
    source: github("Opus1247/Iron-Fleet"),
    build: { buildEnvironment: "V3", builder: "DOCKERFILE", dockerfilePath: "mcp-fleet/Dockerfile" },
    healthcheck: "/healthz",
    healthcheckTimeout: 120,
    replicas: { "sfo": 1 },
    env: { CONTROL_PLANE_TOKEN: preserve(), CONTROL_PLANE_URL: preserve(), MCP_FLEET_TOKEN: preserve(), RUST_LOG: preserve() },
  });
  const IronFleet = service("Iron-Fleet", {
    source: github("Opus1247/Iron-Fleet", { commitSha: "a0bc5fb501fde9e73c75297e53dd38429f2b6c74", upstreamUrl: "https://github.com/Opus1247/Iron-Fleet" }),
    build: { buildEnvironment: "V3", builder: "DOCKERFILE", dockerfilePath: "control-plane/Dockerfile" },
    healthcheck: "/healthz",
    healthcheckTimeout: 120,
    replicas: { "sfo": 1 },
    deploy: { ipv6EgressEnabled: true },
    networking: { privateNetworkEndpoint: "iron-fleet" },
    volumeMounts: { "/data": ironFleetVolume },
    env: { ANTHROPIC_API_KEY: preserve(), ANTHROPIC_WEBHOOK_SIGNING_KEY: preserve(), ANTHROPIC_WORKSPACE: preserve(), CONTROL_PLANE_TOKEN: preserve(), DATABASE_PATH: preserve(), MCP_FLEET_TOKEN: preserve(), MCP_FLEET_URL: preserve(), RUST_LOG: preserve(), SYNC_ON_BOOT: preserve() },
  });

  return project("practical-compassion", {
    resources: [mcpFleet, IronFleet, ironFleetVolume],
  });
});
