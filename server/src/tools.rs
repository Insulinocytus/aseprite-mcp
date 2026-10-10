//! MCP tool surface. Every tool forwards to the Plugin through `Hub::call` via `forward`.

use crate::bridge::Hub;
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const RUN_LUA_DESCRIPTION: &str = "Run Lua in the live Aseprite GUI and modify the document. \
The code is the body of a function: use `return` to give a value back (the first returned value is \
sent as JSON). Errors return the message and a Lua traceback. With an active sprite the whole call \
is one undo step named \"MCP: run_lua\" (one Ctrl+Z reverts it) and is rolled back if the code errors. \
Never open UI (Dialog:show, commands with ui=true): a dialog blocks every later call.";

#[derive(Clone)]
pub struct Tools {
    hub: Arc<Hub>,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct RunLuaArgs {
    /// Lua code, executed as a function body.
    code: String,
}

impl Tools {
    pub fn new(hub: Arc<Hub>) -> Self {
        Self { hub, tool_router: Self::tool_router() }
    }

    /// Runs a call on the Plugin and maps its reply to an MCP result.
    async fn forward(&self, tool: &str, args: Value) -> CallToolResult {
        let text = |s: String| vec![ContentBlock::text(s)];
        match self.hub.call(tool, args).await {
            Err(e) => CallToolResult::error(text(e.to_string())),
            Ok(reply) if reply["ok"] == true => CallToolResult::success(text(reply["result"].to_string())),
            Ok(reply) => CallToolResult::error(text(reply["error"].as_str().unwrap_or("unknown plugin error").to_owned())),
        }
    }
}

#[tool_router]
impl Tools {
    #[tool(description = RUN_LUA_DESCRIPTION)]
    async fn run_lua(&self, Parameters(RunLuaArgs { code }): Parameters<RunLuaArgs>) -> CallToolResult {
        self.forward("run_lua", json!({ "code": code })).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("aseprite-mcp", env!("CARGO_PKG_VERSION")))
    }
}
