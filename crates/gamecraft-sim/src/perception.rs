//! Seeing: whether one character sees another (`docs/design/ballistics.md`, *Perception*).
//!
//! A character `a` sees `b` when
//!
//! 1. the distance from `a`'s eyes to `b`'s eyes is within `a`'s **effective range**,
//! 2. `b` lies within `a`'s field of view (horizontally, around `a`'s facing), and
//! 3. the line from `a`'s eyes to `b`'s eyes, or to `b`'s chest, is clear: the same ray maths
//!    as `query.line_of_sight` ([`crate::ray::line_of_sight`]), against the terrain and every
//!    shape but those of `a` and `b` themselves.
//!
//! `a`'s view range and field of view are its behaviour's `view_range_m` and `fov_deg`, or
//! [`DEFAULT_VIEW_RANGE_M`] and [`DEFAULT_FOV_DEG`] without one. The effective range is
//!
//! ```text
//! range = min(view_range * light, visibility)
//! light = N + (1 - N) * clamp((e - E_night) / (E_day - E_night), 0, 1)
//! visibility = 3.912 / (sigma_mist + sigma_rain)
//! sigma_mist = 3.912 * mist^2 / 120            (1/m; 120 m in a fog of 1)
//! sigma_rain = 3.912 / (400 * (0.8 / rain)^1.5) (1/m; 290 m in a rain of 1)
//! ```
//!
//! with `e` the sun's elevation, [`NIGHT_ELEVATION_DEG`] `E_night`, [`DAY_ELEVATION_DEG`]
//! `E_day` and [`NIGHT_RANGE_FACTOR`] `N`. The visibility is Koschmieder's law over the very
//! extinction coefficients the renderer gives its mist and rain fog (VisionCraft
//! `visioncraft-render/src/atmosphere.rs`, `mist_extinction` and `rain_extinction`, MIT), so
//! what a player cannot see in the picture an enemy cannot see either. Cover in vegetation is
//! not counted yet.

use gamecraft_math::{Portable, yaw_forward};
use gamecraft_world::{Entity, Environment, HeightField, World};

use crate::ray;
use crate::trunk::Trunks;

/// View range of a character without a behaviour, metres.
pub const DEFAULT_VIEW_RANGE_M: f64 = 300.0;
/// Field of view of a character without a behaviour, degrees.
pub const DEFAULT_FOV_DEG: f64 = 120.0;
/// Share of the view range left in full night.
pub const NIGHT_RANGE_FACTOR: f64 = 0.15;
/// Sun elevation at and below which it is full night, degrees (nautical dusk).
pub const NIGHT_ELEVATION_DEG: f64 = -12.0;
/// Sun elevation at and above which it is full day, degrees.
pub const DAY_ELEVATION_DEG: f64 = 6.0;
/// The chest, as a share of a character's height above its feet.
pub const CHEST_SHARE: f64 = 0.65;
/// Koschmieder's constant: visibility times extinction.
const KOSCHMIEDER: f64 = 3.912;
/// Visibility in a mist of 1 (fog), metres.
const FOG_VISIBILITY_M: f64 = 120.0;
/// Visibility in a rain of 0.8 (a downpour), metres.
const DOWNPOUR_VISIBILITY_M: f64 = 400.0;
/// The rain of [`DOWNPOUR_VISIBILITY_M`].
const DOWNPOUR_RAIN: f64 = 0.8;
/// Exponent of the rain's visibility.
const RAIN_EXPONENT: f64 = 1.5;
/// Below this horizontal distance, metres, a target counts as in view whatever the facing.
const MIN_FACING_DISTANCE_M: f64 = 1e-9;

/// The extinction of mist and rain together, 1/m.
fn extinction(env: &Environment) -> f64 {
    let mist = env.mist.clamp(0.0, 1.0);
    let rain = env.rain.clamp(0.0, 1.0);
    let sigma_mist = KOSCHMIEDER * mist * mist / FOG_VISIBILITY_M;
    let sigma_rain = if rain > 0.0 {
        KOSCHMIEDER / (DOWNPOUR_VISIBILITY_M * (DOWNPOUR_RAIN / rain).ppowf(RAIN_EXPONENT))
    } else {
        0.0
    };
    let sigma = sigma_mist + sigma_rain;
    if sigma.is_finite() { sigma } else { 0.0 }
}

/// How far anything can be seen in the mist and rain of `env`, metres; infinite in clear air.
pub fn visibility_m(env: &Environment) -> f64 {
    let sigma = extinction(env);
    if sigma > 0.0 {
        KOSCHMIEDER / sigma
    } else {
        f64::INFINITY
    }
}

/// The share of the view range the light of `env` leaves (1 by day, [`NIGHT_RANGE_FACTOR`] at
/// night).
pub fn light_factor(env: &Environment) -> f64 {
    let e = env.sun().elevation_deg;
    let day =
        ((e - NIGHT_ELEVATION_DEG) / (DAY_ELEVATION_DEG - NIGHT_ELEVATION_DEG)).clamp(0.0, 1.0);
    let day = if day.is_finite() { day } else { 1.0 };
    NIGHT_RANGE_FACTOR + (1.0 - NIGHT_RANGE_FACTOR) * day
}

/// The effective range of a view range of `view_range_m` in `env`, metres.
pub fn effective_range_m(env: &Environment, view_range_m: f64) -> f64 {
    (view_range_m * light_factor(env)).min(visibility_m(env))
}

/// A character's eyes and chest.
fn eye_and_chest(e: &Entity) -> Option<([f64; 3], [f64; 3])> {
    let c = e.character()?;
    let [x, y, z] = e.position;
    Some((
        [x, y + c.eye_height_m, z],
        [x, y + c.height_m * CHEST_SHARE, z],
    ))
}

/// Whether `a` sees `b` (see the module docs). False when either is not a character.
pub(crate) fn sees(
    world: &World,
    heights: Option<&HeightField>,
    trunks: &Trunks,
    a: &Entity,
    b: &Entity,
) -> bool {
    if a.id == b.id {
        return false;
    }
    let (Some((eye, _)), Some((their_eye, chest))) = (eye_and_chest(a), eye_and_chest(b)) else {
        return false;
    };
    let (view, fov) = a
        .behaviour()
        .map_or((DEFAULT_VIEW_RANGE_M, DEFAULT_FOV_DEG), |bh| {
            (bh.view_range_m, bh.fov_deg)
        });
    let range = effective_range_m(&world.environment, view);
    let (dx, dy, dz) = (
        their_eye[0] - eye[0],
        their_eye[1] - eye[1],
        their_eye[2] - eye[2],
    );
    let distance = (dx * dx + dy * dy + dz * dz).sqrt();
    if !(distance.is_finite() && distance <= range) {
        return false;
    }
    let horizontal = (dx * dx + dz * dz).sqrt();
    if horizontal > MIN_FACING_DISTANCE_M && fov < 360.0 {
        let f = yaw_forward(a.rotation_y_deg);
        let cos = (f.x * dx + f.z * dz) / horizontal;
        let half = (fov / 2.0).to_radians().pcos();
        if cos < half {
            return false;
        }
    }
    let skip = |id| id == a.id || id == b.id;
    [their_eye, chest].into_iter().any(|to| {
        ray::line_of_sight(world, heights, trunks, eye, to, skip)
            .is_some_and(|s| s.blocked_by.is_none())
    })
}
