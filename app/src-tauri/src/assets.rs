//! Model download manager. Nothing heavy ships in the installer — the app
//! carries a small curated catalog (`assets/catalog.json`) and downloads GGUF
//! weights on demand into the app data dir, with resume, progress events, and
//! verification. Users can add any Hugging Face repo+file or direct URL.
//!
//! Same idea as whisper.cpp desktop apps: pick a model, download it, or point
//! at a remote endpoint instead.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

const CATALOG_JSON: &str = include_str!("../assets/catalog.json");
const CHUNK: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub gated: bool,
    #[serde(default)]
    pub note: String,
    /// true for user-added entries (stored on disk), false for the bundled catalog
    #[serde(default)]
    pub user: bool,
}

impl ModelEntry {
    fn resolved_url(&self) -> String {
        if !self.url.is_empty() {
            return self.url.clone();
        }
        let rev = if self.revision.is_empty() { "main" } else { &self.revision };
        format!(
            "https://huggingface.co/{}/resolve/{}/{}?download=true",
            self.repo, rev, self.file
        )
    }
}

#[derive(Default)]
struct Job {
    downloaded: AtomicU64,
    total: AtomicU64,
    cancel: AtomicBool,
}

#[derive(Default)]
pub struct AssetManager {
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}

fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn user_registry_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(models_dir(app)?.join("models.json"))
}

fn load_user_models(app: &AppHandle) -> Vec<ModelEntry> {
    let Ok(path) = user_registry_path(app) else {
        return Vec::new();
    };
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str::<Vec<ModelEntry>>(&raw)
            .unwrap_or_default()
            .into_iter()
            .map(|mut m| {
                m.user = true;
                m
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn save_user_models(app: &AppHandle, models: &[ModelEntry]) -> Result<(), String> {
    let path = user_registry_path(app)?;
    fs::write(path, serde_json::to_string_pretty(models).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn curated() -> Vec<ModelEntry> {
    serde_json::from_str::<Value>(CATALOG_JSON)
        .ok()
        .and_then(|v| v.get("models").cloned())
        .and_then(|m| serde_json::from_value::<Vec<ModelEntry>>(m).ok())
        .unwrap_or_default()
}

fn all_entries(app: &AppHandle) -> Vec<ModelEntry> {
    let mut out = curated();
    let ids: std::collections::HashSet<_> = out.iter().map(|m| m.id.clone()).collect();
    for m in load_user_models(app) {
        if !ids.contains(&m.id) {
            out.push(m);
        }
    }
    out
}

fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .replace("--", "-")
}

fn gguf_file(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(models_dir(app)?.join(format!("{id}.gguf")))
}

impl AssetManager {
    /// Absolute path to an installed model, given a catalog id OR an absolute path.
    pub fn resolve(&self, app: &AppHandle, id_or_path: &str) -> Option<PathBuf> {
        let p = PathBuf::from(id_or_path);
        if p.is_absolute() && p.is_file() {
            return Some(p);
        }
        let f = gguf_file(app, id_or_path).ok()?;
        f.is_file().then_some(f)
    }

    pub fn list(&self, app: &AppHandle) -> Value {
        let jobs = self.jobs.lock();
        let models: Vec<Value> = all_entries(app)
            .into_iter()
            .map(|m| {
                let file = gguf_file(app, &m.id).ok();
                let on_disk = file.as_ref().and_then(|f| fs::metadata(f).ok()).map(|md| md.len());
                let part = file
                    .as_ref()
                    .and_then(|f| fs::metadata(f.with_extension("gguf.part")).ok())
                    .map(|md| md.len());
                let job = jobs.get(&m.id).map(|j| {
                    json!({
                        "downloaded": j.downloaded.load(Ordering::Relaxed),
                        "total": j.total.load(Ordering::Relaxed),
                        "cancelling": j.cancel.load(Ordering::Relaxed),
                    })
                });
                json!({
                    "id": m.id, "name": m.name, "repo": m.repo, "file": m.file,
                    "bytes": m.bytes, "license": m.license, "roles": m.roles,
                    "gated": m.gated, "note": m.note, "user": m.user,
                    "url": m.resolved_url(),
                    "installed": on_disk.is_some(),
                    "bytes_on_disk": on_disk,
                    "path": on_disk.and_then(|_| file.as_ref().map(|p| p.display().to_string())),
                    "partial_bytes": part,
                    "downloading": job.is_some(),
                    "job": job,
                })
            })
            .collect();
        json!({ "dir": models_dir(app).ok(), "models": models })
    }

    pub fn add(&self, app: &AppHandle, spec: &Value) -> Result<String, String> {
        let name = spec.get("name").and_then(Value::as_str).unwrap_or("").trim();
        let repo = spec.get("repo").and_then(Value::as_str).unwrap_or("").trim();
        let file = spec.get("file").and_then(Value::as_str).unwrap_or("").trim();
        let url = spec.get("url").and_then(Value::as_str).unwrap_or("").trim();
        if url.is_empty() && (repo.is_empty() || file.is_empty()) {
            return Err("provide a Hugging Face repo + file, or a direct URL".into());
        }
        let base = if !name.is_empty() {
            name.to_string()
        } else if !file.is_empty() {
            file.trim_end_matches(".gguf").to_string()
        } else {
            url.rsplit('/').next().unwrap_or("model").to_string()
        };
        let mut id = slugify(&base);
        if id.is_empty() {
            id = format!("model-{}", chrono_stamp());
        }
        let mut models = load_user_models(app);
        if models.iter().any(|m| m.id == id) || curated().iter().any(|m| m.id == id) {
            id = format!("{id}-{}", chrono_stamp());
        }
        models.push(ModelEntry {
            id: id.clone(),
            name: if name.is_empty() { base } else { name.to_string() },
            repo: repo.to_string(),
            file: file.to_string(),
            revision: spec.get("revision").and_then(Value::as_str).unwrap_or("main").to_string(),
            url: url.to_string(),
            bytes: spec.get("bytes").and_then(Value::as_u64).unwrap_or(0),
            sha256: spec.get("sha256").and_then(Value::as_str).unwrap_or("").to_string(),
            license: spec.get("license").and_then(Value::as_str).unwrap_or("").to_string(),
            roles: vec!["brain".into()],
            gated: false,
            note: String::new(),
            user: true,
        });
        save_user_models(app, &models)?;
        Ok(id)
    }

    pub fn forget(&self, app: &AppHandle, id: &str) -> Result<(), String> {
        self.cancel(id);
        let _ = self.remove_file(app, id);
        let mut models = load_user_models(app);
        models.retain(|m| m.id != id);
        save_user_models(app, &models)
    }

    pub fn remove_file(&self, app: &AppHandle, id: &str) -> Result<(), String> {
        self.cancel(id);
        let f = gguf_file(app, id)?;
        let _ = fs::remove_file(f.with_extension("gguf.part"));
        if f.is_file() {
            fs::remove_file(&f).map_err(|e| e.to_string())?;
        }
        let _ = app.emit("asset-progress", json!({ "id": id, "status": "removed" }));
        Ok(())
    }

    pub fn cancel(&self, id: &str) {
        if let Some(j) = self.jobs.lock().get(id) {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn download(&self, app: &AppHandle, id: &str) -> Result<(), String> {
        let entry = all_entries(app)
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| format!("no model '{id}' in the catalog"))?;
        {
            let mut jobs = self.jobs.lock();
            if jobs.contains_key(id) {
                return Err("already downloading".into());
            }
            jobs.insert(id.to_string(), Arc::new(Job::default()));
        }
        let job = self.jobs.lock().get(id).unwrap().clone();
        let app = app.clone();
        let id = id.to_string();
        let token = app.state::<AppState>().settings.lock().hf_token.clone();

        std::thread::spawn(move || {
            let result = run_download(&app, &entry, &job, &token);
            app.state::<AppState>().assets.jobs.lock().remove(&id);
            match result {
                Ok(()) => {
                    let _ = app.emit(
                        "asset-progress",
                        json!({ "id": id, "status": "done",
                                "downloaded": job.downloaded.load(Ordering::Relaxed),
                                "total": job.total.load(Ordering::Relaxed) }),
                    );
                    let _ = app.emit("backend-log", format!("[models] {id} ready"));
                }
                Err(e) => {
                    let status = if job.cancel.load(Ordering::Relaxed) { "cancelled" } else { "error" };
                    let _ = app.emit("asset-progress", json!({ "id": id, "status": status, "error": e }));
                    let _ = app.emit("backend-log", format!("[models] {id}: {e}"));
                }
            }
        });
        Ok(())
    }
}

fn chrono_stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn run_download(app: &AppHandle, entry: &ModelEntry, job: &Job, token: &str) -> Result<(), String> {
    let dest = gguf_file(app, &entry.id)?;
    if dest.is_file() {
        return Ok(());
    }
    let part = dest.with_extension("gguf.part");
    let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

    let url = entry.resolved_url();
    let mut req = ureq::get(&url).timeout(Duration::from_secs(60));
    if have > 0 {
        req = req.set("Range", &format!("bytes={have}-"));
    }
    if !token.is_empty() {
        req = req.set("Authorization", &format!("Bearer {token}"));
    }

    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(416, _)) => {
            // already have the whole file in .part
            fs::rename(&part, &dest).map_err(|e| e.to_string())?;
            return verify(&dest, entry);
        }
        Err(ureq::Error::Status(code, r)) => {
            let hint = if code == 401 || code == 403 {
                " (gated model — add a Hugging Face token in the Models tab)"
            } else {
                ""
            };
            return Err(format!("HTTP {code}{hint}: {}", r.into_string().unwrap_or_default()));
        }
        Err(e) => return Err(format!("request failed: {e}")),
    };

    let resuming = resp.status() == 206;
    if !resuming {
        have = 0;
        let _ = fs::remove_file(&part);
    }
    let body_len: u64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let total = if entry.bytes > 0 {
        entry.bytes
    } else {
        have + body_len
    };
    job.total.store(total, Ordering::Relaxed);
    job.downloaded.store(have, Ordering::Relaxed);

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&part)
        .map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(have)).map_err(|e| e.to_string())?;

    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; CHUNK];
    let mut done = have;
    let mut last = Instant::now();
    loop {
        if job.cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        job.downloaded.store(done, Ordering::Relaxed);
        if last.elapsed() >= Duration::from_millis(300) {
            last = Instant::now();
            let _ = app.emit(
                "asset-progress",
                json!({ "id": entry.id, "status": "downloading", "downloaded": done, "total": total.max(done) }),
            );
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    verify(&dest, entry)
}

fn verify(dest: &PathBuf, entry: &ModelEntry) -> Result<(), String> {
    let mut f = File::open(dest).map_err(|e| e.to_string())?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if &magic != b"GGUF" {
        let _ = fs::remove_file(dest);
        return Err("downloaded file is not a GGUF model".into());
    }
    if !entry.sha256.is_empty() {
        f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let got = hex::encode(hasher.finalize());
        if !got.eq_ignore_ascii_case(&entry.sha256) {
            let _ = fs::remove_file(dest);
            return Err(format!("sha256 mismatch (got {got})"));
        }
    }
    Ok(())
}
