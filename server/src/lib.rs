mod bridge;
pub mod image;
mod log;
mod tools;

use axum::{
    Router,
    extract::{Query, Request, State, WebSocketUpgrade},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use bridge::Hub;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{collections::HashMap, io, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle, time::Instant};
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
    /// After start the Server never exits for lack of a Bridge within this period.
    pub grace: Duration,
    /// A Bridge that stays gone this long makes the Server exit.
    pub watchdog: Duration,
    /// Pause between bind attempts while the port is taken.
    pub bind_retry_interval: Duration,
    /// Give up binding (and fail `start`) after this long.
    pub bind_retry_for: Duration,
}

impl Config {
    pub fn new(port: u16, token: impl Into<String>) -> Self {
        Self {
            port,
            token: token.into(),
            log: None,
            call_timeout: Duration::from_secs(30),
            grace: Duration::from_secs(15),
            watchdog: Duration::from_secs(10),
            bind_retry_interval: Duration::from_secs(1),
            bind_retry_for: Duration::from_secs(15),
        }
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
        let listener = bind_with_retry(config.port, config.bind_retry_interval, config.bind_retry_for).await?;
        let addr = listener.local_addr()?;
        let shutdown = CancellationToken::new();
        let log = log::Log::create(config.log.as_deref())?;
        let hub = Arc::new(Hub::new(config.call_timeout, log, shutdown.clone()));
        hub.log(format!("listening on {addr}"));
        tokio::spawn(watchdog(hub.watch_connected(), config.grace, config.watchdog, shutdown.clone(), hub.clone()));

        let mcp = StreamableHttpService::new(
            {
                let hub = hub.clone();
                let port = addr.port();
                move || Ok(tools::Tools::new(hub.clone(), port))
            },
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default().with_cancellation_token(shutdown.child_token()),
        );
        let router = Router::new()
            .route("/ws", get(ws_handler))
            .nest_service("/mcp", mcp)
            .with_state(AppState { token: config.token.into(), hub })
            .layer(middleware::from_fn(check_origin_host));

        let task = tokio::spawn({
            let shutdown = shutdown.clone();
            async move {
                let serve = async { axum::serve(listener, router).with_graceful_shutdown(shutdown.clone().cancelled_owned()).await };
                // Idle keep-alive or SSE connections must not keep a Server that was told to stop alive.
                tokio::select! {
                    r = serve => r,
                    _ = async { shutdown.cancelled().await; tokio::time::sleep(Duration::from_secs(2)).await } => Ok(()),
                }
            }
        });
        Ok(Server { addr, shutdown, task })
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
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

async fn bind_with_retry(port: u16, interval: Duration, give_up_after: Duration) -> io::Result<tokio::net::TcpListener> {
    let deadline = Instant::now() + give_up_after;
    loop {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => return Ok(l),
            Err(e) if Instant::now() + interval > deadline => return Err(e),
            Err(_) => tokio::time::sleep(interval).await,
        }
    }
}

/// Ends the Server when no Bridge is present for too long: `grace` from start, `watchdog` after a disconnect.
async fn watchdog(mut connected: watch::Receiver<bool>, grace: Duration, watchdog: Duration, shutdown: CancellationToken, hub: Arc<Hub>) {
    let mut wait = grace;
    loop {
        if !*connected.borrow_and_update() {
            tokio::select! {
                _ = tokio::time::sleep(wait) => {
                    hub.log("no Bridge, shutting down");
                    shutdown.cancel();
                    return;
                }
                r = connected.changed() => if r.is_err() { return },
                _ = shutdown.cancelled() => return,
            }
        } else {
            tokio::select! {
                r = connected.changed() => if r.is_err() { return },
                _ = shutdown.cancelled() => return,
            }
            wait = watchdog;
        }
    }
}

/// Host name of an `Origin` (`scheme://host[:port]`) or `Host` (`host[:port]`) value is loopback.
fn is_loopback(value: &str) -> bool {
    let authority = value.split_once("://").map_or(value, |(_, rest)| rest.split('/').next().unwrap_or(""));
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => authority,
    };
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1"
}

/// DNS-rebinding guard for every route: a missing `Origin` (CLI clients) passes, anything non-loopback is refused.
async fn check_origin_host(req: Request, next: Next) -> Response {
    let allowed = [header::ORIGIN, header::HOST]
        .iter()
        .all(|name| req.headers().get(name).is_none_or(|v| v.to_str().is_ok_and(is_loopback)));
    if allowed { next.run(req).await } else { StatusCode::FORBIDDEN.into_response() }
}
