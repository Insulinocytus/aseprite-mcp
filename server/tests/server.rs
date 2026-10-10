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
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const TOKEN: &str = "test-token";

/// Fake Plugin: answers `run_lua` with `{ok:true,result:<code>}` unless the code is "fail".
async fn connect_fake_plugin(server: &Server, token: &str) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let (mut ws, _) = connect_async(format!("ws://{}/ws?token={token}", server.addr)).await?;
    tokio::spawn(async move {
        ws.send(Message::text(json!({"type": "hello", "version": "test"}).to_string())).await.unwrap();
        while let Some(Ok(Message::Text(text))) = ws.next().await {
            let req: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(req["tool"], "run_lua");
            let code = req["args"]["code"].as_str().unwrap();
            let reply = if code == "fail" {
                json!({"id": req["id"], "ok": false, "error": "boom\nstack traceback:\n\t[run_lua]:1"})
            } else {
                json!({"id": req["id"], "ok": true, "result": code})
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
