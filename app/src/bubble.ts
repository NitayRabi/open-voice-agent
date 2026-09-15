import { VoicePipeline } from "./lib/pipeline.js";
import { getSettings, delegate, listen, emit, invoke, currentWindow, hasTauri } from "./lib/tauri.js";
import { StormOrb } from "./lib/storm-orb.js";
import { TaskToast } from "./lib/task-toast.js";
import { element } from "./lib/dom.js";
import { ensureVoiceBackend, keepVoiceBackendAlive } from "./lib/backend-lifecycle.js";
import type { PipelineState, Settings } from "./lib/types.js";

const orb = element("orb");
const gear = element("gear");
const caption = element("caption");
const visual = new StormOrb(orb);
const taskToast = new TaskToast({ compact: true });

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
  delegate: (request) => delegate(request),
  ensureBackend: ensureVoiceBackend,
  backendActivity: keepVoiceBackendAlive,
});

async function loadSettings(): Promise<Settings> {
  const settings = await getSettings();
  pipeline.configure(settings);
  return settings;
}

function setCaption(state: PipelineState): void {
  caption.textContent = CAPTIONS[state] ?? state;
}

pipeline.addEventListener("state", (e) => {
  const { state } = e.detail;
  orb.className = `orb state-${state}${pipeline.muted ? " state-muted" : ""}${visual.gl ? "" : " orb-webgl-fallback"}`;
  visual.setState(state);
  setCaption(state);
});
pipeline.addEventListener("output-level", (e) => {
  visual.setOutput(e.detail.rms, e.detail.waveform);
});
pipeline.addEventListener("input-level", (e) => {
  if (pipeline.state === "listening" || pipeline.state === "user_speaking") {
    visual.setInput(e.detail.rms, e.detail.waveform);
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
pipeline.addEventListener("task", (e) => taskToast.update(e.detail));

// ── input handling: quick tap = toggle, drag = move window ────────────────

interface PointerOrigin {
  x: number;
  y: number;
  t: number;
  dragging: boolean;
}

const onGear = (target: EventTarget | null): boolean =>
  target instanceof Element && !!target.closest("#gear");

let down: PointerOrigin | null = null;
orb.addEventListener("pointerdown", (ev) => {
  if (onGear(ev.target)) return;
  down = { x: ev.clientX, y: ev.clientY, t: Date.now(), dragging: false };
});
orb.addEventListener("pointermove", (ev) => {
  if (!down || down.dragging) return;
  if (Math.hypot(ev.clientX - down.x, ev.clientY - down.y) > 6) {
    down.dragging = true;
    void currentWindow()?.startDragging?.();
  }
});
orb.addEventListener("pointerup", async (ev) => {
  if (onGear(ev.target)) return;
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
  invoke("show_settings").catch((e: unknown) => console.error(e));
});
orb.addEventListener("contextmenu", (ev) => {
  ev.preventDefault();
  invoke("show_settings").catch(() => {});
});

// ── wiring to the Rust side ──────────────────────────────────────────────

void listen("speech-toggle", async () => {
  try {
    await pipeline.toggle();
  } catch (e) {
    console.error(e);
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

const settings = await loadSettings();
setCaption("idle");

if (settings.autostart_listening) {
  // give the backend a moment, then try; errors just leave the orb idle
  setTimeout(() => pipeline.start().catch(() => {}), 1200);
}

if (!hasTauri) {
  caption.textContent = "browser preview";
}
