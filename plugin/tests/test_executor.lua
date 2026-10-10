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

local function query(code) return executor.execute("query_lua", { code = code }) end

test("query_lua adds no undo step, even when it errors", function()
  local s = new_sprite()
  app.transaction("baseline", function() s:newLayer() end) -- layers: 2
  local r = query("return #app.sprite.layers")
  assert(r.ok and r.result == 2, tostring(r.error))
  r = query("error('read boom')")
  assert(not r.ok and r.error:find("read boom", 1, true), tostring(r.error))
  app.undo()
  assert(layers(s) == 1, "the first undo must revert the baseline, so query_lua left no step")
  s:close()
end)

test("print is captured line by line and restored", function()
  local original = print
  local r = run("print('a'); print('b', 1); print(); return 1")
  assert(r.ok and r.result == 1, tostring(r.error))
  assert(#r.output == 3 and r.output[1] == "a" and r.output[2] == "b\t1" and r.output[3] == "", #r.output)
  assert(print == original, "print must be restored")
  r = query("print('q'); return 2")
  assert(r.ok and #r.output == 1 and r.output[1] == "q")
  assert(print == original)
  r = query("return 3")
  assert(r.ok and #r.output == 0)
end)

test("print is restored and output kept when the code errors", function()
  local original = print
  local s = new_sprite()
  local r = run("print('before'); error('late')")
  assert(not r.ok and r.output[1] == "before", tostring(r.error))
  assert(print == original)
  r = query("print('x'); return (")
  assert(not r.ok and print == original)
  s:close()
end)

test("view is restored after the call", function()
  local s1 = new_sprite()
  local s2 = Sprite(8, 8)
  s2:newLayer()
  s2:newFrame()
  local l1, f1 = s2.layers[1], s2.frames[1]
  app.sprite, app.layer, app.frame = s2, l1, f1
  local r = run("app.sprite = Sprite(4, 4); return #app.sprite.layers")
  assert(r.ok, tostring(r.error))
  assert(app.sprite == s2 and app.layer == l1 and app.frame == f1, "view must be restored")
  r = query("app.sprite = nil; return 1") -- also on query_lua
  assert(app.sprite == s2 and app.layer == l1 and app.frame == f1)
  -- a recorded sprite that the code closed is not restored (and must not error)
  r = run("app.sprite:close(); return 1")
  assert(r.ok, tostring(r.error))
  assert(app.sprite ~= s2)
  s1:close()
end)

test("serialization: nested, mixed and empty tables, userdata, nil", function()
  local r = query("return { { 1, 2 }, { x = 'y' } }")
  assert(r.ok and r.result[1][2] == 2 and r.result[2].x == "y")
  r = query("return { 1, 2, k = 3 }")
  assert(r.ok and r.result.k == 3 and r.result["1"] == 1, "non-sequence tables become objects")
  r = query("return {}")
  assert(r.ok and type(r.result) ~= "string")
  r = query("return Rectangle(1, 2, 3, 4)")
  assert(r.ok and type(r.result) == "string")
  r = query("return 1, 2")
  assert(r.ok and r.result == 1, "first return value only")
  local out = json.decode(executor.handle_message(json.encode { id = 1, tool = "query_lua", args = { code = "return nil" } }))
  assert(out.ok == true and out.result == nil and out.id == 1)
end)

test("status lists sprites and adds no undo step", function()
  local a = new_sprite()
  a.filename = "first.aseprite"
  local b = Sprite(8, 4, ColorMode.INDEXED)
  app.transaction("baseline", function() b:newLayer() end) -- b is modified, layers: 2
  local r = executor.execute("status", {})
  assert(r.ok, tostring(r.error))
  assert(type(r.result.aseprite_version) == "string" and #r.result.aseprite_version > 0)
  assert(#r.result.sprites == 2, #r.result.sprites)
  local by = {}
  for _, e in ipairs(r.result.sprites) do by[e.filename:match("[^/\\]+$")] = e end
  local ea = by["first.aseprite"]
  assert(ea and ea.width == 16 and ea.height == 16 and ea.color_mode == "rgb" and ea.active == false and ea.modified == false)
  local eb
  for _, e in ipairs(r.result.sprites) do if e ~= ea then eb = e end end
  assert(eb.width == 8 and eb.height == 4 and eb.color_mode == "indexed" and eb.active == true and eb.modified == true)
  app.undo()
  assert(layers(b) == 1, "status must not add an undo step; undo must revert the baseline, got " .. layers(b))
end)

-- RGBA bytes of pixel (x, y) from a get_image reply, as "r,g,b,a".
local function px(r, x, y)
  local i = (y * r.width + x) * 4 + 1
  return table.concat({ r.bytes:byte(i, i + 3) }, ",")
end

local function get_image(args) return executor.execute("get_image", args or {}) end

test("get_image: rgb composite excludes hidden layers; a given layer renders alone, even hidden", function()
  local s = Sprite(4, 2, ColorMode.RGB)
  s.layers[1].name = "bottom"
  s.cels[1].image:putPixel(0, 0, app.pixelColor.rgba(255, 0, 0, 255))
  local top = s:newLayer()
  top.name = "top"
  local img = Image(2, 1, ColorMode.RGB)
  img:putPixel(0, 0, app.pixelColor.rgba(0, 255, 0, 128))
  s:newCel(top, 1, img, Point(2, 1))
  local r = get_image()
  assert(r.ok, tostring(r.error))
  assert(r.width == 4 and r.height == 2 and #r.bytes == 32)
  assert(px(r, 0, 0) == "255,0,0,255" and px(r, 2, 1) == "0,255,0,128" and px(r, 1, 0) == "0,0,0,0", px(r, 2, 1))
  top.isVisible = false
  r = get_image()
  assert(px(r, 2, 1) == "0,0,0,0" and px(r, 0, 0) == "255,0,0,255", "hidden layer must not composite")
  r = get_image { layer = "top" }
  assert(px(r, 2, 1) == "0,255,0,128" and px(r, 0, 0) == "0,0,0,0", "layer alone, at its position")
  r = get_image { layer = "bottom" }
  assert(px(r, 0, 0) == "255,0,0,255" and px(r, 2, 1) == "0,0,0,0")
end)

test("get_image: grayscale", function()
  local s = Sprite(2, 1, ColorMode.GRAY)
  s.cels[1].image:putPixel(0, 0, app.pixelColor.graya(100, 255))
  for _, args in ipairs { {}, { layer = s.layers[1].name } } do
    local r = get_image(args)
    assert(r.ok, tostring(r.error))
    assert(px(r, 0, 0) == "100,100,100,255" and px(r, 1, 0) == "0,0,0,0", px(r, 0, 0))
  end
end)

test("get_image: indexed applies the palette, transparent index is empty (composite and layer)", function()
  local s = Sprite(2, 1, ColorMode.INDEXED)
  s.palettes[1]:resize(3)
  s.palettes[1]:setColor(1, Color { r = 10, g = 20, b = 30, a = 255 })
  s.palettes[1]:setColor(2, Color { r = 200, g = 100, b = 50, a = 255 })
  s.cels[1].image:putPixel(0, 0, 1)
  s.cels[1].image:putPixel(1, 0, 2)
  for _, args in ipairs { {}, { layer = s.layers[1].name } } do
    local r = get_image(args)
    assert(r.ok, tostring(r.error))
    assert(px(r, 0, 0) == "10,20,30,255" and px(r, 1, 0) == "200,100,50,255", px(r, 0, 0))
  end
  s.cels[1].image:putPixel(0, 0, 0)
  assert(px(get_image { layer = s.layers[1].name }, 0, 0) == "0,0,0,0")
end)

test("get_image: frame and sprite selection, errors", function()
  local a = Sprite(2, 1, ColorMode.RGB)
  a.filename = "pic.aseprite"
  a.cels[1].image:putPixel(0, 0, app.pixelColor.rgba(1, 2, 3, 255))
  local f2 = a:newEmptyFrame()
  a:newCel(a.layers[1], f2, Image(2, 1, ColorMode.RGB), Point(0, 0))
  a.cels[2].image:putPixel(1, 0, app.pixelColor.rgba(4, 5, 6, 255))
  local b = Sprite(3, 3, ColorMode.RGB) -- active
  local r = get_image { sprite = "pic.aseprite", frame = 2 }
  assert(r.ok and r.width == 2 and px(r, 1, 0) == "4,5,6,255" and px(r, 0, 0) == "0,0,0,0", tostring(r.error))
  assert(px(get_image { sprite = "pic.aseprite", frame = 1 }, 0, 0) == "1,2,3,255")
  assert(get_image().width == 3, "default is the active sprite")
  app.sprite = a
  app.frame = a.frames[2]
  assert(px(get_image(), 1, 0) == "4,5,6,255", "default frame is the current one")
  r = get_image { frame = 3 }
  assert(not r.ok and r.error:find("frame out of range", 1, true), tostring(r.error))
  r = get_image { layer = "nope" }
  assert(not r.ok and r.error:find("layer not found: nope", 1, true), tostring(r.error))
  r = get_image { sprite = "nope.aseprite" }
  assert(not r.ok and r.error:find("sprite not found", 1, true), tostring(r.error))
end)

test("get_image: no undo step; handle_message gives a compact header and the raw bytes", function()
  local s = new_sprite()
  app.transaction("baseline", function() s:newLayer() end) -- layers: 2
  local header, bytes = executor.handle_message(json.encode { id = 5, tool = "get_image", args = {} })
  local h = json.decode(header)
  assert(h.id == 5 and h.ok == true and h.width == 16 and h.height == 16 and h.bytes == nil, header)
  assert(not header:find("\n", 1, true) and #bytes == 16 * 16 * 4)
  app.undo()
  assert(layers(s) == 1, "get_image must not add an undo step, got " .. layers(s))
  header, bytes = executor.handle_message(json.encode { id = 6, tool = "get_image", args = { layer = "nope" } })
  assert(bytes == nil and json.decode(header).ok == false)
end)

for _, t in ipairs(tests) do
  local ok, err = pcall(t.fn)
  print((ok and "PASS " or "FAIL ") .. t.name .. (ok and "" or (": " .. tostring(err))))
  if not ok then error("test failed", 0) end -- script error => non-zero exit for the runner
  for i = #app.sprites, 1, -1 do app.sprites[i]:close() end
end
print("ALL PASS")
