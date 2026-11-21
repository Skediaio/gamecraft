//! A first-person view model (a character's `held_model`) that is never hidden by a wall in
//! front of the camera and casts no shadow, without a pass of its own:
//!
//! - **Not through walls.** The model is shrunk towards the eye by a share `k` (a uniform scale
//!   about the eye). Seen from the eye that changes nothing: every point keeps its direction,
//!   so the picture is the same; only its depth shrinks. `k` is chosen so the whole model lies
//!   nearer the eye than [`VIEW_MODEL_CLEAR_SHARE`] of the character's radius, where no solid
//!   can be (the simulation keeps every solid at least the radius from a character's axis,
//!   which the eye is on), but its nearest point stays beyond [`VIEW_MODEL_NEAR_M`] (the
//!   camera's near plane, with a margin); when both cannot hold, the near plane wins and the
//!   far end of a long model may meet a wall.
//! - **No shadow.** The model's item has a second level of detail, an empty mesh, beyond
//!   [`VIEW_MODEL_LOD_M`]: the view draws the first level (the model is at the eye), the shadow
//!   passes draw one level coarser, which is nothing.

use gamecraft_math::{DMat4, DVec3};

/// The nearest a shrunk view model comes to the eye, metres: beyond the first-person camera's
/// near plane (0.05 m), with a margin.
pub const VIEW_MODEL_NEAR_M: f64 = 0.06;
/// The share of the holder's radius the shrunk view model fits within.
pub const VIEW_MODEL_CLEAR_SHARE: f64 = 0.9;
/// The distance up to which a view model draws its own mesh, metres (it is always at the eye);
/// its next level, which the shadow passes draw, is empty.
pub const VIEW_MODEL_LOD_M: f32 = 1000.0;

/// The box a model's vertices lie in, in the model's own axes and units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelExtent {
    /// The smallest x, y and z.
    pub min: [f64; 3],
    /// The largest x, y and z.
    pub max: [f64; 3],
}

impl ModelExtent {
    /// The box of `positions`, `None` without a finite one.
    pub fn of(positions: impl Iterator<Item = [f32; 3]>) -> Option<ModelExtent> {
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for p in positions {
            for ((lo, hi), v) in min.iter_mut().zip(max.iter_mut()).zip(p) {
                let v = f64::from(v);
                if v.is_finite() {
                    *lo = lo.min(v);
                    *hi = hi.max(v);
                }
            }
        }
        min.iter()
            .chain(&max)
            .all(|v| v.is_finite())
            .then_some(ModelExtent { min, max })
    }
}

/// The share `k` (0 to 1) a view model is shrunk by towards the eye: its origin is `offset`
/// from the eye in the aim's axes (right, up, back), its vertices lie in `extent` times
/// `scale`, and nothing must stand within `clear_m` of the eye. 1 when nothing is known or
/// nothing helps (the eye inside the model's box).
pub fn depth_share(offset: DVec3, scale: f64, extent: &ModelExtent, clear_m: f64) -> f64 {
    if !(scale.is_finite() && scale > 0.0 && offset.is_finite() && clear_m.is_finite()) {
        return 1.0;
    }
    // The eye in the model's own units: a vertex v is at offset + scale * v from the eye.
    let eye = -offset / scale;
    let lo = DVec3::from_array(extent.min);
    let hi = DVec3::from_array(extent.max);
    let nearest = (eye.clamp(lo, hi) - eye).length() * scale;
    let farthest = (0..8)
        .map(|corner| {
            let pick = |bit: i32, a: f64, b: f64| if corner & bit == 0 { a } else { b };
            DVec3::new(
                pick(1, lo.x, hi.x),
                pick(2, lo.y, hi.y),
                pick(4, lo.z, hi.z),
            )
        })
        .map(|c| (c - eye).length() * scale)
        .fold(0.0, f64::max);
    if !(farthest.is_finite() && nearest.is_finite() && nearest > 0.0) {
        return 1.0;
    }
    let k = (clear_m.max(0.0) / farthest).min(1.0);
    if k * nearest < VIEW_MODEL_NEAR_M {
        (VIEW_MODEL_NEAR_M / nearest).min(1.0)
    } else {
        k
    }
}

/// `model` (model to world) shrunk by `k` about the point `eye`.
pub fn shrink_towards(eye: DVec3, model: DMat4, k: f64) -> DMat4 {
    DMat4::from_translation(eye)
        * DMat4::from_scale(DVec3::splat(k))
        * DMat4::from_translation(-eye)
        * model
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// A crate 1 m on a side standing on its origin.
    const CRATE: ModelExtent = ModelExtent {
        min: [-0.5, 0.0, -0.5],
        max: [0.5, 1.0, 0.5],
    };

    fn distances(offset: DVec3, scale: f64, k: f64) -> (f64, f64) {
        let mut near = f64::INFINITY;
        let mut far: f64 = 0.0;
        for x in [CRATE.min[0], CRATE.max[0]] {
            for y in [CRATE.min[1], CRATE.max[1]] {
                for z in [CRATE.min[2], CRATE.max[2]] {
                    let d = (k * (offset + DVec3::new(x, y, z) * scale)).length();
                    near = near.min(d);
                    far = far.max(d);
                }
            }
        }
        (near, far)
    }

    #[test]
    fn a_shrunk_model_fits_between_the_near_plane_and_the_nearest_wall() {
        // Half a crate a metre ahead and a little low: shrunk into 0.27 m (a radius of 0.3).
        let offset = DVec3::new(0.0, -0.2, -1.0);
        let k = depth_share(offset, 0.5, &CRATE, 0.27);
        assert!(k < 0.3 && k > 0.0, "{k}");
        let (near, far) = distances(offset, 0.5, k);
        assert!(far <= 0.27 + 1e-9, "{far}");
        assert!(near >= VIEW_MODEL_NEAR_M - 1e-9, "{near}");
        // Already small and near: left as it is.
        assert_eq!(
            depth_share(DVec3::new(0.0, -0.05, -0.15), 0.05, &CRATE, 0.27),
            1.0
        );
        // A long model reaching from the eye: the near plane wins.
        let k = depth_share(DVec3::new(0.0, -0.05, -0.12), 1.0, &CRATE, 0.27);
        assert!(k <= 1.0 && k > 0.0, "{k}");
        // The eye inside the box, or numbers that are not finite: nothing helps.
        assert_eq!(
            depth_share(DVec3::new(0.0, -0.5, 0.0), 1.0, &CRATE, 0.27),
            1.0
        );
        assert_eq!(depth_share(DVec3::splat(f64::NAN), 1.0, &CRATE, 0.27), 1.0);
        assert_eq!(depth_share(offset, 0.0, &CRATE, 0.27), 1.0);
    }

    #[test]
    fn shrinking_about_the_eye_keeps_every_direction_seen_from_it() {
        let eye = DVec3::new(3.0, 1.6, -2.0);
        let model = DMat4::from_scale_rotation_translation(
            DVec3::splat(0.5),
            gamecraft_math::DQuat::from_rotation_y(0.7),
            eye + DVec3::new(0.1, -0.2, -1.0),
        );
        let shrunk = shrink_towards(eye, model, 0.2);
        for p in [
            DVec3::ZERO,
            DVec3::new(0.5, 1.0, -0.5),
            DVec3::new(-0.5, 0.3, 0.5),
        ] {
            let a = model.transform_point3(p) - eye;
            let b = shrunk.transform_point3(p) - eye;
            assert!((a.normalize() - b.normalize()).length() < 1e-12);
            assert!((b.length() - 0.2 * a.length()).abs() < 1e-12);
        }
        let extent = ModelExtent::of([[1.0, -2.0, 0.5], [-1.0, 3.0, f32::NAN]].into_iter());
        assert_eq!(extent.unwrap().max[2], 0.5, "a NaN is left out");
        assert_eq!(ModelExtent::of(std::iter::empty()), None);
        let extent = ModelExtent::of([[1.0, -2.0, 0.5], [-1.0, 3.0, 0.0]].into_iter()).unwrap();
        assert_eq!(
            (extent.min, extent.max),
            ([-1.0, -2.0, 0.0], [1.0, 3.0, 0.5])
        );
    }
}
