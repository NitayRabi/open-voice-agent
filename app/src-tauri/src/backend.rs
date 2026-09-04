//! Supervisor for the fixed Parakeet → conversational LLM → TTS process.
//! The app injects the selected model, tails output into the UI, and stops the
//! process on quit.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::Settings;
use crate::AppState;

#[derive(Default)]
pub struct BackendManager {
    child: Mutex<Option<Child>>,
}

fn looks_like_repo(p: &std::path::Path) -> bool {
    p.join("hf-s2s").is_dir() || p.join("run.sh").is_file()
}

fn has_speech_runtime(p: &std::path::Path) -> bool {
    p.join(".tmp/speech-to-speech/.venv/bin/speech-to-speech")
        .is_file()
        && p.join(".tmp/qwen3-tts-hip/target/release/tts-server")
            .is_file()
}

fn main_worktree(p: &std::path::Path) -> Option<PathBuf> {
    let git_file = std::fs::read_to_string(p.join(".git")).ok()?;
    let git_dir = PathBuf::from(git_file.trim().strip_prefix("gitdir: ")?);
    git_dir.ancestors().nth(3).map(PathBuf::from)
}

fn repo_root_guess() -> PathBuf {
    // dev: <repo>/app/src-tauri/target/<profile>/open-voice-agent
    // Walk up from the executable and from the cwd looking for the repo markers.
    let mut fallbacks = Vec::new();
    for start in [std::env::current_exe().ok(), std::env::current_dir().ok()]
        .into_iter()
        .flatten()
    {
        let mut p = start;
        for _ in 0..8 {
            if looks_like_repo(&p) {
                if has_speech_runtime(&p) {
                    return p;
                }
                if let Some(main) = main_worktree(&p) {
                    if has_speech_runtime(&main) {
                        return main;
                    }
                }
                fallbacks.push(p.clone());
            }
            if !p.pop() {
                break;
            }
        }
    }
    fallbacks
        .into_iter()
        .next()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

impl BackendManager {
    pub fn is_running(&self) -> bool {
        let mut guard = self.child.lock();
        match guard.as_mut() {
            Some(c) => match c.try_wait() {
                Ok(Some(_)) => {
                    *guard = None;
                    false
                }
                Ok(None) => true,
                Err(_) => true,
            },
            None => false,
        }
    }

    pub fn start(&self, app: &AppHandle, cfg: &Settings) -> Result<(), String> {
        if self.is_running() {
            return Err("backend already running".into());
        }
        let mut parts = cfg.launch_command.iter();
        let program = parts.next().ok_or("launch_command is empty")?;

        let cwd = if cfg.launch_cwd.trim().is_empty() {
            repo_root_guess()
        } else {
            PathBuf::from(&cfg.launch_cwd)
        };
        let is_bundled_voice_launcher = cfg.launch_command.get(0).map(String::as_str) == Some("bash")
            && cfg.launch_command.get(1).map(String::as_str) == Some("hf-s2s/run-comparison.sh");
        if is_bundled_voice_launcher && !has_speech_runtime(&cwd) {
            return Err(
                "the Parakeet/TTS runtime is not installed; install the required voice runtime before completing Setup"
                    .into(),
            );
        }

        let mut cmd = Command::new(program);
        cmd.args(parts)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &cfg.launch_env {
            cmd.env(k, v);
        }
        if cfg.speech_model_source == "remote" {
            if cfg.speech_remote_base_url.trim().is_empty()
                || cfg.speech_remote_model.trim().is_empty()
            {
                return Err("remote conversational model requires an endpoint and model name".into());
            }
            cmd.env("HF_S2S_LLM_BASE_URL", cfg.speech_remote_base_url.trim());
            cmd.env("HF_S2S_LLM_NAME", cfg.speech_remote_model.trim());
            cmd.env("HF_S2S_LLM_API_KEY", cfg.speech_remote_api_key.trim());
        } else {
            let model = app
                .state::<AppState>()
                .assets
                .resolve(app, cfg.speech_model.trim())
                .ok_or_else(|| {
                    "speech model is missing; complete Setup and download or select a model"
                        .to_string()
                })?;
            cmd.env("HF_S2S_LLM_MODEL", model);
        }
        if let Some(tls_dir) = std::path::Path::new(cfg.web_tls_cert.trim()).parent() {
            cmd.env("SSL_DIR", tls_dir);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to launch {program}: {e}"))?;

        let stdout: Option<Box<dyn std::io::Read + Send>> =
            child.stdout.take().map(|s| Box::new(s) as _);
        let stderr: Option<Box<dyn std::io::Read + Send>> =
            child.stderr.take().map(|s| Box::new(s) as _);
        for (name, pipe) in [("stdout", stdout), ("stderr", stderr)] {
            if let Some(stream) = pipe {
                let app = app.clone();
                std::thread::spawn(move || {
                    let reader = BufReader::new(stream);
                    for line in reader.lines().map_while(Result::ok) {
                        let _ = app.emit("backend-log", format!("[{name}] {line}"));
                    }
                });
            }
        }

        let _ = app.emit(
            "backend-log",
            format!("[app] launched: {} (cwd {})", cfg.launch_command.join(" "), cwd.display()),
        );
        *self.child.lock() = Some(child);
        let _ = app.emit("backend-status", true);
        Ok(())
    }

    pub fn stop(&self, app: &AppHandle) {
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = app.emit("backend-log", "[app] backend stopped".to_string());
        }
        let _ = app.emit("backend-status", false);
    }
}
