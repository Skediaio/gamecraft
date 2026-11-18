//! PNG encoding of pictures.

use crate::RenderError;

/// Encodes sRGB RGBA8 pixels (rows top to bottom) as a PNG file.
///
/// A buffer whose length is not `width * height * 4`, or a zero side, is a
/// [`RenderError::BadRequest`].
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, RenderError> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    if width == 0 || height == 0 || expected != Some(rgba.len()) {
        return Err(RenderError::BadRequest(format!(
            "the RGBA buffer holds {} bytes, but a {width} x {height} picture needs {} \
             (4 bytes per pixel, both sides at least 1)",
            rgba.len(),
            expected.map_or_else(|| "more than fits in memory".to_string(), |n| n.to_string())
        )));
    }
    let failed = |e: png::EncodingError| RenderError::Failed(format!("PNG encoding failed: {e}"));
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(failed)?;
        writer.write_image_data(rgba).map_err(failed)?;
        writer.finish().map_err(failed)?;
    }
    Ok(out)
}
