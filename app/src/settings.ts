import { getSettings, saveSettings, invoke, listen, isWeb } from "./lib/tauri.js";
import { element as $, input as $in, select as $sel, valueElement } from "./lib/dom.js";
import { errorText } from "./lib/errors.js";
import type { ModelInfo, ModelSource, ModelsList, Settings } from "./lib/types.js";

const logEl = $("log");
const statusEl = $("status");

function newPairingCode(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(10));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** How a settings field is stored in the DOM, and how it round-trips to JSON. */
type FieldKind = "value" | "int" | "float" | "checked";

const FIELDS = {
  speech_model: "value",
  speech_model_source: "value",
  speech_remote_base_url: "value",
  speech_remote_model: "value",
  speech_remote_api_key: "value",
  voice: "value",
  sample_rate: "int",
  mic_device_id: "value",
  noise_gate_db: "int",
  autostart_listening: "checked",
  app_autostart: "checked",
  instructions: "value",
  hotkey: "value",
  delegation_enabled: "checked",
  brain_source: "value",
  brain_base_url: "value",
  brain_api_key: "value",
  brain_model: "value",
  brain_local_model: "value",
  llama_server_bin: "value",
  brain_local_port: "int",
  brain_local_ctx: "int",
  brain_local_ngl: "int",
  hf_token: "value",
  brain_system_prompt: "value",
  brain_temperature: "float",
  delegation_tool_name: "value",
  delegation_tool_description: "value",
  delegation_timeout_s: "int",
  delegation_speak_result: "checked",
  web_enabled: "checked",
  web_bind: "value",
  web_port: "int",
  web_token: "value",
  web_tls_cert: "value",
  web_tls_key: "value",
  web_tailscale: "checked",
  web_tailscale_binary: "value",
} as const satisfies Partial<Record<keyof Settings, FieldKind>>;

type FieldId = keyof typeof FIELDS;
const fieldEntries = Object.entries(FIELDS) as [FieldId, FieldKind][];

function applyToForm(s: Settings): void {
  for (const [id, kind] of fieldEntries) {
    const el = valueElement(id);
    if (!el) continue;
    const v = s[id];
    if (kind === "checked") (el as HTMLInputElement).checked = !!v;
    else el.value = v == null ? "" : String(v);
  }
  updateGateLabel();
  updateBrainVisibility();
  updateSpeechVisibility();
  $in("speech_model_path").value = s.speech_model?.includes("/") ? s.speech_model : "";
}

function readForm(base: Settings): Settings {
  const s: Settings = { ...base };
  // The field kind decides the JSON type, so the write is keyed dynamically.
  const out = s as unknown as Record<FieldId, unknown>;
  for (const [id, kind] of fieldEntries) {
    const el = valueElement(id);
    if (!el) continue;
    if (kind === "checked") out[id] = (el as HTMLInputElement).checked;
    else if (kind === "int" || kind === "float") out[id] = Number(el.value);
    else out[id] = el.value;
  }
  return s;
}

function updateGateLabel(): void {
  const v = Number($in("noise_gate_db").value);
  $("gate_val").textContent = v <= -100 ? "off" : `${v} dB`;
}
function updateBrainVisibility(): void {
  const src = $sel("brain_source").value;
  document.querySelectorAll<HTMLElement>("[data-brain]").forEach((el) => {
    el.classList.toggle("show", el.dataset.brain === src);
  });
}
function updateSpeechVisibility(): void {
  const src = $sel("speech_model_source").value || "local";
  document.querySelectorAll<HTMLElement>("[data-speech-source]").forEach((el) => {
    el.classList.toggle("show", el.dataset.speechSource === src);
  });
}

// ── tabs ────────────────────────────────────────────────────────────────
document.querySelectorAll<HTMLButtonElement>("nav.tabs button").forEach((b) => {
  b.addEventListener("click", () => {
    document.querySelectorAll("nav.tabs button").forEach((x) => x.classList.remove("active"));
    document.querySelectorAll(".tab").forEach((x) => x.classList.remove("active"));
    b.classList.add("active");
    $(`tab-${b.dataset.tab}`).classList.add("active");
  });
});

$in("noise_gate_db").addEventListener("input", updateGateLabel);
$sel("brain_source").addEventListener("change", updateBrainVisibility);
$sel("speech_model_source").addEventListener("change", updateSpeechVisibility);
$sel("speech_model").addEventListener("change", () => ($in("speech_model_path").value = ""));

$("generate_web_token").addEventListener("click", () => {
  $in("web_token").value = newPairingCode();
  $in("web_token").type = "text";
  flash("new pairing code generated — Save to apply it");
});

// ── state ───────────────────────────────────────────────────────────────
let current = {} as Settings;
let flashTimer: ReturnType<typeof setTimeout> | undefined;

function flash(msg: string): void {
  statusEl.textContent = msg;
  clearTimeout(flashTimer);
  flashTimer = setTimeout(() => (statusEl.textContent = ""), 4000);
}

function appendLog(line: string): void {
  const at = logEl.scrollTop + logEl.clientHeight >= logEl.scrollHeight - 8;
  logEl.textContent += (logEl.textContent ? "\n" : "") + line;
  if ($in("autoscroll").checked || at) logEl.scrollTop = logEl.scrollHeight;
}

async function save(): Promise<Settings> {
  if ($in("web_enabled").checked && $sel("web_bind").value !== "127.0.0.1" && !$in("web_token").value.trim()) {
    $in("web_token").value = newPairingCode();
    $in("web_token").type = "text";
  }
  current = readForm(current);
  if (current.speech_model_source === "remote") {
    if (!current.speech_remote_base_url.trim() || !current.speech_remote_model.trim()) {
      throw new Error("remote conversational model requires an endpoint and model name");
    }
  } else if ($in("speech_model_path").value.trim()) {
    current.speech_model = $in("speech_model_path").value.trim();
  }
  await saveSettings(current);
  flash("saved");
  return current;
}

$("save").addEventListener("click", () => void save().catch((e: unknown) => flash(`save failed: ${errorText(e)}`)));
$("clear_log").addEventListener("click", () => (logEl.textContent = ""));

$("web_open").addEventListener("click", async () => {
  try {
    await save();
    const url = await invoke("web_url").catch(() => null);
    if (url && url.includes("<this-machine-ip>")) {
      flash(`web server on port ${$in("web_port").value} — reach it at http(s)://<this-host>:${$in("web_port").value}/`);
    } else if (url) {
      window.open(url, "_blank");
    } else {
      flash("enable the web server, Save, then try again");
    }
  } catch (e) {
    flash(`failed: ${errorText(e)}`);
  }
});

$("test_delegate").addEventListener("click", async () => {
  const req = prompt("Delegation request to test:");
  if (!req) return;
  $("test_delegate_out").textContent = "running…";
  try {
    const ans = await invoke("delegate", { request: req });
    $("test_delegate_out").textContent = ans;
  } catch (e) {
    $("test_delegate_out").textContent = `error: ${errorText(e)}`;
  }
});

// ── backend events ──────────────────────────────────────────────────────
void listen("backend-log", (e) => appendLog(e.payload));
void listen("ova-transcript", (e) => {
  if (!e.payload) return;
  appendLog(`  ${e.payload.role}: ${e.payload.text}`);
});
void listen("web-status", (e) => {
  $("web_state").textContent = e.payload ? "running" : "stopped";
});

// ── models ──────────────────────────────────────────────────────────────
const fmtBytes = (n: number | null | undefined): string => {
  if (!n) return "";
  const u = ["B", "KiB", "MiB", "GiB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i ? 1 : 0)} ${u[i]}`;
};

function escapeHtml(value: string): string {
  const span = document.createElement("span");
  span.textContent = value;
  return span.innerHTML;
}

function badges(model: ModelInfo): string {
  return (model.badges || []).map((badge) => badge === "recommended"
    ? '<span class="model-badge recommended">Recommended</span>'
    : badge === "smallest" ? '<span class="model-badge smallest">Smallest</span>' : "").join("");
}

function modelLabel(model: ModelInfo): string {
  const labels = (model.badges || []).map((badge) => badge === "recommended" ? "Recommended" : "Smallest");
  return `${model.name}${labels.length ? ` (${labels.join(", ")})` : ""}`;
}

let modelsCache: ModelsList = { dir: null, models: [] };
let refreshTimer: ReturnType<typeof setTimeout> | undefined;

async function renderModels(): Promise<void> {
  let data: ModelsList;
  try {
    data = await invoke("models_list");
  } catch (e) {
    $("model_list").textContent = `error: ${errorText(e)}`;
    return;
  }
  modelsCache = data;
  const box = $("model_list");
  box.innerHTML = "";
  for (const m of data.models) {
    const row = document.createElement("div");
    row.className = "model-row";
    const pct = m.job && m.job.total ? Math.round((m.job.downloaded / m.job.total) * 100) : 0;
    const status = m.installed
      ? `<span class="tag">installed ${fmtBytes(m.bytes_on_disk)}</span>`
      : m.downloading
        ? `<span class="tag off">downloading ${pct}%</span>`
        : `<span class="tag off">${fmtBytes(m.bytes) || "not installed"}</span>`;
    row.innerHTML = `
      <div class="top"><span class="nm">${escapeHtml(m.name)}</span>${badges(m)}${status}</div>
      <div class="meta">${escapeHtml([m.license, m.gated ? "gated" : "", m.note].filter(Boolean).join(" · "))}</div>
      ${m.downloading ? `<div class="bar"><i style="width:${pct}%"></i></div>` : ""}
      <div class="actions"></div>`;
    const actions = row.querySelector(".actions")!;
    const btn = (label: string, cls: string, fn: () => void) => {
      const b = document.createElement("button");
      b.className = cls;
      b.textContent = label;
      b.onclick = fn;
      actions.appendChild(b);
    };
    if (m.downloading) {
      btn("Cancel", "secondary", () => void invoke("model_cancel", { id: m.id }));
    } else if (m.installed) {
      btn("Use as local brain", "secondary", () => {
        $sel("brain_source").value = "local";
        updateBrainVisibility();
        populateLocalModels();
        $sel("brain_local_model").value = m.id;
        document.querySelector<HTMLButtonElement>('nav.tabs button[data-tab="delegation"]')?.click();
      });
      btn("Delete", "secondary", async () => {
        await invoke("model_remove", { id: m.id });
        void renderModels();
      });
    } else if (m.downloadable) {
      btn("Download", "secondary", () =>
        void invoke("model_download", { id: m.id }).catch((e: unknown) => flash(errorText(e))));
    }
    if (m.user) {
      btn("Forget", "secondary", async () => {
        await invoke("model_forget", { id: m.id });
        void renderModels();
      });
    }
    box.appendChild(row);
  }
  populateLocalModels();
  populateSpeechModels();
  populateSetupModels();
}

function option(select: HTMLSelectElement, value: string, label: string): void {
  const o = document.createElement("option");
  o.value = value;
  o.textContent = label;
  select.appendChild(o);
}

function populateSpeechModels(): void {
  const select = $sel("speech_model");
  const chosen = select.value || current.speech_model || "";
  const compatible = modelsCache.models.filter((m) => m.roles?.includes("cascade_llm") && (m.downloadable || m.installed));
  select.innerHTML = "";
  for (const m of compatible) {
    option(select, m.id, `${modelLabel(m)}${m.installed ? " — installed" : " — download required"}`);
  }
  if (chosen && !compatible.some((m) => m.id === chosen)) {
    option(select, chosen, chosen.includes("/") ? chosen : `${chosen} (not installed)`);
  }
  if (!select.options.length) option(select, "", "No compatible models available");
  select.value = chosen;
}

function populateLocalModels(): void {
  const sel = $sel("brain_local_model");
  const chosen = sel.value || current.brain_local_model || "";
  const installed = modelsCache.models.filter((m) => m.installed);
  sel.innerHTML = "";
  for (const m of installed) {
    option(sel, m.id, modelLabel(m));
  }
  // allow an arbitrary path that isn't in the catalog
  if (chosen && !installed.some((m) => m.id === chosen)) {
    option(sel, chosen, chosen.includes("/") ? chosen : `${chosen} (missing)`);
  }
  if (!installed.length && !chosen) {
    option(sel, "", "— download one in the Models tab —");
  }
  sel.value = chosen;
}

// ── first-run setup ────────────────────────────────────────────────────
let setupStep = 0;

function populateSetupModels(): void {
  const speech = $sel("setup_speech_model");
  const configuredSpeech = current.speech_model?.includes("/") ? "" : current.speech_model;
  const compatible = modelsCache.models.filter((m) => m.roles?.includes("cascade_llm") && (m.downloadable || m.installed));
  const requested = speech.value || configuredSpeech;
  const previousSpeech = compatible.find((m) => m.id === requested)?.id
    || compatible.find((m) => m.badges?.includes("recommended"))?.id
    || compatible[0]?.id || "";
  speech.innerHTML = "";
  for (const m of compatible) {
    option(speech, m.id, `${modelLabel(m)}${m.installed ? " — installed" : " — download required"}`);
  }
  speech.value = previousSpeech;

  const brain = $sel("setup_brain_model");
  const previousBrain = brain.value || current.brain_local_model || "";
  brain.innerHTML = "";
  for (const m of modelsCache.models.filter((m) => m.installed && m.roles?.includes("brain"))) {
    option(brain, m.id, modelLabel(m));
  }
  if (!brain.options.length) option(brain, "", "No local model installed");
  brain.value = previousBrain || brain.options[0]?.value || "";

  const selected = modelsCache.models.find((m) => m.id === speech.value);
  $("setup_model_badges").innerHTML = selected ? badges(selected) : "";
  ($("setup_download_speech") as HTMLButtonElement).disabled = !selected?.downloadable || selected.installed || selected.downloading;
  if (selected?.installed) {
    $("setup_speech_status").textContent = `Installed (${fmtBytes(selected.bytes_on_disk)})`;
  } else if (selected?.downloading) {
    const pct = selected.job?.total ? Math.round((selected.job.downloaded / selected.job.total) * 100) : 0;
    $("setup_speech_status").textContent = `Downloading… ${pct}%`;
  } else {
    $("setup_speech_status").textContent = "Not installed";
  }
}

function setSetupStep(step: number): void {
  setupStep = step;
  document.querySelectorAll<HTMLElement>(".setup-step").forEach((el) => {
    el.classList.toggle("active", Number(el.dataset.setupStep) === step);
  });
  document.querySelectorAll(".setup-progress i").forEach((el, i) => {
    el.classList.toggle("active", i <= step);
  });
  $("setup_back").hidden = step === 0;
  $("setup_next").textContent = step === 0 ? "Get started" : step === 3 ? "Finish & start" : "Continue";
  $("setup_error").hidden = true;
  if (step === 3) renderSetupReview();
}

function showSetup(show: boolean): void {
  $("setup").hidden = !show;
  document.querySelectorAll<HTMLElement>("body > header, body > nav, body > main, body > footer").forEach((el) => {
    el.inert = show;
  });
}

function setupError(message: string): void {
  $("setup_error").textContent = message;
  $("setup_error").hidden = false;
}

function selectedSpeechModel(): string {
  return $in("setup_speech_path").value.trim() || $sel("setup_speech_model").value;
}

function syncSetupSpeech(): void {
  const local = $sel("setup_speech_source").value !== "remote";
  $("setup_speech_local").hidden = !local;
  $("setup_speech_remote").hidden = local;
}

function selectedBrainModel(): string {
  return $in("setup_brain_path").value.trim() || $sel("setup_brain_model").value;
}

function syncSetupBrain(): void {
  const local = $sel("setup_brain_source").value === "local";
  $("setup_brain_remote").hidden = local;
  $("setup_brain_local").hidden = !local;
}

function initSetupForm(): void {
  $sel("setup_speech_source").value = current.speech_model_source || "local";
  $in("setup_speech_path").value = current.speech_model?.includes("/") ? current.speech_model : "";
  $in("setup_speech_remote_url").value = current.speech_remote_base_url || "";
  $in("setup_speech_remote_name").value = current.speech_remote_model || "";
  $in("setup_speech_remote_key").value = current.speech_remote_api_key || "";
  $sel("setup_brain_source").value = current.brain_source || "remote";
  $in("setup_brain_url").value = current.brain_base_url || "";
  $in("setup_brain_name").value = current.brain_model || "";
  $in("setup_brain_key").value = current.brain_api_key || "";
  $in("setup_brain_path").value = current.brain_local_model?.includes("/") ? current.brain_local_model : "";
  $in("setup_llama_server").value = current.llama_server_bin || "";
  syncSetupBrain();
  syncSetupSpeech();
  populateSetupModels();
  setSetupStep(0);
}

async function validateSetupStep(): Promise<void> {
  if (setupStep === 1) {
    if ($sel("setup_speech_source").value === "remote") {
      if (!$in("setup_speech_remote_url").value.trim()) throw new Error("Enter the conversational model endpoint.");
      if (!$in("setup_speech_remote_name").value.trim()) throw new Error("Enter the model name expected by that endpoint.");
    } else {
      const selected = selectedSpeechModel();
      if (!selected) throw new Error("Choose or download a conversational model.");
      const path = await invoke("model_resolve", { idOrPath: selected });
      if (!path) throw new Error("That conversational model is not available yet. Download it or open Advanced and enter an existing GGUF path.");
    }
  }
  if (setupStep === 2) {
    if ($sel("setup_brain_source").value === "remote") {
      if (!$in("setup_brain_url").value.trim()) throw new Error("Enter the capable model endpoint.");
      if (!$in("setup_brain_name").value.trim()) throw new Error("Enter the model name expected by that endpoint.");
    } else {
      const id = selectedBrainModel();
      if (!id || !(await invoke("model_resolve", { idOrPath: id }))) {
        throw new Error("Install and choose a local model for delegation.");
      }
    }
  }
}

function renderSetupReview(): void {
  const review = $("setup_review");
  review.innerHTML = "";
  const speech = modelsCache.models.find((m) => m.id === selectedSpeechModel());
  const speechDescription = $sel("setup_speech_source").value === "remote"
    ? `${$in("setup_speech_remote_name").value} at ${$in("setup_speech_remote_url").value}`
    : (speech?.name || selectedSpeechModel());
  const items: [string, string | null | undefined][] = [
    ["Voice pipeline", "Parakeet → conversational model → speech"],
    ["Conversational model", speechDescription],
    ["Delegation", $sel("setup_brain_source").value === "local"
      ? (selectedBrainModel().includes("/") ? selectedBrainModel() : $sel("setup_brain_model").selectedOptions[0]?.textContent)
      : `${$in("setup_brain_name").value} at ${$in("setup_brain_url").value}`],
  ];
  for (const [label, value] of items) {
    const row = document.createElement("div");
    const strong = document.createElement("strong");
    strong.textContent = label;
    const span = document.createElement("span");
    span.textContent = value || "Not configured";
    row.append(strong, span);
    review.appendChild(row);
  }
}

$sel("setup_brain_source").addEventListener("change", syncSetupBrain);
$sel("setup_speech_source").addEventListener("change", syncSetupSpeech);
$sel("setup_speech_model").addEventListener("change", () => {
  $in("setup_speech_path").value = "";
  populateSetupModels();
});
$sel("setup_brain_model").addEventListener("change", () => ($in("setup_brain_path").value = ""));
$("setup_download_speech").addEventListener("click", async () => {
  try {
    const id = $sel("setup_speech_model").value;
    if (!id) throw new Error("Choose a model first.");
    await invoke("model_download", { id });
    $("setup_speech_status").textContent = "Starting download…";
    await renderModels();
  } catch (e) {
    setupError(errorText(e));
  }
});
$("setup_back").addEventListener("click", () => setSetupStep(Math.max(0, setupStep - 1)));
$("setup_next").addEventListener("click", async () => {
  try {
    await validateSetupStep();
    if (setupStep < 3) {
      setSetupStep(setupStep + 1);
      return;
    }

    current.speech_model_source = $sel("setup_speech_source").value as ModelSource;
    if (current.speech_model_source === "remote") {
      current.speech_remote_base_url = $in("setup_speech_remote_url").value.trim();
      current.speech_remote_model = $in("setup_speech_remote_name").value.trim();
      current.speech_remote_api_key = $in("setup_speech_remote_key").value.trim();
    } else {
      current.speech_model = selectedSpeechModel();
    }
    current.delegation_enabled = true;
    current.brain_source = $sel("setup_brain_source").value as ModelSource;
    if (current.brain_source === "remote") {
      current.brain_base_url = $in("setup_brain_url").value.trim();
      current.brain_model = $in("setup_brain_name").value.trim();
      current.brain_api_key = $in("setup_brain_key").value.trim();
    } else {
      current.brain_local_model = selectedBrainModel();
      current.llama_server_bin = $in("setup_llama_server").value.trim();
    }
    current.server_url = "ws://127.0.0.1:8766/v1/realtime";
    current.launch_command = ["bash", "hf-s2s/run-comparison.sh"];
    current.health_url = "http://127.0.0.1:8766/";
    current.manage_backend = true;
    current.setup_completed = true;
    await saveSettings(current);
    try {
      await invoke("backend_start");
    } catch (e) {
      current.setup_completed = false;
      await saveSettings(current);
      throw new Error(`The voice engine could not start: ${errorText(e)}`);
    }
    showSetup(false);
    applyToForm(current);
    if (isWeb) location.replace("/");
  } catch (e) {
    setupError(errorText(e));
  }
});

void listen("asset-progress", () => {
  clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => void renderModels(), 400);
});

async function refreshWebState(): Promise<void> {
  try {
    const url = await invoke("web_url").catch(() => null);
    $("web_state").textContent = url ? "running" : "stopped";
  } catch {}
}

async function renderDevices(): Promise<void> {
  const box = $("device_list");
  try {
    const devices = await invoke("paired_devices");
    box.innerHTML = "";
    if (!devices.length) { box.textContent = "No paired devices."; return; }
    for (const device of devices) {
      const row = document.createElement("div"); row.className = "device-row";
      const info = document.createElement("div");
      info.innerHTML = `<strong>${escapeHtml(device.label)}</strong><div class="meta">${escapeHtml(device.type)} · added ${new Date(device.created_at).toLocaleString()}</div>`;
      const revoke = document.createElement("button"); revoke.className = "secondary"; revoke.textContent = "Revoke";
      revoke.onclick = () => void invoke("revoke_device", { id: device.id }).then(renderDevices).catch((e: unknown) => flash(errorText(e)));
      row.append(info, revoke); box.appendChild(row);
    }
  } catch (e) { box.textContent = `Unavailable: ${errorText(e)}`; }
}

$("issue_device").addEventListener("click", () => void (async () => {
  try {
    const label = $in("device_label").value.trim() || "Paired device";
    const issued = await invoke("issue_device", { deviceType: $sel("device_type").value, label });
    const origin = (await invoke("web_url").catch(() => null)) || "http(s)://this-host";
    const link = `${origin.replace(/\/$/, "")}/c#${issued.access_token}`;
    const out = $("issued_device"); out.hidden = false;
    out.innerHTML = `Connection link (shown once):<br><code>${escapeHtml(link)}</code>`;
    await navigator.clipboard?.writeText(link).catch(() => undefined);
    flash("device connection copied — treat it like a password"); await renderDevices();
  } catch (e) { flash(`could not create device: ${errorText(e)}`); }
})());

// ── mic devices ─────────────────────────────────────────────────────────
async function loadMics(): Promise<void> {
  try {
    // a transient getUserMedia unlocks device labels
    const tmp = await navigator.mediaDevices.getUserMedia({ audio: true }).catch(() => null);
    const devices = await navigator.mediaDevices.enumerateDevices();
    const sel = $sel("mic_device_id");
    const chosen = current.mic_device_id || "";
    sel.innerHTML = '<option value="">System default</option>';
    for (const d of devices.filter((x) => x.kind === "audioinput")) {
      option(sel, d.deviceId, d.label || `microphone ${sel.length}`);
    }
    sel.value = chosen;
    if (tmp) tmp.getTracks().forEach((t) => t.stop());
  } catch (e) {
    console.warn("mic enum failed", e);
  }
}

// ── init ────────────────────────────────────────────────────────────────
// Browser first-run redirects include this flag, so cover the settings UI
// before any API or device-enumeration work can paint underneath it.
if (new URLSearchParams(location.search).get("setup") === "1") showSetup(true);

current = await getSettings();
applyToForm(current);
const shouldShowSetup = !current.setup_completed
  || new URLSearchParams(location.search).get("setup") === "1";
if (shouldShowSetup) showSetup(true);
await renderModels();
applyToForm(current); // re-apply brain_local_model now that the dropdown is populated
initSetupForm();
// Device enumeration can wait on browser permission UI. Never hold the
// settings/setup screen behind that prompt.
void refreshWebState();
void renderDevices();
void loadMics();
$("version").textContent = "v" + ((await invoke("app_version").catch(() => "")) || "");
if (isWeb) {
  const back = document.createElement("a");
  back.href = "./";
  back.textContent = "← Voice";
  back.style.cssText = "color:#9db8ff;text-decoration:none;font-size:.85rem;margin-left:.6rem";
  $("version").after(back);
}
