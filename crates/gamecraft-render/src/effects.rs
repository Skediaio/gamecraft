//! The effects of combat as plain data, and the GPU-free geometry they are drawn with: tracer
//! streaks, impact puffs and the crosshair. The caller (the window) fills [`Effects`] from the
//! running simulation each frame; this crate never sees the simulation.

use crate::Camera;
use gamecraft_math::{DMat4, DVec3, deg_to_rad};

/// How long an impact puff lasts, seconds: an [`Impact`] older than this is not drawn.
pub const IMPACT_LIFE_S: f64 = 1.0;
/// The largest radius of an impact puff, metres.
pub const IMPACT_RADIUS_M: f64 = 0.3;
/// The share of [`IMPACT_LIFE_S`] a puff takes to swell to its full radius.
const IMPACT_SWELL_SHARE: f64 = 0.12;
/// Linear emissive colour of a puff: hot dust.
pub const IMPACT_EMISSIVE: [f32; 3] = [1.0, 0.33, 0.04];
/// A tracer's core colour, sRGB: near white, warm.
pub const TRACER_CORE_RGB: [u8; 3] = [255, 246, 205];
/// A tracer's glow colour, sRGB: orange.
pub const TRACER_GLOW_RGB: [u8; 3] = [255, 150, 40];
/// A tracer's core thickness, physical pixels.
pub const TRACER_CORE_PX: f32 = 3.0;
/// A tracer's glow thickness, physical pixels.
pub const TRACER_GLOW_PX: f32 = 8.0;
/// Seconds of flight after which a tracer has dimmed to [`TRACER_MIN_ALPHA`].
const TRACER_DIM_S: f64 = 3.0;
/// The faintest a tracer gets with age, 0 to 1.
const TRACER_MIN_ALPHA: f64 = 0.4;
/// How far in front of the eye the crosshair is placed, metres.
const CROSSHAIR_DEPTH_M: f64 = 1.0;
/// The gap in the middle of the crosshair, and where its arms end, pixels from the centre.
pub const CROSSHAIR_INNER_PX: f64 = 5.0;
/// Where the crosshair's arms end, pixels from the centre.
pub const CROSSHAIR_OUTER_PX: f64 = 14.0;

/// A projectile's streak: a short bright line behind its head.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tracer {
    /// The tail, world metres.
    pub from: [f64; 3],
    /// The head, world metres.
    pub to: [f64; 3],
    /// Seconds since the projectile was fired (older ones are a little dimmer).
    pub age_s: f64,
}

/// Where a projectile hit: a puff that swells and shrinks away.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impact {
    /// The point hit, world metres.
    pub point: [f64; 3],
    /// The surface's outward normal there (the puff rises off the surface along it).
    pub normal: [f64; 3],
    /// Seconds since the hit; at [`IMPACT_LIFE_S`] the puff is gone.
    pub age_s: f64,
}

/// What is in flight and what just hit, to draw on top of the world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Effects {
    /// The projectiles' streaks.
    pub tracers: Vec<Tracer>,
    /// The recent impacts.
    pub impacts: Vec<Impact>,
}

/// One straight piece of a drawn effect, with its look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// Start, world metres.
    pub from: [f64; 3],
    /// End, world metres.
    pub to: [f64; 3],
    /// sRGB colour with alpha.
    pub rgba: [u8; 4],
    /// Thickness, physical pixels.
    pub width_px: f32,
}

fn alpha_byte(alpha: f64) -> u8 {
    (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The segments of a tracer: a wide dim glow under a thin bright core. A tracer with a
/// non-finite end is left out.
pub fn tracer_segments(t: &Tracer) -> Vec<Segment> {
    let finite = t.from.iter().chain(&t.to).all(|v| v.is_finite());
    if !finite {
        return Vec::new();
    }
    let age = if t.age_s.is_finite() {
        t.age_s.max(0.0)
    } else {
        0.0
    };
    let alpha = 1.0 - (1.0 - TRACER_MIN_ALPHA) * (age / TRACER_DIM_S).min(1.0);
    let [gr, gg, gb] = TRACER_GLOW_RGB;
    let [cr, cg, cb] = TRACER_CORE_RGB;
    vec![
        Segment {
            from: t.from,
            to: t.to,
            rgba: [gr, gg, gb, alpha_byte(alpha * 0.45)],
            width_px: TRACER_GLOW_PX,
        },
        Segment {
            from: t.from,
            to: t.to,
            rgba: [cr, cg, cb, alpha_byte(alpha)],
            width_px: TRACER_CORE_PX,
        },
    ]
}

/// The puff of an impact as (centre, radius): it swells fast to [`IMPACT_RADIUS_M`], then
/// shrinks to nothing at [`IMPACT_LIFE_S`]. `None` when it is over, not yet born or not finite.
pub fn impact_puff(i: &Impact) -> Option<([f64; 3], f64)> {
    let finite = i
        .point
        .iter()
        .chain(&i.normal)
        .chain(std::iter::once(&i.age_s))
        .all(|v| v.is_finite());
    if !finite || i.age_s < 0.0 || i.age_s >= IMPACT_LIFE_S {
        return None;
    }
    let swell = IMPACT_SWELL_SHARE * IMPACT_LIFE_S;
    let share = if i.age_s < swell {
        i.age_s / swell
    } else {
        1.0 - (i.age_s - swell) / (IMPACT_LIFE_S - swell)
    };
    let radius = IMPACT_RADIUS_M * share;
    let normal = DVec3::from_array(i.normal).normalize_or_zero();
    // The puff sits a little off the surface, so half of it is not buried.
    let centre = DVec3::from_array(i.point) + normal * (radius * 0.5);
    Some((centre.to_array(), radius))
}

/// The model matrix of a puff: a unit sphere scaled to `radius` at `centre`.
pub fn puff_matrix(centre: [f64; 3], radius: f64) -> glam::Mat4 {
    DMat4::from_scale_rotation_translation(
        DVec3::splat(radius * 2.0),
        gamecraft_math::DQuat::IDENTITY,
        DVec3::from_array(centre),
    )
    .as_mat4()
}

/// The crosshair of a first-person view: four arms round a gap, as world segments one metre in
/// front of the eye whose length is a fixed number of pixels on a frame `height_px` tall. Dark
/// outlines come first, the bright arms after.
pub fn crosshair_segments(camera: &Camera, height_px: u32) -> Vec<Segment> {
    let eye = DVec3::from_array(camera.position);
    let forward = (DVec3::from_array(camera.target) - eye).normalize_or_zero();
    if forward == DVec3::ZERO || height_px == 0 {
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
        2.0 * CROSSHAIR_DEPTH_M * deg_to_rad(camera.fov_y_deg / 2.0).tan() / f64::from(height_px);
    let centre = eye + forward * CROSSHAIR_DEPTH_M;
    let arm = |axis: DVec3, from_px: f64, to_px: f64, rgba: [u8; 4], width_px: f32| Segment {
        from: (centre + axis * from_px * metres_per_px).to_array(),
        to: (centre + axis * to_px * metres_per_px).to_array(),
        rgba,
        width_px,
    };
    let axes = [right, -right, up, -up];
    let outline = axes.map(|a| {
        arm(
            a,
            CROSSHAIR_INNER_PX - 1.0,
            CROSSHAIR_OUTER_PX + 1.0,
            [0, 0, 0, 170],
            4.0,
        )
    });
    let bright = axes.map(|a| {
        arm(
            a,
            CROSSHAIR_INNER_PX,
            CROSSHAIR_OUTER_PX,
            [255, 255, 255, 230],
            2.0,
        )
    });
    outline.into_iter().chain(bright).collect()
}
