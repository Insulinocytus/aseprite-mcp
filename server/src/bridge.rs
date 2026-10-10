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
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub enum CallError {
    NotConnected,
    /// The Bridge closed while the call was in flight; the Plugin may or may not have run it.
    Disconnected,
    Timeout,
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected => f.write_str("Aseprite is not connected: open Aseprite with the MCP extension installed"),
            Self::Disconnected => {
                f.write_str("Aseprite disconnected during the call; the Lua code may or may not have run")
            }
            Self::Timeout => f.write_str("timed out; the Lua code may still be running in Aseprite"),
        }
    }
}

/// A Plugin reply: the JSON message plus the payload of a binary frame (empty for text frames).
pub struct Reply {
    pub msg: Value,
    pub bytes: Vec<u8>,
}

pub struct Hub {
    call_timeout: Duration,
    log: Log,
    shutdown: CancellationToken,
    /// (connection id, outgoing text frames) of the current Bridge.
    bridge: Mutex<Option<(u64, mpsc::UnboundedSender<String>)>>,
    /// Waiting calls by id, tagged with the connection they were sent on.
    pending: Mutex<HashMap<u64, (u64, oneshot::Sender<Reply>)>>,
    next_id: AtomicU64,
    /// Held for a whole call so only one call is in flight toward the Plugin; the rest queue here.
    gate: AsyncMutex<()>,
    /// Whether a Bridge is registered; the lifecycle watchdog follows it.
    connected: watch::Sender<bool>,
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
            gate: AsyncMutex::new(()),
            connected: watch::channel(false).0,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.bridge.lock().is_some()
    }

    pub fn log(&self, msg: impl AsRef<str>) {
        self.log.line(msg);
    }

    pub fn watch_connected(&self) -> watch::Receiver<bool> {
        self.connected.subscribe()
    }

    /// Sends a tool call to the Plugin and returns its reply message (`{"id","ok",...}`).
    pub async fn call(&self, tool: &str, args: Value) -> Result<Value, CallError> {
        self.call_binary(tool, args).await.map(|r| r.msg)
    }

    /// Like `call`, plus the raw bytes of a binary reply (empty for text replies).
    pub async fn call_binary(&self, tool: &str, args: Value) -> Result<Reply, CallError> {
        // Fail at once when nothing is connected instead of queueing behind a running call.
        if self.bridge.lock().is_none() {
            return Err(CallError::NotConnected);
        }
        let _turn = self.gate.lock().await;
        let (conn, tx) = self.bridge.lock().clone().ok_or(CallError::NotConnected)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = oneshot::channel();
        self.pending.lock().insert(id, (conn, reply_tx));
        if tx.send(json!({ "id": id, "tool": tool, "args": args }).to_string()).is_err() {
            self.pending.lock().remove(&id);
            return Err(CallError::NotConnected);
        }
        match tokio::time::timeout(self.call_timeout, reply_rx).await {
            Ok(Ok(reply)) => Ok(reply),
            // The connection ended and dropped our waiter: no reply can come any more.
            Ok(Err(_)) => Err(CallError::Disconnected),
            Err(_) => {
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
        self.connected.send_replace(true);
        self.log("bridge connected");

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                out = rx.recv() => match out {
                    Some(text) => if socket.send(Message::Text(text.into())).await.is_err() { break },
                    None => break,
                },
                msg = socket.recv() => match msg {
                    Some(Ok(Message::Text(text))) => self.on_text(&text),
                    Some(Ok(Message::Binary(frame))) => self.on_binary(&frame),
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                },
            }
        }

        {
            let mut bridge = self.bridge.lock();
            if matches!(&*bridge, Some((c, _)) if *c == conn) {
                *bridge = None;
                self.connected.send_replace(false);
            }
        }
        // Calls sent on this connection can never be answered: fail them now, not after the timeout.
        self.pending.lock().retain(|_, (c, _)| *c != conn);
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
            _ => self.deliver(msg, Vec::new()),
        }
    }

    /// Binary frame: `<compact JSON header>\n<raw bytes>`, split at the first `\n`
    /// (the header never contains a raw newline, the bytes may).
    fn on_binary(&self, frame: &[u8]) {
        let parsed = frame
            .iter()
            .position(|&b| b == b'\n')
            .and_then(|i| Some((serde_json::from_slice::<Value>(&frame[..i]).ok()?, &frame[i + 1..])));
        match parsed {
            Some((header, bytes)) => self.deliver(header, bytes.to_vec()),
            None => self.log("bad binary frame from plugin"),
        }
    }

    fn deliver(&self, msg: Value, bytes: Vec<u8>) {
        let id = msg.get("id").and_then(Value::as_u64).unwrap_or(0);
        match self.pending.lock().remove(&id) {
            Some((_, waiter)) => {
                let _ = waiter.send(Reply { msg, bytes });
            }
            None => self.log(format!("dropping reply for unknown or timed-out call {id}")),
        }
    }
}
