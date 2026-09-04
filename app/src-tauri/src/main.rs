// Prevents an extra console window on Windows in release. Does nothing on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    open_voice_agent_lib::run()
}
