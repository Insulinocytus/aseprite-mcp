# External Server process with a WebSocket Bridge

Aseprite's Lua API can only open WebSocket *client* connections (no server binding; native Lua C modules are unusable because Lua is compiled as C++), so the Plugin cannot host the MCP endpoint itself. We bundle a separate Server executable in the extension, have the Plugin spawn it on `init()`, and connect to it over a WebSocket Bridge on which the Server pushes tool calls for the Plugin to execute in the running GUI.

## Considered Options

- **Fork Aseprite and add a Lua binding for the already-compiled IXWebSocket server**: pure-plugin solution, but requires maintaining a fork and cannot run on official/Steam builds. Rejected.
- **`aseprite --batch` per tool call** (as in diivi/aseprite-mcp): no live GUI, no undo, re-saves files each call. Rejected; we also chose no batch fallback when the GUI is not running.

## Consequences

- Minimum supported Aseprite is v1.3.17 (current Steam build); WebSocket and script-security behaviour are identical to v1.3.18.6.
- The Server must clean up after itself (watchdog on Bridge disconnect) because Aseprite crashes skip `exit()`. Validated on branch `prototype/bridge-spike` (`spike/VERDICT.md`): crash → exit after 10 s; relaunch over a stale Server binds after retry.
- First run shows one permission prompt; "Give full trust to this script" covers both execute and open-socket. `start "" /B` returns immediately and, with a GUI-subsystem Server, shows no console window.
