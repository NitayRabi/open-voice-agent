//! Supervises a local `llama-server` for "local brain" mode: the app runs it on
//! a downloaded GGUF and points `brain::delegate` at `http://127.0.0.1:<port>/v1`.
//! The server binary itself is NOT bundled — the user installs llama.cpp or
//! points `llama_server_bin` at their build.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::Settings;
use crate::AppState;

#[derive(Default)]
pub struct LocalBrain {
    child: Mutex<Option<Child>>,
    /// the settings signature the running process was started with
    running_sig: Mutex<Option<Settings>>,
}

impl LocalBrain {
    pub fn is_running(&self) -> bool {
        let mut g = self.child.lock();
        match g.as_mut() {
            Some(c) => match c.try_wait() {
                Ok(Some(_)) => {
                    *g = None;
                    false
                }
                _ => true,
            },
            None => false,
        }
    }

    pub fn base_url(&self, cfg: &Settings) -> String {
        format!("http://127.0.0.1:{}/v1", cfg.brain_local_port)
    }

    /// Bring the local server in line with settings: stop it for remote mode,
    /// (re)start it for local mode when the config changed.
    pub fn reconcile(&self, app: &AppHandle) {
        let cfg = app.state::<AppState>().settings.lock().clone();
        if cfg.brain_source != "local" || !cfg.delegation_enabled {
            self.stop(app);
            return;
        }
        let unchanged = self
            .running_sig
            .lock()
            .as_ref()
            .map(|s| s.brain_local_eq(&cfg))
            .unwrap_or(false);
        if unchanged && self.is_running() {
            return;
        }
        self.stop(app);
        if let Err(e) = self.start(app, &cfg) {
            let _ = app.emit("backend-log", format!("[brain] {e}"));
        }
    }

    /// Make sure the server is up before a delegation; returns once the port
    /// answers or errors out.
    pub fn ensure(&self, app: &AppHandle, cfg: &Settings) -> Result<(), String> {
        let matches = self.running_sig.lock().as_ref().map(|running| running.brain_local_eq(cfg)).unwrap_or(false);
        if !matches && self.is_running() {
            self.stop(app);
        }
        if !self.is_running() {
            self.start(app, cfg)?;
        }
        let health = format!("http://127.0.0.1:{}/health", cfg.brain_local_port);
        for _ in 0..120 {
            if ureq::get(&health)
                .timeout(std::time::Duration::from_millis(800))
                .call()
                .is_ok()
            {
                return Ok(());
            }
            if !self.is_running() {
                return Err("llama-server exited during startup (see the Log tab)".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        Err("llama-server did not become ready in 60s".into())
    }

    fn start(&self, app: &AppHandle, cfg: &Settings) -> Result<(), String> {
        let model = app
            .state::<AppState>()
            .assets
            .resolve(app, &cfg.brain_local_model)
            .ok_or_else(|| {
                "no local brain model — download or add one in the Models tab".to_string()
            })?;

        // Empty means "auto-managed": reuse an existing install (Homebrew,
        // PATH) or download a pinned build, rather than requiring the user
        // to install llama.cpp and type its path in themselves.
        let bin = if cfg.llama_server_bin.trim().is_empty() {
            crate::runtime::ensure_llama_server(app)?
        } else {
            std::path::PathBuf::from(&cfg.llama_server_bin)
        };

        let mut cmd = Command::new(&bin);
        cmd.args([
            "-m",
            &model.to_string_lossy(),
            "--host",
            "127.0.0.1",
            "--port",
            &cfg.brain_local_port.to_string(),
            "-c",
            &cfg.brain_local_ctx.to_string(),
            "-ngl",
            &cfg.brain_local_ngl.to_string(),
            "--no-webui",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            format!("could not start {}: {e}", bin.display())
        })?;

        let pipes = [child.stdout.take().map(as_read), child.stderr.take().map(as_read)];
        for stream in pipes.into_iter().flatten() {
            let app = app.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    let _ = app.emit("backend-log", format!("[brain] {line}"));
                }
            });
        }

        let _ = app.emit(
            "backend-log",
            format!("[brain] llama-server on :{} ({})", cfg.brain_local_port, model.display()),
        );
        *self.child.lock() = Some(child);
        *self.running_sig.lock() = Some(cfg.clone());
        Ok(())
    }

    pub fn stop(&self, app: &AppHandle) {
        if let Some(mut c) = self.child.lock().take() {
            let _ = c.kill();
            let _ = c.wait();
            let _ = app.emit("backend-log", "[brain] llama-server stopped".to_string());
        }
        *self.running_sig.lock() = None;
    }
}

fn as_read<R: std::io::Read + Send + 'static>(r: R) -> Box<dyn std::io::Read + Send> {
    Box::new(r)
}
