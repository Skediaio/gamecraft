//! Angle helpers in the engine's conventions: degrees at the surface, Y up, yaw 0 faces -Z.

use crate::Portable;
use glam::DVec3;

/// Degrees to radians.
pub fn deg_to_rad(deg: f64) -> f64 {
    deg * (core::f64::consts::PI / 180.0)
}

/// Radians to degrees.
pub fn rad_to_deg(rad: f64) -> f64 {
    rad * (180.0 / core::f64::consts::PI)
}

/// Wraps an angle in degrees into [0, 360). Non-finite input returns 0.
pub fn wrap_deg(deg: f64) -> f64 {
    if !deg.is_finite() {
        return 0.0;
    }
    let r = deg - 360.0 * (deg / 360.0).floor();
    // Rounding can land exactly on 360 (for a tiny negative input) or, for huge inputs,
    // outside the range; both mean 0.
    if (0.0..360.0).contains(&r) { r } else { 0.0 }
}

/// Wraps an angle in degrees into (-180, 180]. Non-finite input returns 0.
pub fn wrap_deg_signed(deg: f64) -> f64 {
    let w = wrap_deg(deg);
    if w > 180.0 { w - 360.0 } else { w }
}

/// The horizontal unit vector a yaw faces: `(-sin yaw, 0, -cos yaw)`.
///
/// Yaw is in degrees, counter-clockwise about +Y seen from above; yaw 0 faces north (-Z) and
/// yaw 90 faces west (-X). Non-finite input gives a non-finite vector, never a panic.
pub fn yaw_forward(yaw_deg: f64) -> DVec3 {
    let (s, c) = deg_to_rad(yaw_deg).psin_cos();
    DVec3::new(-s, 0.0, -c)
}
