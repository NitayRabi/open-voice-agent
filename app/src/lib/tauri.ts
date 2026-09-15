// Dual bridge: talk to the Rust side either through the Tauri IPC (desktop
// windows) or through the embedded web server's /api (a browser tab). Same
// surface either way: invoke / listen / emit.

import type {
  AcpxStatus,
  AssetProgress,
  ModelsList,
  PairedDevice,
  IssuedDeviceCredential,
  Settings,
  TranscriptDetail,
} from "./types.js";

/** `withGlobalTauri` puts the IPC on `window.__TAURI__` — no npm dependency. */
interface TauriGlobal {
  core: { invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> };
  event: {
    listen(event: string, handler: (event: TauriEvent<never>) => void): Promise<UnlistenFn>;
    emit(event: string, payload?: unknown): Promise<void>;
  };
  window?: { getCurrentWindow?(): TauriWindow };
}

/** Only the slice of the window API the bubble uses. */
export interface TauriWindow {
  startDragging?(): Promise<void>;
}

export interface TauriEvent<T> {
  event: string;
  payload: T;
}

export type UnlistenFn = () => void;

const T = (globalThis as { __TAURI__?: TauriGlobal }).__TAURI__;
export const hasTauri = !!T;
export const isWeb = !hasTauri;

interface ApiOptions {
  method?: "GET" | "POST" | "DELETE";
  body?: unknown;
}

async function api<T>(path: string, { method = "GET", body }: ApiOptions = {}): Promise<T> {
  const res = await fetch(path, {
    method,
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  const data = (text ? JSON.parse(text) : {}) as T & { error?: string };
  if (!res.ok) throw new Error(data.error || `${res.status} ${res.statusText}`);
  return data;
}

/**
 * Every command the frontend may invoke, with its argument and result shapes.
 * `args: void` means the command takes none. Commands missing from `WEB` below
 * are desktop-only and reject in a browser tab.
 */
export interface Commands {
  get_settings: { args: void; result: Settings };
  save_settings: { args: { settings: Settings }; result: void };
  delegate: { args: { request: string }; result: string };
  acpx_status: { args: void; result: AcpxStatus };
  acpx_install: { args: void; result: string };
  acpx_install_agent: { args: { id: string }; result: void };
  app_version: { args: void; result: string };
  backend_running: { args: void; result: boolean };
  backend_start: { args: void; result: void };
  backend_stop: { args: void; result: void };
  backend_ready: { args: void; result: boolean };
  backend_touch: { args: void; result: void };
  web_start: { args: void; result: string };
  web_stop: { args: void; result: void };
  web_url: { args: void; result: string | null };
  show_settings: { args: void; result: void };
  quit_app: { args: void; result: void };
  models_list: { args: void; result: ModelsList };
  model_download: { args: { id: string }; result: void };
  model_cancel: { args: { id: string }; result: void };
  model_remove: { args: { id: string }; result: void };
  model_forget: { args: { id: string }; result: void };
  model_resolve: { args: { idOrPath: string }; result: string | null };
  paired_devices: { args: void; result: PairedDevice[] };
  issue_device: { args: { deviceType: string; label: string }; result: IssuedDeviceCredential };
  revoke_device: { args: { id: string }; result: boolean };
}

export type CommandName = keyof Commands;
type Args<K extends CommandName> = Commands[K]["args"];
type Result<K extends CommandName> = Commands[K]["result"];

/** Browser implementations, hitting the same state over the embedded server. */
type WebImpls = { [K in CommandName]?: (args: Args<K>) => Promise<Result<K>> | Result<K> };

const WEB: WebImpls = {
  get_settings: () => api<Settings>("/api/settings"),
  save_settings: ({ settings }) => api<void>("/api/settings", { method: "POST", body: { settings } }),
  delegate: ({ request }) =>
    api<{ answer: string }>("/api/delegate", { method: "POST", body: { request } }).then((d) => d.answer),
  acpx_status: () => api<AcpxStatus>("/api/acpx/status"),
  acpx_install: () => api<{ path: string }>("/api/acpx/install", { method: "POST" }).then((d) => d.path),
  acpx_install_agent: ({ id }) => api<void>("/api/acpx/install_agent", { method: "POST", body: { id } }),
  app_version: () => api<{ version: string }>("/api/version").then((d) => d.version),
  backend_running: () => api<{ running: boolean }>("/api/backend/status").then((d) => d.running),
  backend_start: () => api<void>("/api/backend/start", { method: "POST" }),
  backend_stop: () => api<void>("/api/backend/stop", { method: "POST" }),
  backend_ready: () => api<{ ready: boolean }>("/api/backend/ready").then((d) => d.ready),
  backend_touch: () => api<void>("/api/backend/touch", { method: "POST" }),
  web_url: () => location.origin + "/",
  show_settings: () => {
    location.href = "/settings";
  },
  models_list: () => api<ModelsList>("/api/models"),
  model_download: ({ id }) => api<void>("/api/models/download", { method: "POST", body: { id } }),
  model_cancel: ({ id }) => api<void>("/api/models/cancel", { method: "POST", body: { id } }),
  model_remove: ({ id }) => api<void>("/api/models/remove", { method: "POST", body: { id } }),
  model_forget: ({ id }) => api<void>("/api/models/forget", { method: "POST", body: { id } }),
  model_resolve: ({ idOrPath }) =>
    api<{ path: string | null }>("/api/models/resolve", {
      method: "POST",
      body: { id_or_path: idOrPath },
    }).then((d) => d.path),
  paired_devices: () => api<{ devices: PairedDevice[] }>("/api/access/devices").then((d) => d.devices),
  issue_device: ({ deviceType, label }) => api<IssuedDeviceCredential>("/api/access/devices", {
    method: "POST", body: { device: { type: deviceType, label } },
  }),
  revoke_device: ({ id }) => api<void>(`/api/access/devices/${encodeURIComponent(id)}`, { method: "DELETE" }).then(() => true),
};

export async function invoke<K extends CommandName>(
  cmd: K,
  ...[args]: Args<K> extends void ? [args?: undefined] : [args: Args<K>]
): Promise<Result<K>> {
  if (T) return (await T.core.invoke(cmd, args ?? {})) as Result<K>;
  const fn = WEB[cmd];
  if (!fn) throw new Error(`command '${cmd}' is not available in the web UI`);
  return fn((args ?? {}) as Args<K>);
}

// ── events ───────────────────────────────────────────────────────────────

/** Events the Rust side (or another client) can push at the UI. */
export interface AppEvents {
  "backend-log": string;
  "web-status": boolean;
  "acpx-status": null;
  "asset-progress": AssetProgress;
  "ova-transcript": TranscriptDetail;
  "settings-changed": null;
  "speech-toggle": null;
}

export type AppEventName = keyof AppEvents;

type Handler<K extends AppEventName> = (event: TauriEvent<AppEvents[K]>) => void;

// event name -> its subscribers, for the browser long-poll transport
const handlers = new Map<string, Set<Handler<AppEventName>>>();
let polling = false;

interface PollResponse {
  cursor: number;
  events: { id: number; type: string; payload: unknown }[];
}

async function pollLoop(): Promise<void> {
  if (polling) return;
  polling = true;
  let since = 0;
  while (polling) {
    try {
      const r = await api<PollResponse>(`/api/events?since=${since}`);
      if (typeof r.cursor === "number") since = r.cursor;
      for (const e of r.events || []) {
        for (const cb of handlers.get(e.type) || []) {
          cb({ event: e.type, payload: e.payload as AppEvents[AppEventName] });
        }
      }
    } catch {
      await new Promise((res) => setTimeout(res, 2000));
    }
  }
}

export async function listen<K extends AppEventName>(event: K, handler: Handler<K>): Promise<UnlistenFn> {
  if (T) return T.event.listen(event, handler as Handler<AppEventName> as (e: TauriEvent<never>) => void);
  let set = handlers.get(event);
  if (!set) handlers.set(event, (set = new Set()));
  set.add(handler as Handler<AppEventName>);
  void pollLoop();
  return () => {
    handlers.get(event)?.delete(handler as Handler<AppEventName>);
  };
}

export async function emit<K extends AppEventName>(event: K, payload?: AppEvents[K]): Promise<void> {
  if (T) return T.event.emit(event, payload);
  await api<void>("/api/emit", { method: "POST", body: { event, payload: payload ?? null } });
}

export function currentWindow(): TauriWindow | null {
  return T?.window?.getCurrentWindow?.() ?? null;
}

export async function getSettings(): Promise<Settings> {
  return invoke("get_settings");
}
export async function saveSettings(settings: Settings): Promise<void> {
  return invoke("save_settings", { settings });
}
export async function delegate(request: string): Promise<string> {
  return invoke("delegate", { request });
}
