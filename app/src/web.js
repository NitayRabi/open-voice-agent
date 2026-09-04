import { VoicePipeline } from "./lib/pipeline.js";
import { getSettings, delegate, listen } from "./lib/tauri.js";
import { StormOrb } from "./lib/storm-orb.js";

const orb = document.getElementById("orb");
const caption = document.getElementById("caption");
const notice = document.getElementById("notice");
const feed = document.getElementById("transcript");
const settingsLink = document.getElementById("settings-link");
const visual = new StormOrb(orb);

const CAPTIONS = {
  idle: "tap to talk",
  connecting: "connecting…",
  listening: "listening",
  user_speaking: "…",
  thinking: "thinking",
  speaking: "speaking",
  delegating: "delegating…",
  error: "disconnected — tap to retry",
};

const pipeline = new VoicePipeline({ delegate: (r) => delegate(r) });
let settings = null;

async function loadSettings() {
  settings = await getSettings();
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

function addMsg(role, text) {
  const cls = role === "user" ? "user" : role === "tool" ? "tool" : "assistant";
  const el = document.createElement("div");
  el.className = `msg ${cls}`;
  const who = document.createElement("span");
  who.className = "who";
  who.textContent = role;
  el.append(who, document.createTextNode(text));
  feed.append(el);
  while (feed.children.length > 40) feed.firstChild.remove();
  el.scrollIntoView({ block: "nearest" });
}

async function toggle() {
  try {
    await pipeline.toggle();
  } catch (err) {
    console.error(err);
    notice.hidden = false;
    notice.textContent = String(err.message || err);
  }
}

orb.addEventListener("click", toggle);
orb.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    toggle();
  }
});

listen("settings-changed", async () => {
  const wasRunning = pipeline.running;
  await loadSettings();
  if (wasRunning) {
    pipeline.stop();
    setTimeout(() => pipeline.start().catch((e) => console.error(e)), 250);
  }
});
listen("speech-toggle", () => toggle());

await loadSettings();

if (!window.isSecureContext && location.hostname !== "localhost" && location.hostname !== "127.0.0.1") {
  notice.hidden = false;
  notice.textContent =
    "Microphone needs a secure context. Open this over HTTPS (set a TLS cert in Settings → Web) " +
    "or via an SSH tunnel to localhost.";
}
