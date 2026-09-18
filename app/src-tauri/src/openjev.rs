//! OpenJEV Semantic Decision Routing Engine.
//!
//! Evaluates candidate agent profiles in a single forward pass / fast scoring request
//! using the already running conversational small model server (e.g. Gemma 4 E4B / LFM
//! via llama-server on LLM_PORT or remote conversational endpoint), eliminating duplicate
//! GPU memory overhead.

use std::time::Duration;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::config::{DelegationAgent, Settings};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub agent_id: String,
    pub agent_alias: String,
    pub ack_prompt: String,
}

/// Resolves the conversational LLM base URL and optional api key.
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

/// Routes a task request to the appropriate agent.
///
/// If `selected_agent` is specified and is not "orchestrator" / empty:
/// Focuses purely on that single agent definition.
///
/// If `selected_agent` is None or "orchestrator":
/// Runs OpenJEV option selection across all available agent definitions using
/// the running conversational small model (E4B / LFM).
pub fn route_task(
    _app: &AppHandle,
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

    // 3. Multi-agent Orchestrator mode: Run OpenJEV selection using the small model
    match select_agent_openjev(cfg, agents, clean_req) {
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

fn select_agent_openjev(
    cfg: &Settings,
    agents: &[DelegationAgent],
    request: &str,
) -> Result<usize> {
    let (base_url, model_name, api_key) = conversational_endpoint(cfg)?;
    let url = format!("{}/chat/completions", base_url);

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
        "You are OpenJEV semantic router. Decide which agent should handle this task.\n\n\
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

    // Parse index from response, e.g. "[1]" or "1" or "[0] Milo"
    for char in text.chars() {
        if let Some(digit) = char.to_digit(10) {
            let idx = digit as usize;
            if idx < agents.len() {
                return Ok(idx);
            }
        }
    }

    bail!("could not parse agent choice from: {text}")
}
