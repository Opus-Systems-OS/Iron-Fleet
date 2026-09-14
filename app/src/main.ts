import { invoke } from "@tauri-apps/api/core";

// Read-only Fleet Dashboard (build order stage 3). Every value on screen is
// re-fetched from the control plane on each poll — nothing here is cached
// fleet state, per CLAUDE.md ("clients hold no fleet state").

const POLL_MS = 5000;

interface ConnectionStatus {
  configured: boolean;
  url: string | null;
}

interface Agent {
  slug: string;
  agent_id: string;
  agent_version: number;
  max_list_cost_cents: string;
  effort: string;
  default_environment: string;
  synced_at: string;
}

interface MoneyAmount {
  amount: string;
  currency: string;
}

interface SessionUsage {
  list_cost?: MoneyAmount;
  active_seconds?: number;
}

interface SessionBudget {
  max_list_cost?: MoneyAmount;
}

interface Session {
  id: string;
  status: string;
  title?: string | null;
  metadata?: Record<string, string>;
  usage?: SessionUsage;
  budget?: SessionBudget;
  updated_at?: string;
  created_at?: string;
}

interface SessionListEnvelope {
  data: Session[];
}

const el = <T extends HTMLElement>(id: string): T => {
  const found = document.getElementById(id);
  if (!found) throw new Error(`missing #${id}`);
  return found as T;
};

const connectionBadge = el<HTMLSpanElement>("connection-badge");
const lastUpdated = el<HTMLSpanElement>("last-updated");
const errorBanner = el<HTMLDivElement>("error-banner");
const settingsPanel = el<HTMLElement>("settings-panel");
const settingsToggle = el<HTMLButtonElement>("settings-toggle");
const settingsForm = el<HTMLFormElement>("settings-form");
const settingsUrl = el<HTMLInputElement>("settings-url");
const settingsToken = el<HTMLInputElement>("settings-token");
const settingsError = el<HTMLSpanElement>("settings-error");
const agentsBody = el<HTMLTableSectionElement>("agents-body");
const sessionsBody = el<HTMLTableSectionElement>("sessions-body");

let pollTimer: ReturnType<typeof setInterval> | undefined;

function centsToDollars(cents: string): string {
  const n = Number(cents);
  if (!Number.isFinite(n)) return cents;
  return `$${(n / 100).toFixed(2)}`;
}

function formatRelative(iso: string | undefined | null): string {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;
  const seconds = Math.max(0, Math.round((Date.now() - then) / 1000));
  if (seconds < 5) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

function statusClass(status: string): string {
  switch (status) {
    case "running":
      return "badge-running";
    case "idle":
      return "badge-idle";
    case "budget_reached":
    case "failed":
      return "badge-alert";
    default:
      return "badge-unknown";
  }
}

function setConnectionBadge(status: ConnectionStatus) {
  if (!status.configured) {
    connectionBadge.textContent = "not connected";
    connectionBadge.className = "badge badge-alert";
    return;
  }
  connectionBadge.textContent = status.url ?? "connected";
  connectionBadge.className = "badge badge-idle";
  connectionBadge.title = status.url ?? "";
}

function showError(message: string | null) {
  if (!message) {
    errorBanner.hidden = true;
    errorBanner.textContent = "";
    return;
  }
  errorBanner.hidden = false;
  errorBanner.textContent = message;
}

function renderEmptyRow(tbody: HTMLTableSectionElement, colspan: number, text: string) {
  tbody.innerHTML = "";
  const tr = document.createElement("tr");
  tr.className = "empty-row";
  const td = document.createElement("td");
  td.colSpan = colspan;
  td.textContent = text;
  tr.appendChild(td);
  tbody.appendChild(tr);
}

function renderAgents(agents: Agent[]) {
  if (agents.length === 0) {
    renderEmptyRow(agentsBody, 6, "No agents synced yet.");
    return;
  }
  agentsBody.innerHTML = "";
  for (const a of agents) {
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td class="mono">${escapeHtml(a.slug)}</td>
      <td>${escapeHtml(a.effort)}</td>
      <td class="mono">${centsToDollars(a.max_list_cost_cents)}</td>
      <td>${escapeHtml(a.default_environment)}</td>
      <td class="mono">v${a.agent_version}</td>
      <td class="dim">${formatRelative(a.synced_at)}</td>
    `;
    agentsBody.appendChild(tr);
  }
}

function renderSessions(sessions: Session[]) {
  if (sessions.length === 0) {
    renderEmptyRow(sessionsBody, 7, "No sessions yet.");
    return;
  }
  sessionsBody.innerHTML = "";
  for (const s of sessions) {
    const cost = s.usage?.list_cost?.amount;
    const cap = s.budget?.max_list_cost?.amount;
    const costCap = cost !== undefined && cap !== undefined ? `${centsToDollars(cost)} / ${centsToDollars(cap)}` : "—";
    const active = s.usage?.active_seconds !== undefined ? `${s.usage.active_seconds.toFixed(1)}s` : "—";
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td><span class="badge ${statusClass(s.status)}">${escapeHtml(s.status)}</span></td>
      <td class="title-cell" title="${escapeHtml(s.id)}">${escapeHtml(s.title ?? s.id)}</td>
      <td>${escapeHtml(s.metadata?.iron_fleet_agent ?? "—")}</td>
      <td>${escapeHtml(s.metadata?.iron_fleet_environment ?? "—")}</td>
      <td class="mono">${costCap}</td>
      <td class="mono">${active}</td>
      <td class="dim">${formatRelative(s.updated_at)}</td>
    `;
    sessionsBody.appendChild(tr);
  }
}

function escapeHtml(value: string): string {
  const div = document.createElement("div");
  div.textContent = value;
  return div.innerHTML;
}

async function refresh() {
  try {
    const [agents, sessions] = await Promise.all([
      invoke<Agent[]>("list_agents"),
      invoke<SessionListEnvelope>("list_sessions"),
    ]);
    renderAgents(agents);
    renderSessions(sessions.data ?? []);
    showError(null);
    lastUpdated.textContent = `updated ${new Date().toLocaleTimeString()}`;
  } catch (e) {
    showError(String(e));
  }
}

async function checkConnection(): Promise<ConnectionStatus> {
  const status = await invoke<ConnectionStatus>("connection_status");
  setConnectionBadge(status);
  settingsPanel.hidden = status.configured;
  return status;
}

function startPolling() {
  if (pollTimer) clearInterval(pollTimer);
  void refresh();
  pollTimer = setInterval(refresh, POLL_MS);
}

async function init() {
  const status = await checkConnection();
  if (status.configured) startPolling();

  settingsToggle.addEventListener("click", () => {
    settingsPanel.hidden = !settingsPanel.hidden;
  });

  settingsForm.addEventListener("submit", async (e) => {
    e.preventDefault();
    settingsError.textContent = "";
    try {
      const status = await invoke<ConnectionStatus>("set_connection", {
        url: settingsUrl.value,
        token: settingsToken.value,
      });
      setConnectionBadge(status);
      settingsPanel.hidden = true;
      settingsToken.value = "";
      startPolling();
    } catch (err) {
      settingsError.textContent = String(err);
    }
  });
}

window.addEventListener("DOMContentLoaded", () => {
  void init();
});
