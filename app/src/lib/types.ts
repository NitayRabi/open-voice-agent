// Shared shapes for the payloads that cross the Rust boundary, plus the small
// vocabulary the UI modules agree on. The settings shape mirrors
// `src-tauri/src/config.rs`; the model shapes mirror `AssetManager::list`.

/** Where a model runs: a downloaded GGUF, or an OpenAI-compatible endpoint. */
export type ModelSource = "local" | "remote";

export interface PairedDevice {
  id: string;
  type: string;
  label: string;
  created_at: number;
  last_used_at: number | null;
}

export interface IssuedDeviceCredential {
  access_token: string;
  token_type: "Bearer";
  device: PairedDevice;
}

/** Lifecycle of the voice pipeline; also the orb's visual state. */
export type PipelineState =
  | "idle"
  | "connecting"
  | "listening"
  | "user_speaking"
  | "thinking"
  | "delegating"
  | "speaking"
  | "error";

export interface Settings {
  /** Internal Realtime WebSocket URL. Browser clients receive a host-aware URL. */
  server_url: string;

  voice: string;
  instructions: string;
  hotkey: string;
  sample_rate: number;
  /** Catalog id or absolute GGUF path used by the built-in speech pipeline. */
  speech_model: string;
  speech_model_source: ModelSource;
  speech_remote_base_url: string;
  speech_remote_model: string;
  speech_remote_api_key: string;
  mic_device_id: string;
  output_device_id: string;
  /** Noise-gate threshold in dB; -100 means "off". */
  noise_gate_db: number;
  autostart_listening: boolean;
  /** Register the app to launch at login; the managed backend starts with it. */
  app_autostart: boolean;

  // ── delegation to the "brain" ──────────────────────────────────
  delegation_enabled: boolean;
  brain_source: ModelSource;
  brain_base_url: string;
  brain_api_key: string;
  brain_model: string;
  /** local mode: a Models-tab entry id, or an absolute path to a .gguf. */
  brain_local_model: string;
  llama_server_bin: string;
  brain_local_port: number;
  brain_local_ctx: number;
  brain_local_ngl: number;
  hf_token: string;
  brain_system_prompt: string;
  brain_temperature: number;
  delegation_tool_name: string;
  delegation_tool_description: string;
  delegation_timeout_s: number;
  delegation_speak_result: boolean;

  // ── engine / backend supervisor ─────────────────────────────────
  manage_backend: boolean;
  launch_command: string[];
  launch_cwd: string;
  launch_env: Record<string, string>;
  health_url: string;

  setup_completed: boolean;

  // ── embedded web server ─────────────────────────────────────────
  web_enabled: boolean;
  web_bind: string;
  web_port: number;
  web_token: string;
  web_tls_cert: string;
  web_tls_key: string;
  web_tailscale: boolean;
  web_tailscale_binary: string;
}

/** Progress of an in-flight download, as reported by `models_list`. */
export interface ModelJob {
  downloaded: number;
  total: number;
  cancelling: boolean;
}

export interface ModelInfo {
  id: string;
  name: string;
  repo: string;
  file: string;
  bytes: number;
  license: string;
  badges: ("recommended" | "smallest")[];
  downloadable: boolean;
  roles: string[];
  gated: boolean;
  note: string;
  /** true for user-added entries, false for the bundled catalog */
  user: boolean;
  url: string;
  installed: boolean;
  bytes_on_disk: number | null;
  path: string | null;
  partial_bytes: number | null;
  downloading: boolean;
  job: ModelJob | null;
}

export interface ModelsList {
  dir: string | null;
  models: ModelInfo[];
}

export interface AssetProgress {
  id: string;
  status: "downloading" | "done" | "cancelled" | "removed" | "error";
  downloaded?: number;
  total?: number;
  error?: string;
}

export interface TranscriptDetail {
  role: "user" | "assistant" | "tool";
  text: string;
}

export interface TaskDetail {
  id: string;
  request: string;
  status: "running" | "completed" | "failed";
  result?: string;
  error?: string;
}

/** One analyser reading: overall loudness plus the waveform the orb draws. */
export interface LevelDetail {
  rms: number;
  waveform: Float32Array;
}

export interface AudioInputDevice {
  id: string;
  label: string;
}
