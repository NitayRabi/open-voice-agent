//! Native localhost WebSocket bridge for the desktop webview.
//!
//! WKWebView can reject a plaintext loopback WebSocket from Tauri's custom
//! asset scheme as mixed content, even though the endpoint never leaves the
//! machine. Keep microphone/audio processing in the webview, but move this
//! one transport hop into Rust. The ordinary web UI continues to use its
//! browser WebSocket directly.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tungstenite::Message;

enum BridgeCommand {
    Start,
    Text(String),
    Close,
}

struct Connection {
    id: u64,
    tx: Sender<BridgeCommand>,
}

#[derive(Default)]
pub struct RealtimeBridge {
    next_id: AtomicU64,
    connection: Mutex<Option<Connection>>,
}

#[derive(Serialize, Clone)]
struct MessageEvent {
    id: u64,
    message: String,
}

#[derive(Serialize, Clone)]
struct CloseEvent {
    id: u64,
    error: Option<String>,
}

impl RealtimeBridge {
    pub fn connect(&self, app: &AppHandle, url: &str) -> Result<u64, String> {
        if !url.starts_with("ws://127.0.0.1:") || !url.ends_with("/v1/realtime") {
            return Err(
                "desktop realtime bridge only accepts the managed loopback endpoint".into(),
            );
        }
        self.disconnect(None);
        let (mut socket, _) = tungstenite::connect(url).map_err(|e| e.to_string())?;
        if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_millis(4)))
                .map_err(|e| e.to_string())?;
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::channel();
        *self.connection.lock() = Some(Connection { id, tx });
        let app = app.clone();

        std::thread::spawn(move || {
            // Do not consume the server's immediate session.created event
            // until the frontend has installed its Tauri event listeners.
            match rx.recv() {
                Ok(BridgeCommand::Start) => {}
                _ => return,
            }

            let mut close_error = None;
            'socket: loop {
                loop {
                    match rx.try_recv() {
                        Ok(BridgeCommand::Text(text)) => {
                            if let Err(error) = socket.send(Message::Text(text.into())) {
                                close_error = Some(error.to_string());
                                break 'socket;
                            }
                        }
                        Ok(BridgeCommand::Close) | Err(TryRecvError::Disconnected) => {
                            let _ = socket.close(None);
                            break 'socket;
                        }
                        Ok(BridgeCommand::Start) => {}
                        Err(TryRecvError::Empty) => break,
                    }
                }

                match socket.read() {
                    Ok(Message::Text(text)) => {
                        let _ = app.emit(
                            "realtime-message",
                            MessageEvent {
                                id,
                                message: text.to_string(),
                            },
                        );
                    }
                    Ok(Message::Close(frame)) => {
                        close_error = frame.map(|f| format!("{} {}", u16::from(f.code), f.reason));
                        break;
                    }
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(error))
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) => {}
                    Err(tungstenite::Error::ConnectionClosed) => break,
                    Err(error) => {
                        close_error = Some(error.to_string());
                        break;
                    }
                }
                let _ = socket.flush();
                std::thread::sleep(Duration::from_millis(2));
            }
            let _ = app.emit(
                "realtime-close",
                CloseEvent {
                    id,
                    error: close_error,
                },
            );
        });
        Ok(id)
    }

    pub fn start(&self, id: u64) -> Result<(), String> {
        self.send_command(id, BridgeCommand::Start)
    }

    pub fn send(&self, id: u64, message: String) -> Result<(), String> {
        self.send_command(id, BridgeCommand::Text(message))
    }

    pub fn disconnect(&self, id: Option<u64>) {
        let mut connection = self.connection.lock();
        if id.is_none_or(|id| connection.as_ref().is_some_and(|current| current.id == id)) {
            if let Some(current) = connection.take() {
                let _ = current.tx.send(BridgeCommand::Close);
            }
        }
    }

    fn send_command(&self, id: u64, command: BridgeCommand) -> Result<(), String> {
        let connection = self.connection.lock();
        let current = connection
            .as_ref()
            .ok_or("realtime connection is not open")?;
        if current.id != id {
            return Err("realtime connection is stale".into());
        }
        current.tx.send(command).map_err(|e| e.to_string())
    }
}
