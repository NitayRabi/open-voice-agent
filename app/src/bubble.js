import { VoicePipeline } from "./lib/pipeline.js";
import { getSettings, delegate, listen, emit, invoke, currentWindow, hasTauri } from "./lib/tauri.js";

const orb = document.getElementById("orb");
const gear = document.getElementById("gear");
const caption = document.getElementById("caption");

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

const pipeline = new VoicePipeline({
  delegate: (request) => delegate(request),
});

let settings = null;
async function loadSettings() {
  settings = await getSettings();
  pipeline.configure(settings);
}

function setCaption(state) {
  caption.textContent = CAPTIONS[state] ?? state;
}

pipeline.addEventListener("state", (e) => {
  const { state } = e.detail;
  orb.className = `orb state-${state}${pipeline._muted ? " state-muted" : ""}`;
  setCaption(state);
});
pipeline.addEventListener("output-level", (e) => {
  const rms = Math.min(1, e.detail.rms * 4);
  orb.style.setProperty("--lvl", rms.toFixed(3));
  if (pipeline.state === "speaking") {
    orb.querySelector(".ring-mid").style.opacity = String(0.2 + rms * 0.8);
    orb.querySelector(".ring-mid").style.transform = `scale(${1 + rms * 0.25})`;
  }
});
pipeline.addEventListener("input-level", (e) => {
  if (pipeline.state === "listening" || pipeline.state === "user_speaking") {
    const rms = Math.min(1, e.detail.rms * 6);
    const mid = orb.querySelector(".ring-mid");
    mid.style.opacity = String(0.15 + rms);
    mid.style.transform = `scale(${1 + rms * 0.2})`;
  }
});
pipeline.addEventListener("log", (e) => {
  console.log("[pipeline]", e.detail.msg);
});
pipeline.addEventListener("transcript", (e) => {
  console.log("[transcript]", e.detail.role, e.detail.text);
  // forward for the settings window's live log
  emit("ova-transcript", e.detail).catch(() => {});
});

// ── input handling: quick tap = toggle, drag = move window ────────────────

let down = null;
orb.addEventListener("pointerdown", (ev) => {
  if (ev.target.closest("#gear")) return;
  down = { x: ev.clientX, y: ev.clientY, t: Date.now(), dragging: false };
});
orb.addEventListener("pointermove", (ev) => {
  if (!down || down.dragging) return;
  if (Math.hypot(ev.clientX - down.x, ev.clientY - down.y) > 6) {
    down.dragging = true;
    currentWindow()?.startDragging?.();
  }
});
orb.addEventListener("pointerup", async (ev) => {
  if (ev.target.closest("#gear")) return;
  const d = down;
  down = null;
  if (!d || d.dragging) return;
  if (Date.now() - d.t < 500) {
    try {
      await pipeline.toggle();
    } catch (err) {
      console.error(err);
    }
  }
});

gear.addEventListener("click", (ev) => {
  ev.stopPropagation();
  invoke("show_settings").catch((e) => console.error(e));
});
orb.addEventListener("contextmenu", (ev) => {
  ev.preventDefault();
  invoke("show_settings").catch(() => {});
});

// ── wiring to the Rust side ──────────────────────────────────────────────

listen("speech-toggle", async () => {
  try {
    await pipeline.toggle();
  } catch (e) {
    console.error(e);
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

await loadSettings();
setCaption("idle");

if (settings?.autostart_listening) {
  // give the backend a moment, then try; errors just leave the orb idle
  setTimeout(() => pipeline.start().catch(() => {}), 1200);
}

if (!hasTauri) {
  caption.textContent = "browser preview";
}
