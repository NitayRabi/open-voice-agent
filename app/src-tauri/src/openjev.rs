//! OpenJEV / Decider Semantic Decision Routing Engine.
//!
//! Supervises the Decision Agent (configurable in Settings between OpenJEV Decider 0.7B,
//! Decider 2B, and TypeSafe JEV Cloud API). When starting the decider llama-server, CUDA/HIP graph
//! allocation is disabled (`GGML_CUDA_DISABLE_GRAPHS=1`, `GGML_HIP_DISABLE_GRAPHS=1`, and `--no-warmup`)
//! to reduce startup duration and memory overhead. An initial warm-up message is
//! sent on load to prime KV cache and eliminate cold start latency when the user speaks.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{bail, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{DelegationAgent, Settings};
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub agent_id: String,
    pub agent_alias: String,
    pub ack_prompt: String,
}

#[derive(Default)]
pub struct DeciderServer {
    child: Mutex<Option<Child>>,
    running_sig: Mutex<Option<Settings>>,
    warmed_up: AtomicBool,
}

impl DeciderServer {
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
        format!("http://127.0.0.1:{}/v1", cfg.decision_agent_port)
    }

    pub fn reconcile(&self, app: &AppHandle) {
        let cfg = app.state::<AppState>().settings.lock().clone();
        if !cfg.delegation_enabled || !cfg.is_local_decider() {
            self.stop(app);
            return;
        }
        let unchanged = self
            .running_sig
            .lock()
            .as_ref()
            .map(|s| s.decider_local_eq(&cfg))
            .unwrap_or(false);
        if unchanged && self.is_running() {
            return;
        }
        self.stop(app);
        if let Err(e) = self.start(app, &cfg) {
            let _ = app.emit("backend-log", format!("[decider] {e}"));
        }
    }

    pub fn ensure(&self, app: &AppHandle, cfg: &Settings) -> Result<(), String> {
        if !cfg.is_local_decider() {
            return Ok(());
        }
        let matches = self
            .running_sig
            .lock()
            .as_ref()
            .map(|running| running.decider_local_eq(cfg))
            .unwrap_or(false);
        if !matches && self.is_running() {
            self.stop(app);
        }
        if !self.is_running() {
            self.start(app, cfg)?;
        }
        let health = format!("http://127.0.0.1:{}/health", cfg.decision_agent_port);
        for _ in 0..120 {
            if ureq::get(&health)
                .timeout(Duration::from_millis(800))
                .call()
                .is_ok()
            {
                if !self.warmed_up.swap(true, Ordering::SeqCst) {
                    self.send_initial_warmup(app, cfg);
                }
                return Ok(());
            }
            if !self.is_running() {
                return Err("decider server exited during startup (see the Log tab)".into());
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Err("decider server did not become ready in 60s".into())
    }

    fn start(&self, app: &AppHandle, cfg: &Settings) -> Result<(), String> {
        let model = app
            .state::<AppState>()
            .assets
            .resolve(app, &cfg.decision_agent_model)
            .or_else(|| {
                app.state::<AppState>()
                    .assets
                    .resolve(app, &cfg.brain_local_model)
            })
            .ok_or_else(|| {
                format!(
                    "no model available for decision agent ({}) — download or configure one in Settings",
                    cfg.decision_agent_model
                )
            })?;

        let bin = if cfg.llama_server_bin.trim().is_empty() {
            crate::runtime::ensure_llama_server(app)?
        } else {
            std::path::PathBuf::from(&cfg.llama_server_bin)
        };

        let mut cmd = Command::new(&bin);
        // Disable graphs on startup to prevent slow initialization and heavy memory overhead
        cmd.env("GGML_CUDA_DISABLE_GRAPHS", "1")
            .env("GGML_HIP_DISABLE_GRAPHS", "1")
            .args([
                "-m",
                &model.to_string_lossy(),
                "--host",
                "127.0.0.1",
                "--port",
                &cfg.decision_agent_port.to_string(),
                "-c",
                "2048",
                "-ngl",
                "999",
                "--no-warmup",
                "--no-webui",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start decider {}: {e}", bin.display()))?;

        let pipes = [child.stdout.take().map(as_read), child.stderr.take().map(as_read)];
        for stream in pipes.into_iter().flatten() {
            let app = app.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    let _ = app.emit("backend-log", format!("[decider] {line}"));
                }
            });
        }

        let _ = app.emit(
            "backend-log",
            format!(
                "[decider] llama-server (no-graphs) on :{} ({})",
                cfg.decision_agent_port,
                model.display()
            ),
        );
        *self.child.lock() = Some(child);
        *self.running_sig.lock() = Some(cfg.clone());
        self.warmed_up.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Sends an initial first message onload to prevent cold start when the user speaks.
    pub fn send_initial_warmup(&self, app: &AppHandle, cfg: &Settings) {
        let url = format!("http://127.0.0.1:{}/v1/chat/completions", cfg.decision_agent_port);
        let body = json!({
            "model": "decider",
            "messages": [
                { "role": "system", "content": "You are a fast semantic classifier." },
                { "role": "user", "content": "warmup" }
            ],
            "max_tokens": 1,
            "temperature": 0.0,
            "stream": false
        });

        let app_handle = app.clone();
        std::thread::spawn(move || {
            match ureq::post(&url)
                .timeout(Duration::from_secs(10))
                .send_json(body)
            {
                Ok(_) => {
                    let _ = app_handle.emit(
                        "backend-log",
                        "[decider] Initial onload warm-up complete — zero cold start ready".to_string(),
                    );
                }
                Err(e) => {
                    let _ = app_handle.emit(
                        "backend-log",
                        format!("[decider] Onload warm-up notice: {e}"),
                    );
                }
            }
        });
    }

    pub fn stop(&self, app: &AppHandle) {
        if let Some(mut c) = self.child.lock().take() {
            let _ = c.kill();
            let _ = c.wait();
            let _ = app.emit("backend-log", "[decider] server stopped".to_string());
        }
        *self.running_sig.lock() = None;
        self.warmed_up.store(false, Ordering::SeqCst);
    }
}

fn as_read<R: std::io::Read + Send + 'static>(r: R) -> Box<dyn std::io::Read + Send> {
    Box::new(r)
}

/// Resolves the fallback conversational LLM base URL and optional api key.
fn conversational_endpoint(cfg: &Settings) -> Result<(String, String, String)> {
    if cfg.speech_model_source == "remote" {
        let base = cfg.speech_remote_base_url.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            bail!("remote speech endpoint is empty");
        }
        let model = cfg.speech_remote_model.trim().to_string();
        let key = cfg.speech_remote_api_key.trim().to_string();
        Ok((base, model, key))
    } else {
        // Local speech backend llama-server running on LLM_PORT
        let port = cfg
            .launch_env
            .get("HF_S2S_LLM_PORT")
            .and_then(|p| p.trim().parse::<u16>().ok())
            .unwrap_or(8011);
        Ok((
            format!("http://127.0.0.1:{port}/v1"),
            "local-conversation".to_string(),
            String::new(),
        ))
    }
}

/// Resolves decision endpoint: prefers local decider server if available,
/// TypeSafe JEV API if selected, else falls back to running conversational model endpoint.
fn resolve_decision_endpoint(app: &AppHandle, cfg: &Settings) -> Result<(String, String, String)> {
    if cfg.decision_agent_model == "typesafe-jev-api" {
        let base_url = if cfg.decision_agent_base_url.trim().is_empty() {
            "https://api.typesafe.ai/v1".to_string()
        } else {
            cfg.decision_agent_base_url.trim().trim_end_matches('/').to_string()
        };
        let api_key = cfg.decision_agent_api_key.trim().to_string();
        return Ok((base_url, "typesafe-jev".to_string(), api_key));
    }

    let st = app.state::<AppState>();
    // Try to ensure local decider server if model is resolvable
    if st.assets.resolve(app, &cfg.decision_agent_model).is_some()
        || st.assets.resolve(app, &cfg.brain_local_model).is_some()
    {
        if let Ok(()) = st.decider.ensure(app, cfg) {
            return Ok((
                format!("http://127.0.0.1:{}/v1", cfg.decision_agent_port),
                cfg.decision_agent_model.clone(),
                String::new(),
            ));
        }
    }
    // Fallback to conversational endpoint
    conversational_endpoint(cfg)
}

/// Routes a task request to the appropriate agent.
///
/// If `selected_agent` is specified and is not "orchestrator" / empty:
/// Focuses purely on that single agent definition.
///
/// If `selected_agent` is None or "orchestrator":
/// Runs OpenJEV / Decider option selection across all available agent definitions.
pub fn route_task(
    app: &AppHandle,
    cfg: &Settings,
    request: &str,
    selected_agent: Option<&str>,
) -> Result<RouteDecision> {
    let clean_req = request.trim();
    if clean_req.is_empty() {
        bail!("empty request for routing");
    }

    let agents = &cfg.delegation_agents;

    // 1. Single-agent focus mode or explicit agent override
    if let Some(agent_id) = selected_agent
        .map(str::trim)
        .filter(|id| !id.is_empty() && *id != "orchestrator")
    {
        if let Some(agent) = agents.iter().find(|a| a.id == agent_id) {
            return Ok(RouteDecision {
                agent_id: agent.id.clone(),
                agent_alias: agent.alias.clone(),
                ack_prompt: format!(
                    "Handed to {}. Tell the user briefly that you're on it.",
                    agent.alias
                ),
            });
        }
    }

    // 2. If no custom agents defined or only 1 agent defined
    if agents.is_empty() {
        let name = if cfg.brain_source == "acpx" && !cfg.acpx_agent.trim().is_empty() {
            cfg.acpx_agent.trim().to_string()
        } else {
            "the brain".to_string()
        };
        return Ok(RouteDecision {
            agent_id: String::new(),
            agent_alias: name.clone(),
            ack_prompt: format!("Handed to {name}. Tell the user briefly that you're on it."),
        });
    }

    if agents.len() == 1 {
        let a = &agents[0];
        return Ok(RouteDecision {
            agent_id: a.id.clone(),
            agent_alias: a.alias.clone(),
            ack_prompt: format!("Handed to {}. Tell the user briefly that you're on it.", a.alias),
        });
    }

    // 3. Multi-agent Orchestrator mode: Run Decider / OpenJEV selection
    match select_agent_decider(app, cfg, agents, clean_req) {
        Ok(idx) if idx < agents.len() => {
            let a = &agents[idx];
            Ok(RouteDecision {
                agent_id: a.id.clone(),
                agent_alias: a.alias.clone(),
                ack_prompt: format!(
                    "Delegated to {}. Tell the user briefly that you sent it to {} and are on it.",
                    a.alias, a.alias
                ),
            })
        }
        _ => {
            // Fallback to first agent if selection could not complete
            let a = &agents[0];
            Ok(RouteDecision {
                agent_id: a.id.clone(),
                agent_alias: a.alias.clone(),
                ack_prompt: format!(
                    "Delegated to {}. Tell the user briefly that you sent it to {} and are on it.",
                    a.alias, a.alias
                ),
            })
        }
    }
}

fn select_agent_decider(
    app: &AppHandle,
    cfg: &Settings,
    agents: &[DelegationAgent],
    request: &str,
) -> Result<usize> {
    let (base_url, model_name, api_key) = resolve_decision_endpoint(app, cfg)?;
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let mut agent_options = String::new();
    for (i, agent) in agents.iter().enumerate() {
        let desc = if !agent.description.trim().is_empty() {
            agent.description.trim()
        } else if !agent.delegation_tool_description.trim().is_empty() {
            agent.delegation_tool_description.trim()
        } else if !agent.brain_system_prompt.trim().is_empty() {
            agent.brain_system_prompt.trim()
        } else {
            agent.alias.trim()
        };
        agent_options.push_str(&format!("[{i}] {}: {}\n", agent.alias, desc));
    }

    let prompt = format!(
        "You are OpenJEV semantic decider. Decide which agent should handle this task.\n\n\
        Candidate Agents:\n{agent_options}\n\
        User Request: \"{request}\"\n\n\
        Reply ONLY with the number of the selected agent in brackets, e.g. [0]."
    );

    let body = json!({
        "model": model_name,
        "messages": [
            { "role": "system", "content": "You are a fast semantic classifier. Output only the agent index in brackets e.g. [0]." },
            { "role": "user", "content": prompt }
        ],
        "temperature": 0.0,
        "max_tokens": 5,
        "stream": false
    });

    let mut req = ureq::post(&url).timeout(Duration::from_millis(3000));
    if !api_key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {api_key}"));
    }

    let resp = req.send_json(body)?;
    let doc: Value = resp.into_json()?;
    let text = doc
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .or_else(|| doc.pointer("/choices/0/text").and_then(Value::as_str))
        .unwrap_or_default();

    parse_agent_index(text, agents.len())
}

fn parse_agent_index(text: &str, count: usize) -> Result<usize> {
    for char in text.chars() {
        if let Some(digit) = char.to_digit(10) {
            let idx = digit as usize;
            if idx < count {
                return Ok(idx);
            }
        }
    }
    bail!("could not parse valid agent index from decider output: {text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_indices() {
        assert_eq!(parse_agent_index("[0]", 2).unwrap(), 0);
        assert_eq!(parse_agent_index("[1] Milo", 2).unwrap(), 1);
        assert_eq!(parse_agent_index("Agent 1 is selected", 2).unwrap(), 1);
        assert!(parse_agent_index("[5]", 2).is_err());
        assert!(parse_agent_index("invalid", 2).is_err());
    }

    #[test]
    fn settings_decision_agent_defaults() {
        let settings = Settings::default();
        assert_eq!(settings.decision_agent_model, "openjev-decider-0.7b");
        assert_eq!(settings.decision_agent_port, 8130);
        assert_eq!(settings.decision_agent_base_url, "https://api.typesafe.ai/v1");
        assert_eq!(settings.decision_agent_api_key, "");
        assert!(settings.is_local_decider());
    }

    #[test]
    fn typesafe_jev_api_config() {
        let mut settings = Settings::default();
        settings.decision_agent_model = "typesafe-jev-api".into();
        settings.decision_agent_api_key = "apikey_test_123".into();
        assert!(!settings.is_local_decider());
    }
}
