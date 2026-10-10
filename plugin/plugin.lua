-- Plugin entry: spawns the bundled Server, keeps the Bridge to it, and hands every
-- call to executor.lua (which is also loadable without a WebSocket, see plugin/tests).

local PORT = 48653 -- fixed so MCP Client configs stay valid across restarts
local executor
local ws

-- Bridge Token: not a security boundary (that is the loopback bind, ADR 0002), only keeps
-- another process or a second Aseprite from impersonating this Plugin.
local function make_token()
  local addr = tonumber(tostring({}):match("0x(%x+)") or "0", 16) or 0 -- ASLR entropy
  math.randomseed(os.time() + math.floor(os.clock() * 1e6) + addr)
  local parts = {}
  for i = 1, 4 do parts[i] = string.format("%08x", math.random(0, 0x7fffffff)) end
  return table.concat(parts)
end

local function send(text)
  if ws then ws:sendText(text) end
end

function init(plugin)
  executor = dofile(app.fs.joinPath(plugin.path, "executor.lua"))
  local token = make_token()

  local exe = app.fs.joinPath(plugin.path, "aseprite-mcp-server.exe")
  local log = app.fs.joinPath(plugin.path, "server.log")
  -- `start "" /B` returns at once; the Server is a GUI-subsystem exe so no console window appears.
  os.execute(string.format('start "" /B "%s" --port %d --token %s --log "%s"', exe, PORT, token, log))

  ws = WebSocket {
    url = string.format("ws://127.0.0.1:%d/ws?token=%s", PORT, token),
    minreconnectwait = 1,
    maxreconnectwait = 2,
    deflate = false,
    onreceive = function(t, data)
      -- Any error escaping this callback makes Aseprite stop() the socket for good.
      pcall(function()
        if t == WebSocketMessageType.OPEN then
          send(json.encode { type = "hello", version = tostring(app.version) })
        elseif t == WebSocketMessageType.TEXT then
          send(executor.handle_message(data))
        end
      end)
    end,
  }
  ws:connect()
end

function exit(plugin)
  if ws then
    pcall(send, json.encode { type = "bye" })
    pcall(ws.close, ws)
  end
end
