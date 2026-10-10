-- Tool-call executor. Pure Aseprite Lua, no WebSocket: plugin.lua feeds it Bridge messages,
-- plugin/tests/ call it directly. Returns a module table.
--
-- Extension points for later tickets:
--   * add a tool: add a function to `handlers` (args -> reply table)
--   * query_lua: call `run_chunk(code, "query_lua", false)`
--   * print capture, view restore, `mcp` helper env: all belong inside `run_chunk`
local M = {}

local MAX_DEPTH = 32

-- Raw Lua env: globals plus `mcp`. Sibling file, found relative to this one (dofile'd by plugin.lua and tests).
local mcp = dofile(debug.getinfo(1, "S").source:sub(2):gsub("[^/\\]*$", "") .. "mcp.lua")
local function new_env() return setmetatable({ mcp = mcp }, { __index = _G }) end

-- Normalises a Lua value so json.encode accepts it: tables become arrays when they are
-- sequences and objects otherwise, anything else json cannot hold becomes tostring(v).
local function to_json(v, depth)
  local t = type(v)
  if t == "nil" or t == "boolean" or t == "string" then return v end
  if t == "number" then
    if v ~= v or v == math.huge or v == -math.huge then return tostring(v) end
    return v
  end
  if t ~= "table" or depth >= MAX_DEPTH then return tostring(v) end
  local n, count = #v, 0
  for _ in pairs(v) do count = count + 1 end
  local out = {}
  if n == count then
    for i = 1, n do out[i] = to_json(v[i], depth + 1) end
  else
    for k, x in pairs(v) do out[tostring(k)] = to_json(x, depth + 1) end
  end
  return out
end

-- Runs Lua source as a function body. With `transactional` and an active sprite the whole call
-- is one undo step; without a sprite app.transaction would throw an opaque C++ exception, so skip it.
local function run_chunk(code, name, transactional)
  local fn, load_err = load(code, "=" .. name, "t", new_env())
  if not fn then return { ok = false, error = load_err } end

  local result, failure
  local function body()
    -- Capture the traceback where the error happens; it is lost once the error crosses the transaction.
    local ok, r = xpcall(fn, debug.traceback)
    if not ok then
      failure = r
      error(r, 0) -- makes app.transaction roll back
    end
    result = r
  end

  local ok, e
  if transactional and app.sprite then
    ok, e = pcall(app.transaction, "MCP: " .. name, body)
  else
    ok, e = pcall(body)
  end
  if ok then return { ok = true, result = to_json(result, 0) } end
  return { ok = false, error = tostring(failure or e) }
end

local COLOR_MODES = { [ColorMode.RGB] = "rgb", [ColorMode.GRAYSCALE] = "grayscale", [ColorMode.INDEXED] = "indexed" }

-- Read-only: no transaction, so no undo step.
local function status()
  local sprites = {}
  for i, s in ipairs(app.sprites) do
    sprites[i] = {
      filename = s.filename,
      width = s.width,
      height = s.height,
      color_mode = COLOR_MODES[s.colorMode],
      active = s == app.sprite,
      modified = s.isModified,
    }
  end
  return { ok = true, result = { aseprite_version = tostring(app.version), sprites = sprites } }
end

local handlers = {
  run_lua = function(args) return run_chunk(args.code, "run_lua", true) end,
  status = status,
}

-- tool name + args table -> reply table (`ok`, `result` | `error`).
function M.execute(tool, args)
  local handler = handlers[tool]
  if not handler then return { ok = false, error = "unknown tool: " .. tostring(tool) } end
  return handler(args or {})
end

-- Bridge text frame in -> Bridge text frame out.
function M.handle_message(text)
  local okDecode, msg = pcall(json.decode, text)
  -- json.decode returns userdata objects, not plain tables
  if not okDecode or (type(msg) ~= "userdata" and type(msg) ~= "table") then
    return json.encode { ok = false, error = "bad request: " .. tostring(text) }
  end
  local okExec, reply = pcall(M.execute, msg.tool, msg.args)
  if not okExec then reply = { ok = false, error = tostring(reply) } end
  reply.id = msg.id
  local okEncode, out = pcall(json.encode, reply)
  if okEncode then return out end
  return json.encode { id = msg.id, ok = false, error = "result is not JSON-serialisable: " .. tostring(out) }
end

return M
