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
