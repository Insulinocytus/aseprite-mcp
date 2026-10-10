-- Plugin seam for settings.lua (pure logic behind the MCP Server window). Failed assert => script error.
local settings = dofile(app.params.settings)

assert(settings.DEFAULT_PORT == 48653)
assert(settings.url(48653) == "http://127.0.0.1:48653/mcp")
assert(settings.snippet(5000) == "claude mcp add --transport http aseprite http://127.0.0.1:5000/mcp")

assert(settings.parse_port("48653") == 48653)
assert(settings.parse_port("  1024 ") == 1024)
assert(settings.parse_port("65535") == 65535)
assert(settings.parse_port("1") == 1)
assert(settings.parse_port("80") == 80)
for _, bad in ipairs { "", "abc", "0", "00", "65536", "12.5", "-1", "1e4", "0x1F90", "99999999999999999999" } do
  assert(settings.parse_port(bad) == nil, "should reject " .. bad)
end

-- 401 policy: an orphan Server from a crashed Aseprite answers 401 for up to its watchdog, so keep
-- retrying and only give up when 401s persist past the window since the first one.
local clock = 100
local rej = settings.new_rejection(function() return clock end)
assert(settings.REJECT_WINDOW >= 20, "window must outlast watchdog (10 s) plus bind-retry margin")
assert(rej.unauthorized() == false, "first 401 keeps retrying")
clock = clock + settings.REJECT_WINDOW - 1
assert(rej.unauthorized() == false, "still inside the window")
clock = clock + 1
assert(rej.unauthorized() == true, "401s persisted for the whole window: give up")
-- a successful connect resets the window
rej.connected()
clock = clock + 1000
assert(rej.unauthorized() == false, "window restarts at the first 401 after a connect")
clock = clock + settings.REJECT_WINDOW
assert(rej.unauthorized() == true)
rej.connected() -- Restart also starts over
assert(rej.unauthorized() == false)

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
