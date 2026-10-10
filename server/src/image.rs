//! get_image pixel pipeline: RGBA8 from the Plugin -> integer nearest-neighbor upscale -> PNG.

/// Long side the automatic scale aims for, so pixel art stays recognisable to a vision model.
const TARGET_LONG_SIDE: u32 = 256;
/// Upper bound for either side of the output, to keep a bad `scale` from exhausting memory.
const MAX_SIDE: u64 = 16384;

/// Smallest integer scale that makes the long side at least `TARGET_LONG_SIDE` (1 when it already is).
pub fn default_scale(width: u32, height: u32) -> u32 {
    TARGET_LONG_SIDE.div_ceil(width.max(height).max(1))
}

/// Repeats every pixel `scale` x `scale` times. `rgba` must be `width * height * 4` bytes.
pub fn upscale(rgba: &[u8], width: u32, height: u32, scale: u32) -> Vec<u8> {
    let (w, s) = (width as usize, scale as usize);
    let mut out = Vec::with_capacity(rgba.len() * s * s);
    for row in rgba.chunks_exact(w * 4) {
        let start = out.len();
        for px in row.chunks_exact(4) {
            for _ in 0..s {
                out.extend_from_slice(px);
            }
        }
        let scaled_row = start..out.len();
        for _ in 1..s {
            out.extend_from_within(scaled_row.clone());
        }
    }
    debug_assert_eq!(out.len(), (height as usize) * s * s * w * 4);
    out
}

pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .map_err(|e| format!("PNG encoding failed: {e}"))?;
    Ok(out)
}

/// Validates the Plugin's pixels and the requested scale, upscales and encodes. Returns the PNG bytes.
pub fn render_png(rgba: &[u8], width: u32, height: u32, scale: Option<u32>) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || rgba.len() as u64 != width as u64 * height as u64 * 4 {
        return Err(format!("bad image from plugin: {width}x{height} with {} bytes", rgba.len()));
    }
    let scale = scale.unwrap_or_else(|| default_scale(width, height));
    if scale == 0 {
        return Err("scale must be at least 1".to_owned());
    }
    if width as u64 * scale as u64 > MAX_SIDE || height as u64 * scale as u64 > MAX_SIDE {
        return Err(format!("scale {scale} makes the image larger than {MAX_SIDE}px on a side"));
    }
    encode_png(&upscale(rgba, width, height, scale), width * scale, height * scale)
}
