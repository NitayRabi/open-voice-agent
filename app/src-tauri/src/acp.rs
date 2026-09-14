//! Provider-neutral Agent Client Protocol delegation for paired device clients.
//!
//! The registry is shared with the supported `hf-s2s` facade. OpenClaw is a
//! provider configuration, never part of this transport contract.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Registry {
    default_agent: String,
    agents: BTreeMap<String, Provider>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Provider {
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    aliases: Vec<String>,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "default_cwd")]
    cwd: String,
    #[serde(default = "default_timeout")]
    timeout_seconds: f64,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

fn default_cwd() -> String {
    ".".into()
}
fn default_timeout() -> f64 {
    180.0
}

fn registry_path() -> Option<PathBuf> {
    if let Some(value) = env::var_os("OPEN_VOICE_AGENTS") {
        return Some(PathBuf::from(value));
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let compiled_root = manifest_dir.parent()?.parent()?;
    let candidates = [
        compiled_root.join("hf-s2s/agents.json"),
        env::current_dir().ok()?.join("hf-s2s/agents.json"),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

pub fn configured() -> bool {
    registry_path().is_some()
}

fn load_provider(path: &Path, requested: Option<&str>) -> Result<(String, Provider, PathBuf)> {
    let registry: Registry = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("could not read {}", path.display()))?,
    )
    .context("invalid ACP provider registry")?;
    let requested = requested.unwrap_or(&registry.default_agent).trim();
    if requested.is_empty() {
        bail!("no default ACP provider configured");
    }
    let resolved = registry.agents.iter().find(|(id, provider)| {
        id.eq_ignore_ascii_case(requested)
            || provider.display_name.eq_ignore_ascii_case(requested)
            || provider
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(requested))
    });
    let (provider_id, provider) = resolved
        .map(|(id, provider)| (id.clone(), provider.clone()))
        .ok_or_else(|| anyhow!("unknown ACP provider {requested:?}"))?;
    if provider.command.trim().is_empty() {
        bail!("ACP provider {provider_id:?} has no command");
    }
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let cwd = PathBuf::from(&provider.cwd);
    let cwd = if cwd.is_absolute() {
        cwd
    } else {
        base.join(cwd)
    };
    if !cwd.is_dir() {
        bail!(
            "ACP provider working directory does not exist: {}",
            cwd.display()
        );
    }
    Ok((provider_id, provider, cwd))
}

fn resolve_command(command: &str) -> PathBuf {
    let path = PathBuf::from(command);
    if path.components().count() > 1 {
        return path;
    }
    if let Some(found) = env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|dir| dir.join(command))
            .find(|p| p.is_file())
    }) {
        return found;
    }
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return path;
    };
    let direct = [
        home.join(".local/bin").join(command),
        home.join(".local/share/fnm/aliases/default/bin")
            .join(command),
        home.join(".volta/bin").join(command),
        home.join(".local/share/pnpm").join(command),
    ];
    direct.into_iter().find(|p| p.is_file()).unwrap_or(path)
}

fn send(stdin: &mut ChildStdin, value: Value) -> Result<()> {
    serde_json::to_writer(&mut *stdin, &value)?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    Ok(())
}

fn receive(
    child: &mut Child,
    stdin: &mut ChildStdin,
    lines: &Receiver<String>,
    id: u64,
    deadline: Instant,
    chunks: &mut String,
) -> Result<Value> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let _ = child.kill();
            bail!("ACP provider timed out");
        }
        let line = lines
            .recv_timeout(remaining)
            .context("ACP provider exited before replying")?;
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if message.get("id").and_then(Value::as_u64) == Some(id) {
            if let Some(error) = message.get("error") {
                bail!(
                    "ACP provider error: {}",
                    error.get("message").unwrap_or(error)
                );
            }
            return Ok(message.get("result").cloned().unwrap_or_else(|| json!({})));
        }
        if message.get("method").and_then(Value::as_str) == Some("session/update") {
            let update = &message["params"]["update"];
            if update["sessionUpdate"] == "agent_message_chunk" {
                collect_text(&update["content"], chunks);
            }
        } else if message.get("id").is_some() && message.get("method").is_some() {
            // Paired devices are unattended. Never grant an interactive ACP
            // permission prompt that the user cannot see.
            send(
                stdin,
                json!({
                    "jsonrpc": "2.0", "id": message["id"],
                    "result": {"outcome": {"outcome": "cancelled"}}
                }),
            )?;
        }
    }
}

fn collect_text(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => out.push_str(text),
        Value::Array(items) => items.iter().for_each(|item| collect_text(item, out)),
        Value::Object(map) if map.get("type").and_then(Value::as_str) == Some("text") => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                out.push_str(text);
            }
        }
        Value::Object(map) => map.values().for_each(|item| collect_text(item, out)),
        _ => {}
    }
}

/// Execute one task through the registry's default provider.
pub fn delegate(task: &str, requested: Option<&str>) -> Result<String> {
    let task = task.trim();
    if task.is_empty() {
        bail!("empty delegation request");
    }
    let path = registry_path().context("ACP provider registry was not found")?;
    let (provider_id, provider, cwd) = load_provider(&path, requested)?;
    let session_cwd = cwd.display().to_string();
    let command = resolve_command(provider.command.trim());
    let mut cmd = Command::new(&command);
    cmd.args(&provider.args)
        .current_dir(cwd)
        .envs(&provider.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(parent) = command.parent() {
        let mut paths = vec![parent.to_path_buf()];
        if let Some(current) = env::var_os("PATH") {
            paths.extend(env::split_paths(&current));
        }
        if let Ok(value) = env::join_paths(paths) {
            cmd.env("PATH", value);
        }
    }
    let mut child = cmd.spawn().with_context(|| {
        format!(
            "could not start ACP provider {provider_id:?} at {}",
            command.display()
        )
    })?;
    let mut stdin = child
        .stdin
        .take()
        .context("could not open ACP provider input")?;
    let stdout = child
        .stdout
        .take()
        .context("could not open ACP provider output")?;
    let (sender, lines) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let timeout = provider.timeout_seconds.clamp(1.0, 3600.0);
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    let mut chunks = String::new();
    let result = (|| -> Result<String> {
        send(
            &mut stdin,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":1,"clientCapabilities":{},
                "clientInfo":{"name":"open-voice-node","title":"Open Voice Node","version":"0.1.0"}
            }}),
        )?;
        receive(&mut child, &mut stdin, &lines, 1, deadline, &mut chunks)?;
        send(
            &mut stdin,
            json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{
                "cwd": session_cwd, "mcpServers":[]
            }}),
        )?;
        let session = receive(&mut child, &mut stdin, &lines, 2, deadline, &mut chunks)?;
        let session_id = session
            .get("sessionId")
            .and_then(Value::as_str)
            .context("ACP provider did not return a session ID")?;
        send(
            &mut stdin,
            json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{
                "sessionId":session_id,"prompt":[{"type":"text","text":task}]
            }}),
        )?;
        receive(&mut child, &mut stdin, &lines, 3, deadline, &mut chunks)?;
        let answer = chunks.trim();
        Ok(if answer.is_empty() {
            format!(
                "{} completed the task without a text response.",
                if provider.display_name.is_empty() {
                    provider_id.as_str()
                } else {
                    &provider.display_name
                }
            )
        } else {
            answer.into()
        })
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_nested_acp_text() {
        let mut out = String::new();
        collect_text(
            &json!([{"type":"text","text":"hello "},{"content":{"type":"text","text":"world"}}]),
            &mut out,
        );
        assert_eq!(out, "hello world");
    }

    #[test]
    fn tracked_registry_has_a_valid_default_provider() {
        let path = registry_path().expect("tracked provider registry");
        let (id, provider, cwd) = load_provider(&path, None).unwrap();
        assert_eq!(id, "openclaw");
        assert!(!provider.command.is_empty());
        assert!(cwd.is_dir());
        let (alias_id, _, _) = load_provider(&path, Some("MILO")).unwrap();
        assert_eq!(alias_id, "openclaw");
    }

    #[test]
    fn live_openclaw_when_enabled() {
        if env::var_os("OPEN_VOICE_TEST_LIVE_ACP").is_none() {
            return;
        }
        assert_eq!(
            delegate("Reply with exactly: OPEN_VOICE_RUST_ACP_OK", None).unwrap(),
            "OPEN_VOICE_RUST_ACP_OK"
        );
    }
}
