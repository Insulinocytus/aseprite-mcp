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

const RUN_LUA_DESCRIPTION: &str = "Run Lua in the live Aseprite GUI to modify the document. \
Use this for any change: the whole call is one undo step (\"MCP: run_lua\", one Ctrl+Z reverts it) and is rolled back \
if the code errors. For pure reads use query_lua instead (no empty undo step).\n\
\n\
Calling convention: `code` is the body of a Lua function, not a script file. `return` a value to get it back as JSON \
(first returned value only; tables become arrays or objects, userdata becomes a string, nil becomes null). \
`print(...)` output is captured and returned too. Errors return the message plus a Lua traceback. \
With no open sprite the code still runs, just without an undo step.\n\
\n\
Helpers in the global `mcp` table (they raise an error when nothing matches):\n\
- mcp.sprite(name?) -> Sprite: by file name or full path; omit for the active sprite.\n\
- mcp.layer(sprite, \"group/child\") -> Layer: path of layer names, nested groups separated by `/`.\n\
- mcp.cel(layer, frame, create?) -> Cel: frame is a number; returns nil if the cel is missing, or makes an empty one when create=true.\n\
- mcp.edit_image(cel, fn): fn(image) edits a clone of cel.image, which is then assigned back. \
Always use this to change pixels: assigning into cel.image directly is not undoable. If fn errors the cel is left unchanged.\n\
\n\
Never open UI (Dialog:show, app.command.* with ui=true): a dialog blocks this and every later call until timeout. \
Before app.command.* make sure the intended layer/frame is active. Aseprite's own view (active sprite, layer, frame) is restored after the call.\n\
\n\
If one call changes several sprites, the active sprite's changes are in the call's undo step; changes to other sprites go into each sprite's own undo history.";

const QUERY_LUA_DESCRIPTION: &str = "Run read-only Lua in the live Aseprite GUI to inspect the document. \
Use this for pure reads (layers, frames, cels, tags, pixels, properties): it never creates an undo step. \
Do not modify anything with it: the changes cannot be undone by the user. To modify, use run_lua.\n\
\n\
Calling convention: `code` is the body of a Lua function, not a script file. `return` a value to get it back as JSON \
(first returned value only; tables become arrays or objects, userdata becomes a string, nil becomes null). \
`print(...)` output is captured and returned too. Errors return the message plus a Lua traceback.\n\
\n\
Helpers in the global `mcp` table (they raise an error when nothing matches):\n\
- mcp.sprite(name?) -> Sprite: by file name or full path; omit for the active sprite.\n\
- mcp.layer(sprite, \"group/child\") -> Layer: path of layer names, nested groups separated by `/`.\n\
- mcp.cel(layer, frame, create?) -> Cel: frame is a number; returns nil if the cel is missing (do not pass create here).\n\
- mcp.edit_image(cel, fn): modifies pixels; run_lua only.\n\
\n\
Never open UI (Dialog:show, app.command.* with ui=true): a dialog blocks this and every later call until timeout. \
Aseprite's own view (active sprite, layer, frame) is restored after the call.";

#[derive(Clone)]
pub struct Tools {
    hub: Arc<Hub>,
    port: u16,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct RunLuaArgs {
    /// Lua code, executed as a function body.
    code: String,
}

impl Tools {
    pub fn new(hub: Arc<Hub>, port: u16) -> Self {
        Self { hub, port, tool_router: Self::tool_router() + Self::status_router() }
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

const GET_STATUS_DESCRIPTION: &str = "Report whether Aseprite is connected, the Server version and port, \
and, when connected, the Aseprite version and the open sprites (filename, size, color mode, active, \
modified). Always succeeds: without Aseprite it explains why. Creates no undo step.";

#[tool_router(router = status_router)]
impl Tools {
    #[tool(description = GET_STATUS_DESCRIPTION)]
    async fn get_status(&self) -> CallToolResult {
        let mut status = json!({
            "server_version": env!("CARGO_PKG_VERSION"),
            "port": self.port,
            "bridge_connected": false,
            "reason": crate::bridge::CallError::NotConnected.to_string(),
        });
        if self.hub.is_connected() {
            match self.hub.call("status", json!({})).await {
                Ok(reply) if reply["ok"] == true => {
                    status["bridge_connected"] = json!(true);
                    status.as_object_mut().unwrap().remove("reason");
                    status["aseprite_version"] = reply["result"]["aseprite_version"].clone();
                    status["sprites"] = reply["result"]["sprites"].clone();
                }
                Ok(reply) => status["reason"] = json!(reply["error"].as_str().unwrap_or("unknown plugin error")),
                Err(e) => status["reason"] = json!(e.to_string()),
            }
        }
        CallToolResult::success(vec![ContentBlock::text(status.to_string())])
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("aseprite-mcp", env!("CARGO_PKG_VERSION")))
    }
}
