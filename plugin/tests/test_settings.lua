-- Plugin seam for settings.lua (pure logic behind the MCP Server window). Failed assert => script error.
local settings = dofile(app.params.settings)

assert(settings.DEFAULT_PORT == 48653)
assert(settings.url(48653) == "http://127.0.0.1:48653/mcp")
assert(settings.snippet(5000) == "claude mcp add --transport http aseprite http://127.0.0.1:5000/mcp")

assert(settings.parse_port("48653") == 48653)
assert(settings.parse_port("  1024 ") == 1024)
assert(settings.parse_port("65535") == 65535)
for _, bad in ipairs { "", "abc", "80", "1023", "65536", "12.5", "-1", "1e4", "0x1F90", "99999999999999999999" } do
  assert(settings.parse_port(bad) == nil, "should reject " .. bad)
end

-- Verbatim reason Aseprite hands to onreceive (IXWebSocketHandshake.cpp) when the Server answers 401.
local reason401 = "Expecting status 101 (Switching Protocol), got 401 status connecting to ws://127.0.0.1:48653/ws?token=ab, HTTP Status line: HTTP/1.1 401 Unauthorized\r\n"
assert(settings.is_unauthorized(reason401))
assert(not settings.is_unauthorized("Expecting status 101 (Switching Protocol), got 403 status connecting to ws://127.0.0.1:1/ws, HTTP Status line: HTTP/1.1 403 Forbidden"))
assert(not settings.is_unauthorized("Connection refused"))
assert(not settings.is_unauthorized(""))
assert(not settings.is_unauthorized(nil))
-- a port or token that happens to contain 401 must not match
assert(not settings.is_unauthorized("Connection refused ws://127.0.0.1:40101/ws?token=got 4010"))

local s, b = settings.describe("connected")
assert(s == "running" and b == "connected")
s, b = settings.describe("connecting")
assert(s == "starting" and b == "connecting")
s, b = settings.describe("rejected")
assert(s == "owned by another Aseprite instance" and b == "rejected (401)")

print("ALL PASS")
