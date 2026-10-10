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

for _, t in ipairs(tests) do
  local ok, err = pcall(t.fn)
  print((ok and "PASS " or "FAIL ") .. t.name .. (ok and "" or (": " .. tostring(err))))
  if not ok then error("test failed", 0) end -- script error => non-zero exit for the runner
  while app.sprite do app.sprite:close() end
end
print("ALL PASS")
