//! MCP tool surface. Every tool forwards to the Plugin through `Hub::call` via `forward`.

use crate::bridge::Hub;
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use base64::{Engine, engine::general_purpose::STANDARD};
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
- mcp.sprite(name?) -> Sprite: by file name or full path (a file name shared by several open sprites is an error: use the path); omit for the active sprite.\n\
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
- mcp.sprite(name?) -> Sprite: by file name or full path (a file name shared by several open sprites is an error: use the path); omit for the active sprite.\n\
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
        Self { hub, port, tool_router: Self::tool_router() + Self::status_router() + Self::image_router() }
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
                Ok(reply) => {
                    status["bridge_connected"] = json!(true);
                    status["reason"] = json!(reply["error"].as_str().unwrap_or("unknown plugin error"));
                }
                // A timeout leaves the Bridge connected: report what the hub sees, not a blanket false.
                Err(e) => {
                    status["bridge_connected"] = json!(self.hub.is_connected());
                    status["reason"] = json!(e.to_string());
                }
            }
        }
        CallToolResult::success(vec![ContentBlock::text(status.to_string())])
    }
}

const GET_IMAGE_DESCRIPTION: &str = "Return a PNG of a sprite so you can see the artwork. By default it shows the active sprite's \
current frame with all visible layers composited, as the user sees it. Pixel art is enlarged automatically \
(integer nearest-neighbor, smallest factor that makes the long side at least 256 px), so the returned image is larger than \
the canvas; divide coordinates by `scale` to get canvas pixels. Creates no undo step.\n\
\n\
Arguments (all optional): `sprite` is a file name or full path (default: the active sprite); `frame` is a 1-based frame number \
(default: the current frame); `layer` is a layer path of names separated by `/` such as \"group/child\" \
(default: the visible layers composited; with a layer, only that layer's pixels in that frame are drawn, even if it is hidden; \
with a group, the visible layers inside the group are composited); \
`scale` is an integer factor of at least 1 (default: automatic).";

#[derive(Deserialize, schemars::JsonSchema)]
struct GetImageArgs {
    /// Sprite file name or full path; default is the active sprite.
    sprite: Option<String>,
    /// 1-based frame number; default is the current frame.
    frame: Option<u32>,
    /// Layer path such as "group/child"; default is the visible layers composited.
    layer: Option<String>,
    /// Integer upscale factor; default makes the long side at least 256 px.
    scale: Option<u32>,
}

#[tool_router(router = image_router)]
impl Tools {
    #[tool(description = GET_IMAGE_DESCRIPTION)]
    async fn get_image(&self, Parameters(args): Parameters<GetImageArgs>) -> CallToolResult {
        let fail = |msg: String| CallToolResult::error(vec![ContentBlock::text(msg)]);
        let mut plugin_args = serde_json::Map::new();
        if let Some(sprite) = args.sprite {
            plugin_args.insert("sprite".into(), json!(sprite));
        }
        if let Some(frame) = args.frame {
            plugin_args.insert("frame".into(), json!(frame));
        }
        if let Some(layer) = args.layer {
            plugin_args.insert("layer".into(), json!(layer));
        }
        let reply = match self.hub.call_binary("get_image", Value::Object(plugin_args)).await {
            Ok(reply) => reply,
            Err(e) => return fail(e.to_string()),
        };
        if reply.msg["ok"] != true {
            return fail(reply.msg["error"].as_str().unwrap_or("unknown plugin error").to_owned());
        }
        let dim = |key: &str| reply.msg[key].as_u64().and_then(|n| u32::try_from(n).ok());
        let (Some(width), Some(height)) = (dim("width"), dim("height")) else {
            return fail("bad image reply from plugin: missing width/height".to_owned());
        };
        match crate::image::render_png(&reply.bytes, width, height, args.scale) {
            Ok(png) => CallToolResult::success(vec![ContentBlock::image(STANDARD.encode(png), "image/png")]),
            Err(e) => fail(e),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("aseprite-mcp", env!("CARGO_PKG_VERSION")))
    }
}
