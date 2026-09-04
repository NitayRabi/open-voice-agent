mod assets;
mod backend;
mod brain;
mod config;
mod hotkey;
mod localbrain;
mod webserver;

use parking_lot::Mutex;
use rand::{distributions::Alphanumeric, Rng};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent,
};

use crate::assets::AssetManager;
use crate::backend::BackendManager;
use crate::config::Settings;
use crate::localbrain::LocalBrain;
use crate::webserver::WebServer;
use tauri_plugin_autostart::ManagerExt;

pub struct AppState {
    settings: Mutex<Settings>,
    backend: BackendManager,
    web: WebServer,
    assets: AssetManager,
    brain_server: LocalBrain,
}

// ── commands ───────────────────────────────────────────────────────────────

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().clone()
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    mut settings: Settings,
) -> Result<(), String> {
    settings.server_url = "ws://127.0.0.1:8766/v1/realtime".into();
    if settings.web_enabled
        && settings.web_bind.trim() != "127.0.0.1"
        && settings.web_token.trim().is_empty()
    {
        settings.web_token = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(20)
            .map(char::from)
            .collect();
    }
    let (backend_changed, autostart_changed) = {
        let previous = state.settings.lock();
        (
            !previous.backend_config_eq(&settings),
            previous.app_autostart != settings.app_autostart,
        )
    };
    settings.save(&app).map_err(|e| e.to_string())?;
    let hk = settings.hotkey.clone();
    let app_autostart = settings.app_autostart;
    let manage_backend = settings.manage_backend;
    let web_changed = !state.settings.lock().web_config_eq(&settings);
    *state.settings.lock() = settings;
    hotkey::apply(&app, &hk)?;
    if autostart_changed {
        apply_autostart(&app, app_autostart)?;
    }
    if web_changed {
        apply_web(&app);
    }
    if backend_changed {
        if state.backend.is_running() {
            state.backend.stop(&app);
        }
        if manage_backend {
            let cfg = state.settings.lock().clone();
            state.backend.start(&app, &cfg)?;
        }
    }
    state.brain_server.reconcile(&app);
    app.emit("settings-changed", ()).ok();
    Ok(())
}

fn apply_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|e| format!("could not update launch-at-login: {e}"))
}

/// Bring the embedded web server in line with the current settings.
fn apply_web(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.web.stop(app);
    if state.settings.lock().web_enabled {
        if let Err(e) = state.web.start(app) {
            let _ = app.emit("backend-log", format!("[web] start failed: {e}"));
        }
    }
}

#[tauri::command]
async fn delegate(
    app: AppHandle,
    state: State<'_, AppState>,
    request: String,
) -> Result<String, String> {
    let cfg = state.settings.lock().clone();
    if !cfg.delegation_enabled {
        return Err("delegation is disabled in settings".into());
    }
    tauri::async_runtime::spawn_blocking(move || brain::delegate(&app, &cfg, &request))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn backend_start(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let cfg = state.settings.lock().clone();
    state.backend.start(&app, &cfg)
}

#[tauri::command]
fn backend_stop(app: AppHandle, state: State<'_, AppState>) {
    state.backend.stop(&app);
}

#[tauri::command]
fn backend_running(state: State<'_, AppState>) -> bool {
    state.backend.is_running()
}

#[tauri::command]
fn web_start(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    state.web.start(&app)
}

#[tauri::command]
fn web_stop(app: AppHandle, state: State<'_, AppState>) {
    state.web.stop(&app);
}

#[tauri::command]
fn web_url(state: State<'_, AppState>) -> Option<String> {
    state.web.url()
}

#[tauri::command]
fn models_list(app: AppHandle, state: State<'_, AppState>) -> serde_json::Value {
    state.assets.list(&app)
}

#[tauri::command]
fn model_add(
    app: AppHandle,
    state: State<'_, AppState>,
    spec: serde_json::Value,
) -> Result<String, String> {
    state.assets.add(&app, &spec)
}

#[tauri::command]
fn model_download(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.assets.download(&app, &id)
}

#[tauri::command]
fn model_cancel(state: State<'_, AppState>, id: String) {
    state.assets.cancel(&id);
}

#[tauri::command]
fn model_remove(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.assets.remove_file(&app, &id)
}

#[tauri::command]
fn model_forget(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.assets.forget(&app, &id)
}

#[tauri::command]
fn model_resolve(app: AppHandle, state: State<'_, AppState>, id_or_path: String) -> Option<String> {
    state
        .assets
        .resolve(&app, &id_or_path)
        .map(|p| p.display().to_string())
}

#[tauri::command]
fn show_settings(app: AppHandle) {
    open_settings(&app);
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
fn quit_app(app: AppHandle, state: State<'_, AppState>) {
    state.web.stop(&app);
    state.brain_server.stop(&app);
    state.backend.stop(&app);
    app.exit(0);
}

// ── helpers ────────────────────────────────────────────────────────────────

fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn toggle_bubble(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("bubble") {
        match w.is_visible() {
            Ok(true) => {
                let _ = w.hide();
            }
            _ => {
                let _ = w.show();
            }
        }
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let toggle_speech = MenuItem::with_id(app, "toggle_speech", "Toggle speech", true, None::<&str>)?;
    let bubble = MenuItem::with_id(app, "toggle_bubble", "Show / hide bubble", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let web_open = MenuItem::with_id(app, "web_open", "Open web UI", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &toggle_speech, &bubble, &settings, &sep, &web_open, &sep, &quit,
        ],
    )?;

    let mut tray = TrayIconBuilder::with_id("main");
    match tauri::image::Image::from_bytes(include_bytes!("../icons/128x128.png")) {
        Ok(icon) => tray = tray.icon(icon),
        Err(e) => eprintln!("[tray] icon load failed: {e}"),
    }
    tray.tooltip("Open Voice Agent")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle_speech" => {
                let _ = app.emit("speech-toggle", ());
            }
            "toggle_bubble" => toggle_bubble(app),
            "settings" => open_settings(app),
            "web_open" => {
                let state = app.state::<AppState>();
                let url = state.web.url().or_else(|| state.web.start(app).ok());
                match url {
                    Some(url) => {
                        use tauri_plugin_opener::OpenerExt;
                        let _ = app.opener().open_url(url, None::<&str>);
                    }
                    None => {
                        let _ = app.emit(
                            "backend-log",
                            "[web] enable the web server in Settings first".to_string(),
                        );
                        open_settings(app);
                    }
                }
            }
            "quit" => {
                app.state::<AppState>().web.stop(app);
                app.state::<AppState>().brain_server.stop(app);
                app.state::<AppState>().backend.stop(app);
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

// ── entrypoint ─────────────────────────────────────────────────────────────

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(hotkey::on_event)
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            delegate,
            backend_start,
            backend_stop,
            backend_running,
            web_start,
            web_stop,
            web_url,
            models_list,
            model_add,
            model_download,
            model_cancel,
            model_remove,
            model_forget,
            model_resolve,
            show_settings,
            app_version,
            quit_app,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let settings = Settings::load(&handle);
            let hk = settings.hotkey.clone();
            let manage_backend = settings.manage_backend;
            let app_autostart = settings.app_autostart;

            let web_enabled = settings.web_enabled;
            let setup_completed = settings.setup_completed;
            let local_brain = settings.brain_source == "local" && settings.delegation_enabled;
            app.manage(AppState {
                settings: Mutex::new(settings),
                backend: BackendManager::default(),
                web: WebServer::default(),
                assets: AssetManager::default(),
                brain_server: LocalBrain::default(),
            });

            if let Err(e) = hotkey::apply(&handle, &hk) {
                eprintln!("[hotkey] {e}");
            }
            build_tray(&handle)?;

            // Reconcile the OS registration with config on every launch too, so
            // manually edited or restored config files take effect.
            if let Err(e) = apply_autostart(&handle, app_autostart) {
                eprintln!("[autostart] {e}");
            }

            if !setup_completed {
                open_settings(&handle);
                if let Some(bubble) = app.get_webview_window("bubble") {
                    let _ = bubble.hide();
                }
            }

            if manage_backend {
                let state = handle.state::<AppState>();
                let cfg = state.settings.lock().clone();
                if let Err(e) = state.backend.start(&handle, &cfg) {
                    eprintln!("[backend] autostart failed: {e}");
                }
            }

            if web_enabled {
                match handle.state::<AppState>().web.start(&handle) {
                    Ok(url) => println!("[web] {url}"),
                    Err(e) => eprintln!("[web] start failed: {e}"),
                }
            }

            if local_brain {
                let h = handle.clone();
                std::thread::spawn(move || h.state::<AppState>().brain_server.reconcile(&h));
            }

            // Nudge the bubble toward the lower-right of the primary monitor.
            if let Some(win) = app.get_webview_window("bubble") {
                if let Ok(Some(monitor)) = win.primary_monitor() {
                    let size = monitor.size();
                    let scale = monitor.scale_factor();
                    let bubble = 148.0 * scale;
                    let margin = 32.0 * scale;
                    let x = (size.width as f64 - bubble - margin).max(0.0);
                    let y = (size.height as f64 - bubble - margin * 3.0).max(0.0);
                    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    "settings" => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    "bubble" => {
                        let app = window.app_handle();
                        app.state::<AppState>().web.stop(app);
                        app.state::<AppState>().brain_server.stop(app);
                        app.state::<AppState>().backend.stop(app);
                    }
                    _ => {}
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Open Voice Agent");
}
