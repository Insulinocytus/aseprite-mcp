//! Bridge hub: owns the single Plugin WebSocket and correlates tool calls with replies.
//! `Hub::call` is the one chokepoint every tool goes through; serialisation of calls,
//! binary (get_image) replies and the watchdog hook in here.

use crate::log::Log;
use axum::extract::ws::{Message, WebSocket};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fmt,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub enum CallError {
    NotConnected,
    Timeout,
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected => f.write_str("Aseprite is not connected: open Aseprite with the MCP extension installed"),
            Self::Timeout => f.write_str("timed out; the Lua code may still be running in Aseprite"),
        }
    }
}

pub struct Hub {
    call_timeout: Duration,
    log: Log,
    shutdown: CancellationToken,
    /// (connection id, outgoing text frames) of the current Bridge.
    bridge: Mutex<Option<(u64, mpsc::UnboundedSender<String>)>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    next_id: AtomicU64,
}

impl Hub {
    pub fn new(call_timeout: Duration, log: Log, shutdown: CancellationToken) -> Self {
        Self {
            call_timeout,
            log,
            shutdown,
            bridge: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn log(&self, msg: impl AsRef<str>) {
        self.log.line(msg);
    }

    /// Sends a tool call to the Plugin and returns its reply message (`{"id","ok",...}`).
    pub async fn call(&self, tool: &str, args: Value) -> Result<Value, CallError> {
        let tx = self.bridge.lock().as_ref().map(|(_, tx)| tx.clone()).ok_or(CallError::NotConnected)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = oneshot::channel();
        self.pending.lock().insert(id, reply_tx);
        if tx.send(json!({ "id": id, "tool": tool, "args": args }).to_string()).is_err() {
            self.pending.lock().remove(&id);
            return Err(CallError::NotConnected);
        }
        match tokio::time::timeout(self.call_timeout, reply_rx).await {
            Ok(Ok(reply)) => Ok(reply),
            _ => {
                // A late reply finds no pending entry and is dropped.
                self.pending.lock().remove(&id);
                Err(CallError::Timeout)
            }
        }
    }

    /// Drives one accepted `/ws` connection until it closes or is replaced by a newer one.
    pub async fn serve_bridge(&self, mut socket: WebSocket) {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let conn = self.next_id.fetch_add(1, Ordering::Relaxed);
        // Replacing the sender drops the old one, which ends the old connection's loop.
        *self.bridge.lock() = Some((conn, tx));
        self.log("bridge connected");

        loop {
            tokio::select! {
                out = rx.recv() => match out {
                    Some(text) => if socket.send(Message::Text(text.into())).await.is_err() { break },
                    None => break,
                },
                msg = socket.recv() => match msg {
                    Some(Ok(Message::Text(text))) => self.on_text(&text),
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                },
            }
        }

        let mut bridge = self.bridge.lock();
        if matches!(&*bridge, Some((c, _)) if *c == conn) {
            *bridge = None;
        }
        self.log("bridge disconnected");
    }

    fn on_text(&self, text: &str) {
        let Ok(msg) = serde_json::from_str::<Value>(text) else {
            self.log(format!("bad json from plugin: {text}"));
            return;
        };
        match msg.get("type").and_then(Value::as_str) {
            Some("hello") => self.log(format!("hello: {msg}")),
            Some("bye") => {
                self.log("bye from plugin, shutting down");
                self.shutdown.cancel();
            }
            _ => {
                let id = msg.get("id").and_then(Value::as_u64).unwrap_or(0);
                match self.pending.lock().remove(&id) {
                    Some(waiter) => {
                        let _ = waiter.send(msg);
                    }
                    None => self.log(format!("dropping reply for unknown or timed-out call {id}")),
                }
            }
        }
    }
}
