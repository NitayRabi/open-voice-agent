//! Lazily installs everything the desktop app needs to run the managed
//! Apple Silicon speech stack, so nobody has to open a terminal: a standalone
//! `uv`, a prebuilt `llama-server`, and the speech-to-speech Python venv (uv
//! fetches its own Python 3.12 — no system Python required either).
//!
//! Mirrors what `hf-s2s/setup-macos.sh` did by hand, just triggered
//! automatically the first time it's needed, with progress in the Log tab.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

// Pinned known-good versions, same spirit as `HF_S2S_REF` in setup-macos.sh —
// a specific build we've verified works, not "latest".
const UV_VERSION: &str = "0.12.10";
const LLAMA_CPP_BUILD: &str = "b10819";
const UPSTREAM_URL_DEFAULT: &str = "https://github.com/huggingface/speech-to-speech.git";
const UPSTREAM_REF_DEFAULT: &str = "e34312cf47cd0159ee82f0d34b02e72353b7752e";

pub(crate) fn log(app: &AppHandle, line: impl AsRef<str>) {
    let _ = app.emit("backend-log", format!("[runtime] {}", line.as_ref()));
}

pub(crate) fn runtime_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("runtime");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub(crate) fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).map_err(|e| e.to_string())?;
    }
    let _ = path;
    Ok(())
}

/// Download to `dest`. No overall deadline — only a connect timeout and a
/// per-read stall timeout — so a large-but-healthy transfer isn't cut off by
/// a fixed wall-clock budget. (assets.rs's model downloader had exactly this
/// bug: ureq's `.timeout()` is an absolute deadline covering the whole body
/// read, not an inactivity timeout.)
pub(crate) fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(60))
        .build();
    let resp = agent.get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let mut file = File::create(dest).map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 256 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn extract_tar_gz(archive: &Path, dest_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(dest_dir)
        .status()
        .map_err(|e| format!("tar: {e}"))?;
    if !status.success() {
        return Err(format!("tar exited with {status}"));
    }
    Ok(())
}

/// Run a command to completion, streaming stdout/stderr into the same Log
/// tab the speech backend already uses, prefixed so their origin is obvious.
pub(crate) fn run_streamed(app: &AppHandle, mut cmd: Command, prefix: &str) -> Result<(), String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{prefix}: {e}"))?;
    let mut threads = Vec::new();
    for (name, pipe) in [
        ("out", child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>)),
        ("err", child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)),
    ] {
        if let Some(stream) = pipe {
            let app = app.clone();
            let prefix = prefix.to_string();
            threads.push(std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    let _ = app.emit("backend-log", format!("[{prefix}:{name}] {line}"));
                }
            }));
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    for t in threads {
        let _ = t.join();
    }
    if !status.success() {
        return Err(format!("{prefix} exited with {status}"));
    }
    Ok(())
}

/// True once the Python speech runtime (Parakeet STT + Qwen3 TTS on macOS)
/// is installed and ready to serve.
pub fn has_speech_runtime(repo_root: &Path) -> bool {
    let speech_cli = repo_root.join(".tmp/speech-to-speech/.venv/bin/speech-to-speech");
    if !speech_cli.is_file() {
        return false;
    }
    #[cfg(target_os = "macos")]
    return true;

    #[cfg(not(target_os = "macos"))]
    repo_root
        .join(".tmp/qwen3-tts-hip/target/release/tts-server")
        .is_file()
}

/// A standalone `uv`, downloaded once into the app data dir. `uv` manages its
/// own Python installs, so nothing else Python-related needs to pre-exist.
pub fn ensure_uv(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = runtime_dir(app)?.join("uv");
    let bin = dir.join("uv");
    if bin.is_file() {
        return Ok(bin);
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    log(app, "downloading uv (installs the Python speech runtime)…");
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    let url = format!(
        "https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/uv-{arch}-apple-darwin.tar.gz"
    );
    let archive = dir.join("uv.tar.gz");
    download_file(&url, &archive).map_err(|e| format!("uv download failed: {e}"))?;
    extract_tar_gz(&archive, &dir)?;
    let _ = fs::remove_file(&archive);
    let nested = dir.join(format!("uv-{arch}-apple-darwin"));
    if nested.is_dir() {
        for name in ["uv", "uvx"] {
            let _ = fs::rename(nested.join(name), dir.join(name));
        }
        let _ = fs::remove_dir_all(&nested);
    }
    if !bin.is_file() {
        return Err("uv download did not produce the expected binary".into());
    }
    make_executable(&bin)?;
    Ok(bin)
}

fn existing_llama_server() -> Option<PathBuf> {
    for candidate in ["/opt/homebrew/bin/llama-server", "/usr/local/bin/llama-server"] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let p = dir.join("llama-server");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Resolve a working `llama-server`: reuse an existing install (Homebrew or
/// PATH) if there is one, otherwise download a pinned prebuilt binary.
pub fn ensure_llama_server(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(p) = existing_llama_server() {
        return Ok(p);
    }

    #[cfg(not(target_os = "macos"))]
    return Err("llama-server not found on PATH; install llama.cpp".into());

    #[cfg(target_os = "macos")]
    {
        let base = runtime_dir(app)?.join("llama-cpp");
        let dir = base.join(format!("llama-{LLAMA_CPP_BUILD}"));
        let bin = dir.join("llama-server");
        if bin.is_file() {
            return Ok(bin);
        }
        fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        log(app, "downloading llama.cpp (runs the on-device language models)…");
        let asset_arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
        let url = format!(
            "https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_CPP_BUILD}/llama-{LLAMA_CPP_BUILD}-bin-macos-{asset_arch}.tar.gz"
        );
        let archive = base.join("llama.tar.gz");
        download_file(&url, &archive).map_err(|e| format!("llama.cpp download failed: {e}"))?;
        extract_tar_gz(&archive, &base)?;
        let _ = fs::remove_file(&archive);
        if !bin.is_file() {
            return Err("llama.cpp download did not produce the expected binary".into());
        }
        make_executable(&bin)?;
        Ok(bin)
    }
}

/// Clone the pinned speech-to-speech ref (if needed) and install its Python
/// env with `uv`. A no-op once `has_speech_runtime` is already true.
pub fn ensure_speech_runtime(app: &AppHandle, repo_root: &Path) -> Result<(), String> {
    if has_speech_runtime(repo_root) {
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    return Err("automatic voice-runtime installation is only implemented for macOS".into());

    #[cfg(target_os = "macos")]
    {
        if !cfg!(target_arch = "aarch64") {
            return Err("the managed speech runtime requires Apple Silicon (arm64)".into());
        }
        let uv = ensure_uv(app)?;
        let upstream = repo_root.join(".tmp/speech-to-speech");
        let upstream_url =
            std::env::var("HF_S2S_REPOSITORY").unwrap_or_else(|_| UPSTREAM_URL_DEFAULT.into());
        let upstream_ref =
            std::env::var("HF_S2S_REF").unwrap_or_else(|_| UPSTREAM_REF_DEFAULT.into());

        if !upstream.join(".git").is_dir() {
            if upstream.exists() {
                return Err(format!(
                    "{} exists but is not a git checkout; move it aside and try again",
                    upstream.display()
                ));
            }
            if let Some(parent) = upstream.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            log(app, "cloning the speech-to-speech runtime…");
            let mut clone = Command::new("git");
            clone.args(["clone", &upstream_url]).arg(&upstream);
            run_streamed(app, clone, "git-clone")?;

            let mut checkout = Command::new("git");
            checkout
                .current_dir(&upstream)
                .args(["checkout", "--detach", &upstream_ref]);
            run_streamed(app, checkout, "git-checkout")?;
        } else {
            let out = Command::new("git")
                .current_dir(&upstream)
                .args(["rev-parse", "HEAD"])
                .output()
                .map_err(|e| e.to_string())?;
            let current = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if current != upstream_ref {
                return Err(format!(
                    "{} is checked out at {current}, not the expected {upstream_ref}; move it aside and try again",
                    upstream.display()
                ));
            }
        }

        log(app, "installing the Python speech runtime (first time only; can take several minutes)…");
        let mut sync = Command::new(&uv);
        sync.args(["sync", "--project"])
            .arg(&upstream)
            .args(["--python", "3.12"]);
        run_streamed(app, sync, "uv-sync")?;

        if !upstream.join(".venv/bin/speech-to-speech").is_file() {
            return Err("runtime installation finished without creating the expected CLI".into());
        }
        log(app, "voice runtime ready.");
        Ok(())
    }
}
