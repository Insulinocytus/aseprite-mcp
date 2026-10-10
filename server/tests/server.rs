//! Server seam: start the Server in-process on a random port, attach a fake Plugin to the
//! Bridge and drive `/mcp` with an rmcp client.

use aseprite_mcp_server::{Config, Server};
use futures_util::{SinkExt, StreamExt};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ClientConfig},
    service::RunningService,
    transport::{StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

const TOKEN: &str = "test-token";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Fake Plugin: answers `run_lua` with `{ok:true,result:<code>}` unless the code is "fail".
async fn connect_fake_plugin(server: &Server, token: &str) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    fake_plugin(server, token, "").await
}

/// Like `connect_fake_plugin`, but run_lua results are prefixed with `label` so tests can tell Plugins apart.
async fn fake_plugin(server: &Server, token: &str, label: &'static str) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let (mut ws, _) = connect_async(format!("ws://{}/ws?token={token}", server.addr)).await?;
    tokio::spawn(async move {
        ws.send(Message::text(json!({"type": "hello", "version": "test"}).to_string())).await.unwrap();
        while let Some(Ok(Message::Text(text))) = ws.next().await {
            let req: Value = serde_json::from_str(&text).unwrap();
            if req["tool"] == "status" {
                let sprites = json!([{"filename": "a.aseprite", "width": 16, "height": 8, "color_mode": "rgb", "active": true, "modified": false}]);
                let reply = json!({"id": req["id"], "ok": true, "result": {"aseprite_version": "1.3.test", "sprites": sprites}});
                ws.send(Message::text(reply.to_string())).await.unwrap();
                continue;
            }
            assert_eq!(req["tool"], "run_lua");
            let code = req["args"]["code"].as_str().unwrap();
            let reply = if code == "fail" {
                json!({"id": req["id"], "ok": false, "error": "boom\nstack traceback:\n\t[run_lua]:1"})
            } else {
                json!({"id": req["id"], "ok": true, "result": format!("{label}{code}")})
            };
            ws.send(Message::text(reply.to_string())).await.unwrap();
        }
    });
    Ok(())
}

async fn mcp_client(server: &Server) -> RunningService<RoleClient, ClientConfig> {
    let transport = StreamableHttpClientTransport::from_config(StreamableHttpClientTransportConfig::with_uri(format!(
        "http://{}/mcp",
        server.addr
    )));
    ClientConfig::default().serve(transport).await.unwrap()
}

/// The Bridge registers asynchronously after the WebSocket handshake, so retry while it is "not connected".
async fn run_lua(client: &RunningService<RoleClient, ClientConfig>, code: &str) -> CallToolResult {
    let args = json!({ "code": code }).as_object().unwrap().clone();
    for _ in 0..50 {
        let result = client.call_tool(CallToolRequestParams::new("run_lua").with_arguments(args.clone())).await.unwrap();
        if !text(&result).contains("not connected") {
            return result;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("Bridge never connected");
}

fn text(result: &CallToolResult) -> &str {
    &result.content[0].as_text().unwrap().text
}

#[tokio::test]
async fn run_lua_returns_plugin_result_as_json() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    connect_fake_plugin(&server, TOKEN).await.unwrap();
    let client = mcp_client(&server).await;

    let result = run_lua(&client, "return 42").await;

    assert_eq!(result.is_error, Some(false));
    assert_eq!(text(&result), r#""return 42""#);
    server.shutdown();
}

#[tokio::test]
async fn run_lua_error_carries_message_and_traceback() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    connect_fake_plugin(&server, TOKEN).await.unwrap();
    let client = mcp_client(&server).await;

    let result = run_lua(&client, "fail").await;

    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).starts_with("boom"));
    assert!(text(&result).contains("stack traceback"));
    server.shutdown();
}

#[tokio::test]
async fn bridge_rejects_wrong_token_with_401() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();

    let err = connect_fake_plugin(&server, "wrong").await.unwrap_err();

    match err {
        tokio_tungstenite::tungstenite::Error::Http(resp) => assert_eq!(resp.status(), 401),
        other => panic!("expected HTTP 401, got {other:?}"),
    }
    server.shutdown();
}

/// Fake Plugin for the Raw Lua Tools: replies `{tool, code}` as the result plus two print lines.
/// Code "fail" makes it an error reply (still with output).
async fn connect_echo_plugin(server: &Server) {
    let (mut ws, _) = connect_async(format!("ws://{}/ws?token={TOKEN}", server.addr)).await.unwrap();
    tokio::spawn(async move {
        ws.send(Message::text(json!({"type": "hello", "version": "test"}).to_string())).await.unwrap();
        while let Some(Ok(Message::Text(text))) = ws.next().await {
            let req: Value = serde_json::from_str(&text).unwrap();
            let output = json!(["line 1", "line 2"]);
            let reply = if req["args"]["code"] == "fail" {
                json!({"id": req["id"], "ok": false, "error": "boom", "output": output})
            } else {
                json!({"id": req["id"], "ok": true, "result": {"tool": req["tool"], "code": req["args"]["code"]}, "output": output})
            };
            ws.send(Message::text(reply.to_string())).await.unwrap();
        }
    });
}

async fn call_raw(client: &RunningService<RoleClient, ClientConfig>, tool: &'static str, code: &str) -> CallToolResult {
    let args = json!({ "code": code }).as_object().unwrap().clone();
    for _ in 0..50 {
        let result = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args.clone())).await.unwrap();
        if !text(&result).contains("not connected") {
            return result;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("Bridge never connected");
}

#[tokio::test]
async fn query_lua_round_trips_result_and_print_output() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    connect_echo_plugin(&server).await;
    let client = mcp_client(&server).await;

    let result = call_raw(&client, "query_lua", "return 1").await;

    assert_eq!(result.is_error, Some(false));
    let value: Value = serde_json::from_str(text(&result)).unwrap();
    assert_eq!(value, json!({"tool": "query_lua", "code": "return 1"}));
    assert_eq!(result.content[1].as_text().unwrap().text, "line 1\nline 2");
    server.shutdown();
}

#[tokio::test]
async fn print_output_is_returned_with_errors_and_run_lua_too() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    connect_echo_plugin(&server).await;
    let client = mcp_client(&server).await;

    let result = call_raw(&client, "run_lua", "fail").await;

    assert_eq!(result.is_error, Some(true));
    assert_eq!(text(&result), "boom");
    assert_eq!(result.content[1].as_text().unwrap().text, "line 1\nline 2");
    server.shutdown();
}

async fn get_status(client: &RunningService<RoleClient, ClientConfig>) -> Value {
    let result = client.call_tool(CallToolRequestParams::new("get_status")).await.unwrap();
    assert_eq!(result.is_error, Some(false));
    serde_json::from_str(text(&result)).unwrap()
}

#[tokio::test]
async fn get_status_without_bridge_explains_why() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    let client = mcp_client(&server).await;

    let status = get_status(&client).await;

    assert_eq!(status["bridge_connected"], false);
    assert_eq!(status["port"], server.addr.port());
    assert_eq!(status["server_version"], env!("CARGO_PKG_VERSION"));
    assert!(status["reason"].as_str().unwrap().contains("not connected"));
    assert!(status.get("sprites").is_none());
    server.shutdown();
}

#[tokio::test]
async fn get_status_with_bridge_lists_aseprite_version_and_sprites() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    connect_fake_plugin(&server, TOKEN).await.unwrap();
    let client = mcp_client(&server).await;

    let mut status = get_status(&client).await;
    for _ in 0..50 {
        if status["bridge_connected"] == true {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        status = get_status(&client).await;
    }

    assert_eq!(status["bridge_connected"], true);
    assert!(status.get("reason").is_none());
    assert_eq!(status["aseprite_version"], "1.3.test");
    assert_eq!(
        status["sprites"],
        json!([{"filename": "a.aseprite", "width": 16, "height": 8, "color_mode": "rgb", "active": true, "modified": false}])
    );
    server.shutdown();
}

async fn connect_raw(server: &Server) -> Ws {
    let (mut ws, _) = connect_async(format!("ws://{}/ws?token={TOKEN}", server.addr)).await.unwrap();
    ws.send(Message::text(json!({"type": "hello", "version": "test"}).to_string())).await.unwrap();
    ws
}

async fn recv_call(ws: &mut Ws) -> Value {
    loop {
        match ws.next().await.unwrap().unwrap() {
            Message::Text(t) => return serde_json::from_str(&t).unwrap(),
            _ => {}
        }
    }
}

async fn reply(ws: &mut Ws, call: &Value) {
    let code = call["args"]["code"].as_str().unwrap();
    ws.send(Message::text(json!({"id": call["id"], "ok": true, "result": code}).to_string())).await.unwrap();
}

/// One `run_lua` call without the not-connected retry.
fn spawn_call(client: &RunningService<RoleClient, ClientConfig>, code: &'static str) -> tokio::task::JoinHandle<CallToolResult> {
    let peer = client.peer().clone();
    let args = json!({ "code": code }).as_object().unwrap().clone();
    tokio::spawn(async move { peer.call_tool(CallToolRequestParams::new("run_lua").with_arguments(args)).await.unwrap() })
}

fn short_lifecycle(grace_ms: u64, watchdog_ms: u64) -> Config {
    let mut c = Config::new(0, TOKEN);
    c.grace = Duration::from_millis(grace_ms);
    c.watchdog = Duration::from_millis(watchdog_ms);
    c
}

async fn exits_within(server: Server, limit: Duration) -> Duration {
    let t = Instant::now();
    tokio::time::timeout(limit, server.wait()).await.expect("server did not exit").unwrap();
    t.elapsed()
}

#[tokio::test]
async fn exits_after_grace_when_no_bridge_ever_connects() {
    let server = Server::start(short_lifecycle(600, 100)).await.unwrap();

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!server.is_finished(), "exited inside the grace period");

    exits_within(server, Duration::from_secs(3)).await;
}

#[tokio::test]
async fn exits_when_bridge_stays_gone_past_watchdog() {
    let server = Server::start(short_lifecycle(200, 500)).await.unwrap();
    let mut ws = connect_raw(&server).await;
    tokio::time::sleep(Duration::from_millis(400)).await; // past grace, still connected
    assert!(!server.is_finished());

    ws.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(!server.is_finished(), "exited before the watchdog period");

    exits_within(server, Duration::from_secs(3)).await;
}

#[tokio::test]
async fn bridge_reconnecting_within_watchdog_keeps_server_alive() {
    let server = Server::start(short_lifecycle(200, 600)).await.unwrap();
    let mut ws = connect_raw(&server).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    ws.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ws = connect_raw(&server).await;

    tokio::time::sleep(Duration::from_millis(900)).await;

    assert!(!server.is_finished());
    server.shutdown();
}

#[tokio::test]
async fn bye_stops_server_immediately() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap(); // default 15 s grace / 10 s watchdog
    let mut ws = connect_raw(&server).await;

    ws.send(Message::text(json!({"type": "bye"}).to_string())).await.unwrap();

    assert!(exits_within(server, Duration::from_secs(1)).await < Duration::from_secs(1));
}

#[tokio::test]
async fn bind_is_retried_until_the_port_frees_up_and_log_is_created_only_then() {
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = blocker.local_addr().unwrap().port();
    let log = std::env::temp_dir().join(format!("aseprite-mcp-test-{}-{port}.log", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let mut config = Config::new(port, TOKEN);
    config.log = Some(log.clone());
    config.bind_retry_interval = Duration::from_millis(50);
    config.bind_retry_for = Duration::from_secs(5);
    let start = tokio::spawn(Server::start(config));

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!start.is_finished(), "gave up while still retrying");
    assert!(!log.exists(), "log created before the bind succeeded");

    drop(blocker);
    let server = start.await.unwrap().unwrap();
    assert!(log.exists());
    server.shutdown();
    let _ = std::fs::remove_file(&log);
}

#[tokio::test]
async fn bind_gives_up_after_retry_budget_without_creating_log() {
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = blocker.local_addr().unwrap().port();
    let log = std::env::temp_dir().join(format!("aseprite-mcp-test-giveup-{}-{port}.log", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let mut config = Config::new(port, TOKEN);
    config.log = Some(log.clone());
    config.bind_retry_interval = Duration::from_millis(50);
    config.bind_retry_for = Duration::from_millis(300);

    let t = Instant::now();
    let result = Server::start(config).await;

    assert!(result.is_err());
    assert!(t.elapsed() >= Duration::from_millis(250), "gave up before the retry budget");
    assert!(!log.exists());
}

#[tokio::test]
async fn new_connection_replaces_the_old_bridge() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    fake_plugin(&server, TOKEN, "A:").await.unwrap();
    let client = mcp_client(&server).await;
    assert_eq!(text(&run_lua(&client, "x").await), r#""A:x""#);

    fake_plugin(&server, TOKEN, "B:").await.unwrap();

    let mut last = String::new();
    for _ in 0..50 {
        last = text(&run_lua(&client, "x").await).to_owned();
        if last == r#""B:x""# {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(last, r#""B:x""#);
    assert_eq!(text(&run_lua(&client, "y").await), r#""B:y""#); // the old connection ending does not unregister the new one
    server.shutdown();
}

#[tokio::test]
async fn calls_reach_the_plugin_one_at_a_time_in_order() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    let mut plugin = connect_raw(&server).await;
    let client = mcp_client(&server).await;
    let _ = run_lua_when_connected(&client, &mut plugin).await;

    let a = spawn_call(&client, "a");
    let first = recv_call(&mut plugin).await;
    let b = spawn_call(&client, "b");

    // The second call must stay queued while the first is unanswered.
    let early = tokio::time::timeout(Duration::from_millis(300), recv_call(&mut plugin)).await;
    assert!(early.is_err(), "second call was sent before the first was answered");

    reply(&mut plugin, &first).await;
    let second = recv_call(&mut plugin).await;
    reply(&mut plugin, &second).await;

    assert_eq!(first["args"]["code"], "a");
    assert_eq!(second["args"]["code"], "b");
    assert_eq!(text(&a.await.unwrap()), r#""a""#);
    assert_eq!(text(&b.await.unwrap()), r#""b""#);
    server.shutdown();
}

/// Waits until the raw fake Plugin is registered (first call gets through) by answering a probe call.
async fn run_lua_when_connected(client: &RunningService<RoleClient, ClientConfig>, plugin: &mut Ws) -> CallToolResult {
    let probe = tokio::spawn({
        let client = client.peer().clone();
        async move {
            let args = json!({ "code": "probe" }).as_object().unwrap().clone();
            for _ in 0..50 {
                let r = client.call_tool(CallToolRequestParams::new("run_lua").with_arguments(args.clone())).await.unwrap();
                if !text(&r).contains("not connected") {
                    return r;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("Bridge never connected");
        }
    });
    let call = recv_call(plugin).await;
    reply(plugin, &call).await;
    probe.await.unwrap()
}

#[tokio::test]
async fn timed_out_call_reports_it_and_its_late_reply_is_dropped() {
    let mut config = Config::new(0, TOKEN);
    config.call_timeout = Duration::from_millis(300);
    let server = Server::start(config).await.unwrap();
    let mut plugin = connect_raw(&server).await;
    let client = mcp_client(&server).await;
    let _ = run_lua_when_connected(&client, &mut plugin).await;

    let slow = spawn_call(&client, "slow");
    let slow_call = recv_call(&mut plugin).await;
    let result = slow.await.unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("timed out") && text(&result).contains("may still be running"), "{}", text(&result));

    // The late reply must not be mistaken for the next call's answer.
    let next = spawn_call(&client, "next");
    reply(&mut plugin, &slow_call).await;
    let next_call = recv_call(&mut plugin).await;
    reply(&mut plugin, &next_call).await;
    assert_eq!(text(&next.await.unwrap()), r#""next""#);
    server.shutdown();
}

#[tokio::test]
async fn call_in_flight_fails_at_once_when_the_bridge_drops() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap(); // default 30 s call timeout
    let mut plugin = connect_raw(&server).await;
    let client = mcp_client(&server).await;
    let _ = run_lua_when_connected(&client, &mut plugin).await;

    let call = spawn_call(&client, "x");
    let _ = recv_call(&mut plugin).await;
    let t = Instant::now();
    plugin.close(None).await.unwrap();
    let result = call.await.unwrap();

    assert!(t.elapsed() < Duration::from_secs(2), "waited {:?}", t.elapsed());
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("disconnected"), "{}", text(&result));
    server.shutdown();
}

#[tokio::test]
async fn get_status_after_timeout_still_reports_bridge_connected() {
    let mut config = Config::new(0, TOKEN);
    config.call_timeout = Duration::from_millis(300);
    let server = Server::start(config).await.unwrap();
    let mut plugin = connect_raw(&server).await;
    let client = mcp_client(&server).await;
    let _ = run_lua_when_connected(&client, &mut plugin).await;

    // The raw Plugin never answers `status`, so the call times out while the Bridge stays up.
    let status = get_status(&client).await;

    assert_eq!(status["bridge_connected"], true, "{status}");
    assert!(status["reason"].as_str().unwrap().contains("timed out"), "{status}");
    server.shutdown();
}

#[tokio::test]
async fn tool_call_without_bridge_fails_at_once_with_open_aseprite_hint() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    let client = mcp_client(&server).await;

    let t = Instant::now();
    let result = spawn_call(&client, "x").await.unwrap();

    assert!(t.elapsed() < Duration::from_secs(1));
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("not connected") && text(&result).contains("open Aseprite"), "{}", text(&result));
    server.shutdown();
}

/// Status code of `GET /mcp` (or `/ws`) sent with the given raw `Host` / optional `Origin` headers.
async fn http_status(server: &Server, path: &str, host: &str, origin: Option<&str>) -> u16 {
    let mut s = TcpStream::connect(server.addr).await.unwrap();
    let origin = origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{origin}Connection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    buf.split(' ').nth(1).unwrap().parse().unwrap()
}

#[tokio::test]
async fn non_loopback_origin_or_host_is_refused() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    let local = format!("127.0.0.1:{}", server.addr.port());

    for (host, origin) in [
        (local.as_str(), None),
        ("localhost", None),
        (local.as_str(), Some("http://localhost:5173")),
        (local.as_str(), Some("http://127.0.0.1")),
    ] {
        assert_ne!(http_status(&server, "/mcp", host, origin).await, 403, "{host} {origin:?} must pass");
    }
    for (host, origin) in [
        ("evil.example", None),
        ("evil.example:80", Some("http://127.0.0.1")),
        (local.as_str(), Some("http://evil.example")),
        (local.as_str(), Some("http://127.0.0.1.evil.example")),
        (local.as_str(), Some("null")),
    ] {
        assert_eq!(http_status(&server, "/mcp", host, origin).await, 403, "{host} {origin:?} must be refused");
    }
    assert_eq!(http_status(&server, "/ws?token=test-token", &local, Some("http://evil.example")).await, 403);
    server.shutdown();
}

// ---- get_image ----

use aseprite_mcp_server::image::{default_scale, render_png, upscale};
use base64::{Engine, engine::general_purpose::STANDARD};

fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!((info.color_type, info.bit_depth), (png::ColorType::Rgba, png::BitDepth::Eight));
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

/// 3x2 RGBA with distinct pixels; the second pixel's red byte is 0x0a (a newline).
const PIXELS: [u8; 24] = [
    255, 0, 0, 255, 10, 20, 30, 40, 0, 0, 255, 0, //
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
];

#[test]
fn default_scale_is_smallest_integer_reaching_256() {
    assert_eq!(default_scale(16, 16), 16);
    assert_eq!(default_scale(100, 50), 3);
    assert_eq!(default_scale(50, 100), 3);
    assert_eq!(default_scale(256, 10), 1);
    assert_eq!(default_scale(1000, 1000), 1);
}

#[test]
fn upscale_repeats_each_pixel() {
    let out = upscale(&[1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4], 2, 2, 2);
    let px = |v: u8| [v, v, v, v];
    let row = |a: u8, b: u8| [px(a), px(a), px(b), px(b)].concat();
    assert_eq!(out, [row(1, 2), row(1, 2), row(3, 4), row(3, 4)].concat());
}

#[test]
fn png_decodes_back_to_original_pixels() {
    let (w, h, pixels) = decode_png(&render_png(&PIXELS, 3, 2, Some(1)).unwrap());
    assert_eq!((w, h), (3, 2));
    assert_eq!(pixels, PIXELS);

    let (w, h, pixels) = decode_png(&render_png(&PIXELS, 3, 2, Some(4)).unwrap());
    assert_eq!((w, h), (12, 8));
    assert_eq!(pixels, upscale(&PIXELS, 3, 2, 4));
    // spot-check one pixel of the 4x block of source pixel (1, 1) = [5,6,7,8]
    let at = |x: usize, y: usize| pixels[(y * 12 + x) * 4..][..4].to_vec();
    assert_eq!(at(4, 4), [5, 6, 7, 8]);
    assert_eq!(at(7, 7), [5, 6, 7, 8]);
    assert_eq!(at(8, 4), [9, 10, 11, 12]);
}

#[test]
fn render_png_rejects_bad_input() {
    assert!(render_png(&PIXELS[..20], 3, 2, None).is_err());
    assert!(render_png(&PIXELS, 3, 2, Some(0)).is_err());
    assert!(render_png(&PIXELS, 3, 2, Some(100_000)).is_err());
}

/// Fake Plugin for get_image: records the args it got and replies with a binary frame
/// (`PIXELS`, 3x2), or with a text error when `layer` is "missing".
async fn connect_image_plugin(server: &Server) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
    let (mut ws, _) = connect_async(format!("ws://{}/ws?token={TOKEN}", server.addr)).await.unwrap();
    let (seen_tx, seen_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        ws.send(Message::text(json!({"type": "hello", "version": "test"}).to_string())).await.unwrap();
        while let Some(Ok(Message::Text(text))) = ws.next().await {
            let req: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(req["tool"], "get_image");
            seen_tx.send(req["args"].clone()).unwrap();
            if req["args"]["layer"] == "missing" {
                let reply = json!({"id": req["id"], "ok": false, "error": "layer not found: missing"});
                ws.send(Message::text(reply.to_string())).await.unwrap();
                continue;
            }
            let mut frame = json!({"id": req["id"], "ok": true, "width": 3, "height": 2}).to_string().into_bytes();
            frame.push(b'\n');
            frame.extend_from_slice(&PIXELS);
            ws.send(Message::Binary(frame.into())).await.unwrap();
        }
    });
    seen_rx
}

async fn get_image(client: &RunningService<RoleClient, ClientConfig>, args: Value) -> CallToolResult {
    let args = args.as_object().unwrap().clone();
    for _ in 0..50 {
        let result = client.call_tool(CallToolRequestParams::new("get_image").with_arguments(args.clone())).await.unwrap();
        if result.is_error != Some(true) || !text(&result).contains("not connected") {
            return result;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("Bridge never connected");
}

#[tokio::test]
async fn get_image_returns_upscaled_png_from_binary_frame() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    let mut seen = connect_image_plugin(&server).await;
    let client = mcp_client(&server).await;

    let result = get_image(&client, json!({"sprite": "a.aseprite", "frame": 2, "scale": 2})).await;

    assert_eq!(result.is_error, Some(false));
    let image = result.content[0].as_image().unwrap();
    assert_eq!(image.mime_type, "image/png");
    let (w, h, pixels) = decode_png(&STANDARD.decode(&image.data).unwrap());
    assert_eq!((w, h), (6, 4));
    assert_eq!(pixels, upscale(&PIXELS, 3, 2, 2));
    assert_eq!(seen.recv().await.unwrap(), json!({"sprite": "a.aseprite", "frame": 2}));

    // default scale: 3x2 -> ceil(256/3) = 86
    let result = get_image(&client, json!({})).await;
    let (w, h, pixels) = decode_png(&STANDARD.decode(&result.content[0].as_image().unwrap().data).unwrap());
    assert_eq!((w, h), (3 * 86, 2 * 86));
    assert_eq!(&pixels[..4], [255, 0, 0, 255]);
    assert_eq!(seen.recv().await.unwrap(), json!({}));
    server.shutdown();
}

#[tokio::test]
async fn get_image_plugin_error_is_a_tool_error() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    let mut seen = connect_image_plugin(&server).await;
    let client = mcp_client(&server).await;

    let result = get_image(&client, json!({"layer": "missing"})).await;

    assert_eq!(result.is_error, Some(true));
    assert_eq!(text(&result), "layer not found: missing");
    assert_eq!(seen.recv().await.unwrap(), json!({"layer": "missing"}));
    server.shutdown();
}

#[tokio::test]
async fn list_tools_is_exactly_the_four_documented_tools() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    let client = mcp_client(&server).await;

    let mut names: Vec<String> = client.list_all_tools().await.unwrap().into_iter().map(|t| t.name.to_string()).collect();

    names.sort();
    assert_eq!(names, ["get_image", "get_status", "query_lua", "run_lua"]);
    server.shutdown();
}

#[tokio::test]
async fn invalid_arguments_are_rejected_before_reaching_the_plugin() {
    let server = Server::start(Config::new(0, TOKEN)).await.unwrap();
    let mut seen = connect_image_plugin(&server).await;
    let client = mcp_client(&server).await;
    let call = |tool: &'static str, args: Value| {
        client.call_tool(CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone()))
    };

    for (tool, args) in [
        ("run_lua", json!({})),
        ("query_lua", json!({})),
        ("run_lua", json!({"code": 5})),
        ("get_image", json!({"scale": 1.5})),
        ("get_image", json!({"scale": "2"})),
    ] {
        let outcome = call(tool, args.clone()).await;
        assert!(outcome.is_err() || outcome.unwrap().is_error == Some(true), "{tool} {args} must be rejected");
    }
    assert!(seen.try_recv().is_err(), "rejected calls must not reach the Plugin");
    server.shutdown();
}
