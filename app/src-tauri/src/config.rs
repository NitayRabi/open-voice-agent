//! Persistent settings, stored as JSON in the platform config dir
//! (`~/.config/ai.openvoice.agent/config.json` on Linux).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// One independently configured delegation target. The speech-to-speech
/// settings intentionally stay on `Settings`; only the capable backend is
/// duplicated per named agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationAgent {
    pub id: String,
    pub alias: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "d_brain_source")]
    pub brain_source: String,
    #[serde(default = "d_brain_base_url")]
    pub brain_base_url: String,
    #[serde(default)]
    pub brain_api_key: String,
    #[serde(default = "d_brain_model")]
    pub brain_model: String,
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
    #[serde(default)]
    pub acpx_agent: String,
    #[serde(default)]
    pub acpx_custom_command: String,
    #[serde(default = "d_acpx_permissions")]
    pub acpx_permissions: String,
    #[serde(default)]
    pub acpx_model: String,
    #[serde(default)]
    pub acpx_cwd: String,
    #[serde(default)]
    pub acpx_bin: String,
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
    #[serde(default = "d_true")]
    pub delegation_speak_result: bool,
}

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

pub fn local_server_url_for(settings: &Settings) -> String {
    let port = settings
        .launch_env
        .get("HF_S2S_BACKEND_PORT")
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(BACKEND_PORT);
    format!("ws://127.0.0.1:{port}/v1/realtime")
}

#[cfg(test)]
mod local_server_url_tests {
    use super::*;

    #[test]
    fn follows_the_managed_backend_port_override() {
        let mut settings = Settings::default();
        settings
            .launch_env
            .insert("HF_S2S_BACKEND_PORT".into(), "61629".into());

        assert_eq!(
            local_server_url_for(&settings),
            "ws://127.0.0.1:61629/v1/realtime"
        );
    }

    #[test]
    fn ignores_an_invalid_managed_backend_port_override() {
        let mut settings = Settings::default();
        settings
            .launch_env
            .insert("HF_S2S_BACKEND_PORT".into(), "invalid".into());

        assert_eq!(local_server_url_for(&settings), local_server_url());
    }
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
fn d_acpx_permissions() -> String {
    "read".into()
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

/// Kokoro's built-in voices. The first letter is the language, the second the
/// gender: a=American English, b=British, e=Spanish, f=French, h=Hindi,
/// i=Italian, p=Portuguese.
///
/// Kokoro also ships Japanese (j*) and Chinese (z*) voices, left out here
/// because they need extra G2P packages this runtime does not have --
/// `pyopenjtalk` for Japanese, `ordered_set` for Chinese. Selecting one would
/// fail at synthesis time, so they are only worth listing once those install.
///
/// Only `af_heart` and `bm_fable` ship in the model snapshot; the rest are
/// fetched from the Hub on first use, so a new voice needs one online run.
pub const KOKORO_VOICES: &[&str] = &[
    "af_alloy", "af_aoede", "af_bella", "af_heart", "af_jessica", "af_kore",
    "af_nicole", "af_nova", "af_river", "af_sarah", "af_sky",
    "am_adam", "am_echo", "am_eric", "am_fenrir", "am_liam", "am_michael",
    "am_onyx", "am_puck", "am_santa",
    "bf_alice", "bf_emma", "bf_isabella", "bf_lily",
    "bm_daniel", "bm_fable", "bm_george", "bm_lewis",
    "ef_dora", "em_alex", "em_santa",
    "ff_siwis",
    "hf_alpha", "hf_beta", "hm_omega", "hm_psi",
    "if_sara", "im_nicola",
    "pf_dora", "pm_alex", "pm_santa",
];

/// Qwen3-TTS CustomVoice speakers.
pub const QWEN3_SPEAKERS: &[&str] = &[
    "Aiden", "Ryan", "Dylan", "Eric", "Ono_Anna", "Serena", "Sohee", "Uncle_Fu", "Vivian",
];

impl Settings {
    /// The voice to hand the launcher. `voice` is shared by both engines and
    /// their names do not overlap, so a voice left over from the other engine
    /// falls back to this one's default rather than failing the backend start.
    pub fn voice_for_engine(&self) -> &str {
        let v = self.voice.trim();
        match self.speech_tts.trim() {
            "kokoro" if KOKORO_VOICES.contains(&v) => v,
            "kokoro" => "bm_fable",
            _ if QWEN3_SPEAKERS.contains(&v) => v,
            _ => "Aiden",
        }
    }
}

fn d_speech_tts() -> String {
    "qwen3".into()
}
fn d_speech_stt() -> String {
    "parakeet".into()
}
fn d_decision_agent_model() -> String {
    "openjev-decider-0.7b".into()
}
fn d_decision_agent_port() -> u16 {
    8130
}
fn d_decision_agent_base_url() -> String {
    "https://api.typesafe.ai/v1".into()
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
    /// Speech recognition: "parakeet" (fastest, European languages only) or
    /// "whisper-turbo" (auto-detects the language per turn, e.g. Hebrew).
    #[serde(default = "d_speech_stt")]
    pub speech_stt: String,
    /// Speech synthesis: "qwen3" (CustomVoice, needs the GPU) or "kokoro"
    /// (82M, runs on the CPU and leaves the GPU to the model and STT).
    #[serde(default = "d_speech_tts")]
    pub speech_tts: String,
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

    // ── delegation to the "brain" ─────────────────────────────────────────
    #[serde(default = "d_true")]
    pub delegation_enabled: bool,
    /// "remote" = call an OpenAI-compatible endpoint; "local" = the app runs a
    /// llama.cpp server on a downloaded GGUF and calls that; "acpx" = hand the
    /// task to a coding agent (Claude Code, Codex, …) over ACP via acpx.
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
    /// acpx mode: an acpx built-in agent id ("claude", "codex", …), "custom"
    /// for `acpx_custom_command`, or empty to use the first installed agent.
    #[serde(default)]
    pub acpx_agent: String,
    /// acpx mode, agent "custom": a raw ACP server command (`acpx --agent`).
    #[serde(default)]
    pub acpx_custom_command: String,
    /// acpx mode: tool permissions — "read" (approve reads, deny writes),
    /// "all", or "none".
    #[serde(default = "d_acpx_permissions")]
    pub acpx_permissions: String,
    /// acpx mode: agent model id; empty = the agent's default.
    #[serde(default)]
    pub acpx_model: String,
    /// acpx mode: the agent's working directory; empty = the home directory.
    #[serde(default)]
    pub acpx_cwd: String,
    /// acpx executable; empty = auto-detected, else installed automatically.
    #[serde(default)]
    pub acpx_bin: String,
    /// Named backend profiles shown beside the orb. Empty keeps the legacy
    /// single-backend behaviour.
    #[serde(default)]
    pub delegation_agents: Vec<DelegationAgent>,
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

    /// Configurable decision agent model: "openjev-decider-0.7b" or "decider-2b".
    #[serde(default = "d_decision_agent_model")]
    pub decision_agent_model: String,
    #[serde(default = "d_decision_agent_port")]
    pub decision_agent_port: u16,
    #[serde(default)]
    pub decision_agent_api_key: String,
    #[serde(default = "d_decision_agent_base_url")]
    pub decision_agent_base_url: String,

    // ── engine / backend supervisor ──────────────────────────────────────
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
    pub fn validate_delegation_agents(&self) -> Result<()> {
        let mut ids = std::collections::HashSet::new();
        for agent in &self.delegation_agents {
            let id = agent.id.trim();
            if id.is_empty() || id.len() > 128 {
                anyhow::bail!("agent profile ids must contain 1–128 characters");
            }
            if agent.alias.trim().is_empty() {
                anyhow::bail!("agent profile {id:?} needs a display name");
            }
            if !ids.insert(id) {
                anyhow::bail!("duplicate agent profile id {id:?}");
            }
            if !matches!(agent.brain_source.as_str(), "acpx" | "acp" | "remote" | "local") {
                anyhow::bail!("agent profile {:?} has an unknown backend type", agent.alias);
            }
        }
        Ok(())
    }

    /// Apply a client-selected backend profile to a copy of these settings.
    /// Unknown ids are rejected instead of silently sending work elsewhere.
    pub fn for_delegation_agent(&self, selected: Option<&str>) -> Result<Option<Settings>> {
        let Some(selected) = selected.map(str::trim).filter(|id| !id.is_empty() && *id != "orchestrator") else {
            return Ok(None);
        };
        let agent = self
            .delegation_agents
            .iter()
            .find(|agent| agent.id == selected)
            .with_context(|| format!("unknown delegation agent {selected:?}"))?;
        let mut out = self.clone();
        out.brain_source = agent.brain_source.clone();
        out.brain_base_url = agent.brain_base_url.clone();
        out.brain_api_key = agent.brain_api_key.clone();
        out.brain_model = agent.brain_model.clone();
        out.brain_local_model = agent.brain_local_model.clone();
        out.llama_server_bin = agent.llama_server_bin.clone();
        out.brain_local_port = agent.brain_local_port;
        out.brain_local_ctx = agent.brain_local_ctx;
        out.brain_local_ngl = agent.brain_local_ngl;
        out.acpx_agent = agent.acpx_agent.clone();
        out.acpx_custom_command = agent.acpx_custom_command.clone();
        out.acpx_permissions = agent.acpx_permissions.clone();
        out.acpx_model = agent.acpx_model.clone();
        out.acpx_cwd = agent.acpx_cwd.clone();
        out.acpx_bin = agent.acpx_bin.clone();
        out.brain_system_prompt = agent.brain_system_prompt.clone();
        out.brain_temperature = agent.brain_temperature;
        if !agent.delegation_tool_name.is_empty() {
            out.delegation_tool_name = agent.delegation_tool_name.clone();
        }
        if !agent.description.is_empty() {
            out.delegation_tool_description = agent.description.clone();
        } else if !agent.delegation_tool_description.is_empty() {
            out.delegation_tool_description = agent.delegation_tool_description.clone();
        }
        out.delegation_timeout_s = agent.delegation_timeout_s;
        out.delegation_speak_result = agent.delegation_speak_result;
        Ok(Some(out))
    }

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
                // The speech stack is an implementation detail, not a user
                // choice, but managed side-by-side/dev installs may assign it
                // a different loopback port. The client and launcher must use
                // the same value.
                settings.server_url = local_server_url_for(&settings);
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

    pub fn is_local_decider(&self) -> bool {
        self.decision_agent_model != "typesafe-jev-api"
    }

    /// True when nothing that affects the decider server has changed.
    pub fn decider_local_eq(&self, o: &Settings) -> bool {
        self.decision_agent_model == o.decision_agent_model
            && self.decision_agent_port == o.decision_agent_port
            && self.llama_server_bin == o.llama_server_bin
            && self.is_local_decider() == o.is_local_decider()
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
            && self.speech_stt == o.speech_stt
            && self.speech_tts == o.speech_tts
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
            && self.web_tailscale == o.web_tailscale
            && self.web_tailscale_binary == o.web_tailscale_binary
    }

    pub fn save(&self, app: &AppHandle) -> Result<()> {
        let path = Self::config_path(app)? ;
        let json = serde_json::to_string_pretty(self).context("serialise settings")?;
        fs::write(&path, json).with_context(|| format!("write {path:?}"))
    }
}


#[cfg(test)]
mod voice_tests {
    use super::{Settings, KOKORO_VOICES, QWEN3_SPEAKERS};

    /// The two engines share one `voice` setting, so a voice belonging to the
    /// other engine must not reach the launcher — Kokoro cannot speak "Aiden"
    /// and Qwen3 cannot speak "bm_fable", and either would fail at synthesis.
    #[test]
    fn voice_falls_back_when_it_belongs_to_the_other_engine() {
        let mut cfg = Settings::default();

        cfg.speech_tts = "kokoro".into();
        cfg.voice = "Aiden".into();
        assert_eq!(cfg.voice_for_engine(), "bm_fable");

        cfg.speech_tts = "qwen3".into();
        cfg.voice = "bm_fable".into();
        assert_eq!(cfg.voice_for_engine(), "Aiden");
    }

    #[test]
    fn a_voice_matching_the_engine_is_kept() {
        let mut cfg = Settings::default();

        cfg.speech_tts = "kokoro".into();
        cfg.voice = "am_michael".into();
        assert_eq!(cfg.voice_for_engine(), "am_michael");

        cfg.speech_tts = "qwen3".into();
        cfg.voice = "Vivian".into();
        assert_eq!(cfg.voice_for_engine(), "Vivian");
    }

    /// Every listed voice's first letter is its Kokoro language code, which is
    /// what the launcher derives `--kokoro_lang_code` from.
    #[test]
    fn kokoro_voice_names_encode_a_supported_language() {
        for voice in KOKORO_VOICES {
            let lang = voice.chars().next().unwrap();
            assert!(
                "abefhip".contains(lang),
                "{voice} starts with unsupported language code {lang:?}"
            );
            assert!(voice.len() > 3 && voice.as_bytes()[2] == b'_', "{voice} is not xx_name");
        }
        // Japanese and Chinese need G2P packages the runtime does not have.
        assert!(!KOKORO_VOICES.iter().any(|v| v.starts_with('j') || v.starts_with('z')));
    }

    /// The UI's voice list and the validation table must not drift apart.
    #[test]
    fn defaults_are_present_in_their_engine_tables() {
        assert!(KOKORO_VOICES.contains(&"bm_fable"));
        assert!(QWEN3_SPEAKERS.contains(&"Aiden"));
        assert_eq!(Settings::default().voice, "Aiden");
    }
}
