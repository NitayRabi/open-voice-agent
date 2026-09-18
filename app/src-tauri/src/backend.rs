//! Supervisor for the fixed Parakeet → conversational LLM → TTS process.
//! The app injects the selected model, tails output into the UI, and stops the
//! process on quit.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::Settings;
use crate::runtime::has_speech_runtime;
use crate::AppState;

pub struct BackendManager {
    child: Mutex<Option<Child>>,
    /// Serializes spawn and stop so concurrent desktop/web requests cannot
    /// launch duplicate stacks or race a shutdown.
    launch_lock: Mutex<()>,
    /// guards against two overlapping automatic runtime installs
    installing: AtomicBool,
    /// Last signal from an active voice client, as Unix epoch milliseconds.
    last_activity_ms: AtomicU64,
    /// The watchdog is intentionally one thread for the lifetime of the app.
    watchdog_started: AtomicBool,
}

const IDLE_TIMEOUT: Duration = Duration::from_secs(3 * 60);
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
const TERMINATE_GRACE: Duration = Duration::from_secs(5);

impl Default for BackendManager {
    fn default() -> Self {
        Self {
            child: Mutex::new(None),
            launch_lock: Mutex::new(()),
            installing: AtomicBool::new(false),
            last_activity_ms: AtomicU64::new(now_ms()),
            watchdog_started: AtomicBool::new(false),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn looks_like_repo(p: &std::path::Path) -> bool {
    p.join("hf-s2s").is_dir()
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
                Ok(Some(status)) => {
                    eprintln!("[backend] process exited with status: {status:?}");
                    #[cfg(unix)]
                    unsafe {
                        // The supervisor normally cleans its group via its EXIT
                        // trap. Cover an abnormal supervisor exit as well.
                        libc::kill(-(c.id() as i32), libc::SIGKILL);
                    }
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
        self.touch(app);
        if self.is_running() {
            return Ok(());
        }
        if cfg.launch_command.is_empty() {
            return Err("launch_command is empty".into());
        }
        let cwd = if cfg.launch_cwd.trim().is_empty() {
            repo_root_guess()
        } else {
            PathBuf::from(&cfg.launch_cwd)
        };
        let is_bundled_voice_launcher = cfg.launch_command.get(0).map(String::as_str)
            == Some("bash")
            && cfg.launch_command.get(1).map(String::as_str) == Some("hf-s2s/run-comparison.sh");

        #[cfg(target_os = "macos")]
        if is_bundled_voice_launcher {
            self.start_with_auto_install(app, cfg, cwd);
            return Ok(());
        }

        #[cfg(not(target_os = "macos"))]
        if is_bundled_voice_launcher && !has_speech_runtime(&cwd) {
            return Err("the Parakeet/TTS runtime is not installed; install the required voice runtime before completing Setup".into());
        }

        self.spawn_process(app, cfg)
    }

    /// Make sure `llama-server` and (if needed) the Python speech runtime are
    /// installed before spawning, doing the installation lazily in the
    /// background so the caller never has to run anything by hand. A no-op
    /// beyond a couple of cheap checks once everything is already in place.
    #[cfg(target_os = "macos")]
    fn start_with_auto_install(&self, app: &AppHandle, cfg: &Settings, cwd: PathBuf) {
        if self
            .installing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            let _ = app.emit(
                "backend-log",
                "[app] voice runtime setup already in progress…".to_string(),
            );
            return;
        }
        let app = app.clone();
        let cfg = cfg.clone();
        std::thread::spawn(move || {
            let outcome = (|| -> Result<Settings, String> {
                let mut cfg = cfg;
                if cfg.speech_model_source != "remote" {
                    let llama_bin = crate::runtime::ensure_llama_server(&app)?;
                    cfg.launch_env
                        .entry("HF_S2S_LLM_BIN".into())
                        .or_insert_with(|| llama_bin.display().to_string());
                }
                if !has_speech_runtime(&cwd) {
                    let _ = app.emit(
                        "backend-log",
                        "[app] voice runtime not installed yet; installing it automatically \
                         (first time only, can take several minutes)…"
                            .to_string(),
                    );
                    crate::runtime::ensure_speech_runtime(&app, &cwd)?;
                }
                Ok(cfg)
            })();

            let state = app.state::<AppState>();
            state.backend.installing.store(false, Ordering::SeqCst);
            match outcome {
                Ok(cfg) => {
                    if !state.backend.is_running() {
                        if let Err(e) = state.backend.spawn_process(&app, &cfg) {
                            let _ = app.emit(
                                "backend-log",
                                format!("[app] voice engine failed to start: {e}"),
                            );
                        }
                    }
                }
                Err(e) => {
                    let _ = app.emit(
                        "backend-log",
                        format!("[app] automatic runtime setup failed: {e}"),
                    );
                }
            }
        });
    }

    fn spawn_process(&self, app: &AppHandle, cfg: &Settings) -> Result<(), String> {
        let _launch = self.launch_lock.lock();
        if self.is_running() {
            self.touch(app);
            return Ok(());
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
        // The launcher and every model process it creates share a dedicated
        // process group. This lets shutdown reach the whole voice stack, while
        // still giving Bash's cleanup trap a chance to reap its children.
        #[cfg(unix)]
        cmd.process_group(0);
        for (k, v) in &cfg.launch_env {
            cmd.env(k, v);
        }
        if cfg.speech_model_source == "remote" {
            if cfg.speech_remote_base_url.trim().is_empty()
                || cfg.speech_remote_model.trim().is_empty()
            {
                return Err(
                    "remote conversational model requires an endpoint and model name".into(),
                );
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
                        eprintln!("[{name}] {line}");
                        let _ = app.emit("backend-log", format!("[{name}] {line}"));
                    }
                });
            }
        }

        eprintln!(
            "[app] launched: {} (cwd {})",
            cfg.launch_command.join(" "),
            cwd.display()
        );
        let _ = app.emit(
            "backend-log",
            format!(
                "[app] launched: {} (cwd {})",
                cfg.launch_command.join(" "),
                cwd.display()
            ),
        );
        *self.child.lock() = Some(child);
        self.touch(app);
        let _ = app.emit("backend-status", true);
        Ok(())
    }

    /// Record that a voice UI is warming or actively using the engine.
    pub fn touch(&self, app: &AppHandle) {
        self.last_activity_ms.store(now_ms(), Ordering::SeqCst);
        if self
            .watchdog_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let app = app.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(WATCHDOG_INTERVAL);
                let state = app.state::<AppState>();
                if !state.backend.is_running() {
                    continue;
                }
                let idle_ms =
                    now_ms().saturating_sub(state.backend.last_activity_ms.load(Ordering::SeqCst));
                if idle_ms >= IDLE_TIMEOUT.as_millis() as u64 {
                    let _ = app.emit(
                        "backend-log",
                        "[app] voice engine idle for 3 minutes; releasing GPU memory".to_string(),
                    );
                    state.backend.stop(&app);
                }
            });
        }
    }

    /// Cheap health probe used while the UI displays its warming state.
    pub fn is_ready(&self, health_url: &str) -> bool {
        if !self.is_running() {
            return false;
        }
        match ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_millis(400))
            .timeout_read(Duration::from_millis(400))
            .build()
            .get(health_url)
            .call()
        {
            Ok(_) | Err(ureq::Error::Status(_, _)) => true,
            Err(ureq::Error::Transport(_)) => false,
        }
    }

    pub fn stop(&self, app: &AppHandle) {
        let _launch = self.launch_lock.lock();
        if let Some(child) = self.child.lock().take() {
            terminate_tree(child);
            let _ = app.emit("backend-log", "[app] backend stopped".to_string());
        }
        let _ = app.emit("backend-status", false);
    }
}

fn terminate_tree(mut child: Child) {
    #[cfg(unix)]
    {
        let process_group = -(child.id() as i32);
        // SAFETY: kill(2) is called with a process-group id created by
        // CommandExt::process_group above and constant, valid signals.
        unsafe {
            libc::kill(process_group, libc::SIGTERM);
        }
        let deadline = std::time::Instant::now() + TERMINATE_GRACE;
        let mut leader_reaped = false;
        while std::time::Instant::now() < deadline {
            if !leader_reaped {
                leader_reaped = matches!(child.try_wait(), Ok(Some(_)));
            }
            // Signal 0 checks the entire group. Do not stop merely because the
            // Bash leader exited: a stubborn model worker may still own VRAM.
            let group_exists = unsafe { libc::kill(process_group, 0) == 0 };
            if !group_exists {
                if !leader_reaped {
                    let _ = child.wait();
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        unsafe {
            libc::kill(process_group, libc::SIGKILL);
        }
        if !leader_reaped {
            let _ = child.wait();
        }
    }

    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}
