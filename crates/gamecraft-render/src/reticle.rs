//! The dot of a collimated sight (a held model's `reticle`, a red dot): drawn over everything
//! while the eye is down the sights, where the barrel points. A collimated sight shows its dot
//! at infinity along the sight's axis, so the dot is a direction, not a place: it marks the
//! aim with its sway and recoil, and wanders over the picture as the barrel does while the
//! camera looks where the head looks.

use crate::Camera;
use crate::effects::Segment;
use gamecraft_math::{DQuat, DVec3, deg_to_rad};
use gamecraft_world::Reticle;

/// How far up to the sights the eye must be before the dot is seen through the optic (0 at
/// the hip, 1 at the sights).
pub const RETICLE_MIN_ADS: f64 = 0.9;
/// The smallest radius the dot is drawn with, physical pixels: a real dot of a few
/// milliradians would be a pixel or less on a screen.
pub const RETICLE_MIN_RADIUS_PX: f64 = 1.5;
/// The glow round the dot, as a multiple of its radius.
const RETICLE_GLOW_SHARE: f64 = 1.8;
/// How far along the aim the dot is placed, metres (it is drawn over everything; only its
/// direction counts).
const RETICLE_DEPTH_M: f64 = 1.0;

/// The direction an aim of `yaw_deg` (counter-clockwise from north seen from above) and
/// `pitch_deg` (up from level) points, as a held model faces it: north is -Z.
pub fn aim_forward(yaw_deg: f64, pitch_deg: f64) -> DVec3 {
    let turn =
        DQuat::from_rotation_y(deg_to_rad(yaw_deg)) * DQuat::from_rotation_x(deg_to_rad(pitch_deg));
    turn * DVec3::NEG_Z
}

/// The camera's right and up axes and its pixels per unit of the image plane one metre ahead,
/// for a frame `height_px` tall. `None` for a degenerate camera.
fn basis(camera: &Camera, height_px: u32) -> Option<(DVec3, DVec3, DVec3, f64)> {
    let eye = DVec3::from_array(camera.position);
    let forward = (DVec3::from_array(camera.target) - eye).normalize_or_zero();
    if forward == DVec3::ZERO || height_px == 0 {
        return None;
    }
    let up_hint = if forward.y.abs() > 0.999 {
        DVec3::NEG_Z
    } else {
        DVec3::Y
    };
    let right = forward.cross(up_hint).normalize_or_zero();
    let up = right.cross(forward);
    let px_per_unit = f64::from(height_px) / 2.0 / deg_to_rad(camera.fov_y_deg / 2.0).tan();
    px_per_unit
        .is_finite()
        .then_some((forward, right, up, px_per_unit))
}

/// Where a dot at infinity along `direction` appears in a frame `height_px` tall seen from
/// `camera`: pixels from the frame's centre, x right and y up. `None` when the direction is
/// not ahead of the camera or a number is not finite.
pub fn dot_offset_px(camera: &Camera, direction: DVec3, height_px: u32) -> Option<[f64; 2]> {
    let (forward, right, up, px_per_unit) = basis(camera, height_px)?;
    let d = direction.normalize_or_zero();
    let ahead = d.dot(forward);
    if ahead.is_nan() || ahead <= 1e-6 {
        return None;
    }
    let at = [
        d.dot(right) / ahead * px_per_unit,
        d.dot(up) / ahead * px_per_unit,
    ];
    at.iter().all(|v| v.is_finite()).then_some(at)
}

/// The dot's radius in pixels for a frame `height_px` tall from `camera`: its angle, at least
/// [`RETICLE_MIN_RADIUS_PX`].
pub fn dot_radius_px(camera: &Camera, reticle: &Reticle, height_px: u32) -> f64 {
    let px = basis(camera, height_px).map_or(0.0, |(_, _, _, px_per_unit)| {
        (reticle.radius_mrad * 0.001).tan() * px_per_unit
    });
    if px.is_finite() {
        px.max(RETICLE_MIN_RADIUS_PX)
    } else {
        RETICLE_MIN_RADIUS_PX
    }
}

/// The dot as world segments (round points: a dim glow under a bright core) for an aim of
/// `yaw_deg`, `pitch_deg` that is `ads` up to the sights, seen from `camera` in a frame
/// `height_px` tall. Empty below [`RETICLE_MIN_ADS`], with a brightness of 0, a degenerate
/// camera or an aim not ahead of it.
pub fn reticle_segments(
    camera: &Camera,
    reticle: &Reticle,
    (yaw_deg, pitch_deg, ads): (f64, f64, f64),
    height_px: u32,
) -> Vec<Segment> {
    let brightness = if reticle.brightness.is_finite() {
        reticle.brightness.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if ads.is_nan() || ads < RETICLE_MIN_ADS || brightness <= 0.0 {
        return Vec::new();
    }
    let direction = aim_forward(yaw_deg, pitch_deg);
    if dot_offset_px(camera, direction, height_px).is_none() {
        return Vec::new();
    }
    let at = (DVec3::from_array(camera.position) + direction * RETICLE_DEPTH_M).to_array();
    let radius = dot_radius_px(camera, reticle, height_px);
    let [r, g, b] = reticle.color_srgb.map(|c| {
        if c.is_finite() {
            (c.clamp(0.0, 1.0) * 255.0).round() as u8
        } else {
            0
        }
    });
    let alpha = |a: f64| (a * brightness * 255.0).round().clamp(0.0, 255.0) as u8;
    let point = |rgba: [u8; 4], width_px: f64| Segment {
        from: at,
        to: at,
        rgba,
        width_px: width_px as f32,
    };
    vec![
        point([r, g, b, alpha(0.25)], 2.0 * radius * RETICLE_GLOW_SHARE),
        point([r, g, b, alpha(1.0)], 2.0 * radius),
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    /// A level camera at the eye looking north, 60 degrees tall.
    fn north() -> Camera {
        Camera {
            position: [0.0, 1.6, 0.0],
            target: [0.0, 1.6, -10.0],
            fov_y_deg: 60.0,
        }
    }

    const H: u32 = 1000;

    /// Pixels per unit of the image plane: (H / 2) / tan(30 degrees).
    fn scale() -> f64 {
        500.0 / 30f64.to_radians().tan()
    }

    #[test]
    fn the_dot_is_at_the_centre_when_the_barrel_points_where_the_camera_looks() {
        let at = dot_offset_px(&north(), aim_forward(0.0, 0.0), H).unwrap();
        assert!(at[0].abs() < 1e-9 && at[1].abs() < 1e-9, "{at:?}");
    }

    #[test]
    fn sway_and_recoil_move_the_dot_by_the_tangent_of_their_angle() {
        // Recoil lifts the barrel 1 degree: the dot rises by tan(1 degree) of the image plane.
        let up = dot_offset_px(&north(), aim_forward(0.0, 1.0), H).unwrap();
        assert!(up[0].abs() < 1e-9, "{up:?}");
        assert!(
            (up[1] - 1f64.to_radians().tan() * scale()).abs() < 1e-9,
            "{up:?}"
        );
        // Sway turns it 2 degrees to the left (yaw counter-clockwise): the dot goes left.
        let left = dot_offset_px(&north(), aim_forward(2.0, 0.0), H).unwrap();
        assert!(
            (left[0] + 2f64.to_radians().tan() * scale()).abs() < 1e-9,
            "{left:?}"
        );
        assert!(left[1].abs() < 1e-9, "{left:?}");
        // 10 mrad down (a 0.573 degree pull) at a camera tilted 20 degrees up.
        let tilted = Camera {
            target: [0.0, 1.6 + 10.0 * 20f64.to_radians().tan(), -10.0],
            ..north()
        };
        let pitch = 20.0 - 0.01f64.to_degrees();
        let down = dot_offset_px(&tilted, aim_forward(0.0, pitch), H).unwrap();
        assert!((down[1] + 0.01f64.tan() * scale()).abs() < 1e-6, "{down:?}");
        // The camera's own projection agrees.
        let d = aim_forward(3.0, -1.5);
        let at = dot_offset_px(&north(), d, H).unwrap();
        let p = DVec3::from_array(north().position) + d * 5.0;
        let ndc = north().project(p.to_array(), 1.0).unwrap();
        assert!((ndc[0] * 500.0 - at[0]).abs() < 1e-6, "{ndc:?} {at:?}");
        assert!((ndc[1] * 500.0 - at[1]).abs() < 1e-6, "{ndc:?} {at:?}");
    }

    #[test]
    fn the_dot_is_seen_only_through_the_optic_and_ahead() {
        let reticle = Reticle::default();
        let cam = north();
        assert!(reticle_segments(&cam, &reticle, (0.0, 0.0, 0.5), H).is_empty());
        assert!(reticle_segments(&cam, &reticle, (0.0, 0.0, f64::NAN), H).is_empty());
        let shown = reticle_segments(&cam, &reticle, (0.0, 0.0, 1.0), H);
        assert_eq!(shown.len(), 2);
        let core = shown[1];
        assert_eq!(core.from, core.to, "a round point");
        assert_eq!(core.rgba, [255, 20, 13, 255]);
        let ndc = cam.project(core.from, 1.0).unwrap();
        assert!(ndc[0].abs() < 1e-9 && ndc[1].abs() < 1e-9);
        // Behind the camera: nothing.
        assert!(reticle_segments(&cam, &reticle, (180.0, 0.0, 1.0), H).is_empty());
        let off = Reticle {
            brightness: 0.0,
            ..Reticle::default()
        };
        assert!(reticle_segments(&cam, &off, (0.0, 0.0, 1.0), H).is_empty());
    }

    #[test]
    fn the_dot_is_its_angle_wide_but_never_smaller_than_a_few_pixels() {
        let big = Reticle {
            radius_mrad: 10.0,
            ..Reticle::default()
        };
        let r = dot_radius_px(&north(), &big, H);
        assert!((r - 0.01f64.tan() * scale()).abs() < 1e-9, "{r}");
        let tiny = Reticle {
            radius_mrad: 0.1,
            ..Reticle::default()
        };
        assert_eq!(dot_radius_px(&north(), &tiny, H), RETICLE_MIN_RADIUS_PX);
    }
}
