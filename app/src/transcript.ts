// Renders a session's event stream as a chat transcript. Used by the Fleet
// tab's session panel and by the Jarvis view; each owns a `Transcript` over
// its own container. `render` also returns a small summary of what the event
// meant so callers react (speak a message, flip a status badge) without
// re-parsing the event themselves.

export interface MoneyAmount {
  amount: string;
  currency: string;
}

export interface SessionUsage {
  list_cost?: MoneyAmount;
  active_seconds?: number;
}

export interface SessionBudget {
  max_list_cost?: MoneyAmount;
}

/** One Anthropic session event, as the API sent it. Only the fields we render are typed. */
export interface SessionEvent {
  type: string;
  id?: string;
  content?: ContentBlock[];
  name?: string;
  input?: unknown;
  stop_reason?: { type: string };
  error?: { message?: string };
  usage?: SessionUsage;
  budget?: SessionBudget;
  // event_start / event_delta previews
  event?: { type: string; id: string };
  event_id?: string;
  delta?: { type: string; index: number; content?: ContentBlock };
}

export interface ContentBlock {
  type: string;
  text?: string;
}

export interface SessionEventPayload {
  session_id: string;
  event: SessionEvent;
}

export interface StreamStatePayload {
  session_id: string;
  state: "open" | "reconnecting" | "closed" | "error";
  message?: string;
}

export type Rendered =
  | { kind: "user_message"; id?: string; text: string }
  | { kind: "agent_message"; id?: string; text: string }
  | { kind: "tool" }
  | { kind: "running" }
  | { kind: "idle"; stop_reason: string }
  | { kind: "error"; message: string }
  | { kind: "usage"; usage?: SessionUsage; budget?: SessionBudget | null }
  | { kind: "other" };

export function textOf(content: ContentBlock[] | undefined): string {
  return (content ?? [])
    .filter((b) => b.type === "text" && typeof b.text === "string")
    .map((b) => b.text as string)
    .join("");
}

export function truncate(value: string, max: number): string {
  return value.length > max ? `${value.slice(0, max)}…` : value;
}

export class Transcript {
  constructor(private readonly container: HTMLElement) {}

  clear() {
    this.container.innerHTML = "";
  }

  render(ev: SessionEvent): Rendered {
    const c = this.container;
    const atBottom = c.scrollHeight - c.scrollTop - c.clientHeight < 40;
    let out: Rendered = { kind: "other" };
    switch (ev.type) {
      case "user.message": {
        const text = textOf(ev.content);
        this.appendEntry("entry-user", text, ev.id);
        out = { kind: "user_message", id: ev.id, text };
        break;
      }
      case "user.interrupt":
        this.appendEntry("entry-system", "interrupt sent", ev.id);
        break;
      case "agent.message": {
        const pending = ev.id ? c.querySelector(`.entry.pending[data-event-id="${CSS.escape(ev.id)}"]`) : null;
        if (pending) pending.remove();
        const text = textOf(ev.content);
        this.appendEntry("entry-agent", text, ev.id);
        out = { kind: "agent_message", id: ev.id, text };
        break;
      }
      case "event_start":
        if (ev.event?.type === "agent.message" && ev.event.id) this.pendingBubble(ev.event.id);
        break;
      case "event_delta":
        if (ev.event_id && ev.delta?.content?.type === "text" && ev.delta.content.text) {
          this.pendingBubble(ev.event_id).textContent += ev.delta.content.text;
        }
        break;
      case "agent.tool_use":
      case "agent.mcp_tool_use":
      case "agent.custom_tool_use": {
        const input = ev.input === undefined ? "" : JSON.stringify(ev.input, null, 1);
        this.appendToolEntry(`⚙ ${ev.name ?? ev.type} ${truncate(input.replace(/\s+/g, " "), 120)}`, input);
        out = { kind: "tool" };
        break;
      }
      case "agent.tool_result":
        this.appendToolEntry(`↳ result ${truncate(textOf(ev.content).replace(/\s+/g, " "), 120)}`, textOf(ev.content));
        out = { kind: "tool" };
        break;
      case "session.status_running":
        this.appendEntry("entry-system", "running", ev.id);
        out = { kind: "running" };
        break;
      case "session.status_idle": {
        const reason = ev.stop_reason?.type ?? "idle";
        this.appendEntry(`entry-system${reason === "budget_reached" ? " alert" : ""}`, `idle (${reason})`, ev.id);
        out = { kind: "idle", stop_reason: reason };
        break;
      }
      case "session.status_error":
      case "session.error": {
        const message = ev.error?.message ?? ev.type;
        this.appendEntry("entry-system alert", `error: ${message}`, ev.id);
        out = { kind: "error", message };
        break;
      }
      case "session.usage":
        out = { kind: "usage", usage: ev.usage, budget: ev.budget };
        break;
      case "agent.thinking":
      case "span.model_request_start":
      case "span.model_request_end":
        break; // noise for this view
      default:
        this.appendEntry("entry-system", ev.type, ev.id);
    }
    if (atBottom) c.scrollTop = c.scrollHeight;
    return out;
  }

  private appendEntry(className: string, text: string, id?: string): HTMLDivElement {
    const div = document.createElement("div");
    div.className = `entry ${className}`;
    div.textContent = text;
    if (id) div.dataset.eventId = id;
    this.container.appendChild(div);
    return div;
  }

  private appendToolEntry(summary: string, detail: string) {
    const details = document.createElement("details");
    details.className = "entry entry-tool";
    const s = document.createElement("summary");
    s.textContent = summary;
    details.appendChild(s);
    if (detail) {
      const pre = document.createElement("div");
      pre.textContent = truncate(detail, 2000);
      details.appendChild(pre);
    }
    this.container.appendChild(details);
  }

  /** Streams a token preview into a placeholder bubble; the persisted event replaces it. */
  private pendingBubble(eventId: string): HTMLDivElement {
    const existing = this.container.querySelector<HTMLDivElement>(
      `.entry.pending[data-event-id="${CSS.escape(eventId)}"]`,
    );
    return existing ?? this.appendEntry("entry-agent pending", "", eventId);
  }
}
