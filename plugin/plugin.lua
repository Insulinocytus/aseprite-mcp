-- Plugin entry: spawns the bundled Server, keeps the Bridge to it, and hands every
-- call to executor.lua (which is also loadable without a WebSocket, see plugin/tests).

local executor
local settings
local ws
local state = "connecting" -- Bridge state, see settings.describe
local dlg -- the open settings window, if any
local port
local rejected_alerted = false

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

local function refresh()
  if not dlg then return end
  local server, bridge = settings.describe(state)
  dlg:modify { id = "server", text = server }
  dlg:modify { id = "bridge", text = bridge }
  dlg:modify { id = "url", text = settings.url(port) }
  dlg:modify { id = "snippet", text = settings.snippet(port) }
end

local function set_state(s)
  state = s
  refresh()
end

-- Spawns the Server and opens the Bridge to it with a fresh Bridge Token.
local function start(plugin)
  local token = make_token()
  local exe = app.fs.joinPath(plugin.path, "aseprite-mcp-server.exe")
  local log = app.fs.joinPath(plugin.path, "server.log")
  -- `start "" /B` returns at once; the Server is a GUI-subsystem exe so no console window appears.
  os.execute(string.format('start "" /B "%s" --port %d --token %s --log "%s"', exe, port, token, log))

  local socket
  socket = WebSocket {
    url = string.format("ws://127.0.0.1:%d/ws?token=%s", port, token),
    minreconnectwait = 1,
    maxreconnectwait = 2,
    deflate = false,
    onreceive = function(t, data, reason)
      -- Any error escaping this callback makes Aseprite stop() the socket for good.
      pcall(function()
        if socket ~= ws then return end -- stale socket from before a restart
        if t == WebSocketMessageType.OPEN then
          set_state("connected")
          send(json.encode { type = "hello", version = tostring(app.version) })
        elseif t == WebSocketMessageType.TEXT then
          send(executor.handle_message(data))
        elseif t == WebSocketMessageType.CLOSE then
          set_state("connecting")
        elseif t == WebSocketMessageType.ERROR then
          if settings.is_unauthorized(reason) then
            -- Another Aseprite instance owns the Server on this port: stop retrying, tell the user once.
            ws:close()
            set_state("rejected")
            if not rejected_alerted then
              rejected_alerted = true
              app.alert("MCP 已由另一个 Aseprite 实例提供")
            end
          else
            set_state("connecting")
          end
        end
      end)
    end,
  }
  ws = socket
  state = "connecting"
  ws:connect()
end

-- Old Server gets `bye` and exits (its port bind is retried by the new one); then a new Server + Bridge.
local function restart(plugin, new_port)
  if ws then
    pcall(send, json.encode { type = "bye" })
    pcall(ws.close, ws)
    ws = nil
  end
  plugin.preferences.port = new_port
  port = new_port
  rejected_alerted = false
  start(plugin)
  refresh()
end

local function open_settings(plugin)
  if dlg then return end
  local server, bridge = settings.describe(state)
  dlg = Dialog {
    title = "MCP Server",
    onclose = function() dlg = nil end,
  }
  dlg:label { id = "server", label = "Server:", text = server }
  dlg:label { id = "bridge", label = "Bridge:", text = bridge }
  dlg:label { id = "url", label = "MCP URL:", text = settings.url(port) }
  dlg:entry { id = "snippet", label = "Claude Code:", text = settings.snippet(port) }
  dlg:entry { id = "port", label = "Port:", text = tostring(port) }
  dlg:button {
    id = "restart",
    text = "Restart",
    onclick = function()
      local p = settings.parse_port(dlg.data.port)
      if not p then
        app.alert("Port must be a number from 1024 to 65535")
        return
      end
      restart(plugin, p)
    end,
  }
  dlg:show { wait = false }
end

function init(plugin)
  executor = dofile(app.fs.joinPath(plugin.path, "executor.lua"))
  settings = dofile(app.fs.joinPath(plugin.path, "settings.lua"))
  -- Fixed by default so MCP Client configs stay valid across restarts.
  port = settings.parse_port(plugin.preferences.port or settings.DEFAULT_PORT) or settings.DEFAULT_PORT
  plugin:newCommand {
    id = "AsepriteMcpServer",
    title = "MCP Server…",
    group = "help_about",
    onclick = function() open_settings(plugin) end,
  }
  start(plugin)
end

function exit(plugin)
  if ws then
    pcall(send, json.encode { type = "bye" })
    pcall(ws.close, ws)
  end
end
