import { VoicePipeline } from "./lib/pipeline.js";
import { getSettings, delegate, listen } from "./lib/tauri.js";
import { StormOrb } from "./lib/storm-orb.js";
import { TaskToast } from "./lib/task-toast.js";
import { element } from "./lib/dom.js";
import { ensureVoiceBackend, keepVoiceBackendAlive } from "./lib/backend-lifecycle.js";
import { errorText } from "./lib/errors.js";
import type { PipelineState, TranscriptDetail } from "./lib/types.js";

const orb = element("orb");
const caption = element("caption");
const notice = element("notice");
const feed = element("transcript");
const visual = new StormOrb(orb);
const taskToast = new TaskToast();

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
  delegate: (r) => delegate(r),
  ensureBackend: ensureVoiceBackend,
  backendActivity: keepVoiceBackendAlive,
});

async function loadSettings(): Promise<void> {
  const settings = await getSettings();
  if (!settings.setup_completed) {
    location.replace("/settings?setup=1");
    return;
  }
  pipeline.configure(settings);
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

function addMsg(role: TranscriptDetail["role"], text: string): void {
  const cls = role === "user" ? "user" : role === "tool" ? "tool" : "assistant";
  const el = document.createElement("div");
  el.className = `msg ${cls}`;
  const who = document.createElement("span");
  who.className = "who";
  who.textContent = role;
  el.append(who, document.createTextNode(text));
  feed.append(el);
  while (feed.children.length > 40) feed.firstChild?.remove();
  el.scrollIntoView({ block: "nearest" });
}

async function toggle(): Promise<void> {
  try {
    await pipeline.toggle();
  } catch (err) {
    console.error(err);
    notice.hidden = false;
    notice.textContent = errorText(err);
  }
}

orb.addEventListener("click", () => void toggle());
orb.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    void toggle();
  }
});

void listen("settings-changed", async () => {
  const wasRunning = pipeline.running;
  await loadSettings();
  if (wasRunning) {
    pipeline.stop();
    setTimeout(() => pipeline.start().catch((e: unknown) => console.error(e)), 250);
  }
});
void listen("speech-toggle", () => void toggle());

await loadSettings();

if (!window.isSecureContext && location.hostname !== "localhost" && location.hostname !== "127.0.0.1") {
  notice.hidden = false;
  notice.textContent =
    "Microphone needs a secure context. Open this over HTTPS (set a TLS cert in Settings → Web) " +
    "or via an SSH tunnel to localhost.";
}
