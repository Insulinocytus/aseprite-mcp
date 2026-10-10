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

-- Puts back the sprite/layer/frame the user had, for those still alive. Best effort: a stale
-- object must never turn a finished call into an error.
local function restore_view(sprite, layer, frame)
  if not sprite then return end
  local alive = false
  for _, s in ipairs(app.sprites) do alive = alive or s == sprite end
  if not alive then return end
  pcall(function() app.sprite = sprite end)
  if layer and pcall(function() assert(layer.sprite == sprite) end) then pcall(function() app.layer = layer end) end
  if frame and pcall(function() assert(frame.sprite == sprite and frame.frameNumber <= #sprite.frames) end) then
    pcall(function() app.frame = frame end)
  end
end

-- Runs Lua source as a function body. With `transactional` and an active sprite the whole call
-- is one undo step; without a sprite app.transaction would throw an opaque C++ exception, so skip it.
-- `print` is swapped for a collector while the code runs and always restored; the user's
-- active sprite/layer/frame are restored afterwards.
local function run_chunk(code, name, transactional)
  local fn, load_err = load(code, "=" .. name, "t", new_env())
  if not fn then return { ok = false, error = load_err, output = {} } end

  local result, failure
  local output = {}
  local function body()
    -- Capture the traceback where the error happens; it is lost once the error crosses the transaction.
    local ok, r = xpcall(fn, debug.traceback)
    if not ok then
      failure = r
      error(r, 0) -- makes app.transaction roll back
    end
    result = r
  end

  local sprite, layer, frame = app.sprite, app.layer, app.frame
  local original_print = print
  print = function(...)
    local parts = {}
    for i = 1, select("#", ...) do parts[i] = tostring((select(i, ...))) end
    output[#output + 1] = table.concat(parts, "\t")
  end
  local ok, e
  if transactional and app.sprite then
    ok, e = pcall(app.transaction, "MCP: " .. name, body)
  else
    ok, e = pcall(body)
  end
  print = original_print
  restore_view(sprite, layer, frame)

  if ok then return { ok = true, result = to_json(result, 0), output = output } end
  return { ok = false, error = tostring(failure or e), output = output }
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

-- Renders `frame` of `sprite` as an RGB-mode (RGBA8 bytes) Image: the visible-layer composite, or
-- only `layer`'s cel. drawImage ignores the palette of indexed cels, so those are converted by hand.
-- shortcut: single-layer render ignores layer/cel opacity and blend mode, upgrade if asked.
local function render(sprite, frame, layer)
  local img = Image(sprite.width, sprite.height, ColorMode.RGB)
  if not layer then
    img:drawSprite(sprite, frame)
    return img
  end
  local cel = mcp.cel(layer, frame)
  if not cel then return img end
  local src = cel.image
  if sprite.colorMode == ColorMode.INDEXED then
    local palette, transparent = sprite.palettes[1], sprite.transparentColor
    local rgb = Image(src.width, src.height, ColorMode.RGB)
    for it in src:pixels() do
      local index = it()
      if index ~= transparent and index < #palette then
        local c = palette:getColor(index)
        rgb:putPixel(it.x, it.y, app.pixelColor.rgba(c.red, c.green, c.blue, c.alpha))
      end
    end
    src = rgb
  end
  img:drawImage(src, cel.position)
  return img
end

-- Read-only: no transaction, no view change, so no undo step. The pixels ride in `bytes`
-- (handle_message sends them as a binary frame); only width/height go in the JSON header.
local function get_image(args)
  local ok, res = pcall(function()
    local sprite = mcp.sprite(args.sprite)
    local frame = args.frame or (sprite == app.sprite and app.frame.frameNumber) or 1
    if math.type(frame) == "float" and frame == math.floor(frame) then frame = math.tointeger(frame) end
    if math.type(frame) ~= "integer" or frame < 1 or frame > #sprite.frames then
      error("frame out of range: " .. tostring(frame) .. " (sprite has " .. #sprite.frames .. ")", 0)
    end
    local layer = args.layer and mcp.layer(sprite, args.layer)
    return render(sprite, frame, layer)
  end)
  if not ok then return { ok = false, error = (tostring(res):gsub("^[^:]+:%d+: ", "")) } end
  return { ok = true, width = res.width, height = res.height, bytes = res.bytes }
end

local handlers = {
  run_lua = function(args) return run_chunk(args.code, "run_lua", true) end,
  query_lua = function(args) return run_chunk(args.code, "query_lua", false) end,
  status = status,
  get_image = get_image,
}

-- tool name + args table -> reply table (`ok`, `result` | `error`).
function M.execute(tool, args)
  local handler = handlers[tool]
  if not handler then return { ok = false, error = "unknown tool: " .. tostring(tool) } end
  return handler(args or {})
end

-- Bridge text frame in -> (text frame out) or (JSON header, raw bytes) for a binary frame out.
function M.handle_message(text)
  local okDecode, msg = pcall(json.decode, text)
  -- json.decode returns userdata objects, not plain tables
  if not okDecode or (type(msg) ~= "userdata" and type(msg) ~= "table") then
    return json.encode { ok = false, error = "bad request: " .. tostring(text) }
  end
  local okExec, reply = pcall(M.execute, msg.tool, msg.args)
  if not okExec then reply = { ok = false, error = tostring(reply) } end
  reply.id = msg.id
  -- Binary replies (get_image): the raw bytes travel after the JSON header, not inside it.
  local bytes = reply.bytes
  reply.bytes = nil
  local okEncode, out = pcall(json.encode, reply)
  if okEncode then return out, bytes end
  return json.encode { id = msg.id, ok = false, error = "result is not JSON-serialisable: " .. tostring(out) }
end

return M
