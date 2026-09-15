//! Persistent settings, stored as JSON in the platform config dir
//! (`~/.config/ai.openvoice.agent/config.json` on Linux).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Ports the bundled `hf-s2s/run-comparison.sh` stack listens on.
///
/// The speech backend itself binds loopback only and speaks plain ws. When a
/// TLS cert is configured, the launcher additionally runs a socat wrapper on
/// `WSS_PORT` that re-exposes it over TLS on every interface; that wrapper is
/// the only realtime endpoint another device can reach. The scheme and the port
/// must therefore be chosen together — see `webserver::browser_settings`.
pub const BACKEND_PORT: u16 = 8766;
pub const WSS_PORT: u16 = 8765;

/// The loopback realtime endpoint. The speech stack is an implementation
/// detail, not a user choice, so this is forced rather than edited.
pub fn local_server_url() -> String {
    format!("ws://127.0.0.1:{BACKEND_PORT}/v1/realtime")
}

fn d_server_url() -> String {
    local_server_url()
}
fn d_voice() -> String {
    "Aiden".into()
}
fn d_instructions() -> String {
    "You are a natural, concise voice assistant — you mainly talk with the user in one \
or two short sentences. You have one tool, delegate_task. The moment the user wants \
something actually done or worked out — a calculation, a lookup, a plan, code, a \
decision, an action — call delegate_task with the full request, then tell the user \
you're on it. Don't try to do those things yourself; just hand them over and relay \
the result when it comes back."
        .into()
}
fn d_hotkey() -> String {
    "CommandOrControl+Shift+Space".into()
}
fn d_sample_rate() -> u32 {
    16_000
}
fn d_true() -> bool {
    true
}
fn d_brain_source() -> String {
    "remote".into()
}
fn d_brain_base_url() -> String {
    "http://127.0.0.1:8080/v1".into()
}
fn d_brain_model() -> String {
    String::new()
}
fn d_llama_server_bin() -> String {
    // Empty = auto-managed: reuse an existing install (Homebrew, PATH) if
    // there is one, otherwise download a pinned build automatically. Non-mac
    // keeps the old PATH-lookup default since that auto-download isn't
    // implemented there.
    #[cfg(target_os = "macos")]
    return String::new();

    #[cfg(not(target_os = "macos"))]
    "llama-server".into()
}
fn d_brain_local_port() -> u16 {
    8127
}
fn d_brain_local_ctx() -> u32 {
    8192
}
fn d_brain_local_ngl() -> i32 {
    999
}
fn d_brain_temperature() -> f64 {
    0.3
}
fn d_brain_prompt() -> String {
    "You are the capable half of a voice assistant. The conversational front-end has \
handed you a task because it needs real work or a real answer. Do the task or work out \
the answer, then reply with the result in one or two plain spoken sentences — no \
markdown, no lists, no preamble."
        .into()
}
fn d_tool_name() -> String {
    "delegate_task".into()
}
fn d_tool_desc() -> String {
    "Hand a task to the more capable brain model. Call this whenever the user wants \
something actually done or worked out — a calculation, a lookup, a plan, code, a \
decision — rather than just chat. Pass the full task in one clear sentence."
        .into()
}
fn d_timeout() -> u64 {
    120
}
fn d_gate_db() -> f64 {
    // -100 is the UI's "off" sentinel
    -100.0
}
fn d_launch_cmd() -> Vec<String> {
    vec!["bash".into(), "hf-s2s/run-comparison.sh".into()]
}
fn d_health_url() -> String {
    format!("http://127.0.0.1:{BACKEND_PORT}/")
}
fn d_speech_model() -> String {
    crate::assets::RECOMMENDED_MODEL_ID.into()
}
fn d_local() -> String {
    "local".into()
}
fn d_web_bind() -> String {
    "127.0.0.1".into()
}
fn d_web_port() -> u16 {
    1730
}

// NOTE: no container-level `#[serde(default)]` — that would make `from_str`
// call `Settings::default()`, which calls `from_str`, forever. Every field
// carries its own `#[serde(default ...)]` instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Internal Realtime WebSocket URL. Browser clients receive a host-aware URL.
    #[serde(default = "d_server_url")]
    pub server_url: String,

    #[serde(default = "d_voice")]
    pub voice: String,
    #[serde(default = "d_instructions")]
    pub instructions: String,
    #[serde(default = "d_hotkey")]
    pub hotkey: String,
    #[serde(default = "d_sample_rate")]
    pub sample_rate: u32,
    /// Catalog id or absolute GGUF path used by the built-in speech pipeline.
    #[serde(default = "d_speech_model")]
    pub speech_model: String,
    /// "local" runs a downloaded GGUF; "remote" calls an OpenAI-compatible API.
    #[serde(default = "d_local")]
    pub speech_model_source: String,
    #[serde(default)]
    pub speech_remote_base_url: String,
    #[serde(default)]
    pub speech_remote_model: String,
    #[serde(default)]
    pub speech_remote_api_key: String,
    #[serde(default)]
    pub mic_device_id: String,
    #[serde(default)]
    pub output_device_id: String,
    /// Noise-gate threshold in dB; -100 means "off".
    #[serde(default = "d_gate_db")]
    pub noise_gate_db: f64,
    /// Start listening automatically as soon as the backend is reachable.
    #[serde(default)]
    pub autostart_listening: bool,
    /// Register the Tauri app to launch when the user logs in. The voice engine
    /// remains demand-loaded even when the app itself starts automatically.
    #[serde(default)]
    pub app_autostart: bool,

    // ── delegation to the "brain" ──────────────────────────────────
    #[serde(default = "d_true")]
    pub delegation_enabled: bool,
    /// "remote" = call an OpenAI-compatible endpoint; "local" = the app runs a
    /// llama.cpp server on a downloaded GGUF and calls that.
    #[serde(default = "d_brain_source")]
    pub brain_source: String,
    /// OpenAI-compatible base URL, including `/v1` (llama.cpp, vLLM, OpenAI, …).
    #[serde(default = "d_brain_base_url")]
    pub brain_base_url: String,
    /// Bearer token; leave empty for a local server.
    #[serde(default)]
    pub brain_api_key: String,
    /// Model name to send in the request (remote mode).
    #[serde(default = "d_brain_model")]
    pub brain_model: String,
    /// local mode: a Models-tab entry id, or an absolute path to a .gguf.
    #[serde(default)]
    pub brain_local_model: String,
    #[serde(default = "d_llama_server_bin")]
    pub llama_server_bin: String,
    #[serde(default = "d_brain_local_port")]
    pub brain_local_port: u16,
    #[serde(default = "d_brain_local_ctx")]
    pub brain_local_ctx: u32,
    #[serde(default = "d_brain_local_ngl")]
    pub brain_local_ngl: i32,
    /// Hugging Face token for gated repos / higher rate limits (model downloads).
    #[serde(default)]
    pub hf_token: String,
    /// System prompt for the brain.
    #[serde(default = "d_brain_prompt")]
    pub brain_system_prompt: String,
    #[serde(default = "d_brain_temperature")]
    pub brain_temperature: f64,
    #[serde(default = "d_tool_name")]
    pub delegation_tool_name: String,
    #[serde(default = "d_tool_desc")]
    pub delegation_tool_description: String,
    #[serde(default = "d_timeout")]
    pub delegation_timeout_s: u64,
    /// Speak the finished delegation result back through the model (vs. a bare ack).
    #[serde(default = "d_true")]
    pub delegation_speak_result: bool,

    // ── engine / backend supervisor ─────────────────────────────────
    /// Let the app start/stop the speech backend process.
    #[serde(default)]
    pub manage_backend: bool,
    #[serde(default = "d_launch_cmd")]
    pub launch_command: Vec<String>,
    /// Working directory for the launch command (relative to the app resource dir
    /// or absolute). Empty = the repo root guessed from the executable.
    #[serde(default)]
    pub launch_cwd: String,
    /// Extra env for the launch command. These are the engine knobs — STT device,
    /// LLM binary/model, TTS backend — surfaced from hf-s2s/run-comparison.sh.
    #[serde(default)]
    pub launch_env: BTreeMap<String, String>,
    /// HTTP URL polled to decide the backend is up.
    #[serde(default = "d_health_url")]
    pub health_url: String,

    /// Set after the required first-run voice and delegation setup succeeds.
    #[serde(default)]
    pub setup_completed: bool,

    // ── embedded web server (config + voice UI over HTTP) ───────────
    /// Serve the config + voice UI over HTTP so a browser or another device
    /// can use it.
    #[serde(default)]
    pub web_enabled: bool,
    /// Bind address. "127.0.0.1" = this machine only; "0.0.0.0" = the LAN.
    #[serde(default = "d_web_bind")]
    pub web_bind: String,
    #[serde(default = "d_web_port")]
    pub web_port: u16,
    /// Browser pairing code and API bearer token. Strongly recommended when
    /// binding to 0.0.0.0.
    #[serde(default)]
    pub web_token: String,
    /// Paths to a TLS cert + key (PEM). Needed for microphone access from any
    /// origin that isn't localhost. Empty = plain HTTP.
    #[serde(default)]
    pub web_tls_cert: String,
    #[serde(default)]
    pub web_tls_key: String,
    /// Publish the complete HTTP + authenticated realtime origin privately via
    /// a foreground `tailscale serve` process owned by the app.
    #[serde(default)]
    pub web_tailscale: bool,
    /// Optional Tailscale CLI override (useful for the macOS app bundle).
    #[serde(default)]
    pub web_tailscale_binary: String,
}

impl Default for Settings {
    fn default() -> Self {
        serde_json::from_str("{}").expect("empty object deserialises via serde defaults")
    }
}

impl Settings {
    pub fn config_path(app: &AppHandle) -> Result<PathBuf> {
        let dir = app
            .path()
            .app_config_dir()
            .context("no app config dir")?;
        fs::create_dir_all(&dir).ok();
        Ok(dir.join("config.json"))
    }

    pub fn load(app: &AppHandle) -> Settings {
        let Ok(path) = Self::config_path(app) else {
            return Settings::default();
        };
        match fs::read_to_string(&path) {
            Ok(raw) => {
                let mut settings = serde_json::from_str(&raw).unwrap_or_else(|e| {
                    eprintln!("[config] {path:?} is invalid ({e}); using defaults");
                    Settings::default()
                });
                // The speech stack is an implementation detail, not a user choice.
                settings.server_url = d_server_url();
                settings
            }
            Err(_) => Settings::default(),
        }
    }

    /// True when nothing that affects the local brain llama-server has changed.
    pub fn brain_local_eq(&self, o: &Settings) -> bool {
        self.brain_source == o.brain_source
            && self.brain_local_model == o.brain_local_model
            && self.llama_server_bin == o.llama_server_bin
            && self.brain_local_port == o.brain_local_port
            && self.brain_local_ctx == o.brain_local_ctx
            && self.brain_local_ngl == o.brain_local_ngl
    }

    /// True when nothing that affects the managed backend process has changed.
    pub fn backend_config_eq(&self, o: &Settings) -> bool {
        self.manage_backend == o.manage_backend
            && self.launch_command == o.launch_command
            && self.launch_cwd == o.launch_cwd
            && self.launch_env == o.launch_env
            && self.speech_model == o.speech_model
            && self.speech_model_source == o.speech_model_source
            && self.speech_remote_base_url == o.speech_remote_base_url
            && self.speech_remote_model == o.speech_remote_model
            && self.speech_remote_api_key == o.speech_remote_api_key
            && self.web_tls_cert == o.web_tls_cert
            && self.web_tls_key == o.web_tls_key
            && self.web_tailscale == o.web_tailscale
            && self.web_tailscale_binary == o.web_tailscale_binary
    }

    /// True when nothing that affects the embedded web server has changed.
    pub fn web_config_eq(&self, o: &Settings) -> bool {
        self.web_enabled == o.web_enabled
            && self.web_bind == o.web_bind
            && self.web_port == o.web_port
            && self.web_token == o.web_token
            && self.web_tls_cert == o.web_tls_cert
            && self.web_tls_key == o.web_tls_key
    }

    pub fn save(&self, app: &AppHandle) -> Result<()> {
        let path = Self::config_path(app)?;
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(&path, raw).with_context(|| format!("writing {path:?}"))?;
        Ok(())
    }
}
