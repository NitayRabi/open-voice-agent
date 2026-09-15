//! Delegation to a coding agent over the Agent Client Protocol, through
//! [`acpx`](https://github.com/openclaw/acpx) — a headless ACP client that
//! speaks the same protocol to Claude Code, Codex, Gemini CLI, and the rest.
//!
//! Nothing is bundled. The app finds `acpx` on the user's PATH (a login-shell
//! PATH, so fnm/nvm/Homebrew installs are seen from a GUI launch too), or
//! installs a pinned build into the app data dir with npm — downloading Node.js
//! itself first when no new-enough one exists. Agents are detected by their
//! CLI; the ones that ship on npm can be installed the same way.
//!
//! Each delegation is one stateless `acpx <agent> exec`, prompt on stdin, final
//! answer on stdout (`--format quiet`).

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::Settings;
use crate::runtime;

/// Pinned known-good acpx, installed when no new-enough one is on PATH.
const ACPX_VERSION: &str = "0.15.1";
/// Oldest system acpx we reuse; older builds lack agent profiles we list.
const ACPX_MIN_SYSTEM: (u64, u64, u64) = (0, 15, 0);
/// acpx's own `engines.node` requirement.
const NODE_MIN: (u64, u64, u64) = (22, 13, 0);
/// Pinned Node.js LTS, downloaded only when no suitable one exists.
const NODE_VERSION: &str = "24.21.0";

/// The special agent id that runs `acpx_custom_command` via `--agent`.
pub const CUSTOM_AGENT: &str = "custom";

pub struct AgentProfile {
    /// acpx's built-in agent name.
    pub id: &'static str,
    pub label: &'static str,
    /// Executables that mean the agent is installed. Empty = acpx fetches the
    /// adapter with `npx` on first use, so only Node.js is required.
    pub bins: &'static [&'static str],
    /// npm package that provides `bins`, when the app can install it.
    pub npm: Option<&'static str>,
    /// Whether the adapter honours `--append-system-prompt`; otherwise the
    /// brain prompt is prepended to the task text.
    pub system_prompt_flag: bool,
}

const fn agent(
    id: &'static str,
    label: &'static str,
    bins: &'static [&'static str],
    npm: Option<&'static str>,
) -> AgentProfile {
    AgentProfile { id, label, bins, npm, system_prompt_flag: false }
}

/// acpx 0.15's built-in agent registry, in auto-selection preference order.
pub const AGENTS: &[AgentProfile] = &[
    AgentProfile {
        system_prompt_flag: true,
        ..agent("claude", "Claude Code", &["claude"], Some("@anthropic-ai/claude-code"))
    },
    agent("codex", "Codex", &["codex"], Some("@openai/codex")),
    agent("gemini", "Gemini CLI", &["gemini"], Some("@google/gemini-cli")),
    agent("opencode", "OpenCode", &[], None),
    agent("copilot", "GitHub Copilot CLI", &["copilot"], Some("@github/copilot")),
    agent("cursor", "Cursor Agent", &["cursor-agent"], None),
    agent("qwen", "Qwen Code", &["qwen"], Some("@qwen-code/qwen-code")),
    agent("droid", "Factory Droid", &["droid"], Some("droid")),
    agent("pi", "Pi", &["pi"], Some("@mariozechner/pi-coding-agent")),
    agent("openclaw", "OpenClaw", &["openclaw"], Some("openclaw")),
    agent("kilocode", "Kilo Code", &[], None),
    agent("mux", "Mux", &[], None),
    agent("iflow", "iFlow CLI", &["iflow"], Some("@iflow-ai/iflow-cli")),
    agent("kimi", "Kimi CLI", &["kimi"], None),
    agent("kiro", "Kiro CLI", &["kiro-cli-chat"], None),
    agent("grok-build", "Grok Build", &["grok"], None),
    agent("fast-agent", "fast-agent", &["uvx"], None),
    agent("qoder", "Qoder CLI", &["qodercli"], None),
    agent("trae", "Trae CLI", &["traecli"], None),
    agent("mcode", "mcode", &["mcode"], None),
    agent("pool", "Pool", &["pool"], None),
    agent("zeroclaw", "ZeroClaw", &["zeroclaw"], None),
];

pub fn profile(id: &str) -> Option<&'static AgentProfile> {
    AGENTS.iter().find(|a| a.id == id)
}

// ── status reported to the UI ──────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct AgentStatus {
    pub id: &'static str,
    pub label: &'static str,
    /// "ready" (CLI found), "npx" (adapter fetched on first use), "missing".
    pub status: &'static str,
    pub path: Option<String>,
    /// The app can install it with npm.
    pub installable: bool,
}

#[derive(Serialize, Clone)]
pub struct AcpxStatus {
    /// Resolved acpx executable, if any.
    pub path: Option<String>,
    pub version: Option<String>,
    /// "custom" (settings), "managed" (app-installed), "system", or "missing".
    pub source: &'static str,
    /// Version of the Node.js acpx will run on, if one is usable.
    pub node: Option<String>,
    pub installing: bool,
    /// The agent `acpx_agent = ""` resolves to right now.
    pub auto_agent: Option<&'static str>,
    pub agents: Vec<AgentStatus>,
}

#[derive(Default)]
pub struct Acpx {
    installing: AtomicBool,
    /// Serialises npm installs into the shared runtime dirs.
    install_lock: Mutex<()>,
}

impl Acpx {
    pub fn status(&self, app: &AppHandle, cfg: &Settings) -> AcpxStatus {
        let dirs = search_dirs(app);
        let (path, source) = match locate_acpx(app, cfg, &dirs) {
            Some((p, s)) => (Some(p), s),
            None => (None, "missing"),
        };
        let version = path.as_deref().and_then(|p| tool_version(p, &dirs));
        let node = find_node(app, &dirs).map(|(_, v)| format_version(v));
        let agents: Vec<AgentStatus> = AGENTS.iter().map(|a| agent_status(a, &dirs)).collect();
        let auto_agent = pick_auto_agent(&agents);
        AcpxStatus {
            path: path.map(|p| p.display().to_string()),
            version,
            source,
            node,
            installing: self.installing.load(Ordering::Relaxed),
            auto_agent,
            agents,
        }
    }

    /// Resolve a runnable acpx, installing Node.js and acpx when needed.
    pub fn ensure(&self, app: &AppHandle, cfg: &Settings) -> Result<PathBuf, String> {
        let dirs = search_dirs(app);
        if let Some((p, _)) = locate_acpx(app, cfg, &dirs) {
            return Ok(p);
        }
        let _guard = self.install_lock.lock();
        // Another caller may have finished the install while we waited.
        if let Some((p, _)) = locate_acpx(app, cfg, &search_dirs(app)) {
            return Ok(p);
        }
        self.installing.store(true, Ordering::Relaxed);
        let _ = app.emit("acpx-status", ());
        let out = install_acpx(app);
        self.installing.store(false, Ordering::Relaxed);
        let _ = app.emit("acpx-status", ());
        out
    }

    /// Install an agent CLI from npm into the app's runtime dir.
    pub fn install_agent(&self, app: &AppHandle, id: &str) -> Result<(), String> {
        let profile = profile(id).ok_or_else(|| format!("unknown agent '{id}'"))?;
        let package = profile
            .npm
            .ok_or_else(|| format!("{} can't be installed automatically", profile.label))?;
        let _guard = self.install_lock.lock();
        self.installing.store(true, Ordering::Relaxed);
        let _ = app.emit("acpx-status", ());
        let out = (|| {
            let node_bin = ensure_node(app)?;
            let prefix = agents_dir(app)?;
            runtime::log(app, format!("installing {} ({package})…", profile.label));
            npm_install(app, &node_bin, &prefix, package)?;
            runtime::log(app, format!("{} installed.", profile.label));
            Ok(())
        })();
        self.installing.store(false, Ordering::Relaxed);
        let _ = app.emit("acpx-status", ());
        out
    }

    /// Install acpx in the background once ACP delegation is selected, so the
    /// first spoken task doesn't wait on npm.
    pub fn reconcile(&self, app: &AppHandle) {
        let cfg = app.state::<crate::AppState>().settings.lock().clone();
        if cfg.brain_source != "acpx" || !cfg.delegation_enabled {
            return;
        }
        if locate_acpx(app, &cfg, &search_dirs(app)).is_some()
            || self.installing.load(Ordering::Relaxed)
        {
            return;
        }
        let app = app.clone();
        std::thread::spawn(move || {
            let st = app.state::<crate::AppState>();
            if let Err(e) = st.acpx.ensure(&app, &cfg) {
                let _ = app.emit("backend-log", format!("[acpx] {e}"));
            }
        });
    }
}

// ── delegation ─────────────────────────────────────────────────────────────

/// Blocking. Run one `acpx exec` against the configured agent.
pub fn delegate(app: &AppHandle, cfg: &Settings, request: &str) -> Result<String> {
    let st = app.state::<crate::AppState>();
    let bin = st.acpx.ensure(app, cfg).map_err(|e| anyhow!("acpx: {e}"))?;
    let dirs = search_dirs(app);

    let agent_id = match cfg.acpx_agent.trim() {
        "" => {
            let statuses: Vec<AgentStatus> =
                AGENTS.iter().map(|a| agent_status(a, &dirs)).collect();
            pick_auto_agent(&statuses).ok_or_else(|| {
                anyhow!("no ACP agent found — install one (e.g. Claude Code or Codex) or pick one in Settings → Delegation")
            })?
        }
        id => id,
    };
    if agent_id == CUSTOM_AGENT && cfg.acpx_custom_command.trim().is_empty() {
        bail!("no custom ACP agent command configured — set one in Settings → Delegation");
    }
    if agent_id != CUSTOM_AGENT && profile(agent_id).is_none() {
        bail!("unknown ACP agent '{agent_id}'");
    }

    let cwd = working_dir(app, cfg)?;
    let timeout_s = cfg.delegation_timeout_s.max(5);
    let args = build_args(cfg, agent_id, &cwd, timeout_s);
    let prompt = build_prompt(cfg, agent_id, request);

    let _ = app.emit("backend-log", format!("[acpx] {agent_id} ← {}", crate::brain::squash(request)));
    let mut child = Command::new(&bin)
        .args(&args)
        .current_dir(&cwd)
        .env("PATH", join_path(&dirs))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("could not start {}: {e}", bin.display()))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())?;
    }

    let stdout = child.stdout.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
            out
        })
    });
    let stderr = child.stderr.take().map(|s| {
        let app = app.clone();
        std::thread::spawn(move || {
            let mut tail = Vec::new();
            for line in BufReader::new(s).lines().map_while(Result::ok) {
                let _ = app.emit("backend-log", format!("[acpx] {line}"));
                tail.push(line);
                if tail.len() > 8 {
                    tail.remove(0);
                }
            }
            tail
        })
    });

    // acpx enforces --timeout itself; this is the backstop for a wedged adapter.
    let deadline = Instant::now() + Duration::from_secs(timeout_s + 15);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{agent_id} did not answer within {timeout_s}s");
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let answer = stdout.and_then(|t| t.join().ok()).unwrap_or_default();
    let tail = stderr.and_then(|t| t.join().ok()).unwrap_or_default();

    if !status.success() {
        let detail = tail
            .iter()
            .filter(|l| !l.starts_with("[acpx] tokens:"))
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        // The full diagnostic is already in the Log tab; this text gets spoken.
        if detail.contains("Failed to spawn agent command") {
            let label = profile(agent_id).map(|p| p.label).unwrap_or("The custom ACP agent");
            bail!("{label} isn't installed or isn't on PATH");
        }
        bail!("{agent_id} failed ({status}): {}", crate::brain::squash(detail.trim()));
    }
    let answer = answer.trim();
    if answer.is_empty() {
        bail!("{agent_id} returned an empty answer");
    }
    Ok(crate::brain::squash(answer))
}

fn working_dir(app: &AppHandle, cfg: &Settings) -> Result<PathBuf> {
    let configured = cfg.acpx_cwd.trim();
    let dir = if configured.is_empty() {
        app.path().home_dir().map_err(|e| anyhow!("no home directory: {e}"))?
    } else {
        PathBuf::from(configured)
    };
    if !dir.is_dir() {
        bail!("agent working directory {} does not exist", dir.display());
    }
    Ok(dir)
}

/// The acpx argv (everything after the executable). The prompt goes on stdin
/// (`-f -`), so task text can never be parsed as a flag.
pub fn build_args(cfg: &Settings, agent_id: &str, cwd: &Path, timeout_s: u64) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--format".into(),
        "quiet".into(),
        "--cwd".into(),
        cwd.display().to_string(),
        "--timeout".into(),
        timeout_s.to_string(),
        "--auth-policy".into(),
        "fail".into(),
        "--non-interactive-permissions".into(),
        "deny".into(),
    ];
    args.push(
        match cfg.acpx_permissions.as_str() {
            "all" => "--approve-all",
            "none" => "--deny-all",
            _ => "--approve-reads",
        }
        .into(),
    );
    let model = cfg.acpx_model.trim();
    if !model.is_empty() {
        args.extend(["--model".into(), model.into()]);
    }
    let system = cfg.brain_system_prompt.trim();
    let flag = profile(agent_id).map(|p| p.system_prompt_flag).unwrap_or(false);
    if flag && !system.is_empty() {
        args.extend(["--append-system-prompt".into(), system.into()]);
    }
    if agent_id == CUSTOM_AGENT {
        args.extend(["--agent".into(), cfg.acpx_custom_command.trim().into()]);
    } else {
        args.push(agent_id.into());
    }
    args.extend(["exec".into(), "-f".into(), "-".into()]);
    args
}

/// The stdin prompt. Agents without a system-prompt hook get the brain prompt
/// inline, ahead of the task.
pub fn build_prompt(cfg: &Settings, agent_id: &str, request: &str) -> String {
    let system = cfg.brain_system_prompt.trim();
    let flag = profile(agent_id).map(|p| p.system_prompt_flag).unwrap_or(false);
    if flag || system.is_empty() {
        request.trim().to_string()
    } else {
        format!("{system}\n\nTask: {}", request.trim())
    }
}

// ── discovery ──────────────────────────────────────────────────────────────

fn exe_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{name}.cmd"), format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    }
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    p.is_file()
}

fn find_bin(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .flat_map(|d| exe_names(name).into_iter().map(move |n| d.join(n)))
        .find(|p| is_executable(p))
}

fn managed_dir(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    Ok(runtime::runtime_dir(app)?.join(name))
}

fn acpx_prefix(app: &AppHandle) -> Result<PathBuf, String> {
    managed_dir(app, "acpx")
}

fn agents_dir(app: &AppHandle) -> Result<PathBuf, String> {
    managed_dir(app, "agents")
}

fn managed_node_bin(app: &AppHandle) -> Option<PathBuf> {
    let (os, arch) = node_platform()?;
    let root = managed_dir(app, "node").ok()?;
    let dir = root.join(format!("node-v{NODE_VERSION}-{os}-{arch}")).join("bin");
    dir.join("node").is_file().then_some(dir)
}

/// PATH as a login shell sees it. Apps launched from Finder or a desktop
/// launcher inherit a bare PATH that misses fnm/nvm/Homebrew/~/.local installs.
fn login_shell_path() -> &'static [PathBuf] {
    static CACHE: OnceLock<Vec<PathBuf>> = OnceLock::new();
    CACHE.get_or_init(|| {
        #[cfg(unix)]
        {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            let Ok(mut child) = Command::new(shell)
                .args(["-lc", "printf '__OVA_PATH__%s' \"$PATH\""])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            else {
                return Vec::new();
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50))
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Vec::new();
                    }
                }
            }
            let mut out = String::new();
            if let Some(mut s) = child.stdout.take() {
                let _ = s.read_to_string(&mut out);
            }
            out.rsplit_once("__OVA_PATH__")
                .map(|(_, p)| std::env::split_paths(p.trim()).collect())
                .unwrap_or_default()
        }
        #[cfg(not(unix))]
        Vec::new()
    })
}

/// Where to look for acpx, Node.js, and agent CLIs — also the PATH handed to
/// acpx, since its adapters run through `npx` and the agents' own CLIs.
pub fn search_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(node) = managed_node_bin(app) {
        dirs.push(node);
    }
    for prefix in [acpx_prefix(app), agents_dir(app)].into_iter().flatten() {
        dirs.push(prefix.join("node_modules").join(".bin"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.extend(login_shell_path().iter().cloned());
    if let Ok(home) = app.path().home_dir() {
        for rel in [".local/bin", ".npm-global/bin", ".bun/bin", ".volta/bin", ".cargo/bin", "bin"] {
            dirs.push(home.join(rel));
        }
    }
    for abs in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        dirs.push(PathBuf::from(abs));
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

fn join_path(dirs: &[PathBuf]) -> std::ffi::OsString {
    std::env::join_paths(dirs).unwrap_or_default()
}

fn agent_status(a: &'static AgentProfile, dirs: &[PathBuf]) -> AgentStatus {
    let found = a.bins.iter().find_map(|b| find_bin(b, dirs));
    let status = match (&found, a.bins.is_empty()) {
        (Some(_), _) => "ready",
        (None, true) => "npx",
        (None, false) => "missing",
    };
    AgentStatus {
        id: a.id,
        label: a.label,
        status,
        path: found.map(|p| p.display().to_string()),
        installable: a.npm.is_some() && status == "missing",
    }
}

/// First installed agent, in `AGENTS` preference order.
pub fn pick_auto_agent(agents: &[AgentStatus]) -> Option<&'static str> {
    agents.iter().find(|a| a.status == "ready").map(|a| a.id)
}

fn locate_acpx(
    app: &AppHandle,
    cfg: &Settings,
    dirs: &[PathBuf],
) -> Option<(PathBuf, &'static str)> {
    let custom = cfg.acpx_bin.trim();
    if !custom.is_empty() {
        return Some((PathBuf::from(custom), "custom"));
    }
    let managed = acpx_prefix(app).ok()?.join("node_modules").join(".bin");
    if let Some(p) = find_bin("acpx", std::slice::from_ref(&managed)) {
        return Some((p, "managed"));
    }
    // A system acpx is only useful with a Node.js it can run on.
    find_node(app, dirs)?;
    let system = find_bin("acpx", dirs)?;
    let version = tool_version(&system, dirs).and_then(|v| parse_version(&v))?;
    (version >= ACPX_MIN_SYSTEM).then_some((system, "system"))
}

fn tool_version(bin: &Path, dirs: &[PathBuf]) -> Option<String> {
    let out = Command::new(bin)
        .arg("--version")
        .env("PATH", join_path(dirs))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let v = text.trim().trim_start_matches('v');
    (out.status.success() && !v.is_empty()).then(|| v.to_string())
}

pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim().trim_start_matches('v').split(['.', '-', '+']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().unwrap_or(0);
    Some((major, minor, patch))
}

fn format_version((a, b, c): (u64, u64, u64)) -> String {
    format!("{a}.{b}.{c}")
}

/// A Node.js new enough for acpx: returns its bin dir and version.
fn find_node(app: &AppHandle, dirs: &[PathBuf]) -> Option<(PathBuf, (u64, u64, u64))> {
    if let Some(dir) = managed_node_bin(app) {
        return Some((dir, parse_version(NODE_VERSION)?));
    }
    dirs.iter().find_map(|d| {
        let node = find_bin("node", std::slice::from_ref(d))?;
        let v = tool_version(&node, dirs).and_then(|v| parse_version(&v))?;
        if v < NODE_MIN {
            return None;
        }
        Some((node.parent()?.to_path_buf(), v))
    })
}

// ── installation ───────────────────────────────────────────────────────────

fn node_platform() -> Option<(&'static str, &'static str)> {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        return None;
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        return None;
    };
    Some((os, arch))
}

/// A usable Node.js bin dir: an existing install, else a pinned download.
fn ensure_node(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some((dir, _)) = find_node(app, &search_dirs(app)) {
        return Ok(dir);
    }
    let (os, arch) = node_platform().ok_or_else(|| {
        format!("Node.js {} or newer is required — install it and try again", format_version(NODE_MIN))
    })?;
    let root = managed_dir(app, "node")?;
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let name = format!("node-v{NODE_VERSION}-{os}-{arch}");
    runtime::log(app, format!("downloading Node.js {NODE_VERSION} (runs acpx and its agents)…"));
    let archive = root.join(format!("{name}.tar.gz"));
    let url = format!("https://nodejs.org/dist/v{NODE_VERSION}/{name}.tar.gz");
    runtime::download_file(&url, &archive).map_err(|e| format!("Node.js download failed: {e}"))?;
    runtime::extract_tar_gz(&archive, &root)?;
    let _ = std::fs::remove_file(&archive);
    managed_node_bin(app).ok_or_else(|| "Node.js download did not produce the expected binary".into())
}

fn npm_install(app: &AppHandle, node_bin: &Path, prefix: &Path, package: &str) -> Result<(), String> {
    std::fs::create_dir_all(prefix).map_err(|e| e.to_string())?;
    let npm = find_bin("npm", std::slice::from_ref(&node_bin.to_path_buf()))
        .ok_or_else(|| format!("npm not found next to node in {}", node_bin.display()))?;
    let mut dirs = vec![node_bin.to_path_buf()];
    dirs.extend(search_dirs(app));
    let mut cmd = Command::new(npm);
    cmd.args(["install", "--no-audit", "--no-fund", "--loglevel=error", "--prefix"])
        .arg(prefix)
        .arg(package)
        .env("PATH", join_path(&dirs));
    runtime::run_streamed(app, cmd, "npm")
}

fn install_acpx(app: &AppHandle) -> Result<PathBuf, String> {
    let node_bin = ensure_node(app)?;
    let prefix = acpx_prefix(app)?;
    runtime::log(app, format!("installing acpx {ACPX_VERSION} (talks ACP to coding agents)…"));
    npm_install(app, &node_bin, &prefix, &format!("acpx@{ACPX_VERSION}"))?;
    let bin = find_bin("acpx", &[prefix.join("node_modules").join(".bin")])
        .ok_or_else(|| "acpx install finished without the expected binary".to_string())?;
    runtime::log(app, "acpx ready.");
    Ok(bin)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Settings {
        Settings::default()
    }

    #[test]
    fn exec_reads_the_prompt_from_stdin() {
        let args = build_args(&cfg(), "codex", Path::new("/work"), 120);
        assert_eq!(&args[args.len() - 4..], ["codex", "exec", "-f", "-"]);
        assert!(args.windows(2).any(|w| w == ["--cwd", "/work"]));
        assert!(args.windows(2).any(|w| w == ["--timeout", "120"]));
        assert!(args.contains(&"--approve-reads".to_string()));
    }

    #[test]
    fn permissions_map_to_acpx_flags() {
        let mut c = cfg();
        c.acpx_permissions = "all".into();
        assert!(build_args(&c, "claude", Path::new("/"), 5).contains(&"--approve-all".into()));
        c.acpx_permissions = "none".into();
        assert!(build_args(&c, "claude", Path::new("/"), 5).contains(&"--deny-all".into()));
    }

    #[test]
    fn custom_agents_use_the_agent_escape_hatch() {
        let mut c = cfg();
        c.acpx_custom_command = "./my-acp-server --stdio".into();
        let args = build_args(&c, CUSTOM_AGENT, Path::new("/"), 5);
        assert!(args.windows(2).any(|w| w == ["--agent", "./my-acp-server --stdio"]));
        assert!(!args.contains(&CUSTOM_AGENT.to_string()));
    }

    #[test]
    fn system_prompt_uses_the_flag_only_where_supported() {
        let mut c = cfg();
        c.brain_system_prompt = "Be brief.".into();
        let claude = build_args(&c, "claude", Path::new("/"), 5);
        assert!(claude.windows(2).any(|w| w == ["--append-system-prompt", "Be brief."]));
        assert_eq!(build_prompt(&c, "claude", " add 2+2 "), "add 2+2");

        let gemini = build_args(&c, "gemini", Path::new("/"), 5);
        assert!(!gemini.contains(&"--append-system-prompt".to_string()));
        assert_eq!(build_prompt(&c, "gemini", "add 2+2"), "Be brief.\n\nTask: add 2+2");
    }

    #[test]
    fn model_is_passed_only_when_set() {
        let mut c = cfg();
        assert!(!build_args(&c, "codex", Path::new("/"), 5).contains(&"--model".into()));
        c.acpx_model = "gpt-5.4".into();
        assert!(build_args(&c, "codex", Path::new("/"), 5).windows(2).any(|w| w == ["--model", "gpt-5.4"]));
    }

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("v24.21.0"), Some((24, 21, 0)));
        assert_eq!(parse_version("0.15.1\n"), Some((0, 15, 1)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert!(parse_version("0.10.0").unwrap() < ACPX_MIN_SYSTEM);
        assert_eq!(parse_version("nope"), None);
    }

    #[test]
    fn auto_agent_prefers_installed_clis_in_order() {
        let statuses = |ready: &[&str]| -> Vec<AgentStatus> {
            AGENTS
                .iter()
                .map(|a| AgentStatus {
                    id: a.id,
                    label: a.label,
                    status: if ready.contains(&a.id) { "ready" } else if a.bins.is_empty() { "npx" } else { "missing" },
                    path: None,
                    installable: false,
                })
                .collect()
        };
        assert_eq!(pick_auto_agent(&statuses(&["gemini", "codex"])), Some("codex"));
        assert_eq!(pick_auto_agent(&statuses(&[])), None);
    }

    #[test]
    fn every_agent_id_is_unique() {
        let mut ids: Vec<_> = AGENTS.iter().map(|a| a.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), AGENTS.len());
    }
}
