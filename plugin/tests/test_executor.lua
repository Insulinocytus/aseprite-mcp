-- Plugin seam: run via scripts/test-plugin.ps1 (aseprite --batch --script, path from $env:ASEPRITE).
-- Drives the executor directly, no WebSocket. Any failed assert aborts the script with an error.
local executor = dofile(app.params.executor)

local function new_sprite()
  local s = Sprite(16, 16)
  assert(app.sprite == s)
  return s
end

local function layers(s) return #s.layers end

local function run(code) return executor.execute("run_lua", { code = code }) end

local tests = {}
local function test(name, fn) tests[#tests + 1] = { name = name, fn = fn } end

test("success is exactly one undo step", function()
  local s = new_sprite()
  app.transaction("baseline", function() s:newLayer() end) -- layers: 2
  local r = run("local s = app.sprite; s:newLayer(); s:newLayer(); return 7")
  assert(r.ok and r.result == 7, tostring(r.error))
  assert(layers(s) == 4)
  app.undo()
  assert(layers(s) == 2, "one undo must revert the whole call, got " .. layers(s))
  app.undo()
  assert(layers(s) == 1, "next undo must revert the baseline step, got " .. layers(s))
  s:close()
end)

test("error rolls back and leaves no undo step; message has traceback", function()
  local s = new_sprite()
  app.transaction("baseline", function() s:newLayer() end) -- layers: 2
  local r = run("app.sprite:newLayer(); error('boom')")
  assert(not r.ok)
  assert(r.error:find("boom", 1, true) and r.error:find("stack traceback", 1, true), r.error)
  assert(layers(s) == 2, "failed call must roll back")
  app.undo()
  assert(layers(s) == 1, "failed call must not leave an undo step")
  s:close()
end)

test("syntax error is reported", function()
  local s = new_sprite()
  local r = run("return (")
  assert(not r.ok and r.error:find("run_lua", 1, true), tostring(r.error))
  s:close()
end)

test("without a sprite the call runs outside a transaction", function()
  assert(app.sprite == nil)
  local r = run("return 5")
  assert(r.ok and r.result == 5, tostring(r.error))
  r = run("error('no sprite boom')")
  assert(not r.ok and r.error:find("no sprite boom", 1, true), tostring(r.error))
  -- creating the first sprite inside a call must work too
  r = run("local s = Sprite(4, 4); local n = #s.layers; s:close(); return n")
  assert(r.ok and r.result == 1, tostring(r.error))
end)

test("nested transactions merge into the call's step", function()
  local s = new_sprite()
  app.transaction("baseline", function() s:newLayer() end) -- layers: 2
  local r = run([[
    local s = app.sprite
    app.transaction("inner 1", function() s:newLayer() end)
    app.transaction("inner 2", function() s:newLayer() end)
  ]])
  assert(r.ok, tostring(r.error))
  assert(layers(s) == 4)
  app.undo()
  assert(layers(s) == 2, "one undo must revert both nested transactions, got " .. layers(s))
  s:close()
end)

test("return values become JSON: arrays, objects, userdata, nil", function()
  local s = new_sprite()
  local r = run("return { 1, 2, 3 }")
  assert(r.ok and #r.result == 3 and r.result[3] == 3)
  r = run("return { a = 1, b = { true } }")
  assert(r.ok and r.result.a == 1 and r.result.b[1] == true)
  r = run("return app.sprite.width")
  assert(r.ok and r.result == 16)
  r = run("return app.sprite")
  assert(r.ok and type(r.result) == "string", "userdata becomes a string")
  r = run("return nil")
  assert(r.ok and r.result == nil)
  s:close()
end)

test("handle_message round-trips an id and reports bad input", function()
  local s = new_sprite()
  local raw = executor.handle_message(json.encode { id = 9, tool = "run_lua", args = { code = "return 'hi'" } })
  local out = json.decode(raw)
  assert(out.id == 9 and out.ok == true and out.result == "hi", raw)
  out = json.decode(executor.handle_message(json.encode { id = 10, tool = "nope", args = {} }))
  assert(out.id == 10 and out.ok == false)
  out = json.decode(executor.handle_message("not json"))
  assert(out.ok == false)
  s:close()
end)

test("mcp.sprite: active by default, by file name or path, error when missing", function()
  local a = new_sprite(); a.filename = "C:/tmp/alpha.aseprite"
  local b = new_sprite(); b.filename = "C:/tmp/beta.aseprite"
  -- Aseprite may normalise filename separators, so compare Sprite identity via a tag in the sprite's data.
  a.data, b.data = "A", "B"
  local r = run("return mcp.sprite().data")
  assert(r.ok and r.result == "B", tostring(r.error))
  r = run("return mcp.sprite('alpha.aseprite').data")
  assert(r.ok and r.result == "A", tostring(r.error))
  r = run("return mcp.sprite('C:/tmp/alpha.aseprite').data")
  assert(r.ok and r.result == "A", tostring(r.error))
  r = run("return mcp.sprite('C:\\\\tmp\\\\beta.aseprite').data")
  assert(r.ok and r.result == "B", tostring(r.error))
  r = run("return mcp.sprite('nope.aseprite')")
  assert(not r.ok and r.error:find("sprite not found: nope.aseprite", 1, true), tostring(r.error))
  b:close(); a:close()
  r = run("return mcp.sprite()")
  assert(not r.ok and r.error:find("no active sprite", 1, true), tostring(r.error))
end)

test("mcp.layer: top-level, nested group path, not-found errors", function()
  local s = new_sprite()
  local r = run([[
    local s = app.sprite
    local g = s:newGroup(); g.name = "grp"
    local inner = s:newGroup(); inner.name = "inner"; inner.parent = g
    local leaf = s:newLayer(); leaf.name = "leaf"; leaf.parent = inner
    return { mcp.layer(s, "grp/inner/leaf").name, mcp.layer(s, "grp").name, mcp.layer(s, "grp/inner").isGroup }
  ]])
  assert(r.ok, tostring(r.error))
  assert(r.result[1] == "leaf" and r.result[2] == "grp" and r.result[3] == true)
  r = run("return mcp.layer(app.sprite, 'grp/missing')")
  assert(not r.ok and r.error:find("layer not found: grp/missing", 1, true), tostring(r.error))
  r = run("return mcp.layer(app.sprite, 'grp/inner/leaf/deeper')")
  assert(not r.ok and r.error:find("layer not found", 1, true), tostring(r.error))
  s:close()
end)

test("mcp.cel: existing, nil when absent, created on demand, bad layer errors", function()
  local s = new_sprite()
  local r = run([[
    local s = app.sprite
    s:newEmptyFrame()
    local l = mcp.layer(s, s.layers[1].name)
    local existing = mcp.cel(l, 1)
    local absent = mcp.cel(l, 2)
    local made = mcp.cel(l, 2, true)
    return { existing ~= nil, absent == nil, made ~= nil and made.frameNumber == 2 }
  ]])
  assert(r.ok, tostring(r.error))
  assert(r.result[1] and r.result[2] and r.result[3], json.encode(r.result))
  r = run("local g = app.sprite:newGroup(); return mcp.cel(g, 1)")
  assert(not r.ok and r.error:find("group", 1, true), tostring(r.error))
  s:close()
end)

test("mcp.edit_image: edits pixels, one undo step, keeps position", function()
  local s = new_sprite()
  local r = run([[
    local cel = mcp.cel(app.sprite.layers[1], 1, true)
    mcp.edit_image(cel, function(img) img:putPixel(2, 3, app.pixelColor.rgba(255, 0, 0, 255)) end)
    return { cel.image:getPixel(2, 3) == app.pixelColor.rgba(255, 0, 0, 255), cel.position.x, cel.position.y }
  ]])
  assert(r.ok and r.result[1] == true, tostring(r.error) .. json.encode(r.result))
  local px = s.cels[1].image:getPixel(2, 3)
  assert(px == app.pixelColor.rgba(255, 0, 0, 255), "pixel not written")
  app.undo()
  local cel = s.cels[1]
  assert(cel ~= nil and cel.image:getPixel(2, 3) == 0, "undo must revert the edit, not drop the cel")
  s:close()
end)

test("mcp.edit_image: callback error leaves cel untouched and rolls the call back", function()
  local s = new_sprite()
  app.transaction("baseline", function() s:newLayer() end)
  local r = run([[
    local cel = mcp.cel(app.sprite.layers[1], 1, true)
    mcp.edit_image(cel, function(img) img:putPixel(0, 0, 1); error("bad edit") end)
  ]])
  assert(not r.ok and r.error:find("bad edit", 1, true), tostring(r.error))
  local cel = s.layers[1]:cel(1)
  assert(cel ~= nil and cel.image:getPixel(0, 0) == 0, "failed edit must not touch the cel")
  assert(#s.layers == 2)
  app.undo()
  assert(#s.layers == 1, "failed call must not leave an undo step")
  s:close()
end)

for _, t in ipairs(tests) do
  local ok, err = pcall(t.fn)
  print((ok and "PASS " or "FAIL ") .. t.name .. (ok and "" or (": " .. tostring(err))))
  if not ok then error("test failed", 0) end -- script error => non-zero exit for the runner
  while app.sprite do app.sprite:close() end
end
print("ALL PASS")
