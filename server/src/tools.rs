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

const QUERY_LUA_DESCRIPTION: &str = "Run read-only Lua in the live Aseprite GUI: inspect sprites, layers, cels, pixels. \
Same calling convention as run_lua (function body, first `return` value sent as JSON, `print` output returned) \
but never an undo step. Do not modify the document here; use run_lua for changes.";

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

    /// Runs a call on the Plugin and maps its reply to an MCP result: the result (or error) text,
    /// followed by one text block with the captured `print` lines when there are any.
    async fn forward(&self, tool: &str, args: Value) -> CallToolResult {
        let reply = match self.hub.call(tool, args).await {
            Ok(reply) => reply,
            Err(e) => return CallToolResult::error(vec![ContentBlock::text(e.to_string())]),
        };
        let ok = reply["ok"] == true;
        let first = if ok {
            reply["result"].to_string()
        } else {
            reply["error"].as_str().unwrap_or("unknown plugin error").to_owned()
        };
        let mut content = vec![ContentBlock::text(first)];
        let lines: Vec<&str> = reply["output"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        if !lines.is_empty() {
            content.push(ContentBlock::text(lines.join("\n")));
        }
        if ok { CallToolResult::success(content) } else { CallToolResult::error(content) }
    }
}

#[tool_router]
impl Tools {
    #[tool(description = RUN_LUA_DESCRIPTION)]
    async fn run_lua(&self, Parameters(RunLuaArgs { code }): Parameters<RunLuaArgs>) -> CallToolResult {
        self.forward("run_lua", json!({ "code": code })).await
    }

    #[tool(description = QUERY_LUA_DESCRIPTION)]
    async fn query_lua(&self, Parameters(RunLuaArgs { code }): Parameters<RunLuaArgs>) -> CallToolResult {
        self.forward("query_lua", json!({ "code": code })).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("aseprite-mcp", env!("CARGO_PKG_VERSION")))
    }
}
