//! The heads-up display of a live frame: stacked lines of text in the bottom right corner (the
//! weapon), lines in the middle (a death screen), a tint over the whole frame and a vignette at
//! its edges, all drawn as lines over everything like the crosshair. The text is a small
//! stroke font of its own (capitals, digits and a little punctuation; any other character is a
//! space), made for this crate. The layout is pure: [`layout`] turns a [`Hud`] and a frame size
//! into pixel segments.

use crate::Camera;
use crate::effects::Segment;
use gamecraft_math::{DVec3, deg_to_rad};

/// How tall a glyph of the usual size is, physical pixels.
pub const HUD_GLYPH_PX: f64 = 16.0;
/// The gap between two stacked lines, in glyph heights.
pub const HUD_LINE_GAP: f64 = 0.7;
/// The widest a centred line is, as a share of the frame's width; a larger one shrinks to fit.
pub const HUD_CENTRE_WIDTH_SHARE: f64 = 0.9;
/// How deep the vignette reaches into the frame, as a share of its height.
pub const HUD_VIGNETTE_DEPTH: f64 = 0.3;
/// The bands a vignette is built of: the more, the smoother its fall-off.
const VIGNETTE_BANDS: u32 = 24;
/// The translucent dark bar behind each corner line, so it reads on bright grass and on sky.
const BACKING_RGBA: [u8; 4] = [0, 0, 0, 105];
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
        '.' => &[&[(2, 0), (2, 1)]],
        '\'' => &[&[(2, 4), (2, 6)]],
        'J' => &[&[(0, 1), (1, 0), (3, 0), (4, 1), (4, 6)]],
        'Q' => &[BOX, &[(2, 2), (4, 0)]],
        'Z' => &[&[(0, 6), (4, 6), (0, 0), (4, 0)]],
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

/// One line of text: its characters, how tall they are and their colour.
#[derive(Clone, Debug, PartialEq)]
pub struct HudLine {
    /// The characters; lower case is drawn as capitals.
    pub text: String,
    /// How tall a glyph is, physical pixels.
    pub glyph_px: f64,
    /// Colour of the strokes, sRGB with alpha.
    pub rgba: [u8; 4],
}

impl HudLine {
    /// A line of `text` with glyphs `glyph_px` tall.
    pub fn new(text: impl Into<String>, glyph_px: f64, rgba: [u8; 4]) -> HudLine {
        HudLine {
            text: text.into(),
            glyph_px,
            rgba,
        }
    }
}

/// What a live frame shows over the world, besides the crosshair.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hud {
    /// Lines in the bottom right corner, the first on top, each right aligned on a backing.
    pub corner: Vec<HudLine>,
    /// Lines in the middle of the frame, the first on top, each centred.
    pub centre: Vec<HudLine>,
    /// A colour laid over the whole frame (its alpha is how much), under the text.
    pub veil: Option<[u8; 4]>,
    /// A colour that darkens the edges of the frame (its alpha is how much at the very edge).
    pub vignette: Option<[u8; 4]>,
}

impl Hud {
    /// Whether it shows nothing.
    pub fn is_empty(&self) -> bool {
        self.corner.is_empty()
            && self.centre.is_empty()
            && self.veil.is_none()
            && self.vignette.is_none()
    }
}

/// A segment in pixels from the frame's centre (x right, y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PxSegment {
    /// One end.
    pub from: [f64; 2],
    /// The other end.
    pub to: [f64; 2],
    /// Colour, sRGB with alpha.
    pub rgba: [u8; 4],
    /// Thickness, physical pixels.
    pub width_px: f64,
}

/// The characters of a line that are drawn: at most [`HUD_MAX_CHARS`], as capitals.
fn drawn(text: &str) -> Vec<char> {
    text.chars()
        .take(HUD_MAX_CHARS)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// How wide a line of `count` characters is at `glyph_px`: the last glyph is 4 units wide, not
/// a full advance.
fn line_width(count: usize, glyph_px: f64) -> f64 {
    let unit = glyph_px / GLYPH_UNITS_HIGH;
    (count as f64 - 1.0).max(0.0) * GLYPH_ADVANCE_UNITS * unit + 4.0 * unit
}

/// The strokes of `text` with its left edge at `left` and its glyphs' bottom at `bottom`,
/// each as (from, to) in pixels from the frame's centre.
pub fn text_strokes_at(
    text: &str,
    glyph_px: f64,
    left: f64,
    bottom: f64,
) -> Vec<([f64; 2], [f64; 2])> {
    let unit = glyph_px / GLYPH_UNITS_HIGH;
    let advance = GLYPH_ADVANCE_UNITS * unit;
    let mut out = Vec::new();
    for (i, c) in drawn(text).iter().enumerate() {
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

/// The strokes of a line of text ending [`HUD_MARGIN_PX`] from the right and bottom edges of a
/// frame `width_px` x `height_px`, glyphs [`HUD_GLYPH_PX`] tall, in pixels from the centre.
pub fn text_strokes(text: &str, width_px: f64, height_px: f64) -> Vec<([f64; 2], [f64; 2])> {
    let width = line_width(drawn(text).len(), HUD_GLYPH_PX);
    text_strokes_at(
        text,
        HUD_GLYPH_PX,
        width_px / 2.0 - HUD_MARGIN_PX - width,
        -height_px / 2.0 + HUD_MARGIN_PX,
    )
}

/// The stroke thickness of a line of glyphs `glyph_px` tall.
fn stroke_px(glyph_px: f64) -> f64 {
    (glyph_px / 9.0).max(1.5)
}

/// A dark outline and the bright strokes of one line, appended to `outlines` and `bright`.
fn push_line(
    line: &HudLine,
    glyph_px: f64,
    (left, bottom): (f64, f64),
    outlines: &mut Vec<PxSegment>,
    bright: &mut Vec<PxSegment>,
) {
    let width = stroke_px(glyph_px);
    for (from, to) in text_strokes_at(&line.text, glyph_px, left, bottom) {
        outlines.push(PxSegment {
            from,
            to,
            rgba: [0, 0, 0, line.rgba[3] / 2 + line.rgba[3] / 4],
            width_px: width + 2.5,
        });
        bright.push(PxSegment {
            from,
            to,
            rgba: line.rgba,
            width_px: width,
        });
    }
}

/// A horizontal bar `x0..x1` at `y`, `height` thick.
fn bar(x0: f64, x1: f64, y: f64, height: f64, rgba: [u8; 4]) -> PxSegment {
    PxSegment {
        from: [x0, y],
        to: [x1, y],
        rgba,
        width_px: height,
    }
}

/// The layout of `hud` in a frame `width_px` x `height_px`: the veil and vignette first, the
/// backings of the corner lines, then the dark outlines of every line and the bright strokes
/// on top. Empty for an empty frame.
pub fn layout(hud: &Hud, width_px: f64, height_px: f64) -> Vec<PxSegment> {
    if !(width_px > 0.0 && height_px > 0.0) || hud.is_empty() {
        return Vec::new();
    }
    let (w, h) = (width_px, height_px);
    let mut out = Vec::new();
    if let Some(rgba) = hud.veil {
        // One bar as thick as the frame is tall, reaching past both sides.
        out.push(bar(-w, w, 0.0, h * 1.5, rgba));
    }
    if let Some([r, g, b, a]) = hud.vignette {
        let edge = f64::from(a) / 255.0;
        let each = 1.0 - (1.0 - edge).powf(1.0 / f64::from(VIGNETTE_BANDS));
        let rgba = [r, g, b, (each * 255.0).round().clamp(0.0, 255.0) as u8];
        for k in 1..=VIGNETTE_BANDS {
            let depth = h * HUD_VIGNETTE_DEPTH * f64::from(k) / f64::from(VIGNETTE_BANDS);
            let (cx, cy) = (w / 2.0 - depth / 2.0, h / 2.0 - depth / 2.0);
            for sign in [-1.0, 1.0] {
                out.push(bar(-w, w, sign * cy, depth, rgba));
                out.push(PxSegment {
                    from: [sign * cx, -h],
                    to: [sign * cx, h],
                    rgba,
                    width_px: depth,
                });
            }
        }
    }
    let mut outlines = Vec::new();
    let mut bright = Vec::new();
    // The corner: the last line on the margin, the others stacked above it.
    let mut cursor = -h / 2.0 + HUD_MARGIN_PX;
    for line in hud.corner.iter().rev() {
        let glyph = line.glyph_px;
        let text_width = line_width(drawn(&line.text).len(), glyph);
        let left = w / 2.0 - HUD_MARGIN_PX - text_width;
        let half = glyph * 0.85;
        let pad = glyph * 0.6;
        let (x0, x1) = (left - pad + half, left + text_width + pad - half);
        let mid = (x0 + x1) / 2.0;
        let (x0, x1) = if x0 < x1 { (x0, x1) } else { (mid, mid) };
        out.push(bar(x0, x1, cursor + glyph / 2.0, half * 2.0, BACKING_RGBA));
        push_line(line, glyph, (left, cursor), &mut outlines, &mut bright);
        cursor += glyph * (1.0 + HUD_LINE_GAP);
    }
    // The middle: the block of lines centred, a little above the centre.
    let fitted: Vec<f64> = hud
        .centre
        .iter()
        .map(|line| {
            let natural = line_width(drawn(&line.text).len(), line.glyph_px);
            if natural > w * HUD_CENTRE_WIDTH_SHARE {
                line.glyph_px * w * HUD_CENTRE_WIDTH_SHARE / natural
            } else {
                line.glyph_px
            }
        })
        .collect();
    let block: f64 = fitted.iter().map(|g| g * (1.0 + HUD_LINE_GAP)).sum::<f64>();
    let mut top = block / 2.0 + h * 0.05;
    for (line, glyph) in hud.centre.iter().zip(&fitted) {
        let text_width = line_width(drawn(&line.text).len(), *glyph);
        let bottom = top - glyph;
        push_line(
            line,
            *glyph,
            (-text_width / 2.0, bottom),
            &mut outlines,
            &mut bright,
        );
        top -= glyph * (1.0 + HUD_LINE_GAP);
    }
    out.extend(outlines);
    out.extend(bright);
    out
}

/// The HUD as world segments one metre in front of the eye of `camera`, for a frame
/// `width_px` x `height_px` ([`layout`]). Empty for an empty HUD, a degenerate camera or an
/// empty frame.
pub fn hud_segments(camera: &Camera, width_px: u32, height_px: u32, hud: &Hud) -> Vec<Segment> {
    let eye = DVec3::from_array(camera.position);
    let forward = (DVec3::from_array(camera.target) - eye).normalize_or_zero();
    if forward == DVec3::ZERO || width_px == 0 || height_px == 0 || hud.is_empty() {
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
    layout(hud, f64::from(width_px), f64::from(height_px))
        .iter()
        .map(|s| Segment {
            from: place(s.from),
            to: place(s.to),
            rgba: s.rgba,
            width_px: s.width_px as f32,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    /// (left, right, bottom, top) of the segments' ends.
    fn bounds(strokes: &[PxSegment]) -> (f64, f64, f64, f64) {
        let xs = strokes.iter().flat_map(|s| [s.from[0], s.to[0]]);
        let ys = strokes.iter().flat_map(|s| [s.from[1], s.to[1]]);
        (
            xs.clone().fold(f64::INFINITY, f64::min),
            xs.fold(f64::NEG_INFINITY, f64::max),
            ys.clone().fold(f64::INFINITY, f64::min),
            ys.fold(f64::NEG_INFINITY, f64::max),
        )
    }

    fn white(segs: &[PxSegment]) -> Vec<PxSegment> {
        segs.iter()
            .copied()
            .filter(|s| s.rgba == [255; 4])
            .collect()
    }

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
        for c in "SAFE SEMI AUTO BURST RELOAD 0123456789/ JQZ.'-".chars() {
            assert_eq!(super::strokes(c).is_empty(), c == ' ', "{c}");
        }
        assert!(text_strokes("", 100.0, 100.0).is_empty());
    }

    #[test]
    fn corner_lines_stack_upwards_from_the_margin_and_end_at_the_right_edge() {
        let hud = Hud {
            corner: vec![
                HudLine::new("AUTO", 18.0, [255; 4]),
                HudLine::new("MAGAZINE FULL", 18.0, [255; 4]),
            ],
            ..Hud::default()
        };
        let segs = layout(&hud, 1280.0, 720.0);
        let (_, right, bottom, top) = bounds(&white(&segs));
        assert!((right - (640.0 - HUD_MARGIN_PX)).abs() < 1e-9, "{right}");
        assert!((bottom - (-360.0 + HUD_MARGIN_PX)).abs() < 1e-9, "{bottom}");
        assert!(
            (top - bottom - 18.0 * (2.0 + HUD_LINE_GAP)).abs() < 1e-9,
            "{top}"
        );
        // A backing under each line comes before the text.
        assert_eq!(segs.iter().filter(|s| s.rgba == BACKING_RGBA).count(), 2);
    }

    #[test]
    fn a_centred_line_is_centred_and_shrinks_to_fit_a_narrow_frame() {
        let hud = Hud {
            centre: vec![HudLine::new("YOU ARE DEAD", 64.0, [255; 4])],
            ..Hud::default()
        };
        let (l, r, _, _) = bounds(&white(&layout(&hud, 1920.0, 1080.0)));
        assert!((l + r).abs() < 1e-9, "centred: {l} {r}");
        assert!(r - l > 600.0, "full size in a wide frame: {}", r - l);
        let (l, r, _, _) = bounds(&white(&layout(&hud, 400.0, 300.0)));
        assert!(r - l <= 400.0 * HUD_CENTRE_WIDTH_SHARE + 1e-9, "{}", r - l);
    }

    #[test]
    fn the_veil_covers_the_frame_and_the_vignette_darkens_the_edges() {
        let hud = Hud {
            veil: Some([60, 0, 0, 150]),
            vignette: Some([160, 0, 0, 200]),
            ..Hud::default()
        };
        let segs = layout(&hud, 1280.0, 720.0);
        let veil = segs[0];
        assert_eq!(veil.rgba, [60, 0, 0, 150]);
        assert!(veil.width_px >= 720.0 && veil.from[0] <= -640.0 && veil.to[0] >= 640.0);
        // The bands on each of the four edges.
        assert_eq!(segs.len(), 1 + 4 * VIGNETTE_BANDS as usize);
        // Stacked, the bands add up to the edge's alpha.
        let each = f64::from(segs[1].rgba[3]) / 255.0;
        let total = 1.0 - (1.0 - each).powi(VIGNETTE_BANDS as i32);
        assert!((total - 200.0 / 255.0).abs() < 0.03, "{total}");
        assert!(layout(&Hud::default(), 1280.0, 720.0).is_empty());
        assert!(layout(&hud, 0.0, 720.0).is_empty());
    }

    #[test]
    fn the_segments_sit_in_front_of_the_eye_and_project_to_the_corner() {
        let camera = Camera {
            position: [0.0, 1.6, 0.0],
            target: [0.0, 1.6, -10.0],
            fov_y_deg: 70.0,
        };
        let hud = Hud {
            corner: vec![HudLine::new("SEMI", 16.0, [255; 4])],
            ..Hud::default()
        };
        let segments = hud_segments(&camera, 1280, 720, &hud);
        assert!(!segments.is_empty());
        for s in segments.iter().filter(|s| s.rgba == [255; 4]) {
            let ndc = camera.project(s.to, 1280.0 / 720.0).unwrap();
            assert!(ndc[0] > 0.5 && ndc[1] < -0.8, "{ndc:?}");
        }
        assert!(hud_segments(&camera, 0, 720, &hud).is_empty());
        assert!(hud_segments(&camera, 1280, 720, &Hud::default()).is_empty());
    }
}
