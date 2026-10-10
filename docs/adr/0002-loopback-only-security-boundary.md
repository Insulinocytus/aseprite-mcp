# Loopback binding is the security boundary; MCP endpoint is unauthenticated

The Server exposes arbitrary Lua execution (`run_lua`) and therefore arbitrary code execution as the user. We rely on binding only to `127.0.0.1` plus `Origin`/`Host` validation (against DNS rebinding from browsers) as the security boundary, and do not require a bearer token on `/mcp`, because a per-launch random token would force users to edit their MCP Client config on every Aseprite start. Any local process can therefore drive Aseprite; this is accepted as normal for a local developer tool.

The Bridge still uses a per-launch random token passed to the Server on its command line. It does not protect Lua execution; it stops another process (or a second Aseprite instance) from impersonating the Plugin and taking over the Bridge.
