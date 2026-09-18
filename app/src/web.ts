import { VoicePipeline } from "./lib/pipeline.js";
import { getSettings, routeDelegation, delegate, listen } from "./lib/tauri.js";
import { StormOrb } from "./lib/storm-orb.js";
import { TaskToast } from "./lib/task-toast.js";
import { element } from "./lib/dom.js";
import { ensureVoiceBackend, keepVoiceBackendAlive } from "./lib/backend-lifecycle.js";
import { errorText } from "./lib/errors.js";
import type { PipelineState } from "./lib/types.js";
import { AgentSelection } from "./lib/agent-selection.js";

const orb = element("orb");
const caption = element("caption");
const notice = element("notice");
const feed = element("transcript");
const visual = new StormOrb(orb);
const taskToast = new TaskToast();
const agentSelection = new AgentSelection(element("agent-switch") as HTMLButtonElement);

const CAPTIONS: Record<PipelineState, string> = {
  idle: "tap to talk",
  connecting: "connecting…",
  listening: "listening",
  user_speaking: "…",
  thinking: "thinking",
  speaking: "speaking",
  delegating: "delegating…",
  error: "disconnected — tap to retry",
};

const pipeline = new VoicePipeline({
  route: (r) => routeDelegation(r, agentSelection.currentId()),
  delegate: (r, agentId) => delegate(r, agentId ?? agentSelection.currentId()),
  shouldSpeakDelegatedResult: () => agentSelection.speakResult(true),
  ensureBackend: ensureVoiceBackend,
  backendActivity: keepVoiceBackendAlive,
});

async function loadSettings(): Promise<void> {
  const settings = await getSettings();
  agentSelection.configure(settings);
  if (!settings.setup_completed) {
    location.replace("/settings?setup=1");
    return;
  }
  pipeline.configure(settings);
}

function setNotice(text: string, isError = false): void {
  notice.textContent = text;
  notice.className = `web-notice ${isError ? "error" : ""}`;
  notice.hidden = !text;
}

function addMsg(role: string, text: string): void {
  const row = document.createElement("div");
  row.className = `msg ${role}`;
  const tag = document.createElement("span");
  tag.className = "tag";
  tag.textContent = role === "user" ? "You" : role === "assistant" ? "Assistant" : "Task";
  const body = document.createElement("span");
  body.className = "body";
  body.textContent = text;
  row.appendChild(tag);
  row.appendChild(body);
  feed.appendChild(row);
  feed.scrollTop = feed.scrollHeight;
}

pipeline.addEventListener("state", (e) => {
  orb.className = `orb state-${e.detail.state}${visual.gl ? "" : " orb-webgl-fallback"}`;
  visual.setState(e.detail.state);
  caption.textContent = CAPTIONS[e.detail.state] ?? e.detail.state;
});
pipeline.addEventListener("output-level", (e) => {
  visual.setOutput(e.detail.rms, e.detail.waveform);
});
pipeline.addEventListener("input-level", (e) => {
  if (pipeline.state !== "listening" && pipeline.state !== "user_speaking") return;
  visual.setInput(e.detail.rms, e.detail.waveform);
});
pipeline.addEventListener("log", (e) => console.log("[pipeline]", e.detail.msg));
pipeline.addEventListener("transcript", (e) => addMsg(e.detail.role, e.detail.text));
pipeline.addEventListener("task", (e) => taskToast.update(e.detail));

orb.addEventListener("click", async () => {
  try {
    setNotice("");
    await pipeline.toggle();
  } catch (err) {
    setNotice(errorText(err), true);
  }
});

void listen("settings-changed", async () => {
  try {
    await loadSettings();
  } catch {
    /* ignore */
  }
});

void loadSettings();
