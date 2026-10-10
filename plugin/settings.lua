-- Pure logic behind the "MCP Server…" window; no app/Dialog dependency so plugin/tests can load it.
local M = {}

M.DEFAULT_PORT = 48653

function M.url(port) return string.format("http://127.0.0.1:%d/mcp", port) end

function M.snippet(port) return "claude mcp add --transport http aseprite " .. M.url(port) end

-- Entry text -> port number (1-65535), or nil.
function M.parse_port(text)
  local digits = tostring(text):match("^%s*(%d+)%s*$")
  if not digits or #digits > 5 then return nil end
  local n = tonumber(digits)
  if n < 1 or n > 65535 then return nil end
  return n
end

-- Aseprite's WebSocket gives onreceive only (type, data, reason); the HTTP status is not exposed.
-- IXWebSocket words a refused handshake "Expecting status 101 ..., got 401 status connecting to ...".
function M.is_unauthorized(reason)
  return type(reason) == "string" and reason:find("got 401 status", 1, true) ~= nil
end

-- A Server orphaned by a force-killed Aseprite keeps answering 401 until its watchdog (10 s) ends it,
-- and the new Server may still be retrying its bind. So 401s alone do not prove another instance
-- owns the port: only give up when they persist this long (seconds) without a successful connect.
M.REJECT_WINDOW = 20

-- Tracks 401s with an injected clock (seconds). unauthorized() -> true once the window since the
-- first 401 has elapsed (stop retrying, alert); connected() starts the window over.
function M.new_rejection(now)
  local first
  return {
    unauthorized = function()
      first = first or now()
      return now() - first >= M.REJECT_WINDOW
    end,
    connected = function() first = nil end,
  }
end

-- Bridge state ("connecting" | "connected" | "rejected") -> Server text, Bridge text.
-- The Plugin cannot probe the Server directly: a live Bridge is the only proof it runs.
function M.describe(state)
  if state == "connected" then return "running", "connected" end
  if state == "rejected" then return "owned by another Aseprite instance", "rejected (401)" end
  return "starting", "connecting"
end

return M
