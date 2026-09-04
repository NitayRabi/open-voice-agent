import { getSettings, saveSettings, invoke, listen, isWeb } from "./lib/tauri.js";

const $ = (id) => document.getElementById(id);
const logEl = $("log");
const statusEl = $("status");

function newPairingCode() {
  const bytes = crypto.getRandomValues(new Uint8Array(10));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

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
};

function applyToForm(s) {
  for (const [id, kind] of Object.entries(FIELDS)) {
    const el = $(id);
    if (!el) continue;
    const v = s[id];
    if (kind === "checked") el.checked = !!v;
    else el.value = v ?? "";
  }
  updateGateLabel();
  updateBrainVisibility();
  updateSpeechVisibility();
  $("speech_model_path").value = s.speech_model?.includes("/") ? s.speech_model : "";
}

function readForm(base) {
  const s = { ...base };
  for (const [id, kind] of Object.entries(FIELDS)) {
    const el = $(id);
    if (!el) continue;
    if (kind === "checked") s[id] = el.checked;
    else if (kind === "int" || kind === "float") s[id] = Number(el.value);
    else s[id] = el.value;
  }
  return s;
}

function updateGateLabel() {
  const v = Number($("noise_gate_db").value);
  $("gate_val").textContent = v <= -100 ? "off" : `${v} dB`;
}
function updateBrainVisibility() {
  const src = $("brain_source").value;
  document.querySelectorAll("[data-brain]").forEach((el) => {
    el.classList.toggle("show", el.dataset.brain === src);
  });
}
function updateSpeechVisibility() {
  const src = $("speech_model_source").value || "local";
  document.querySelectorAll("[data-speech-source]").forEach((el) => {
    el.classList.toggle("show", el.dataset.speechSource === src);
  });
}

// ── tabs ────────────────────────────────────────────────────────────────
document.querySelectorAll("nav.tabs button").forEach((b) => {
  b.addEventListener("click", () => {
    document.querySelectorAll("nav.tabs button").forEach((x) => x.classList.remove("active"));
    document.querySelectorAll(".tab").forEach((x) => x.classList.remove("active"));
    b.classList.add("active");
    $(`tab-${b.dataset.tab}`).classList.add("active");
  });
});

$("noise_gate_db").addEventListener("input", updateGateLabel);
$("brain_source").addEventListener("change", updateBrainVisibility);
$("speech_model_source").addEventListener("change", updateSpeechVisibility);
$("speech_model").addEventListener("change", () => ($("speech_model_path").value = ""));

$("generate_web_token").addEventListener("click", () => {
  $("web_token").value = newPairingCode();
  $("web_token").type = "text";
  flash("new pairing code generated — Save to apply it");
});

// ── state ───────────────────────────────────────────────────────────────
let current = {};

function flash(msg) {
  statusEl.textContent = msg;
  clearTimeout(flash._t);
  flash._t = setTimeout(() => (statusEl.textContent = ""), 4000);
}

function appendLog(line) {
  const at = logEl.scrollTop + logEl.clientHeight >= logEl.scrollHeight - 8;
  logEl.textContent += (logEl.textContent ? "\n" : "") + line;
  if ($("autoscroll").checked || at) logEl.scrollTop = logEl.scrollHeight;
}

async function save() {
  if ($("web_enabled").checked && $("web_bind").value !== "127.0.0.1" && !$("web_token").value.trim()) {
    $("web_token").value = newPairingCode();
    $("web_token").type = "text";
  }
  current = readForm(current);
  if (current.speech_model_source === "remote") {
    if (!current.speech_remote_base_url.trim() || !current.speech_remote_model.trim()) {
      throw new Error("remote conversational model requires an endpoint and model name");
    }
  } else if ($("speech_model_path").value.trim()) {
    current.speech_model = $("speech_model_path").value.trim();
  }
  await saveSettings(current);
  flash("saved");
  return current;
}

$("save").addEventListener("click", () => save().catch((e) => flash(`save failed: ${e}`)));
$("clear_log").addEventListener("click", () => (logEl.textContent = ""));

$("web_open").addEventListener("click", async () => {
  try {
    await save();
    const url = await invoke("web_url").catch(() => null);
    if (url && url.includes("<this-machine-ip>")) {
      flash(`web server on port ${$("web_port").value} — reach it at http(s)://<this-host>:${$("web_port").value}/`);
    } else if (url) {
      window.open(url, "_blank");
    } else {
      flash("enable the web server, Save, then try again");
    }
  } catch (e) {
    flash(`failed: ${e}`);
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
    $("test_delegate_out").textContent = `error: ${e}`;
  }
});

// ── backend events ──────────────────────────────────────────────────────
listen("backend-log", (e) => appendLog(e.payload));
listen("ova-transcript", (e) => {
  const { role, text } = e.payload || {};
  appendLog(`  ${role}: ${text}`);
});
listen("web-status", (e) => {
  $("web_state").textContent = e.payload ? "running" : "stopped";
});

// ── models ──────────────────────────────────────────────────────────────
const fmtBytes = (n) => {
  if (!n) return "";
  const u = ["B", "KB", "MB", "GB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i ? 1 : 0)} ${u[i]}`;
};

let _modelsCache = { models: [] };
let _refreshTimer = 0;

async function renderModels() {
  let data;
  try {
    data = await invoke("models_list");
  } catch (e) {
    $("model_list").textContent = `error: ${e}`;
    return;
  }
  _modelsCache = data;
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
      <div class="top"><span class="nm">${m.name}</span>${status}</div>
      <div class="meta">${[m.license, m.gated ? "gated" : "", m.note].filter(Boolean).join(" · ")}</div>
      ${m.downloading ? `<div class="bar"><i style="width:${pct}%"></i></div>` : ""}
      <div class="actions"></div>`;
    const actions = row.querySelector(".actions");
    const btn = (label, cls, fn) => {
      const b = document.createElement("button");
      b.className = cls;
      b.textContent = label;
      b.onclick = fn;
      actions.appendChild(b);
    };
    if (m.downloading) {
      btn("Cancel", "secondary", () => invoke("model_cancel", { id: m.id }));
    } else if (m.installed) {
      btn("Use as local brain", "secondary", () => {
        $("brain_source").value = "local";
        updateBrainVisibility();
        populateLocalModels();
        $("brain_local_model").value = m.id;
        document.querySelector('nav.tabs button[data-tab="delegation"]').click();
      });
      btn("Delete", "secondary", async () => {
        await invoke("model_remove", { id: m.id });
        renderModels();
      });
    } else {
      btn("Download", "secondary", () => invoke("model_download", { id: m.id }).catch((e) => flash(String(e))));
    }
    if (m.user) {
      btn("Forget", "secondary", async () => {
        await invoke("model_forget", { id: m.id });
        renderModels();
      });
    }
    box.appendChild(row);
  }
  populateLocalModels();
  populateSpeechModels();
  populateSetupModels();
}

function option(select, value, label) {
  const o = document.createElement("option");
  o.value = value;
  o.textContent = label;
  select.appendChild(o);
}

function populateSpeechModels() {
  const select = $("speech_model");
  const chosen = select.value || current.speech_model || "";
  const compatible = _modelsCache.models.filter((m) => m.roles?.includes("cascade_llm"));
  select.innerHTML = "";
  for (const m of compatible) {
    option(select, m.id, `${m.name}${m.installed ? " — installed" : " — download required"}`);
  }
  if (chosen && !compatible.some((m) => m.id === chosen)) {
    option(select, chosen, chosen.includes("/") ? chosen : `${chosen} (not installed)`);
  }
  if (!select.options.length) option(select, "", "No compatible models available");
  select.value = chosen;
}

function populateLocalModels() {
  const sel = $("brain_local_model");
  const chosen = sel.value || current.brain_local_model || "";
  const installed = _modelsCache.models.filter((m) => m.installed);
  sel.innerHTML = "";
  for (const m of installed) {
    const o = document.createElement("option");
    o.value = m.id;
    o.textContent = m.name;
    sel.appendChild(o);
  }
  // allow an arbitrary path that isn't in the catalog
  if (chosen && !installed.some((m) => m.id === chosen)) {
    const o = document.createElement("option");
    o.value = chosen;
    o.textContent = chosen.includes("/") ? chosen : `${chosen} (missing)`;
    sel.appendChild(o);
  }
  if (!installed.length && !chosen) {
    const o = document.createElement("option");
    o.value = "";
    o.textContent = "— download one in the Models tab —";
    sel.appendChild(o);
  }
  sel.value = chosen;
}

// ── first-run setup ────────────────────────────────────────────────────
let setupStep = 0;

function populateSetupModels() {
  const speech = $("setup_speech_model");
  const configuredSpeech = current.speech_model?.includes("/") ? "" : current.speech_model;
  const previousSpeech = speech.value || configuredSpeech || "qwen2.5-3b-instruct-q4km";
  speech.innerHTML = "";
  for (const m of _modelsCache.models.filter((m) => m.roles?.includes("cascade_llm"))) {
    const recommended = m.id === "qwen2.5-3b-instruct-q4km" ? " (Recommended)" : "";
    option(speech, m.id, `${m.name}${recommended}${m.installed ? " — installed" : " — download required"}`);
  }
  speech.value = previousSpeech;

  const brain = $("setup_brain_model");
  const previousBrain = brain.value || current.brain_local_model || "";
  brain.innerHTML = "";
  for (const m of _modelsCache.models.filter((m) => m.installed && m.roles?.includes("brain"))) {
    option(brain, m.id, m.name);
  }
  if (!brain.options.length) option(brain, "", "No local model installed");
  brain.value = previousBrain || brain.options[0]?.value || "";

  const selected = _modelsCache.models.find((m) => m.id === speech.value);
  if (selected?.installed) {
    $("setup_speech_status").textContent = `Installed (${fmtBytes(selected.bytes_on_disk)})`;
  } else if (selected?.downloading) {
    const pct = selected.job?.total ? Math.round((selected.job.downloaded / selected.job.total) * 100) : 0;
    $("setup_speech_status").textContent = `Downloading… ${pct}%`;
  } else {
    $("setup_speech_status").textContent = "Not installed";
  }
}

function setSetupStep(step) {
  setupStep = step;
  document.querySelectorAll(".setup-step").forEach((el) => {
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

function showSetup(show) {
  $("setup").hidden = !show;
  document.querySelectorAll("body > header, body > nav, body > main, body > footer").forEach((el) => {
    el.inert = show;
  });
}

function setupError(message) {
  $("setup_error").textContent = message;
  $("setup_error").hidden = false;
}

function selectedSpeechModel() {
  return $("setup_speech_path").value.trim() || $("setup_speech_model").value;
}

function syncSetupSpeech() {
  const local = $("setup_speech_source").value !== "remote";
  $("setup_speech_local").hidden = !local;
  $("setup_speech_remote").hidden = local;
}

function selectedBrainModel() {
  return $("setup_brain_path").value.trim() || $("setup_brain_model").value;
}

function syncSetupBrain() {
  const local = $("setup_brain_source").value === "local";
  $("setup_brain_remote").hidden = local;
  $("setup_brain_local").hidden = !local;
}

function initSetupForm() {
  $("setup_speech_source").value = current.speech_model_source || "local";
  $("setup_speech_path").value = current.speech_model?.includes("/") ? current.speech_model : "";
  $("setup_speech_remote_url").value = current.speech_remote_base_url || "";
  $("setup_speech_remote_name").value = current.speech_remote_model || "";
  $("setup_speech_remote_key").value = current.speech_remote_api_key || "";
  $("setup_brain_source").value = current.brain_source || "remote";
  $("setup_brain_url").value = current.brain_base_url || "";
  $("setup_brain_name").value = current.brain_model || "";
  $("setup_brain_key").value = current.brain_api_key || "";
  $("setup_brain_path").value = current.brain_local_model?.includes("/") ? current.brain_local_model : "";
  $("setup_llama_server").value = current.llama_server_bin || "llama-server";
  syncSetupBrain();
  syncSetupSpeech();
  populateSetupModels();
  setSetupStep(0);
}

async function validateSetupStep() {
  if (setupStep === 1) {
    if ($("setup_speech_source").value === "remote") {
      if (!$("setup_speech_remote_url").value.trim()) throw new Error("Enter the conversational model endpoint.");
      if (!$("setup_speech_remote_name").value.trim()) throw new Error("Enter the model name expected by that endpoint.");
    } else {
      const selected = selectedSpeechModel();
      if (!selected) throw new Error("Choose or download a conversational model.");
      const path = await invoke("model_resolve", { idOrPath: selected });
      if (!path) throw new Error("That conversational model is not available yet. Download it or open Advanced and enter an existing GGUF path.");
    }
  }
  if (setupStep === 2) {
    if ($("setup_brain_source").value === "remote") {
      if (!$("setup_brain_url").value.trim()) throw new Error("Enter the capable model endpoint.");
      if (!$("setup_brain_name").value.trim()) throw new Error("Enter the model name expected by that endpoint.");
    } else {
      const id = selectedBrainModel();
      if (!id || !(await invoke("model_resolve", { idOrPath: id }))) {
        throw new Error("Install and choose a local model for delegation.");
      }
      if (!$("setup_llama_server").value.trim()) throw new Error("Enter the llama-server executable path.");
    }
  }
}

function renderSetupReview() {
  const review = $("setup_review");
  review.innerHTML = "";
  const speech = _modelsCache.models.find((m) => m.id === selectedSpeechModel());
  const speechDescription = $("setup_speech_source").value === "remote"
    ? `${$("setup_speech_remote_name").value} at ${$("setup_speech_remote_url").value}`
    : (speech?.name || selectedSpeechModel());
  const items = [
    ["Voice pipeline", "Parakeet → conversational model → speech"],
    ["Conversational model", speechDescription],
    ["Delegation", $("setup_brain_source").value === "local"
      ? (selectedBrainModel().includes("/") ? selectedBrainModel() : $("setup_brain_model").selectedOptions[0]?.textContent)
      : `${$("setup_brain_name").value} at ${$("setup_brain_url").value}`],
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

$("setup_brain_source").addEventListener("change", syncSetupBrain);
$("setup_speech_source").addEventListener("change", syncSetupSpeech);
$("setup_speech_model").addEventListener("change", () => {
  $("setup_speech_path").value = "";
  populateSetupModels();
});
$("setup_brain_model").addEventListener("change", () => ($("setup_brain_path").value = ""));
$("setup_download_speech").addEventListener("click", async () => {
  try {
    const id = $("setup_speech_model").value;
    if (!id) throw new Error("Choose a model first.");
    await invoke("model_download", { id });
    $("setup_speech_status").textContent = "Starting download…";
    await renderModels();
  } catch (e) {
    setupError(String(e.message || e));
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

    current.speech_model_source = $("setup_speech_source").value;
    if (current.speech_model_source === "remote") {
      current.speech_remote_base_url = $("setup_speech_remote_url").value.trim();
      current.speech_remote_model = $("setup_speech_remote_name").value.trim();
      current.speech_remote_api_key = $("setup_speech_remote_key").value.trim();
    } else {
      current.speech_model = selectedSpeechModel();
    }
    current.delegation_enabled = true;
    current.brain_source = $("setup_brain_source").value;
    if (current.brain_source === "remote") {
      current.brain_base_url = $("setup_brain_url").value.trim();
      current.brain_model = $("setup_brain_name").value.trim();
      current.brain_api_key = $("setup_brain_key").value.trim();
    } else {
      current.brain_local_model = selectedBrainModel();
      current.llama_server_bin = $("setup_llama_server").value.trim();
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
      throw new Error(`The voice engine could not start: ${e}`);
    }
    showSetup(false);
    applyToForm(current);
    if (isWeb) location.replace("/");
  } catch (e) {
    setupError(String(e.message || e));
  }
});

$("am_add").addEventListener("click", async () => {
  const spec = {
    name: $("am_name").value,
    repo: $("am_repo").value,
    file: $("am_file").value,
    url: $("am_url").value,
  };
  try {
    await invoke("model_add", { spec });
    $("am_name").value = $("am_repo").value = $("am_file").value = $("am_url").value = "";
    renderModels();
  } catch (e) {
    flash(`add failed: ${e}`);
  }
});

listen("asset-progress", () => {
  clearTimeout(_refreshTimer);
  _refreshTimer = setTimeout(renderModels, 400);
});

async function refreshWebState() {
  try {
    const url = await invoke("web_url").catch(() => null);
    $("web_state").textContent = url ? "running" : "stopped";
  } catch {}
}

// ── mic devices ─────────────────────────────────────────────────────────
async function loadMics() {
  try {
    // a transient getUserMedia unlocks device labels
    const tmp = await navigator.mediaDevices.getUserMedia({ audio: true }).catch(() => null);
    const devices = await navigator.mediaDevices.enumerateDevices();
    const sel = $("mic_device_id");
    const chosen = current.mic_device_id || "";
    sel.innerHTML = '<option value="">System default</option>';
    for (const d of devices.filter((x) => x.kind === "audioinput")) {
      const o = document.createElement("option");
      o.value = d.deviceId;
      o.textContent = d.label || `microphone ${sel.length}`;
      sel.appendChild(o);
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

(async () => {
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
  refreshWebState();
  loadMics();
  try {
    $("version").textContent = "v" + ((await invoke("app_version").catch(() => "")) || "");
  } catch {}
  if (isWeb) {
    const back = document.createElement("a");
    back.href = "./";
    back.textContent = "← Voice";
    back.style.cssText = "color:#9db8ff;text-decoration:none;font-size:.85rem;margin-left:.6rem";
    $("version").after(back);
  }
})();
