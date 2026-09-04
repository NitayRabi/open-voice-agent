// Dual bridge: talk to the Rust side either through the Tauri IPC (desktop
// windows) or through the embedded web server's /api (a browser tab). Same
// surface either way: invoke / listen / emit.

const T = globalThis.__TAURI__;
export const hasTauri = !!T;
export const isWeb = !hasTauri;

async function api(path, { method = "GET", body } = {}) {
  const res = await fetch(path, {
    method,
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  const data = text ? JSON.parse(text) : {};
  if (!res.ok) throw new Error(data.error || `${res.status} ${res.statusText}`);
  return data;
}

// command name -> web implementation
const WEB = {
  get_settings: () => api("/api/settings"),
  save_settings: ({ settings }) => api("/api/settings", { method: "POST", body: { settings } }),
  delegate: ({ request }) => api("/api/delegate", { method: "POST", body: { request } }).then((d) => d.answer),
  app_version: () => api("/api/version").then((d) => d.version),
  backend_running: () => api("/api/backend/status").then((d) => d.running),
  backend_start: () => api("/api/backend/start", { method: "POST" }),
  backend_stop: () => api("/api/backend/stop", { method: "POST" }),
  web_url: () => Promise.resolve(location.origin + "/"),
  show_settings: () => {
    location.href = "/settings";
  },
  models_list: () => api("/api/models"),
  model_add: ({ spec }) => api("/api/models/add", { method: "POST", body: spec }),
  model_download: ({ id }) => api("/api/models/download", { method: "POST", body: { id } }),
  model_cancel: ({ id }) => api("/api/models/cancel", { method: "POST", body: { id } }),
  model_remove: ({ id }) => api("/api/models/remove", { method: "POST", body: { id } }),
  model_forget: ({ id }) => api("/api/models/forget", { method: "POST", body: { id } }),
  model_resolve: ({ idOrPath }) =>
    api("/api/models/resolve", { method: "POST", body: { id_or_path: idOrPath } }).then((d) => d.path),
};

export async function invoke(cmd, args = {}) {
  if (T) return T.core.invoke(cmd, args);
  const fn = WEB[cmd];
  if (!fn) throw new Error(`command '${cmd}' is not available in the web UI`);
  return fn(args);
}

// ── events (long-poll in the browser) ────────────────────────────────────

const _handlers = new Map(); // event -> Set<cb>
let _polling = false;

async function pollLoop() {
  if (_polling) return;
  _polling = true;
  let since = 0;
  while (_polling) {
    try {
      const r = await api(`/api/events?since=${since}`);
      if (typeof r.cursor === "number") since = r.cursor;
      for (const e of r.events || []) {
        for (const cb of _handlers.get(e.type) || []) cb({ event: e.type, payload: e.payload });
      }
    } catch {
      await new Promise((res) => setTimeout(res, 2000));
    }
  }
}

export async function listen(event, handler) {
  if (T) return T.event.listen(event, handler);
  if (!_handlers.has(event)) _handlers.set(event, new Set());
  _handlers.get(event).add(handler);
  pollLoop();
  return () => _handlers.get(event)?.delete(handler);
}

export async function emit(event, payload) {
  if (T) return T.event.emit(event, payload);
  return api("/api/emit", { method: "POST", body: { event, payload: payload ?? null } });
}

export function currentWindow() {
  return T?.window?.getCurrentWindow?.() ?? null;
}

export async function getSettings() {
  return invoke("get_settings");
}
export async function saveSettings(settings) {
  return invoke("save_settings", { settings });
}
export async function delegate(request) {
  return invoke("delegate", { request });
}
