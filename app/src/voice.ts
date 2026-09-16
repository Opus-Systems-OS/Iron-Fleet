import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Transcript } from "./transcript";
import type { SessionBudget, SessionEventPayload, SessionUsage, StreamStatePayload } from "./transcript";

// The Jarvis view (centralization Phase 4): hold the orb, talk, release; a
// `jarvis` Managed Agents session answers through the control plane and
// the reply is spoken. Nothing here talks to Claude — every turn is
// `create_session` / `send_session_event` like the Fleet tab, and the reply
// arrives on the same `session-event` feed (watch slot "voice"). Speech
// input is the Rust `speech_*` commands (macOS only; elsewhere `stt` is
// false and the text box is the whole interface). Speech output is the
// webview's `speechSynthesis`.
//
// State held here: the current session id (a bookmark, dropped on "New
// conversation"), the ids already spoken, and the orb state. Not fleet state.

type OrbState = "idle" | "listening" | "thinking" | "speaking" | "error";

interface SpeechSupport {
  stt: boolean;
  reason?: string;
}

interface CreateResponse {
  session_id: string;
  console_url?: string;
}

const AGENT = "jarvis";
const SLOT = "voice";

const el = <T extends HTMLElement>(id: string): T => {
  const found = document.getElementById(id);
  if (!found) throw new Error(`missing #${id}`);
  return found as T;
};

export class VoiceView {
  private readonly orb = el<HTMLDivElement>("orb");
  private readonly caption = el<HTMLDivElement>("orb-caption");
  private readonly form = el<HTMLFormElement>("voice-form");
  private readonly input = el<HTMLInputElement>("voice-input");
  private readonly newButton = el<HTMLButtonElement>("voice-new");
  private readonly sessionLine = el<HTMLDivElement>("voice-session");
  private readonly streamPill = el<HTMLSpanElement>("voice-stream");
  private readonly transcript = new Transcript(el<HTMLDivElement>("voice-transcript"));

  private readonly synth: SpeechSynthesis | null =
    typeof window !== "undefined" && "speechSynthesis" in window ? window.speechSynthesis : null;

  private sessionId: string | null = null;
  private consoleUrl: string | null = null;
  private readonly spoken = new Set<string>();
  private state: OrbState = "idle";
  private support: SpeechSupport | null = null;
  private holding = false;
  private sessionRunning = false;
  private budgetHit = false;
  private queue: string[] = [];
  private speaking = false;

  constructor(private readonly showError: (message: string | null) => void) {}

  async init() {
    await listen<{ text: string }>("speech-partial", ({ payload }) => {
      if (this.holding && payload.text) this.caption.textContent = payload.text;
    });

    this.orb.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      void this.beginListening();
    });
    for (const type of ["pointerup", "pointercancel", "pointerleave"] as const) {
      this.orb.addEventListener(type, () => void this.endListening());
    }
    window.addEventListener("keydown", (e) => {
      if (e.code !== "Space" || e.repeat || !this.orbVisible() || this.typing()) return;
      e.preventDefault();
      void this.beginListening();
    });
    window.addEventListener("keyup", (e) => {
      if (e.code !== "Space" || !this.holding) return;
      e.preventDefault();
      void this.endListening();
    });

    this.form.addEventListener("submit", (e) => {
      e.preventDefault();
      const text = this.input.value.trim();
      if (!text) return;
      this.input.value = "";
      void this.say(text);
    });
    this.newButton.addEventListener("click", () => void this.newConversation());
    this.sessionLine.addEventListener("click", () => {
      if (this.consoleUrl) openUrl(this.consoleUrl).catch((err) => this.showError(String(err)));
    });

    this.setState("idle");
    this.renderSessionLine();
  }

  /** Called each time the tab is shown. The first call asks for speech permission. */
  mounted() {
    if (this.support) return;
    invoke<SpeechSupport>("speech_support")
      .then((support) => {
        this.support = support;
        this.orb.classList.toggle("orb-disabled", !support.stt);
        if (!support.stt) {
          this.caption.textContent = support.reason ?? "speech input unavailable — type below";
        } else if (this.state === "idle") {
          this.caption.textContent = "Hold the orb (or Space) to talk";
        }
        if (!this.synth) this.sessionLine.title = "no speechSynthesis in this webview — replies are text only";
      })
      .catch((err) => this.showError(String(err)));
  }

  // ---- feed from the Rust watcher ------------------------------------------

  onSessionEvent(payload: SessionEventPayload) {
    if (payload.session_id !== this.sessionId) return;
    const r = this.transcript.render(payload.event);
    switch (r.kind) {
      case "agent_message": {
        const key = r.id ?? `${Date.now()}`;
        if (this.spoken.has(key) || !r.text.trim()) break;
        this.spoken.add(key);
        this.speak(r.text);
        break;
      }
      case "running":
        this.sessionRunning = true;
        if (!this.speaking && !this.holding) this.setState("thinking");
        break;
      case "idle":
        this.sessionRunning = false;
        if (r.stop_reason === "budget_reached") {
          this.budgetHit = true;
          this.input.disabled = true;
          this.speak("Session budget reached. Start a new conversation to continue.");
        } else if (!this.speaking && !this.holding) {
          this.setState("idle");
        }
        break;
      case "error":
        this.sessionRunning = false;
        this.setState("error", r.message);
        break;
      case "usage":
        this.renderSessionLine(r.usage, r.budget);
        break;
    }
  }

  onStreamState(payload: StreamStatePayload) {
    if (payload.session_id !== this.sessionId) return;
    this.streamPill.textContent = payload.state;
    this.streamPill.title = payload.message ?? "event stream";
    this.streamPill.className = `stream-pill stream-${
      payload.state === "open" ? "open" : payload.state === "error" ? "error" : payload.state === "reconnecting" ? "reconnecting" : "idle"
    }`;
  }

  // ---- talking ---------------------------------------------------------------

  private async beginListening() {
    if (this.holding || !this.support?.stt || this.budgetHit) return;
    this.holding = true;
    this.stopSpeaking(); // barge-in
    this.setState("listening");
    try {
      await invoke("speech_start");
    } catch (err) {
      this.holding = false;
      this.setState("error", String(err));
    }
  }

  private async endListening() {
    if (!this.holding) return;
    this.holding = false;
    let text = "";
    try {
      text = (await invoke<string>("speech_stop")).trim();
    } catch (err) {
      this.setState("error", String(err));
      return;
    }
    if (!text) {
      this.setState("idle", "Didn't catch that — hold and try again");
      return;
    }
    await this.say(text);
  }

  /** One turn: first utterance creates the session, later ones append to it. */
  private async say(text: string) {
    if (this.budgetHit) return;
    this.setState("thinking", text);
    this.sessionRunning = true;
    try {
      if (!this.sessionId) {
        const created = await invoke<CreateResponse>("create_session", {
          agentSlug: AGENT,
          task: text,
          repositories: [],
        });
        this.sessionId = created.session_id;
        this.consoleUrl = created.console_url ?? null;
        this.renderSessionLine();
        await invoke("watch_session", { id: this.sessionId, slot: SLOT });
      } else {
        await invoke("send_session_event", { id: this.sessionId, task: text });
      }
      this.showError(null);
    } catch (err) {
      this.sessionRunning = false;
      this.setState("error", String(err));
    }
  }

  private async newConversation() {
    this.stopSpeaking();
    if (this.sessionId) await invoke("unwatch_session", { slot: SLOT }).catch(() => undefined);
    this.sessionId = null;
    this.consoleUrl = null;
    this.spoken.clear();
    this.transcript.clear();
    this.budgetHit = false;
    this.sessionRunning = false;
    this.input.disabled = false;
    this.streamPill.textContent = "";
    this.streamPill.className = "stream-pill";
    this.renderSessionLine();
    this.setState("idle");
  }

  // ---- speaking ---------------------------------------------------------------

  private speak(text: string) {
    if (!this.synth) return; // replies stay text-only in the transcript
    this.queue.push(text);
    if (!this.speaking) this.pump();
  }

  private pump() {
    const next = this.queue.shift();
    if (next === undefined || !this.synth) {
      this.speaking = false;
      if (!this.holding) this.setState(this.sessionRunning ? "thinking" : "idle");
      return;
    }
    this.speaking = true;
    this.setState("speaking", next);
    const utterance = new SpeechSynthesisUtterance(next);
    utterance.onend = () => this.pump();
    utterance.onerror = () => this.pump();
    this.synth.speak(utterance);
  }

  private stopSpeaking() {
    this.queue = [];
    if (this.synth?.speaking || this.synth?.pending) this.synth.cancel();
    this.speaking = false;
  }

  // ---- rendering --------------------------------------------------------------

  private setState(state: OrbState, caption?: string) {
    this.state = state;
    this.orb.className = `orb orb-${state}${this.support && !this.support.stt ? " orb-disabled" : ""}`;
    const fallback: Record<OrbState, string> = {
      idle: this.support?.stt ? "Hold the orb (or Space) to talk" : (this.support?.reason ?? "Type below"),
      listening: "Listening…",
      thinking: "Thinking…",
      speaking: "",
      error: "Something went wrong",
    };
    this.caption.textContent = caption ?? fallback[state];
    this.caption.classList.toggle("alert", state === "error");
  }

  private renderSessionLine(usage?: SessionUsage, budget?: SessionBudget | null) {
    if (!this.sessionId) {
      this.sessionLine.textContent = "No conversation yet";
      this.sessionLine.classList.remove("linkish");
      return;
    }
    const cost = usage?.list_cost?.amount;
    const cap = budget?.max_list_cost?.amount;
    const money = cost !== undefined ? ` · $${(Number(cost) / 100).toFixed(2)}${cap !== undefined ? ` / $${(Number(cap) / 100).toFixed(2)}` : ""}` : "";
    this.sessionLine.textContent = `${this.sessionId}${money}`;
    this.sessionLine.classList.toggle("linkish", !!this.consoleUrl);
    this.sessionLine.title = this.consoleUrl ? "Open in the Anthropic Console" : "";
  }

  private orbVisible(): boolean {
    return !!this.orb.offsetParent;
  }

  private typing(): boolean {
    const a = document.activeElement;
    return a instanceof HTMLInputElement || a instanceof HTMLTextAreaElement || a instanceof HTMLSelectElement;
  }
}
