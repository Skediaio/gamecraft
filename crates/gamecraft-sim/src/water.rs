//! Water in the simulation: the water areas of the running world, and wading.
//!
//! A character whose feet are below the surface of a water area wades: it keeps the ground
//! under the water (there is no swimming yet) and goes slower, by depth, from
//! [`WADE_FREE_DEPTH_M`] (knee-deep: as fast as on dry land) to [`WADE_MAX_DEPTH_M`]
//! (chest-deep: [`WADE_SLOWEST_SHARE`] of its speed). Water deeper than [`WADE_MAX_DEPTH_M`]
//! is refused, as most engines refuse deep water without swimming: a character does not step
//! into it (the step is not taken, as a wall blocks it); one that is in it (placed there, or
//! dropped into it) wades at the slowest share until it is out. Rays, lines of sight and
//! projectiles pass through water unchanged.
//!
//! The areas come from the world's `water` components ([`gamecraft_world::water_areas`]) in
//! id order; where several cover a point the highest surface counts. Nothing here has a
//! state of its own: the depth follows from the character's position.

use gamecraft_world::{WaterArea, World, depth_at};

/// Up to this depth of water, metres, a character walks as fast as on dry land (knee-deep).
pub const WADE_FREE_DEPTH_M: f64 = 0.5;
/// From this depth of water, metres, a character goes at [`WADE_SLOWEST_SHARE`] of its speed
/// (chest-deep on a 1.8 m character), and deeper water is refused.
pub const WADE_MAX_DEPTH_M: f64 = 1.5;
/// The share of its speed a character keeps in water [`WADE_MAX_DEPTH_M`] deep and deeper.
pub const WADE_SLOWEST_SHARE: f64 = 0.4;

/// Every water area of the running world, in id order of their entities.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Waters {
    areas: Vec<WaterArea>,
}

impl Waters {
    /// The water areas of `world`.
    pub(crate) fn of(world: &World) -> Waters {
        Waters {
            areas: gamecraft_world::water_areas(world),
        }
    }

    /// Whether the world has no water.
    pub(crate) fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }

    /// The depth of the water over the ground `feet` high at (`x`, `z`): that of the highest
    /// surface covering the point; `None` on dry land.
    pub(crate) fn depth_at(&self, x: f64, z: f64, feet: f64) -> Option<f64> {
        if self.areas.is_empty() {
            return None;
        }
        depth_at(&self.areas, x, z, feet)
    }
}

/// The share of its speed a character keeps with its feet in water `depth` metres deep
/// (`None`: dry land): 1 up to [`WADE_FREE_DEPTH_M`], falling linearly to
/// [`WADE_SLOWEST_SHARE`] at [`WADE_MAX_DEPTH_M`], and that share beyond.
pub fn wade_share(depth: Option<f64>) -> f64 {
    // A depth that is not a number is no water to wade in; an infinite one is deep.
    let Some(depth) = depth.filter(|d| !d.is_nan()) else {
        return 1.0;
    };
    if depth <= WADE_FREE_DEPTH_M {
        return 1.0;
    }
    let over =
        ((depth - WADE_FREE_DEPTH_M) / (WADE_MAX_DEPTH_M - WADE_FREE_DEPTH_M)).clamp(0.0, 1.0);
    1.0 - (1.0 - WADE_SLOWEST_SHARE) * over
}

/// Whether water `depth` metres deep (`None`: dry land) is too deep to step into
/// (deeper than [`WADE_MAX_DEPTH_M`]).
pub fn refused(depth: Option<f64>) -> bool {
    depth.is_some_and(|d| d > WADE_MAX_DEPTH_M)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_share_falls_with_the_depth_and_stops_at_the_slowest() {
        assert_eq!(wade_share(None), 1.0);
        assert_eq!(wade_share(Some(0.0)), 1.0);
        assert_eq!(wade_share(Some(WADE_FREE_DEPTH_M)), 1.0);
        assert!((wade_share(Some(1.0)) - 0.7).abs() < 1e-12);
        assert!((wade_share(Some(WADE_MAX_DEPTH_M)) - WADE_SLOWEST_SHARE).abs() < 1e-12);
        assert_eq!(wade_share(Some(5.0)), WADE_SLOWEST_SHARE);
        assert_eq!(wade_share(Some(f64::NAN)), 1.0);
        assert_eq!(wade_share(Some(f64::INFINITY)), WADE_SLOWEST_SHARE);
        assert!(!refused(None) && !refused(Some(WADE_MAX_DEPTH_M)));
        assert!(refused(Some(WADE_MAX_DEPTH_M + 1e-9)) && refused(Some(5.0)));
    }
}
