import type { DelegationAgent, Settings } from "./types.js";

const STORAGE_KEY = "open-voice-agent.selected-agent";
type AgentChoice = Pick<DelegationAgent, "id" | "alias" | "delegation_speak_result">;

export function agentChoices(settings: Settings): AgentChoice[] {
  const valid = (settings.delegation_agents || []).filter((agent) => agent.id.trim() && agent.alias.trim());
  if (valid.length > 1) {
    return [
      { id: "orchestrator", alias: "Orchestrator", delegation_speak_result: settings.delegation_speak_result },
      ...valid,
    ];
  }
  return valid.length ? valid : [{ id: "", alias: "Agent", delegation_speak_result: settings.delegation_speak_result }];
}

export class AgentSelection {
  private choices: AgentChoice[] = [];
  private index = 0;

  constructor(private readonly button: HTMLButtonElement) {
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      this.next();
    });
  }

  configure(settings: Settings): void {
    this.choices = agentChoices(settings);
    let saved = "";
    try { saved = localStorage.getItem(STORAGE_KEY) || ""; } catch { /* optional */ }
    this.index = Math.max(0, this.choices.findIndex((agent) => agent.id === saved));
    this.render();
  }

  currentId(): string | undefined {
    const id = this.choices[this.index]?.id;
    return id ? id : undefined;
  }

  speakResult(fallback: boolean): boolean {
    return this.choices[this.index]?.delegation_speak_result ?? fallback;
  }

  private next(): void {
    if (this.choices.length < 2) return;
    this.index = (this.index + 1) % this.choices.length;
    try { localStorage.setItem(STORAGE_KEY, this.choices[this.index].id); } catch { /* optional */ }
    this.render();
  }

  private render(): void {
    const agent = this.choices[this.index];
    this.button.textContent = agent?.alias || "Agent";
    this.button.title = this.choices.length > 1 ? "Switch agent / Orchestrator" : "Current agent";
    this.button.disabled = this.choices.length < 2;
    this.button.setAttribute("aria-label", `${this.button.title}: ${agent?.alias || "Agent"}`);
  }
}
