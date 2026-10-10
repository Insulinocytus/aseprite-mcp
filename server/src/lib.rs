mod bridge;
mod log;
mod tools;

use axum::{
    Router,
    extract::{Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use bridge::Hub;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{collections::HashMap, io, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_PORT: u16 = 48653;

pub struct Config {
    /// 0 picks a free port (tests); read the real one from `Server::addr`.
    pub port: u16,
    /// Bridge Token: `/ws` must present it as `?token=`.
    pub token: String,
    pub log: Option<PathBuf>,
    /// Per-call timeout, injectable so tests need not wait 30 s.
    pub call_timeout: Duration,
}

impl Config {
    pub fn new(port: u16, token: impl Into<String>) -> Self {
        Self { port, token: token.into(), log: None, call_timeout: Duration::from_secs(30) }
    }
}

pub struct Server {
    pub addr: SocketAddr,
    shutdown: CancellationToken,
    task: JoinHandle<io::Result<()>>,
}

#[derive(Clone)]
struct AppState {
    token: Arc<str>,
    hub: Arc<Hub>,
}

impl Server {
    /// Binds 127.0.0.1 and starts serving `/mcp` and `/ws`. The log file is created only after the bind succeeds.
    pub async fn start(config: Config) -> io::Result<Server> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port)).await?;
        let addr = listener.local_addr()?;
        let shutdown = CancellationToken::new();
        let log = log::Log::create(config.log.as_deref())?;
        let hub = Arc::new(Hub::new(config.call_timeout, log, shutdown.clone()));
        hub.log(format!("listening on {addr}"));

        let mcp = StreamableHttpService::new(
            {
                let hub = hub.clone();
                move || Ok(tools::Tools::new(hub.clone()))
            },
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default().with_cancellation_token(shutdown.child_token()),
        );
        let router = Router::new()
            .route("/ws", get(ws_handler))
            .nest_service("/mcp", mcp)
            .with_state(AppState { token: config.token.into(), hub });

        let task = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { axum::serve(listener, router).with_graceful_shutdown(shutdown.cancelled_owned()).await }
        });
        Ok(Server { addr, shutdown, task })
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    /// Resolves once the Server has stopped (`shutdown()` or a Plugin `bye`).
    pub async fn wait(self) -> io::Result<()> {
        self.task.await.map_err(io::Error::other)?
    }
}

async fn ws_handler(State(app): State<AppState>, Query(q): Query<HashMap<String, String>>, ws: WebSocketUpgrade) -> Response {
    if q.get("token").map(String::as_str) != Some(&*app.token) {
        app.hub.log("ws rejected: bad token");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| async move { app.hub.serve_bridge(socket).await })
}
