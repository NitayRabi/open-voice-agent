//! Global push-to-toggle hotkey. The Rust side owns registration; a press just
//! emits `speech-toggle`, which the bubble window listens for.

use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

/// (Re)register the single toggle shortcut. Clears any previous binding first.
pub fn apply(app: &AppHandle, accelerator: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();

    let accelerator = accelerator.trim();
    if accelerator.is_empty() {
        return Ok(());
    }
    gs.register(accelerator)
        .map_err(|e| format!("could not register hotkey {accelerator:?}: {e}"))
}

/// Handler installed once at plugin-build time; fires for whatever is registered.
pub fn on_event(
    app: &AppHandle,
    _shortcut: &tauri_plugin_global_shortcut::Shortcut,
    event: tauri_plugin_global_shortcut::ShortcutEvent,
) {
    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
        let _ = app.emit("speech-toggle", ());
    }
}
