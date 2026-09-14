import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

// Fleet Dashboard + Usage tab (build order stage 4: session controls, then
// the Usage tab). Every value on screen is re-fetched from the control plane
// on each poll — nothing here is cached fleet state, per CLAUDE.md ("clients
// hold no fleet state").

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
  console_url?: string;
}

interface SessionListEnvelope {
  data: Session[];
}

interface AgentUsage {
  agent_slug: string;
  session_count: number;
  total_list_cost_cents: number;
  budget_reached_count: number;
}

interface UsageRow {
  session_id: string;
  agent_slug: string;
  environment_slug: string | null;
  list_cost_cents: string | null;
  active_seconds: number | null;
  budget_reached: boolean;
  observed_at: string;
}

interface UsageResponse {
  by_agent: AgentUsage[];
  recent: UsageRow[];
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
const tabs = el<HTMLElement>("tabs");
const tabFleet = el<HTMLElement>("tab-fleet");
const tabUsage = el<HTMLElement>("tab-usage");
const newSessionForm = el<HTMLFormElement>("new-session-form");
const newSessionAgent = el<HTMLSelectElement>("new-session-agent");
const newSessionTask = el<HTMLInputElement>("new-session-task");
const newSessionRepos = el<HTMLInputElement>("new-session-repos");
const newSessionError = el<HTMLSpanElement>("new-session-error");
const usageAgentsBody = el<HTMLTableSectionElement>("usage-agents-body");
const usageRecentBody = el<HTMLTableSectionElement>("usage-recent-body");

let pollTimer: ReturnType<typeof setInterval> | undefined;
let activeTab: "fleet" | "usage" = "fleet";

function centsToDollars(cents: string | number): string {
  const n = typeof cents === "number" ? cents : Number(cents);
  if (!Number.isFinite(n)) return String(cents);
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

function escapeHtml(value: string): string {
  const div = document.createElement("div");
  div.textContent = value;
  return div.innerHTML;
}

// ---- tabs -------------------------------------------------------------

tabs.addEventListener("click", (e) => {
  const button = (e.target as HTMLElement).closest<HTMLButtonElement>(".tab-button");
  if (!button) return;
  const tab = button.dataset.tab === "usage" ? "usage" : "fleet";
  activeTab = tab;
  for (const b of tabs.querySelectorAll(".tab-button")) b.classList.toggle("active", b === button);
  tabFleet.hidden = tab !== "fleet";
  tabUsage.hidden = tab !== "usage";
  void refresh();
});

// ---- fleet tab ----------------------------------------------------------

function renderAgents(agents: Agent[]) {
  if (agents.length === 0) {
    renderEmptyRow(agentsBody, 6, "No agents synced yet.");
  } else {
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

  const previous = newSessionAgent.value;
  newSessionAgent.innerHTML = `<option value="" disabled ${previous ? "" : "selected"}>Agent…</option>`;
  for (const a of agents) {
    const opt = document.createElement("option");
    opt.value = a.slug;
    opt.textContent = a.slug;
    newSessionAgent.appendChild(opt);
  }
  if (agents.some((a) => a.slug === previous)) newSessionAgent.value = previous;
}

function renderSessions(sessions: Session[]) {
  if (sessions.length === 0) {
    renderEmptyRow(sessionsBody, 8, "No sessions yet.");
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
      <td class="actions-cell">
        <button class="mini-button" data-action="message" data-id="${escapeHtml(s.id)}">Message</button>
        <button class="mini-button mini-button-alert" data-action="interrupt" data-id="${escapeHtml(s.id)}">Interrupt</button>
        ${s.console_url ? `<button class="mini-button" data-action="console" data-url="${escapeHtml(s.console_url)}">Console</button>` : ""}
      </td>
    `;
    sessionsBody.appendChild(tr);
  }
}

sessionsBody.addEventListener("click", async (e) => {
  const button = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-action]");
  if (!button) return;
  const action = button.dataset.action;
  try {
    if (action === "console" && button.dataset.url) {
      await openUrl(button.dataset.url);
    } else if (action === "message" && button.dataset.id) {
      const task = window.prompt("Message to send to this session:");
      if (task && task.trim()) {
        await invoke("send_session_event", { id: button.dataset.id, task });
        await refresh();
      }
    } else if (action === "interrupt" && button.dataset.id) {
      if (window.confirm("Interrupt this session's in-flight work?")) {
        await invoke("interrupt_session", { id: button.dataset.id });
        await refresh();
      }
    }
  } catch (err) {
    showError(String(err));
  }
});

newSessionForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  newSessionError.textContent = "";
  try {
    await invoke("create_session", {
      agentSlug: newSessionAgent.value,
      task: newSessionTask.value,
      repositories: newSessionRepos.value
        .split(",")
        .map((r) => r.trim())
        .filter((r) => r.length > 0),
    });
    newSessionTask.value = "";
    newSessionRepos.value = "";
    await refresh();
  } catch (err) {
    newSessionError.textContent = String(err);
  }
});

// ---- usage tab ------------------------------------------------------------

function renderUsage(usage: UsageResponse) {
  if (usage.by_agent.length === 0) {
    renderEmptyRow(usageAgentsBody, 4, "No usage recorded yet.");
  } else {
    usageAgentsBody.innerHTML = "";
    for (const a of usage.by_agent) {
      const tr = document.createElement("tr");
      tr.innerHTML = `
        <td class="mono">${escapeHtml(a.agent_slug)}</td>
        <td class="mono">${a.session_count}</td>
        <td class="mono">${centsToDollars(a.total_list_cost_cents)}</td>
        <td class="mono">${a.budget_reached_count}</td>
      `;
      usageAgentsBody.appendChild(tr);
    }
  }

  if (usage.recent.length === 0) {
    renderEmptyRow(usageRecentBody, 7, "No activity recorded yet.");
    return;
  }
  usageRecentBody.innerHTML = "";
  for (const r of usage.recent) {
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td class="mono" title="${escapeHtml(r.session_id)}">${escapeHtml(r.session_id.slice(0, 18))}…</td>
      <td>${escapeHtml(r.agent_slug)}</td>
      <td>${escapeHtml(r.environment_slug ?? "—")}</td>
      <td class="mono">${r.list_cost_cents !== null ? centsToDollars(r.list_cost_cents) : "—"}</td>
      <td class="mono">${r.active_seconds !== null ? `${r.active_seconds.toFixed(1)}s` : "—"}</td>
      <td>${r.budget_reached ? `<span class="badge badge-alert">yes</span>` : `<span class="badge badge-idle">no</span>`}</td>
      <td class="dim">${formatRelative(r.observed_at)}</td>
    `;
    usageRecentBody.appendChild(tr);
  }
}

// ---- shared -----------------------------------------------------------

async function refresh() {
  try {
    if (activeTab === "fleet") {
      const [agents, sessions] = await Promise.all([
        invoke<Agent[]>("list_agents"),
        invoke<SessionListEnvelope>("list_sessions"),
      ]);
      renderAgents(agents);
      renderSessions(sessions.data ?? []);
    } else {
      const usage = await invoke<UsageResponse>("get_usage");
      renderUsage(usage);
    }
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
