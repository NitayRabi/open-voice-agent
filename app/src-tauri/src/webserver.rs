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

use include_dir::{include_dir, Dir};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Listener, Manager};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::brain;
use crate::config::Settings;
use crate::AppState;

static UI: Dir = include_dir!("$CARGO_MANIFEST_DIR/../src");

const BRIDGED_EVENTS: &[&str] = &[
    "settings-changed",
    "backend-log",
    "backend-status",
    "ova-transcript",
    "speech-toggle",
    "web-status",
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

        *self.inner.lock() = Some(Running {
            stop,
            handle: Some(handle),
            listeners,
            url: url.clone(),
        });
        let _ = app.emit("web-status", true);
        Ok(url)
    }

    pub fn stop(&self, app: &AppHandle) {
        if let Some(mut r) = self.inner.lock().take() {
            r.stop.store(true, Ordering::Relaxed);
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

fn authorized(req: &Request, token: &str) -> bool {
    if token.is_empty() {
        return true;
    }
    if query_param(req.url(), "token").as_deref() == Some(token) {
        return true;
    }
    let cookie_needle = format!("ova_token={token}");
    for h in req.headers() {
        let v = h.value.as_str();
        if h.field.equiv("Authorization") && v.strip_prefix("Bearer ") == Some(token) {
            return true;
        }
        if h.field.equiv("X-OVA-Token") && v == token {
            return true;
        }
        if h.field.equiv("Cookie") && v.split(';').any(|c| c.trim() == cookie_needle) {
            return true;
        }
    }
    false
}

fn path_of(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or("/")
}

fn read_body(req: &mut Request) -> Value {
    let mut s = String::new();
    let _ = req.as_reader().read_to_string(&mut s);
    serde_json::from_str(&s).unwrap_or(Value::Null)
}

fn handle_request(app: &AppHandle, events: &Events, stop: &AtomicBool, mut req: Request) {
    let cfg = app.state::<AppState>().settings.lock().clone();

    if !authorized(&req, &cfg.web_token) {
        let _ = req.respond(json_response(json!({ "error": "unauthorized" }), 401));
        return;
    }

    let method = req.method().clone();
    let url = req.url().to_string();
    let path = path_of(&url).to_string();
    let is_get = matches!(method, Method::Get | Method::Head);

    let _ = match (method, path.as_str()) {
        (_, "/api/events") if is_get => serve_events(events, stop, &url, req),

        (_, "/api/settings") if is_get => {
            req.respond(json_response(serde_json::to_value(&cfg).unwrap_or(Value::Null), 200))
        }
        (Method::Post, "/api/settings") => {
            let body = read_body(&mut req);
            let incoming = body.get("settings").cloned().unwrap_or(body);
            match serde_json::from_value::<Settings>(incoming) {
                Ok(new) => {
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
            if !cfg.delegation_enabled {
                req.respond(json_response(json!({ "error": "delegation disabled" }), 400))
            } else {
                match brain::delegate(app, &cfg, request) {
                    Ok(answer) => req.respond(json_response(json!({ "answer": answer }), 200)),
                    Err(e) => req.respond(json_response(json!({ "error": e.to_string() }), 502)),
                }
            }
        }
        (_, "/api/models") if is_get => {
            req.respond(json_response(app.state::<AppState>().assets.list(app), 200))
        }
        (Method::Post, "/api/models/add") => {
            let body = read_body(&mut req);
            match app.state::<AppState>().assets.add(app, &body) {
                Ok(id) => req.respond(json_response(json!({ "id": id }), 200)),
                Err(e) => req.respond(json_response(json!({ "error": e }), 400)),
            }
        }
        (Method::Post, p @ ("/api/models/download" | "/api/models/cancel" | "/api/models/remove"
            | "/api/models/forget")) => {
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
        (Method::Post, "/api/backend/start") => {
            let st = app.state::<AppState>();
            let c = st.settings.lock().clone();
            match st.backend.start(app, &c) {
                Ok(()) => req.respond(json_response(json!({ "ok": true }), 200)),
                Err(e) => req.respond(json_response(json!({ "error": e }), 500)),
            }
        }
        (Method::Post, "/api/backend/stop") => {
            app.state::<AppState>().backend.stop(app);
            req.respond(json_response(json!({ "ok": true }), 200))
        }
        (Method::Post, "/api/emit") => {
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

        (_, _) if is_get => serve_static(&path, &url, &cfg, req),

        _ => req.respond(json_response(json!({ "error": "not found" }), 404)),
    };
}

fn serve_events(events: &Events, stop: &AtomicBool, url: &str, req: Request) -> io::Result<()> {
    let since: u64 = query_param(url, "since")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let deadline = Instant::now() + LONGPOLL_MAX;

    loop {
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

fn serve_static(path: &str, url: &str, cfg: &Settings, req: Request) -> io::Result<()> {
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
    if !cfg.web_token.is_empty() {
        if let Some(t) = query_param(url, "token") {
            if t == cfg.web_token {
                resp.add_header(header(
                    "Set-Cookie",
                    &format!("ova_token={t}; Path=/; SameSite=Strict; Max-Age=2592000"),
                ));
            }
        }
    }
    req.respond(resp)
}
