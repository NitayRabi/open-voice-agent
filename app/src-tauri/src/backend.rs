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
    p.join("hf-s2s").is_dir()
}

fn has_speech_runtime(p: &std::path::Path) -> bool {
    let speech_cli = p.join(".tmp/speech-to-speech/.venv/bin/speech-to-speech");
    if !speech_cli.is_file() {
        return false;
    }

    // Apple Silicon uses the Python runtime's MLX Parakeet and Qwen3-TTS
    // implementations. Linux keeps using the separate native HIP TTS server.
    #[cfg(target_os = "macos")]
    return true;

    #[cfg(not(target_os = "macos"))]
    p.join(".tmp/qwen3-tts-hip/target/release/tts-server")
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

    // A locally built macOS .app may be opened from Finder after it has been
    // copied out of target/, so neither its executable nor cwd points back to
    // the source checkout that owns the separately installed voice runtime.
    // Cargo embeds the manifest directory; use that location when it still
    // exists (the normal build-from-source installation described in README).
    let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(PathBuf::from);
    if let Some(root) = manifest_root {
        if looks_like_repo(&root) {
            if has_speech_runtime(&root) {
                return root;
            }
            fallbacks.push(root);
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
        let is_bundled_voice_launcher = cfg.launch_command.get(0).map(String::as_str)
            == Some("bash")
            && cfg.launch_command.get(1).map(String::as_str) == Some("hf-s2s/run-comparison.sh");
        if is_bundled_voice_launcher && !has_speech_runtime(&cwd) {
            #[cfg(target_os = "macos")]
            return Err("the Apple Silicon Parakeet/TTS runtime is not installed; run `bash hf-s2s/setup-macos.sh` from the source checkout before completing Setup".into());

            #[cfg(not(target_os = "macos"))]
            return Err("the Parakeet/TTS runtime is not installed; install the required voice runtime before completing Setup".into());
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
        // Hand the launcher the exact cert and key the web server serves the page
        // with, so the socat wss wrapper and the page present one identity and the
        // browser only has to trust a certificate once. Passing a *directory* used
        // to be enough only by luck: the launcher looks for `cert.pem`/`key.pem`
        // inside it, so any other filename silently fell back to the launcher's own
        // development cert, which a remote browser rejects with no way to click
        // through. With neither set, leave both unset — the launcher then skips the
        // TLS wrapper entirely rather than serving a mismatched cert on it.
        let (tls_cert, tls_key) = (cfg.web_tls_cert.trim(), cfg.web_tls_key.trim());
        if !tls_cert.is_empty() && !tls_key.is_empty() {
            cmd.env("SSL_CERT", tls_cert);
            cmd.env("SSL_KEY", tls_key);
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
