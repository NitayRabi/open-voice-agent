const $ = (selector) => document.querySelector(selector);

const root = $("#acp-settings");
const providerList = $("#acp-provider-list");
const defaultSelect = $("#acp-default");
const editor = $("#acp-editor");
const idInput = $("#acp-id");
const nameInput = $("#acp-name");
const aliasesInput = $("#acp-aliases");
const guidanceInput = $("#acp-guidance");
const commandInput = $("#acp-command");
const argsInput = $("#acp-args");
const cwdInput = $("#acp-cwd");
const timeoutInput = $("#acp-timeout");
const envInput = $("#acp-env");
const envHint = $("#acp-env-hint");
const status = $("#acp-status");

let registry = { defaultProvider: null, providers: [] };
let editingId = null;

async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: { "Content-Type": "application/json", ...(options.headers || {}) },
  });
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.detail || `Request failed (${response.status})`);
  return data;
}

function setStatus(message, error = false) {
  status.textContent = message;
  status.classList.toggle("error", error);
}

function button(label, className, onClick) {
  const element = document.createElement("button");
  element.type = "button";
  element.className = className;
  element.textContent = label;
  element.addEventListener("click", onClick);
  return element;
}

function render() {
  defaultSelect.replaceChildren();
  const empty = document.createElement("option");
  empty.value = "";
  empty.textContent = registry.providers.length ? "Choose a provider" : "No provider configured";
  defaultSelect.append(empty);
  for (const provider of registry.providers) {
    const option = document.createElement("option");
    option.value = provider.id;
    option.textContent = provider.id;
    defaultSelect.append(option);
  }
  defaultSelect.value = registry.defaultProvider || "";
  defaultSelect.disabled = registry.providers.length === 0;

  providerList.replaceChildren();
  for (const provider of registry.providers) {
    const row = document.createElement("div");
    row.className = "acp-provider-row";
    const copy = document.createElement("div");
    copy.className = "acp-provider-copy";
    const title = document.createElement("strong");
    title.textContent = provider.displayName || provider.id;
    if (provider.id === registry.defaultProvider) {
      const badge = document.createElement("span");
      badge.className = "acp-badge";
      badge.textContent = "default";
      title.append(" ", badge);
    }
    const detail = document.createElement("small");
    const names = provider.aliases?.length ? ` · ${provider.aliases.join(", ")}` : "";
    detail.textContent = `${provider.id}${names} · ${[provider.command, ...provider.args].join(" ")}`;
    copy.append(title, detail);

    const actions = document.createElement("div");
    actions.className = "acp-row-actions";
    actions.append(
      button("Test", "btn ghost acp-small", () => testProvider(provider.id)),
      button("Edit", "btn ghost acp-small", () => openEditor(provider)),
      button("Remove", "btn ghost acp-small danger", () => removeProvider(provider.id)),
    );
    row.append(copy, actions);
    providerList.append(row);
  }
}

function openEditor(provider = null) {
  editingId = provider?.id || null;
  idInput.value = provider?.id || "";
  idInput.readOnly = Boolean(provider);
  nameInput.value = provider?.displayName || provider?.id || "";
  aliasesInput.value = (provider?.aliases || []).join(", ");
  guidanceInput.value = provider?.guidance || "";
  commandInput.value = provider?.command || "";
  argsInput.value = (provider?.args || []).join("\n");
  cwdInput.value = provider?.cwd || "..";
  timeoutInput.value = String(provider?.timeoutSeconds || 180);
  envInput.value = "";
  envHint.textContent = provider?.envKeys?.length
    ? `Configured variables: ${provider.envKeys.join(", ")}. Leave blank to preserve them.`
    : "Values are saved locally and never returned to the browser.";
  editor.hidden = false;
  idInput.focus();
}

function closeEditor() {
  editor.hidden = true;
  editingId = null;
}

async function saveProvider() {
  try {
    const id = idInput.value.trim();
    let env = null;
    if (envInput.value.trim()) {
      env = JSON.parse(envInput.value);
      if (!env || Array.isArray(env) || typeof env !== "object") {
        throw new Error("Environment must be a JSON object.");
      }
    }
    const payload = {
      displayName: nameInput.value.trim() || id,
      aliases: aliasesInput.value.split(",").map((value) => value.trim()).filter(Boolean),
      guidance: guidanceInput.value.trim(),
      command: commandInput.value.trim(),
      args: argsInput.value.split("\n").map((line) => line.trim()).filter(Boolean),
      cwd: cwdInput.value.trim() || ".",
      timeoutSeconds: Number(timeoutInput.value),
      env,
    };
    setStatus("Saving provider…");
    registry = await api(`api/acp/providers/${encodeURIComponent(id)}`, {
      method: "PUT",
      body: JSON.stringify(payload),
    });
    closeEditor();
    render();
    setStatus(`${id} saved.`);
  } catch (error) {
    setStatus(error.message, true);
  }
}

async function removeProvider(id) {
  if (!window.confirm(`Remove ACP provider “${id}”?`)) return;
  try {
    registry = await api(`api/acp/providers/${encodeURIComponent(id)}`, { method: "DELETE" });
    if (editingId === id) closeEditor();
    render();
    setStatus(`${id} removed.`);
  } catch (error) {
    setStatus(error.message, true);
  }
}

async function testProvider(id) {
  try {
    setStatus(`Testing ${id}…`);
    const result = await api(`api/acp/providers/${encodeURIComponent(id)}/test`, {
      method: "POST",
      body: "{}",
    });
    setStatus(`${id}: ${result.result}`);
  } catch (error) {
    setStatus(error.message, true);
  }
}

$("#acp-add").addEventListener("click", () => openEditor());
$("#acp-cancel").addEventListener("click", closeEditor);
$("#acp-save").addEventListener("click", saveProvider);
defaultSelect.addEventListener("change", async () => {
  if (!defaultSelect.value) return;
  try {
    registry = await api("api/acp/default", {
      method: "PUT",
      body: JSON.stringify({ provider: defaultSelect.value }),
    });
    render();
    setStatus(`${defaultSelect.value} is now the default.`);
  } catch (error) {
    setStatus(error.message, true);
  }
});

async function loadProviders() {
  try {
    registry = await api("api/acp/providers");
    render();
  } catch (error) {
    root.hidden = true;
    console.error("Unable to load ACP providers", error);
  }
}

void loadProviders();
