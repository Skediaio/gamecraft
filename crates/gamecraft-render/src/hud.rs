//! A line of text in the bottom right corner of a live frame (the fire mode and the rounds
//! left), drawn as lines over everything like the crosshair: a small stroke font of its own
//! (capitals, digits, `/`, `-` and `:`; any other character is a space), made for this crate.

use crate::Camera;
use crate::effects::Segment;
use gamecraft_math::{DVec3, deg_to_rad};

/// How tall a glyph is, physical pixels.
pub const HUD_GLYPH_PX: f64 = 16.0;
/// The gap from the frame's right and bottom edges to the text, physical pixels.
pub const HUD_MARGIN_PX: f64 = 28.0;
/// Most characters of a line; the rest is left out.
pub const HUD_MAX_CHARS: usize = 64;
/// How far in front of the eye the text is placed, metres (as the crosshair).
const HUD_DEPTH_M: f64 = 1.0;
/// A glyph's grid: 4 units wide, 6 tall; the advance from one glyph to the next.
const GLYPH_UNITS_HIGH: f64 = 6.0;
const GLYPH_ADVANCE_UNITS: f64 = 6.0;

/// The strokes of a glyph on its 4 x 6 grid (x right, y up), as polylines.
fn strokes(c: char) -> &'static [&'static [(u8, u8)]] {
    const BOX: &[(u8, u8)] = &[(0, 0), (4, 0), (4, 6), (0, 6), (0, 0)];
    match c {
        '0' | 'O' => &[BOX],
        '1' => &[&[(1, 5), (2, 6), (2, 0)], &[(1, 0), (3, 0)]],
        '2' => &[&[(0, 6), (4, 6), (4, 3), (0, 3), (0, 0), (4, 0)]],
        '3' => &[&[(0, 6), (4, 6), (4, 0), (0, 0)], &[(1, 3), (4, 3)]],
        '4' => &[&[(0, 6), (0, 3), (4, 3)], &[(4, 6), (4, 0)]],
        '5' | 'S' => &[&[(4, 6), (0, 6), (0, 3), (4, 3), (4, 0), (0, 0)]],
        '6' => &[&[(4, 6), (0, 6), (0, 0), (4, 0), (4, 3), (0, 3)]],
        '7' => &[&[(0, 6), (4, 6), (1, 0)]],
        '8' => &[BOX, &[(0, 3), (4, 3)]],
        '9' => &[&[(4, 3), (0, 3), (0, 6), (4, 6), (4, 0), (0, 0)]],
        '/' => &[&[(0, 0), (4, 6)]],
        '-' => &[&[(1, 3), (3, 3)]],
        ':' => &[&[(2, 1), (2, 2)], &[(2, 4), (2, 5)]],
        'A' => &[&[(0, 0), (0, 4), (2, 6), (4, 4), (4, 0)], &[(0, 3), (4, 3)]],
        'B' => &[
            &[(0, 0), (0, 6), (3, 6), (4, 5), (4, 4), (3, 3), (0, 3)],
            &[(3, 3), (4, 2), (4, 1), (3, 0), (0, 0)],
        ],
        'C' => &[&[(4, 6), (0, 6), (0, 0), (4, 0)]],
        'D' => &[&[(0, 0), (0, 6), (2, 6), (4, 4), (4, 2), (2, 0), (0, 0)]],
        'E' => &[&[(4, 6), (0, 6), (0, 0), (4, 0)], &[(0, 3), (3, 3)]],
        'F' => &[&[(4, 6), (0, 6), (0, 0)], &[(0, 3), (3, 3)]],
        'G' => &[&[(4, 6), (0, 6), (0, 0), (4, 0), (4, 3), (2, 3)]],
        'H' => &[&[(0, 0), (0, 6)], &[(4, 0), (4, 6)], &[(0, 3), (4, 3)]],
        'I' => &[&[(2, 0), (2, 6)], &[(1, 0), (3, 0)], &[(1, 6), (3, 6)]],
        'K' => &[&[(0, 0), (0, 6)], &[(4, 6), (0, 3), (4, 0)]],
        'L' => &[&[(0, 6), (0, 0), (4, 0)]],
        'M' => &[&[(0, 0), (0, 6), (2, 3), (4, 6), (4, 0)]],
        'N' => &[&[(0, 0), (0, 6), (4, 0), (4, 6)]],
        'P' => &[&[(0, 0), (0, 6), (4, 6), (4, 3), (0, 3)]],
        'R' => &[&[(0, 0), (0, 6), (4, 6), (4, 3), (0, 3)], &[(1, 3), (4, 0)]],
        'T' => &[&[(0, 6), (4, 6)], &[(2, 6), (2, 0)]],
        'U' => &[&[(0, 6), (0, 0), (4, 0), (4, 6)]],
        'V' => &[&[(0, 6), (2, 0), (4, 6)]],
        'W' => &[&[(0, 6), (0, 0), (2, 3), (4, 0), (4, 6)]],
        'X' => &[&[(0, 0), (4, 6)], &[(0, 6), (4, 0)]],
        'Y' => &[&[(0, 6), (2, 3), (4, 6)], &[(2, 3), (2, 0)]],
        _ => &[],
    }
}

/// The text's strokes in pixels from the frame's centre (x right, y up), for a frame
/// `width_px` x `height_px`: glyphs [`HUD_GLYPH_PX`] tall, the line ending [`HUD_MARGIN_PX`]
/// from the right and bottom edges. Lower case is drawn as capitals.
pub fn text_strokes(text: &str, width_px: f64, height_px: f64) -> Vec<([f64; 2], [f64; 2])> {
    let chars: Vec<char> = text
        .chars()
        .take(HUD_MAX_CHARS)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let unit = HUD_GLYPH_PX / GLYPH_UNITS_HIGH;
    let advance = GLYPH_ADVANCE_UNITS * unit;
    // The last glyph is 4 units wide, not a full advance.
    let width = (chars.len() as f64 - 1.0).max(0.0) * advance + 4.0 * unit;
    let left = width_px / 2.0 - HUD_MARGIN_PX - width;
    let bottom = -height_px / 2.0 + HUD_MARGIN_PX;
    let mut out = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        let x0 = left + i as f64 * advance;
        for line in strokes(*c) {
            for pair in line.windows(2) {
                if let [(ax, ay), (bx, by)] = pair {
                    let at =
                        |gx: u8, gy: u8| [x0 + f64::from(gx) * unit, bottom + f64::from(gy) * unit];
                    out.push((at(*ax, *ay), at(*bx, *by)));
                }
            }
        }
    }
    out
}

/// The text as world segments one metre in front of the eye of `camera`, for a frame
/// `width_px` x `height_px`: dark outlines first, the bright strokes after. Empty for an empty
/// text, a degenerate camera or an empty frame.
pub fn hud_segments(camera: &Camera, width_px: u32, height_px: u32, text: &str) -> Vec<Segment> {
    let eye = DVec3::from_array(camera.position);
    let forward = (DVec3::from_array(camera.target) - eye).normalize_or_zero();
    if forward == DVec3::ZERO || width_px == 0 || height_px == 0 || text.is_empty() {
        return Vec::new();
    }
    let up_hint = if forward.y.abs() > 0.999 {
        DVec3::NEG_Z
    } else {
        DVec3::Y
    };
    let right = forward.cross(up_hint).normalize_or_zero();
    let up = right.cross(forward);
    let metres_per_px =
        2.0 * HUD_DEPTH_M * deg_to_rad(camera.fov_y_deg / 2.0).tan() / f64::from(height_px);
    if !metres_per_px.is_finite() {
        return Vec::new();
    }
    let centre = eye + forward * HUD_DEPTH_M;
    let place = |[x, y]: [f64; 2]| (centre + (right * x + up * y) * metres_per_px).to_array();
    let strokes = text_strokes(text, f64::from(width_px), f64::from(height_px));
    let outline = strokes.iter().map(|&(a, b)| Segment {
        from: place(a),
        to: place(b),
        rgba: [0, 0, 0, 170],
        width_px: 4.0,
    });
    let bright = strokes.iter().map(|&(a, b)| Segment {
        from: place(a),
        to: place(b),
        rgba: [255, 255, 255, 235],
        width_px: 2.0,
    });
    outline.chain(bright).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn the_line_ends_at_the_bottom_right_margin() {
        let strokes = text_strokes("AUTO 27/30", 1280.0, 720.0);
        assert!(!strokes.is_empty());
        let xs = strokes.iter().flat_map(|(a, b)| [a[0], b[0]]);
        let ys = strokes.iter().flat_map(|(a, b)| [a[1], b[1]]);
        let right = xs.fold(f64::NEG_INFINITY, f64::max);
        let bottom = ys.fold(f64::INFINITY, f64::min);
        let top = strokes
            .iter()
            .flat_map(|(a, b)| [a[1], b[1]])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!((right - (640.0 - HUD_MARGIN_PX)).abs() < 1e-9, "{right}");
        assert!((bottom - (-360.0 + HUD_MARGIN_PX)).abs() < 1e-9, "{bottom}");
        assert!((top - bottom - HUD_GLYPH_PX).abs() < 1e-9);
        // Every character the window shows has strokes; a space has none.
        for c in "SAFE SEMI AUTO BURST RELOAD 0123456789/".chars() {
            assert_eq!(super::strokes(c).is_empty(), c == ' ', "{c}");
        }
        assert!(text_strokes("", 100.0, 100.0).is_empty());
    }

    #[test]
    fn the_segments_sit_in_front_of_the_eye_and_project_to_the_corner() {
        let camera = Camera {
            position: [0.0, 1.6, 0.0],
            target: [0.0, 1.6, -10.0],
            fov_y_deg: 70.0,
        };
        let segments = hud_segments(&camera, 1280, 720, "SEMI 30/30");
        assert!(!segments.is_empty());
        for s in &segments {
            let ndc = camera.project(s.to, 1280.0 / 720.0).unwrap();
            assert!(ndc[0] > 0.5 && ndc[1] < -0.8, "{ndc:?}");
        }
        assert!(hud_segments(&camera, 0, 720, "SEMI").is_empty());
        assert!(hud_segments(&camera, 1280, 720, "").is_empty());
    }
}
