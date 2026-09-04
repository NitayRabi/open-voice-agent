import { getSettings, saveSettings, invoke, listen, emit, isWeb } from "./lib/tauri.js";

const $ = (id) => document.getElementById(id);
const logEl = $("log");
const statusEl = $("status");

const FIELDS = {
  backend_mode: "value",
  server_url: "value",
  voicechat_url: "value",
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
  manage_backend: "checked",
  launch_command: "cmd",
  launch_cwd: "value",
  health_url: "value",
  launch_env: "env",
  web_enabled: "checked",
  web_bind: "value",
  web_port: "int",
  web_token: "value",
  web_tls_cert: "value",
  web_tls_key: "value",
};

const PRESETS = {
  hf: {
    backend_mode: "hf_realtime",
    server_url: "ws://127.0.0.1:8765/v1/realtime",
    sample_rate: 16000,
    launch_command: "bash ../hf-s2s/run-comparison.sh",
    launch_cwd: "",
    health_url: "http://127.0.0.1:8766/",
    launch_env: {
      HF_S2S_LLM_MODEL: "/home/nitayrabi/models/gemma-4-E4B_q4_0-it.gguf",
      HF_S2S_BACKEND_PORT: "8766",
      HF_S2S_WSS_PORT: "8765",
    },
  },
  vc4: {
    backend_mode: "voicechat_http",
    voicechat_url: "https://127.0.0.1:8999",
    sample_rate: 16000,
    launch_command: "bash ../run.sh --quant Q4_0 --port 8999",
    launch_cwd: "",
    health_url: "https://127.0.0.1:8999/",
    launch_env: {},
  },
  vc8: {
    backend_mode: "voicechat_http",
    voicechat_url: "https://127.0.0.1:9000",
    sample_rate: 16000,
    launch_command: "bash ../run.sh --quant Q8_0 --port 9000",
    launch_cwd: "",
    health_url: "https://127.0.0.1:9000/",
    launch_env: {},
  },
};

function envToText(obj) {
  return Object.entries(obj || {})
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
}
function textToEnv(text) {
  const out = {};
  for (const line of text.split("\n")) {
    const s = line.trim();
    if (!s || s.startsWith("#")) continue;
    const eq = s.indexOf("=");
    if (eq === -1) continue;
    out[s.slice(0, eq).trim()] = s.slice(eq + 1).trim();
  }
  return out;
}

function applyToForm(s) {
  for (const [id, kind] of Object.entries(FIELDS)) {
    const el = $(id);
    if (!el) continue;
    const v = s[id];
    if (kind === "checked") el.checked = !!v;
    else if (kind === "cmd") el.value = Array.isArray(v) ? v.join(" ") : v || "";
    else if (kind === "env") el.value = envToText(v);
    else el.value = v ?? "";
  }
  updateGateLabel();
  updateModeVisibility();
  updateBrainVisibility();
}

function readForm(base) {
  const s = { ...base };
  for (const [id, kind] of Object.entries(FIELDS)) {
    const el = $(id);
    if (!el) continue;
    if (kind === "checked") s[id] = el.checked;
    else if (kind === "int" || kind === "float") s[id] = Number(el.value);
    else if (kind === "cmd") s[id] = el.value.trim().split(/\s+/).filter(Boolean);
    else if (kind === "env") s[id] = textToEnv(el.value);
    else s[id] = el.value;
  }
  return s;
}

function updateGateLabel() {
  const v = Number($("noise_gate_db").value);
  $("gate_val").textContent = v <= -100 ? "off" : `${v} dB`;
}
function updateModeVisibility() {
  const mode = $("backend_mode").value;
  document.querySelectorAll("[data-mode]").forEach((el) => {
    el.classList.toggle("show", el.dataset.mode === mode);
  });
}
function updateBrainVisibility() {
  const src = $("brain_source").value;
  document.querySelectorAll("[data-brain]").forEach((el) => {
    el.classList.toggle("show", el.dataset.brain === src);
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
$("backend_mode").addEventListener("change", updateModeVisibility);
$("brain_source").addEventListener("change", updateBrainVisibility);

document.querySelectorAll("[data-preset]").forEach((b) => {
  b.addEventListener("click", () => {
    const p = PRESETS[b.dataset.preset];
    for (const [k, v] of Object.entries(p)) {
      const el = $(k);
      if (!el) continue;
      if (k === "launch_env") el.value = envToText(v);
      else el.value = v;
    }
    updateModeVisibility();
    flash(`applied ${b.textContent.trim()} preset — review, then Save`);
  });
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
  current = readForm(current);
  await saveSettings(current);
  flash("saved");
  return current;
}

$("save").addEventListener("click", () => save().catch((e) => flash(`save failed: ${e}`)));
$("save_restart").addEventListener("click", async () => {
  try {
    await save();
    await emit("settings-changed");
    flash("saved — speech restarting");
  } catch (e) {
    flash(`failed: ${e}`);
  }
});

$("backend_start").addEventListener("click", async () => {
  try {
    await save();
    await invoke("backend_start");
  } catch (e) {
    flash(`start failed: ${e}`);
  }
});
$("backend_stop").addEventListener("click", () => invoke("backend_stop").catch((e) => flash(String(e))));
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
listen("backend-status", (e) => {
  $("backend_state").textContent = e.payload ? "running" : "stopped";
});
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

async function refreshBackendState() {
  try {
    const running = await invoke("backend_running");
    $("backend_state").textContent = running ? "running" : "stopped";
  } catch {}
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
(async () => {
  current = await getSettings();
  applyToForm(current);
  await loadMics();
  await refreshBackendState();
  await renderModels();
  applyToForm(current); // re-apply brain_local_model now that the dropdown is populated
  try {
    $("version").textContent = "v" + ((await invoke("app_version").catch(() => "")) || "");
  } catch {}
  if (isWeb) {
    const token = new URLSearchParams(location.search).get("token");
    const back = document.createElement("a");
    back.href = "./" + (token ? `?token=${encodeURIComponent(token)}` : "");
    back.textContent = "← Voice";
    back.style.cssText = "color:#9db8ff;text-decoration:none;font-size:.85rem;margin-left:.6rem";
    $("version").after(back);
  }
})();
