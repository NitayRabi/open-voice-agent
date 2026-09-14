//! Delegation to the "brain": a provider-neutral ACP registry when present,
//! or a single model reached over an OpenAI-compatible
//! `/chat/completions` endpoint as a backwards-compatible fallback.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::config::Settings;
use crate::AppState;

#[path = "acp.rs"]
mod acp;

const MAX_SPOKEN_CHARS: usize = 600;

struct Target {
    base: String,
    model: String,
    api_key: String,
}

fn resolve_target(app: &AppHandle, cfg: &Settings) -> Result<Target> {
    if cfg.brain_source == "local" {
        let st = app.state::<AppState>();
        st.brain_server
            .ensure(app, cfg)
            .map_err(|e| anyhow!("local brain: {e}"))?;
        let model = st
            .assets
            .resolve(app, &cfg.brain_local_model)
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "local".into());
        Ok(Target {
            base: st.brain_server.base_url(cfg),
            model,
            api_key: String::new(),
        })
    } else {
        let base = cfg.brain_base_url.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            bail!("no brain endpoint configured — set one in Settings → Delegation");
        }
        if cfg.brain_model.trim().is_empty() {
            bail!("no brain model configured — set one in Settings → Delegation");
        }
        Ok(Target {
            base,
            model: cfg.brain_model.trim().to_string(),
            api_key: cfg.brain_api_key.trim().to_string(),
        })
    }
}

/// Blocking. POST `{base}/chat/completions`, return the assistant message text.
pub fn delegate(app: &AppHandle, cfg: &Settings, request: &str) -> Result<String> {
    let request = request.trim();
    if request.is_empty() {
        bail!("empty delegation request");
    }
    if acp::configured() {
        return acp::delegate(request, None).map(|answer| squash(&answer));
    }
    let target = resolve_target(app, cfg)?;
    let url = format!("{}/chat/completions", target.base);
    let timeout = Duration::from_secs(cfg.delegation_timeout_s.max(5));

    let mut messages = Vec::new();
    let system = cfg.brain_system_prompt.trim();
    if !system.is_empty() {
        messages.push(json!({ "role": "system", "content": system }));
    }
    messages.push(json!({ "role": "user", "content": request }));

    let body = json!({
        "model": target.model,
        "messages": messages,
        "stream": false,
        "temperature": cfg.brain_temperature,
    });

    let mut req = ureq::post(&url).timeout(timeout);
    if !target.api_key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", target.api_key));
    }

    let resp = match req.send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_string().unwrap_or_default();
            bail!("brain returned {code}: {}", squash(&detail));
        }
        Err(e) => bail!("could not reach the brain: {e}"),
    };

    let doc: Value = resp
        .into_json()
        .map_err(|e| anyhow!("brain sent an unreadable response: {e}"))?;

    let text = doc
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .or_else(|| doc.pointer("/choices/0/text").and_then(Value::as_str))
        .unwrap_or_default()
        .trim();

    if text.is_empty() {
        bail!("brain returned an empty answer");
    }
    Ok(squash(text))
}

fn squash(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() > MAX_SPOKEN_CHARS {
        format!("{}…", joined.chars().take(MAX_SPOKEN_CHARS).collect::<String>())
    } else {
        joined
    }
}
