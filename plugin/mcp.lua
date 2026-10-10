-- `mcp` helper table injected into the Raw Lua execution env (see executor.lua), and the lookup
-- functions other tools (get_image) reuse: sprite/layer/cel resolve by name and raise on failure.
local mcp = {}

local function basename(path) return (path:gsub("\\", "/"):match("([^/]*)$")) end

-- Sprite by file name or full path; no argument = active sprite. Raises when none matches.
function mcp.sprite(name)
  if name == nil then
    return app.sprite or error("no active sprite", 2)
  end
  local want = name:gsub("\\", "/")
  for _, s in ipairs(app.sprites) do
    local f = s.filename:gsub("\\", "/")
    if f == want or basename(f) == want then return s end
  end
  error("sprite not found: " .. name, 2)
end

-- Layer by "group/child" path (names), starting at the sprite's top level. Raises when a segment is missing.
function mcp.layer(sprite, path)
  local layers = sprite.layers
  local found
  for part in path:gmatch("[^/]+") do
    found = nil
    for _, l in ipairs(layers or {}) do
      if l.name == part then found = l; break end
    end
    if not found then error("layer not found: " .. path, 2) end
    layers = found.isGroup and found.layers or nil
  end
  return found or error("layer not found: " .. path, 2)
end

-- Cel of `layer` at `frame` (number or Frame). Missing: nil, or a new empty cel when `create`.
function mcp.cel(layer, frame, create)
  if layer.isGroup then error("layer '" .. layer.name .. "' is a group and has no cels", 2) end
  local cel = layer:cel(frame)
  if cel == nil and create then cel = layer.sprite:newCel(layer, frame) end
  return cel
end

-- clone -> fn(image) -> assign back, so the change enters the undo history. If fn errors the cel is untouched.
function mcp.edit_image(cel, fn)
  local image = cel.image:clone()
  fn(image)
  cel.image = image
end

return mcp
