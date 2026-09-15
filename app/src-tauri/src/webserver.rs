//! Embedded HTTP(S) server: serves the same config + voice UI over the network
//! so a browser or another device can use it, and exposes a small JSON API that
//! mirrors the Tauri commands (`get_settings`, `save_settings`, `delegate`, …).
//!
//! App events reach browser clients by long-polling `GET /api/events?since=N`:
//! the handler returns any newer events immediately, or parks up to ~25 s.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use std::process::{Child, Command, Stdio};
use std::io::BufRead;

use include_dir::{include_dir, Dir};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Listener, Manager};
use tiny_http::{Header, Method, Request, Response, Server};
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::{Message, Role};

use crate::brain;
use crate::config::{self, Settings};
use crate::AppState;

static UI: Dir = include_dir!("$CARGO_MANIFEST_DIR/../dist");

const BRIDGED_EVENTS: &[&str] = &[
    "settings-changed",
    "backend-log",
    "backend-status",
    "ova-transcript",
    "speech-toggle",
    "web-status",
    "acpx-status",
    "asset-progress",
];
const EVENT_LOG_CAP: usize = 256;
const LONGPOLL_MAX: Duration = Duration::from_secs(25);

/// (certificate PEM, private-key PEM)
type TlsPem = (Vec<u8>, Vec<u8>);

#[derive(Default)]
struct EventLog {
    seq: u64,
    buf: VecDeque<Value>, // each: { id, type, payload }
}
type Events = Arc<Mutex<EventLog>>;

fn push_event(events: &Events, name: &str, payload: &str) {
    let payload: Value =
        serde_json::from_str(payload).unwrap_or_else(|_| Value::String(payload.to_string()));
    let mut log = events.lock();
    log.seq += 1;
    let id = log.seq;
    log.buf.push_back(json!({ "id": id, "type": name, "payload": payload }));
    while log.buf.len() > EVENT_LOG_CAP {
        log.buf.pop_front();
    }
}

#[derive(Default)]
pub struct WebServer {
    inner: Mutex<Option<Running>>,
}

struct Running {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    listeners: Vec<tauri::EventId>,
    url: String,
    tailscale: Option<Child>,
}

impl WebServer {
    pub fn is_running(&self) -> bool {
        self.inner.lock().is_some()
    }

    pub fn url(&self) -> Option<String> {
        self.inner.lock().as_ref().map(|r| r.url.clone())
    }

    pub fn start(&self, app: &AppHandle) -> Result<String, String> {
        if self.is_running() {
            return Err("web server already running".into());
        }
        let cfg = app.state::<AppState>().settings.lock().clone();
        if !cfg.web_enabled {
            return Err("web server is disabled in settings".into());
        }

        let addr = format!("{}:{}", cfg.web_bind.trim(), cfg.web_port);
        let tls = tls_config(&cfg)?;
        let scheme = if tls.is_some() { "https" } else { "http" };

        let server = match tls {
            #[cfg(feature = "webserver-tls")]
            Some((cert, key)) => Server::https(
                &addr,
                tiny_http::SslConfig {
                    certificate: cert,
                    private_key: key,
                },
            )
            .map_err(|e| format!("bind {addr} (https) failed: {e}"))?,
            #[cfg(not(feature = "webserver-tls"))]
            Some(_) => return Err("this build has no TLS support".into()),
            None => Server::http(&addr).map_err(|e| format!("bind {addr} failed: {e}"))?,
        };
        let server = Arc::new(server);

        let events: Events = Arc::new(Mutex::new(EventLog::default()));
        let stop = Arc::new(AtomicBool::new(false));

        // Bridge Tauri app events into the poll log.
        let mut listeners = Vec::new();
        for &name in BRIDGED_EVENTS {
            let events = events.clone();
            let id = app.listen_any(name, move |event| {
                push_event(&events, name, event.payload());
            });
            listeners.push(id);
        }

        let host_hint = if cfg.web_bind.trim() == "0.0.0.0" {
            "<this-machine-ip>".to_string()
        } else {
            cfg.web_bind.trim().to_string()
        };
        let url = format!("{scheme}://{host_hint}:{}/", cfg.web_port);

        let app_bg = app.clone();
        let server_bg = server.clone();
        let stop_bg = stop.clone();
        let handle = std::thread::Builder::new()
            .name("ova-webserver".into())
            .spawn(move || {
                let _ = app_bg.emit("backend-log", format!("[web] serving {scheme}://{addr}/"));
                while !stop_bg.load(Ordering::Relaxed) {
                    match server_bg.recv_timeout(Duration::from_millis(400)) {
                        Ok(Some(req)) => {
                            let app = app_bg.clone();
                            let events = events.clone();
                            let stop = stop_bg.clone();
                            std::thread::spawn(move || handle_request(&app, &events, &stop, req));
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        let (tailscale, published_url) = if cfg.web_tailscale {
            let target = format!("http://127.0.0.1:{}", cfg.web_port);
            match start_tailscale(&cfg, &target) {
                Ok(value) => (Some(value.0), Some(value.1)),
                Err(error) => {
                    let _ = app.emit("backend-log", format!("[tailscale] {error}"));
                    (None, None)
                }
            }
        } else { (None, None) };
        let url = published_url.unwrap_or(url);
        *self.inner.lock() = Some(Running {
            stop,
            handle: Some(handle),
            listeners,
            url: url.clone(),
            tailscale,
        });
        let _ = app.emit("web-status", true);
        Ok(url)
    }

    pub fn stop(&self, app: &AppHandle) {
        if let Some(mut r) = self.inner.lock().take() {
            r.stop.store(true, Ordering::Relaxed);
            if let Some(mut child) = r.tailscale.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            for id in r.listeners.drain(..) {
                app.unlisten(id);
            }
            if let Some(h) = r.handle.take() {
                let _ = h.join();
            }
            let _ = app.emit("backend-log", "[web] stopped".to_string());
        }
        let _ = app.emit("web-status", false);
    }
}

fn tls_config(cfg: &Settings) -> Result<Option<TlsPem>, String> {
    let (c, k) = (cfg.web_tls_cert.trim(), cfg.web_tls_key.trim());
    match (c.is_empty(), k.is_empty()) {
        (true, true) => Ok(None),
        (false, false) => {
            let cert = std::fs::read(c).map_err(|e| format!("read cert {c}: {e}"))?;
            let key = std::fs::read(k).map_err(|e| format!("read key {k}: {e}"))?;
            Ok(Some((cert, key)))
        }
        _ => Err("set both web_tls_cert and web_tls_key, or neither".into()),
    }
}

fn tailscale_binary(cfg: &Settings) -> String {
    if !cfg.web_tailscale_binary.trim().is_empty() {
        return cfg.web_tailscale_binary.trim().to_string();
    }
    if let Ok(value) = std::env::var("OVA_TAILSCALE_BINARY") {
        if !value.trim().is_empty() { return value; }
    }
    #[cfg(target_os = "macos")]
    if std::path::Path::new("/Applications/Tailscale.app/Contents/MacOS/Tailscale").is_file() {
        return "/Applications/Tailscale.app/Contents/MacOS/Tailscale".into();
    }
    "tailscale".into()
}

fn tailscale_endpoint(line: &str) -> Option<String> {
    let value = line.trim().trim_end_matches('/');
    let host = value.strip_prefix("https://")?;
    if !host.contains('/') && host.ends_with(".ts.net") { Some(value.to_string()) } else { None }
}

/// Start a foreground Serve claim, and report it only after structured status
/// contains both the candidate tailnet host and our exact loopback target.
fn start_tailscale(cfg: &Settings, target: &str) -> Result<(Child, String), String> {
    let binary = tailscale_binary(cfg);
    let mut child = Command::new(&binary)
        .args(["serve", "--yes", target])
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|e| format!("could not start {binary}: {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn io::Read + Send>),
                 child.stderr.take().map(|p| Box::new(p) as Box<dyn io::Read + Send>)]
        .into_iter().flatten()
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in io::BufReader::new(pipe).lines().map_while(|line| line.ok()) { let _ = tx.send(line); }
        });
    }
    drop(tx);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut candidate = None;
    while Instant::now() < deadline {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(400)) {
            if let Some(endpoint) = tailscale_endpoint(&line) { candidate = Some(endpoint); }
        }
        if let Some(endpoint) = candidate.as_ref() {
            if let Ok(output) = Command::new(&binary).args(["serve", "status", "--json"]).output() {
                let status = String::from_utf8_lossy(&output.stdout);
                let host = endpoint.trim_start_matches("https://");
                if output.status.success() && serde_json::from_slice::<Value>(&output.stdout).is_ok()
                    && status.contains(host) && status.contains(target) {
                    return Ok((child, endpoint.clone()));
                }
            }
        }
        if child.try_wait().ok().flatten().is_some() {
            return Err("tailscale serve exited before becoming ready".into());
        }
    }
    let _ = child.kill(); let _ = child.wait();
    Err("tailscale serve did not become ready within 30 seconds".into())
}

// ── request handling ───────────────────────────────────────────────────────

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn security_headers() -> Vec<Header> {
    vec![
        // Scripts stay locked to same-origin (XSS guard, since delegation echoes
        // model output). Everything else is loose so browser extensions that
        // inject styles/fonts don't spam the console or fight the page.
        header(
            "Content-Security-Policy",
            "default-src 'self'; connect-src *; script-src 'self'; \
style-src * 'unsafe-inline'; font-src * data:; img-src * data: blob:; \
media-src 'self' blob: mediastream:",
        ),
        header("Referrer-Policy", "no-referrer"),
        header("X-Content-Type-Options", "nosniff"),
    ]
}

fn json_response(value: Value, status: u16) -> Response<io::Cursor<Vec<u8>>> {
    let mut r = Response::from_string(value.to_string()).with_status_code(status);
    r.add_header(header("Content-Type", "application/json"));
    for h in security_headers() {
        r.add_header(h);
    }
    r
}

fn void_respond<R: std::io::Read>(req: Request, response: Response<R>) {
    let _ = req.respond(response);
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            return Some(urlencoding::decode(v).map(|s| s.into_owned()).unwrap_or_default());
        }
    }
    None
}

#[derive(Clone)]
enum Access {
    Unauthenticated,
    Admin { token: String },
    Device { token: String, id: String },
}

fn presented_token(req: &Request) -> Option<String> {
    for h in req.headers() {
        let v = h.value.as_str();
        if h.field.equiv("Authorization") {
            if let Some(value) = v.strip_prefix("Bearer ") { return Some(value.trim().to_string()); }
        }
        if h.field.equiv("X-OVA-Token") { return Some(v.trim().to_string()); }
        if h.field.equiv("Cookie") {
            if let Some(value) = v.split(';').map(str::trim)
                .find_map(|c| c.strip_prefix("ova_token=")) { return Some(value.to_string()); }
        }
    }
    None
}

fn authorize(req: &Request, app: &AppHandle, configured_token: &str) -> Option<Access> {
    if configured_token.is_empty() { return Some(Access::Unauthenticated); }
    let token = presented_token(req)?;
    if token == configured_token { return Some(Access::Admin { token }); }
    app.state::<AppState>().devices.authenticate(&token)
        .map(|id| Access::Device { token, id })
}

fn privileged(req: &Request, access: &Access) -> bool {
    local_request(req) || matches!(access, Access::Admin { .. })
}

fn local_request(req: &Request) -> bool {
    let forwarded = req.headers().iter().any(|h| {
        let name = h.field.as_str().as_str().to_ascii_lowercase();
        name == "forwarded" || name == "via" || name == "x-real-ip" || name.starts_with("x-forwarded-")
    });
    !forwarded && req.remote_addr().is_some_and(|addr| addr.ip().is_loopback())
}

fn form_param(body: &str, key: &str) -> Option<String> {
    for pair in body.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if urlencoding::decode(k).ok()?.as_ref() == key {
            let value = v.replace('+', " ");
            return Some(
                urlencoding::decode(&value)
                    .map(|s| s.into_owned())
                    .unwrap_or_default(),
            );
        }
    }
    None
}

fn safe_next(raw: &str) -> String {
    if raw.starts_with('/') && !raw.starts_with("//") && !raw.contains(['\r', '\n']) {
        raw.to_string()
    } else {
        "/".to_string()
    }
}

fn html_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn pair_page(next: &str, invalid: bool) -> String {
    let error = if invalid {
        r#"<p class="error" role="alert">That pairing code is not valid.</p>"#
    } else {
        ""
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Pair — Open Voice Agent</title>
  <style>
    :root {{ color-scheme: dark; font-family: system-ui, -apple-system, "Segoe UI", sans-serif; }}
    * {{ box-sizing: border-box; }}
    body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 24px;
      background: radial-gradient(900px 600px at 50% -10%, #17203a, #0b0c10 65%); color: #e8e8ea; }}
    main {{ width: min(420px, 100%); padding: 32px; border: 1px solid #2b3550; border-radius: 18px;
      background: rgba(18, 21, 29, .94); box-shadow: 0 24px 80px rgba(0,0,0,.45); }}
    .mark {{ width: 54px; height: 54px; display: grid; place-items: center; margin-bottom: 20px;
      border-radius: 50%; font-size: 24px; background: linear-gradient(145deg, #5f7cff, #7048b8); }}
    h1 {{ margin: 0 0 8px; font-size: 1.45rem; }}
    p {{ margin: 0 0 22px; color: #aeb8ca; line-height: 1.5; }}
    label {{ display: block; margin-bottom: 8px; font-size: .9rem; font-weight: 600; }}
    input {{ width: 100%; padding: 13px 14px; border: 1px solid #39445e; border-radius: 10px;
      background: #0d1017; color: #fff; font: inherit; letter-spacing: .05em; outline: none; }}
    input:focus {{ border-color: #7890ff; box-shadow: 0 0 0 3px rgba(120,144,255,.18); }}
    button {{ width: 100%; margin-top: 14px; padding: 13px; border: 0; border-radius: 10px;
      background: #6983ff; color: white; font: inherit; font-weight: 650; cursor: pointer; }}
    button:hover {{ background: #7b92ff; }}
    .error {{ margin: 0 0 14px; color: #ff9f9f; font-size: .9rem; }}
    small {{ display: block; margin-top: 16px; color: #798397; line-height: 1.4; }}
  </style>
</head>
<body>
  <main>
    <div class="mark" aria-hidden="true">&#9835;</div>
    <h1>Pair with Open Voice Agent</h1>
    <p>Enter the pairing code shown in the desktop app. This browser will stay paired.</p>
    {error}
    <form method="post" action="/pair">
      <input type="hidden" name="next" value="{}">
      <label for="pairing_code">Pairing code</label>
      <input id="pairing_code" name="pairing_code" type="password" autocomplete="one-time-code"
        autocapitalize="none" spellcheck="false" autofocus required>
      <button type="submit">Pair browser</button>
    </form>
    <small>You can change the code from Settings → Web. Changing it signs out previously paired browsers.</small>
  </main>
</body>
</html>"#,
        html_attr(&safe_next(next))
    )
}

fn serve_pair(req: Request, next: &str, invalid: bool) -> io::Result<()> {
    let status = if invalid { 401 } else { 200 };
    let mut resp = Response::from_string(pair_page(next, invalid)).with_status_code(status);
    resp.add_header(header("Content-Type", "text/html; charset=utf-8"));
    for h in security_headers() {
        resp.add_header(h);
    }
    req.respond(resp)
}

fn pair_success(req: Request, cfg: &Settings, next: &str, token: &str) -> io::Result<()> {
    let mut resp = Response::empty(303);
    resp.add_header(header("Location", &safe_next(next)));
    let forwarded_https = req.headers().iter().find(|h| h.field.equiv("X-Forwarded-Proto"))
        .is_some_and(|h| h.value.as_str().eq_ignore_ascii_case("https"));
    let secure = if cfg.web_tls_cert.trim().is_empty() && !forwarded_https { "" } else { "; Secure" };
    resp.add_header(header(
        "Set-Cookie",
        &format!(
            "ova_token={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000{secure}",
            token
        ),
    ));
    req.respond(resp)
}

fn connection_page() -> &'static str {
    r#"<!doctype html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Connect — Open Voice Agent</title></head>
<body><main><h1>Connect to Open Voice Agent</h1><p id="status">Connecting…</p></main><script>
(async()=>{const n=document.getElementById('status'),token=location.hash.slice(1);history.replaceState(null,'',location.pathname);if(!token){n.textContent='This connection link is incomplete.';return}try{const r=await fetch('/api/access/session',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({token})});const b=await r.json().catch(()=>({}));if(!r.ok)throw Error(b.error||'Connection failed');location.replace('/')}catch(e){n.textContent=e.message}})();
</script></body></html>"#
}

fn serve_connection_page(req: Request) -> io::Result<()> {
    let mut resp = Response::from_string(connection_page());
    resp.add_header(header("Content-Type", "text/html; charset=utf-8"));
    resp.add_header(header("Content-Security-Policy", "default-src 'none'; script-src 'unsafe-inline'; connect-src 'self'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'"));
    resp.add_header(header("Referrer-Policy", "no-referrer"));
    resp.add_header(header("Cache-Control", "no-store"));
    req.respond(resp)
}

fn path_of(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or("/")
}

fn read_body(req: &mut Request) -> Value {
    let mut s = String::new();
    let _ = req.as_reader().read_to_string(&mut s);
    serde_json::from_str(&s).unwrap_or(Value::Null)
}

/// The `Host` header minus its port, keeping a bracketed IPv6 literal intact.
fn host_of(raw_host: &str) -> String {
    if raw_host.starts_with('[') {
        raw_host
            .split_once(']')
            .map(|(h, _)| format!("{h}]"))
            .unwrap_or_else(|| raw_host.to_string())
    } else {
        raw_host
            .rsplit_once(':')
            .filter(|(_, port)| port.parse::<u16>().is_ok())
            .map(|(host, _)| host.to_string())
            .unwrap_or_else(|| raw_host.to_string())
    }
}

/// The realtime endpoint to hand a browser reaching us at `host`.
///
/// Scheme and port are one decision, not two. The socat wrapper on the wss port
/// only exists when the launcher was given a cert (see `backend::start`); the
/// backend port behind it is always plain ws and loopback-only. Deriving the
/// scheme from the cert while hardcoding the TLS port sent browsers `ws://` into
/// a TLS listener whenever web TLS was off — a handshake failure the page could
/// only report as "connecting…" and then "disconnected".
fn realtime_url(host: &str, cfg: &Settings) -> String {
    let env_port = |key: &str, fallback: u16| {
        cfg.launch_env
            .get(key)
            .and_then(|p| p.trim().parse::<u16>().ok())
            .unwrap_or(fallback)
    };
    if !cfg.web_tls_cert.trim().is_empty() && !cfg.web_tls_key.trim().is_empty() {
        format!(
            "wss://{host}:{}/v1/realtime",
            env_port("HF_S2S_WSS_PORT", config::WSS_PORT)
        )
    } else {
        // No TLS wrapper to talk to. Loopback is the only reachable endpoint, and
        // also the only origin a browser grants a microphone to without TLS, so a
        // LAN client fails here the same way it already fails `isSecureContext`.
        format!(
            "ws://{host}:{}/v1/realtime",
            env_port("HF_S2S_BACKEND_PORT", config::BACKEND_PORT)
        )
    }
}

fn realtime_proxy_url(req: &Request, cfg: &Settings) -> String {
    public_origin(req, cfg)
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1)
        + "/api/realtime"
}

fn realtime_backend_url(cfg: &Settings) -> String {
    let port = cfg
        .launch_env
        .get("HF_S2S_BACKEND_PORT")
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(config::BACKEND_PORT);
    format!("ws://127.0.0.1:{port}/v1/realtime")
}

fn browser_settings(req: &Request, cfg: &Settings) -> Settings {
    let mut out = cfg.clone();
    // Pairing credentials can configure the app, but may not mint more devices
    // by reading the reusable pairing code back out of settings.
    out.web_token.clear();
    out.brain_api_key.clear();
    for agent in &mut out.delegation_agents {
        agent.brain_api_key.clear();
    }
    out.speech_remote_api_key.clear();
    out.hf_token.clear();
    for (key, value) in &mut out.launch_env {
        let key = key.to_ascii_uppercase();
        if ["TOKEN", "KEY", "SECRET", "PASSWORD"].iter().any(|needle| key.contains(needle)) {
            value.clear();
        }
    }
    out.server_url = realtime_proxy_url(req, cfg);
    out
}

const PROTOCOL_VERSION: &str = "1.0.0";
const CAPABILITIES: &[&str] = &[
    "access.device-credentials-v1",
    "access.fragment-session-v1",
    "access.per-device-revocation-v1",
    "realtime.authenticated-proxy-v1",
    "realtime.live-revocation-v1",
];

fn public_origin(req: &Request, cfg: &Settings) -> String {
    let host = req.headers().iter().find(|h| h.field.equiv("Host"))
        .map(|h| h.value.as_str()).unwrap_or("127.0.0.1");
    let forwarded_https = req.headers().iter().find(|h| h.field.equiv("X-Forwarded-Proto"))
        .is_some_and(|h| h.value.as_str().eq_ignore_ascii_case("https"));
    let scheme = if forwarded_https || !cfg.web_tls_cert.trim().is_empty() { "https" } else { "http" };
    format!("{scheme}://{host}")
}

fn allowed_pairing_origin(req: &Request, cfg: &Settings) -> bool {
    let Some(origin) = req.headers().iter().find(|h| h.field.equiv("Origin")).map(|h| h.value.as_str()) else {
        // Native clients do not send Origin.
        return true;
    };
    origin == public_origin(req, cfg)
}

fn issued_response(req: &Request, cfg: &Settings, issued: crate::device_access::IssuedCredential) -> Value {
    let origin = public_origin(req, cfg);
    json!({
        "access_token": issued.access_token,
        "token_type": issued.token_type,
        "device": issued.device,
        "base_url": origin,
        "connection_url": format!("{origin}/c#{}", issued.access_token),
        "realtime_url": realtime_proxy_url(req, cfg),
        "protocol_version": PROTOCOL_VERSION,
        "capabilities": CAPABILITIES,
    })
}

fn handle_request(app: &AppHandle, events: &Events, stop: &AtomicBool, mut req: Request) {
    let cfg = app.state::<AppState>().settings.lock().clone();
    let method = req.method().clone();
    let url = req.url().to_string();
    let path = path_of(&url).to_string();
    let is_get = matches!(method, Method::Get | Method::Head);

    if path == "/api/health" {
        let value = json!({
            "ok": true,
            "protocol_version": PROTOCOL_VERSION,
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": CAPABILITIES,
            "paired_devices": app.state::<AppState>().devices.list().len(),
            "realtime_auth": "device-credential",
        });
        let _ = req.respond(json_response(value, 200));
        return;
    }

    if path == "/c" && is_get { let _ = serve_connection_page(req); return; }

    if path == "/api/access/pair" && method == Method::Post {
        if !allowed_pairing_origin(&req, &cfg) {
            let _ = req.respond(json_response(json!({"error":"origin not allowed"}), 403));
            return;
        }
        let body = read_body(&mut req);
        let code = body.get("code").and_then(Value::as_str).unwrap_or_default();
        if !cfg.web_token.is_empty() && code != cfg.web_token {
            let _ = req.respond(json_response(json!({"error":"pairing code is invalid", "code":"pairing_invalid"}), 401));
            return;
        }
        let device = body.get("device").unwrap_or(&Value::Null);
        let kind = device.get("type").and_then(Value::as_str).unwrap_or("remote");
        let label = device.get("label").and_then(Value::as_str).unwrap_or("Paired device");
        match app.state::<AppState>().devices.issue(kind, label) {
            Ok(issued) => { let value = issued_response(&req, &cfg, issued); let _ = req.respond(json_response(value, 201)); }
            Err(e) => { let _ = req.respond(json_response(json!({"error":e.to_string()}), 500)); }
        }
        return;
    }

    if path == "/api/access/session" && method == Method::Post {
        if !allowed_pairing_origin(&req, &cfg) {
            let _ = req.respond(json_response(json!({"error":"origin not allowed"}), 403));
            return;
        }
        let body = read_body(&mut req);
        let token = body.get("token").and_then(Value::as_str).unwrap_or_default();
        if app.state::<AppState>().devices.authenticate(token).is_none() {
            let _ = req.respond(json_response(json!({"error":"device credential is invalid or revoked", "code":"device_credential_invalid"}), 401));
        } else {
            let _ = pair_success(req, &cfg, "/", token);
        }
        return;
    }

    if path == "/pair" {
        let result = match method {
            Method::Post => {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let code = form_param(&body, "pairing_code").unwrap_or_default();
                let next = form_param(&body, "next").unwrap_or_else(|| "/".into());
                if cfg.web_token.is_empty() || code == cfg.web_token {
                    match app.state::<AppState>().devices.issue("web", "Web browser") {
                        Ok(issued) => pair_success(req, &cfg, &next, &issued.access_token),
                        Err(_) => serve_pair(req, &next, true),
                    }
                } else {
                    serve_pair(req, &next, true)
                }
            }
            Method::Get | Method::Head => {
                let next = query_param(&url, "next").unwrap_or_else(|| "/".into());
                serve_pair(req, &next, false)
            }
            _ => req.respond(json_response(json!({ "error": "method not allowed" }), 405)),
        };
        let _ = result;
        return;
    }

    let Some(access) = authorize(&req, app, &cfg.web_token) else {
        let result = if is_get && !path.starts_with("/api/") {
            serve_pair(req, &path, false)
        } else {
            let supplied = presented_token(&req).is_some();
            req.respond(json_response(
                if supplied {
                    json!({ "error": "device credential is invalid or revoked", "code": "device_credential_invalid" })
                } else {
                    json!({ "error": "authentication required", "code": "access_required" })
                },
                401,
            ))
        };
        let _ = result;
        return;
    };

    if path == "/api/realtime" && is_get {
        proxy_realtime(app, &cfg, stop, req, &access);
        return;
    }

    if path == "/api/access/devices" && is_get {
        let status = if local_request(&req) { 200 } else { 403 };
        let value = if status == 200 { json!({"devices": app.state::<AppState>().devices.list()}) }
            else { json!({"error":"paired devices can only be managed locally"}) };
        let _ = req.respond(json_response(value, status)); return;
    }
    if path == "/api/access/devices" && method == Method::Post {
        if !local_request(&req) { let _ = req.respond(json_response(json!({"error":"device credentials can only be issued locally"}), 403)); return; }
        let body = read_body(&mut req); let device = body.get("device").unwrap_or(&Value::Null);
        let kind = device.get("type").and_then(Value::as_str).unwrap_or("remote");
        let label = device.get("label").and_then(Value::as_str).unwrap_or("Paired device");
        match app.state::<AppState>().devices.issue(kind, label) {
            Ok(issued) => { let value = issued_response(&req, &cfg, issued); let _ = req.respond(json_response(value, 201)); }
            Err(e) => { let _ = req.respond(json_response(json!({"error":e.to_string()}), 500)); }
        } return;
    }
    if let Some(id) = path.strip_prefix("/api/access/devices/") {
        if method != Method::Delete { let _ = req.respond(json_response(json!({"error":"method not allowed"}), 405)); return; }
        if !local_request(&req) { let _ = req.respond(json_response(json!({"error":"paired devices can only be managed locally"}), 403)); return; }
        match app.state::<AppState>().devices.revoke(id) {
            Ok(true) => { let _ = req.respond(Response::empty(204)); }
            Ok(false) => { let _ = req.respond(json_response(json!({"error":"paired device not found"}), 404)); }
            Err(e) => { let _ = req.respond(json_response(json!({"error":e.to_string()}), 500)); }
        } return;
    }

    let _ = match (method, path.as_str()) {
        (_, "/api/events") if is_get => serve_events(app, events, stop, &url, req, &access),

        (_, "/api/settings") if is_get => {
            let value = serde_json::to_value(browser_settings(&req, &cfg)).unwrap_or(Value::Null);
            req.respond(json_response(value, 200))
        }
        (Method::Post, "/api/settings") => {
            if !privileged(&req, &access) {
                return void_respond(req, json_response(json!({"error":"settings can only be changed locally or with the admin token"}), 403));
            }
            let body = read_body(&mut req);
            let incoming = body.get("settings").cloned().unwrap_or(body);
            match serde_json::from_value::<Settings>(incoming) {
                Ok(mut new) => {
                    if let Err(error) = new.validate_delegation_agents() {
                        return void_respond(req, json_response(json!({"error": error.to_string()}), 400));
                    }
                    // HTTP settings receive redacted secrets; preserve their
                    // existing values on a blank round-trip. Desktop IPC can
                    // still intentionally clear them.
                    if new.web_token.is_empty() { new.web_token = cfg.web_token.clone(); }
                    if new.brain_api_key.is_empty() { new.brain_api_key = cfg.brain_api_key.clone(); }
                    for agent in &mut new.delegation_agents {
                        if agent.brain_api_key.is_empty() {
                            if let Some(previous) = cfg.delegation_agents.iter().find(|old| old.id == agent.id) {
                                agent.brain_api_key = previous.brain_api_key.clone();
                            }
                        }
                    }
                    if new.speech_remote_api_key.is_empty() { new.speech_remote_api_key = cfg.speech_remote_api_key.clone(); }
                    if new.hf_token.is_empty() { new.hf_token = cfg.hf_token.clone(); }
                    let web_changed = !cfg.web_config_eq(&new);
                    let out = (|| -> Result<(), String> {
                        new.save(app).map_err(|e| e.to_string())?;
                        let hk = new.hotkey.clone();
                        *app.state::<AppState>().settings.lock() = new;
                        crate::hotkey::apply(app, &hk)?;
                        let _ = app.emit("settings-changed", ());
                        Ok(())
                    })();
                    if out.is_ok() {
                        let app2 = app.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(150));
                            if web_changed {
                                crate::apply_web(&app2);
                            }
                            app2.state::<AppState>().brain_server.reconcile(&app2);
                            app2.state::<AppState>().acpx.reconcile(&app2);
                        });
                    }
                    match out {
                        Ok(()) => req.respond(json_response(json!({ "ok": true }), 200)),
                        Err(e) => req.respond(json_response(json!({ "error": e }), 500)),
                    }
                }
                Err(e) => req.respond(json_response(json!({ "error": e.to_string() }), 400)),
            }
        }
        (Method::Post, "/api/delegate") => {
            let body = read_body(&mut req);
            let request = body.get("request").and_then(Value::as_str).unwrap_or_default();
            let agent_id = body.get("agent_id").and_then(Value::as_str);
            if !cfg.delegation_enabled {
                req.respond(json_response(json!({ "error": "delegation disabled" }), 400))
            } else {
                match brain::delegate(app, &cfg, request, agent_id) {
                    Ok(answer) => req.respond(json_response(json!({ "answer": answer }), 200)),
                    Err(e) => req.respond(json_response(json!({ "error": e.to_string() }), 502)),
                }
            }
        }
        (_, "/api/acpx/status") if is_get => {
            let status = app.state::<AppState>().acpx.status(app, &cfg);
            req.respond(json_response(serde_json::to_value(status).unwrap_or(Value::Null), 200))
        }
        (Method::Post, "/api/acpx/install") => {
            match app.state::<AppState>().acpx.ensure(app, &cfg) {
                Ok(path) => req.respond(json_response(json!({ "path": path.display().to_string() }), 200)),
                Err(e) => req.respond(json_response(json!({ "error": e }), 500)),
            }
        }
        (Method::Post, "/api/acpx/install_agent") => {
            let body = read_body(&mut req);
            let id = body.get("id").and_then(Value::as_str).unwrap_or_default();
            match app.state::<AppState>().acpx.install_agent(app, id) {
                Ok(()) => req.respond(json_response(json!({ "ok": true }), 200)),
                Err(e) => req.respond(json_response(json!({ "error": e }), 500)),
            }
        }
        (_, "/api/models") if is_get => {
            req.respond(json_response(app.state::<AppState>().assets.list(app), 200))
        }
        (Method::Post, "/api/models/resolve") => {
            let body = read_body(&mut req);
            let id_or_path = body
                .get("id_or_path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let path = app
                .state::<AppState>()
                .assets
                .resolve(app, id_or_path)
                .map(|p| p.display().to_string());
            req.respond(json_response(json!({ "path": path }), 200))
        }
        (Method::Post, "/api/models/add") => {
            req.respond(json_response(json!({ "error": "Custom downloads are no longer supported. Choose a catalog model or use an existing GGUF path." }), 410))
        }
        (Method::Post, p @ ("/api/models/download" | "/api/models/cancel" | "/api/models/remove"
            | "/api/models/forget")) => {
            if !privileged(&req, &access) {
                return void_respond(req, json_response(json!({"error":"models can only be changed locally or with the admin token"}), 403));
            }
            let body = read_body(&mut req);
            let id = body.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
            let assets = &app.state::<AppState>().assets;
            let out = match p {
                "/api/models/download" => assets.download(app, &id),
                "/api/models/remove" => assets.remove_file(app, &id),
                "/api/models/forget" => assets.forget(app, &id),
                _ => {
                    assets.cancel(&id);
                    Ok(())
                }
            };
            match out {
                Ok(()) => req.respond(json_response(json!({ "ok": true }), 200)),
                Err(e) => req.respond(json_response(json!({ "error": e }), 400)),
            }
        }
        (_, "/api/version") if is_get => req.respond(json_response(
            json!({ "version": app.package_info().version.to_string() }),
            200,
        )),
        (_, "/api/backend/status") if is_get => req.respond(json_response(
            json!({ "running": app.state::<AppState>().backend.is_running() }),
            200,
        )),
        (_, "/api/backend/ready") if is_get => {
            let st = app.state::<AppState>();
            let health_url = st.settings.lock().health_url.clone();
            req.respond(json_response(
                json!({ "ready": st.backend.is_ready(&health_url) }),
                200,
            ))
        },
        (Method::Post, "/api/backend/start") => {
            if !privileged(&req, &access) {
                return void_respond(req, json_response(json!({"error":"backend can only be controlled locally or with the admin token"}), 403));
            }
            let st = app.state::<AppState>();
            let c = st.settings.lock().clone();
            match st.backend.start(app, &c) {
                Ok(()) => req.respond(json_response(json!({ "ok": true }), 200)),
                Err(e) => req.respond(json_response(json!({ "error": e }), 500)),
            }
        }
        (Method::Post, "/api/backend/stop") => {
            if !privileged(&req, &access) {
                return void_respond(req, json_response(json!({"error":"backend can only be controlled locally or with the admin token"}), 403));
            }
            app.state::<AppState>().backend.stop(app);
            req.respond(json_response(json!({ "ok": true }), 200))
        }
        (Method::Post, "/api/backend/touch") => {
            app.state::<AppState>().backend.touch(app);
            req.respond(json_response(json!({ "ok": true }), 200))
        }
        (Method::Post, "/api/emit") => {
            if !privileged(&req, &access) {
                return void_respond(req, json_response(json!({"error":"events can only be emitted locally or with the admin token"}), 403));
            }
            let body = read_body(&mut req);
            let event = body.get("event").and_then(Value::as_str).unwrap_or_default();
            if BRIDGED_EVENTS.contains(&event) {
                let payload = body.get("payload").cloned().unwrap_or(Value::Null);
                let _ = app.emit(event, payload);
                req.respond(json_response(json!({ "ok": true }), 200))
            } else {
                req.respond(json_response(json!({ "error": "event not allowed" }), 400))
            }
        }

        (_, _) if is_get => serve_static(&path, req),

        _ => req.respond(json_response(json!({ "error": "not found" }), 404)),
    };
}

fn proxy_realtime(app: &AppHandle, cfg: &Settings, stop: &AtomicBool, req: Request, access: &Access) {
    let Some(key) = req.headers().iter().find(|h| h.field.equiv("Sec-WebSocket-Key"))
        .map(|h| h.value.as_str().to_string()) else {
        void_respond(req, json_response(json!({"error":"websocket upgrade required"}), 426));
        return;
    };

    let offered_protocols = req.headers().iter().find(|h| h.field.equiv("Sec-WebSocket-Protocol"))
        .map(|h| h.value.as_str()).unwrap_or("");
    let safe_protocols = offered_protocols.split(',').map(str::trim).filter(|value| {
        matches!(*value, "realtime" | "openai-insecure-api-key.open-voice-agent" | "openai-beta.realtime-v1")
    }).collect::<Vec<_>>();
    // The managed launcher supports port overrides for side-by-side/dev runs;
    // the authenticated proxy must follow the same backend port rather than
    // silently dialing the compiled default.
    let mut upstream_request = realtime_backend_url(cfg).into_client_request().unwrap();
    if !safe_protocols.is_empty() {
        upstream_request.headers_mut().insert("Sec-WebSocket-Protocol", safe_protocols.join(", ").parse().unwrap());
    }
    let Ok((mut upstream, _)) = tungstenite::connect(upstream_request) else {
        void_respond(req, json_response(json!({"error":"speech backend unavailable"}), 502));
        return;
    };
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = upstream.get_mut() {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(2)));
    }

    let mut response = Response::empty(101);
    response.add_header(header("Sec-WebSocket-Accept", &tungstenite::handshake::derive_accept_key(key.as_bytes())));
    // Select only a protocol the client offered. Native clients using an
    // Authorization header do not need to offer any subprotocol.
    if safe_protocols.contains(&"realtime") {
        response.add_header(header("Sec-WebSocket-Protocol", "realtime"));
    }
    let stream = req.upgrade("websocket", response);
    let mut client = tungstenite::WebSocket::from_raw_socket(stream, Role::Server, None);
    let revoked = match access {
        Access::Device { id, .. } => Some(app.state::<AppState>().devices.session_flag(id)),
        _ => None,
    };
    let admin_token = match access { Access::Admin { token } => Some(token.clone()), _ => None };

    loop {
        let admin_revoked = admin_token.as_ref().is_some_and(|token| {
            app.state::<AppState>().settings.lock().web_token != *token
        });
        if stop.load(Ordering::Acquire)
            || admin_revoked
            || revoked.as_ref().is_some_and(|flag| flag.load(Ordering::Acquire)) {
            let _ = client.close(None);
            break;
        }
        let incoming = match client.read() {
            Ok(message) => message,
            Err(_) => break,
        };
        let closing = matches!(incoming, Message::Close(_));
        if upstream.send(incoming).is_err() { break; }

        loop {
            match upstream.read() {
                Ok(message) => {
                    let closing = matches!(message, Message::Close(_));
                    if client.send(message).is_err() || closing { return; }
                }
                Err(tungstenite::Error::Io(error))
                    if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => break,
                Err(_) => return,
            }
        }
        if closing { break; }
    }
}

fn serve_events(app: &AppHandle, events: &Events, stop: &AtomicBool, url: &str, req: Request, access: &Access) -> io::Result<()> {
    let since: u64 = query_param(url, "since")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let deadline = Instant::now() + LONGPOLL_MAX;

    loop {
        if let Access::Device { token, id } = access {
            if !app.state::<AppState>().devices.is_valid_for(token, id) {
                return req.respond(json_response(json!({"error":"device credential is invalid or revoked", "code":"device_credential_invalid"}), 401));
            }
        }
        let (cursor, out): (u64, Vec<Value>) = {
            let log = events.lock();
            let out = log
                .buf
                .iter()
                .filter(|e| e["id"].as_u64().unwrap_or(0) > since)
                .cloned()
                .collect::<Vec<_>>();
            (log.seq, out)
        };
        if !out.is_empty() || Instant::now() >= deadline || stop.load(Ordering::Relaxed) {
            return req.respond(json_response(json!({ "cursor": cursor, "events": out }), 200));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn serve_static(path: &str, req: Request) -> io::Result<()> {
    let rel = match path {
        "/" | "" => "web.html",
        "/settings" | "/settings/" => "index.html",
        "/bubble" => "bubble.html",
        p => p.trim_start_matches('/'),
    };

    let Some(file) = UI.get_file(rel) else {
        return req.respond(json_response(json!({ "error": "not found", "path": rel }), 404));
    };

    let mut resp = Response::from_data(file.contents().to_vec());
    resp.add_header(header("Content-Type", mime_for(rel)));
    for h in security_headers() {
        resp.add_header(h);
    }
    req.respond(resp)
}

#[cfg(test)]
mod tests {
    use super::{form_param, host_of, pair_page, realtime_backend_url, realtime_url, safe_next, tailscale_endpoint};
    use crate::config::Settings;

    fn tls_settings() -> Settings {
        Settings {
            web_tls_cert: "/etc/ova/cert.pem".into(),
            web_tls_key: "/etc/ova/key.pem".into(),
            ..Settings::default()
        }
    }

    #[test]
    fn strips_the_port_from_the_host_header() {
        assert_eq!(host_of("192.168.68.46:1730"), "192.168.68.46");
        assert_eq!(host_of("agent.local"), "agent.local");
        assert_eq!(host_of("[::1]:1730"), "[::1]");
    }

    /// The wss port is a TLS listener and the backend port is not, so the scheme
    /// has to follow the port rather than being chosen independently of it.
    #[test]
    fn realtime_scheme_matches_the_port_it_names() {
        let tls = realtime_url("192.168.68.46", &tls_settings());
        assert_eq!(tls, "wss://192.168.68.46:8765/v1/realtime");

        let plain = realtime_url("127.0.0.1", &Settings::default());
        assert_eq!(plain, "ws://127.0.0.1:8766/v1/realtime");
    }

    #[test]
    fn realtime_url_honours_the_launcher_port_overrides() {
        let mut cfg = tls_settings();
        cfg.launch_env.insert("HF_S2S_WSS_PORT".into(), "9443".into());
        assert_eq!(realtime_url("host", &cfg), "wss://host:9443/v1/realtime");

        let mut cfg = Settings::default();
        cfg.launch_env.insert("HF_S2S_BACKEND_PORT".into(), "9766".into());
        assert_eq!(realtime_url("host", &cfg), "ws://host:9766/v1/realtime");
        assert_eq!(realtime_backend_url(&cfg), "ws://127.0.0.1:9766/v1/realtime");
    }

    /// A half-configured pair means the web server itself refuses to start with
    /// TLS, so the URL must not promise a wss endpoint the launcher never opened.
    #[test]
    fn realtime_url_needs_both_cert_and_key_for_wss() {
        let cfg = Settings {
            web_tls_cert: "/etc/ova/cert.pem".into(),
            ..Settings::default()
        };
        assert!(realtime_url("host", &cfg).starts_with("ws://"));
    }

    #[test]
    fn parses_pairing_form() {
        let body = "pairing_code=a%2Bb+code&next=%2Fsettings";
        assert_eq!(form_param(body, "pairing_code").as_deref(), Some("a+b code"));
        assert_eq!(form_param(body, "next").as_deref(), Some("/settings"));
    }

    #[test]
    fn pairing_redirect_stays_on_this_origin() {
        assert_eq!(safe_next("/settings"), "/settings");
        assert_eq!(safe_next("//example.com"), "/");
        assert_eq!(safe_next("https://example.com"), "/");
        assert_eq!(safe_next("/\r\nLocation: bad"), "/");
    }

    #[test]
    fn pairing_page_escapes_next_path() {
        let html = pair_page("/settings?x=\"<", false);
        assert!(html.contains("value=\"/settings?x=&quot;&lt;\""));
        assert!(!html.contains("value=\"/settings?x=\"<\""));
    }

    #[test]
    fn accepts_only_private_https_tailscale_endpoint_lines() {
        assert_eq!(tailscale_endpoint(" https://voice.tail123.ts.net/ ").as_deref(), Some("https://voice.tail123.ts.net"));
        assert_eq!(tailscale_endpoint("https://example.com"), None);
        assert_eq!(tailscale_endpoint("Visit https://voice.tail123.ts.net"), None);
        assert_eq!(tailscale_endpoint("http://voice.tail123.ts.net"), None);
    }
}
