---
name: aseprite-lua
description: Write correct Aseprite Lua scripts (Sprite, Cel, Image, Layer, app.command, pixel colors, undo). Use before authoring or debugging any Lua that runs inside Aseprite.
---

Pure Aseprite scripting API. Verified against Aseprite 1.3.18.6-dev; do not rely on APIs newer than 1.3.17.

## Pitfalls

**Undo: clone, mutate, assign back.** `cel.image` is a live reference; mutating it in place (`cel.image:drawPixel(...)`) is not recorded in undo history and is not rolled back when the script errors. Recorded form:

```lua
local img = cel.image:clone()
img:drawPixel(x, y, color)
cel.image = img
```

**Cel position.** `cel.image` pixel (0,0) sits at `cel.position`, not at sprite (0,0). Convert: sprite coords = image coords + `cel.position`. `Sprite:newCel(layer, frame, image, Point(x, y))` sets the offset; `cel.bounds` gives the sprite-space rectangle.

**Pixel values follow the sprite's color mode.** `image:getPixel` / `drawPixel` use raw pixel values:
- RGB: `app.pixelColor.rgba(r,g,b,a)`; read with `rgbaR/G/B/A`.
- Grayscale: `app.pixelColor.graya(v,a)`; read with `grayaV/A`.
- Indexed: the palette index (integer, e.g. `1`); resolve colors via `sprite.palettes[1]:getColor(i)`.

A value built for one mode silently means something else in another. Read `sprite.colorMode` first; `Image(w, h, ColorMode.RGB)` is independent of the sprite's mode.

**`app.command.*` acts on the active context.** Set `app.sprite`, `app.layer` and `app.frame` to the targets first. Verified: with two layers, `app.command.Clear()` after `app.layer = layer1` cleared only layer 1. Creating a new `Sprite` makes it active; `app.sprite = nil` leaves no active layer. Command names are in `references/api/app_command.md` (`MaskAll`, not `SelectAll`).

**`app.sprites` order is not creation order.** Find sprites by property (`filename`, `width`, a held reference), never by index.

**`app.transaction` needs an active sprite.** With `app.sprite == nil` it throws a C++ error whose Lua value is useless (`function: 0x...`); guard with `if app.sprite then`. A transaction that changes nothing still adds an empty undo step, so keep read-only code outside transactions. Nested transactions merge into the outer one. Edits to a different sprite land in that sprite's own undo history.

**Rendering.** `img:drawSprite(sprite, frame)` composites visible layers correctly in every color mode. `img:drawImage(cel.image, cel.position)` does not apply the palette for indexed cels.

## Fallback

When this skill and `references/` do not answer the question, `https://github.com/aseprite/api` is the authority. Check the running Aseprite's version (`get_status`) against the snapshot commit in `references/COMMIT` and read the upstream file at the matching tag/commit when versions differ.

## References

`references/api/*.md` is a pinned snapshot of aseprite/api (one file per class, plus `command/` parameters). Read the file named after the class you use (`cel.md`, `image.md`, `sprite.md`, `app.md`, ...) before calling an unfamiliar method. Refresh with `scripts/fetch-aseprite-api.sh`.
