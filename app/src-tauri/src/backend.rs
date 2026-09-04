//! Optional supervisor for the speech backend process (the Hugging Face
//! `speech-to-speech` cascade, or the VoiceChat 11B stack). The app can start it
//! with the engine knobs from Settings passed through as environment, tail its
//! output into the UI, and stop it on quit.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};

use crate::config::Settings;

#[derive(Default)]
pub struct BackendManager {
    child: Mutex<Option<Child>>,
}

fn looks_like_repo(p: &std::path::Path) -> bool {
    p.join("hf-s2s").is_dir() || p.join("run.sh").is_file()
}

fn repo_root_guess() -> PathBuf {
    // dev: <repo>/app/src-tauri/target/<profile>/open-voice-agent
    // Walk up from the executable and from the cwd looking for the repo markers.
    for start in [std::env::current_exe().ok(), std::env::current_dir().ok()]
        .into_iter()
        .flatten()
    {
        let mut p = start;
        for _ in 0..8 {
            if looks_like_repo(&p) {
                return p;
            }
            if !p.pop() {
                break;
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
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

        let mut cmd = Command::new(program);
        cmd.args(parts)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &cfg.launch_env {
            cmd.env(k, v);
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
