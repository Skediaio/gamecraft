//! Water: an area of the ground plane covered by a flat surface at one level (the `water`
//! component), as VisionCraft's sea level with an outline is.
//!
//! **The recipe** is a level and an outline: the surface stands `level_m` above the entity's
//! position, over the polygon `outline` (`[x, z]` offsets from the entity's position, turned by
//! its `rotation_y_deg` and multiplied by its scale), or everywhere when the outline is empty
//! (the sea out to the horizon). Nothing else is stored: whether a point is under water follows
//! from the outline and the level, and its depth from the ground there.
//!
//! **The surface is derived** ([`Water::outline_in_world`], [`inside_outline`]): the outline in
//! world axes, and the even-odd rule over it, the rule the renderer's water uses too, so the
//! picture and the simulation agree on where the water is. A point is under water when an area
//! covers it and its ground lies below the area's level; of areas that overlap, the highest
//! surface counts (a lake in a flooded plain is the lake). Everything is `f64` with `+`, `-`,
//! `*` and `/` only: deterministic on every platform.

use crate::error::Ctx;
use crate::limits::{MAX_COORDINATE_M, WATER_POINT_GAP_MIN_M, WATER_POINTS_MAX};
use crate::{Entity, EntityId, World, WorldError};
use gamecraft_math::{Portable, deg_to_rad};
use serde::{Deserialize, Serialize};

/// An area of water: a flat surface at a level over an outline of the ground plane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Water {
    /// Height of the surface above the entity's position, metres, within ±1e6. Default 0;
    /// written only when it is not 0.
    #[serde(default, skip_serializing_if = "is_zero_level")]
    pub level_m: f64,
    /// The outline of the area, `[x, z]` offsets from the entity's position in metres (turned
    /// by its `rotation_y_deg`, times its scale): 3 to 64 points, consecutive points (the last
    /// and the first included) at least 0.01 m apart. Empty (the default): water everywhere,
    /// the sea out to the horizon. Written only when it is not empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outline: Vec<[f64; 2]>,
}

impl Default for Water {
    /// The sea everywhere at the entity's position.
    fn default() -> Self {
        Water {
            level_m: 0.0,
            outline: Vec::new(),
        }
    }
}

// Bit equality: only the exact default is left out, so every other value reads back the same.
fn is_zero_level(v: &f64) -> bool {
    v.to_bits() == 0f64.to_bits()
}

impl Water {
    /// Water `level_m` above its entity over `outline` (empty: everywhere).
    pub fn new(level_m: f64, outline: Vec<[f64; 2]>) -> Water {
        Water { level_m, outline }
    }

    /// The height of the surface in the world: the entity's position plus `level_m`; `None`
    /// when that is not a finite number within the world's coordinate limit.
    pub fn surface_y(&self, entity: &Entity) -> Option<f64> {
        let y = entity.position[1] + self.level_m;
        (y.is_finite() && y.abs() <= MAX_COORDINATE_M).then_some(y)
    }

    /// The outline in world axes: every offset scaled by the entity's scale along x and z and
    /// turned by its yaw, as a road's points are, then moved to the entity's position (its x
    /// and z). Points that are not finite are left out.
    pub fn outline_in_world(&self, entity: &Entity) -> Vec<[f64; 2]> {
        let (sin, cos) = deg_to_rad(entity.rotation_y_deg).psin_cos();
        let [px, _, pz] = entity.position;
        self.outline
            .iter()
            .map(|&[x, z]| {
                let (lx, lz) = (x * entity.scale.x, z * entity.scale.z);
                // Local to world: counter-clockwise seen from above, as the simulation's
                // solids turn.
                let (wx, wz) = (lx * cos + lz * sin, -lx * sin + lz * cos);
                [px + wx, pz + wz]
            })
            .filter(|p| p.iter().all(|v| v.is_finite()))
            .collect()
    }

    pub(crate) fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        ctx.range(
            "water.level_m",
            self.level_m,
            -MAX_COORDINATE_M,
            MAX_COORDINATE_M,
            " metres",
        )?;
        let n = self.outline.len();
        if !self.outline.is_empty() && !(3..=WATER_POINTS_MAX).contains(&n) {
            return Err(ctx.invalid(
                "water.outline",
                format!("{n} points"),
                format!(
                    "an outline has 3 to {WATER_POINTS_MAX} points; leave it out for water \
                     everywhere (the sea)"
                ),
            ));
        }
        for (i, point) in self.outline.iter().enumerate() {
            for (axis, &v) in point.iter().enumerate() {
                ctx.range(
                    format!("water.outline[{i}][{axis}]"),
                    v,
                    -MAX_COORDINATE_M,
                    MAX_COORDINATE_M,
                    " metres",
                )?;
            }
        }
        // Closed outline: the last point meets the first.
        let pairs = self
            .outline
            .windows(2)
            .filter_map(|w| match w {
                [a, b] => Some((*a, *b)),
                _ => None,
            })
            .chain(
                self.outline
                    .first()
                    .zip(self.outline.last())
                    .map(|(a, b)| (*b, *a)),
            );
        for (i, (a, b)) in pairs.enumerate() {
            let gap = ((b[0] - a[0]) * (b[0] - a[0]) + (b[1] - a[1]) * (b[1] - a[1])).sqrt();
            if gap.is_nan() || gap < WATER_POINT_GAP_MIN_M {
                let next = if i + 1 < n { i + 1 } else { 0 };
                return Err(ctx.invalid(
                    format!("water.outline[{next}]"),
                    format!("[{}, {}]", b[0], b[1]),
                    format!(
                        "it is {gap} m from the point before it; consecutive points of an \
                         outline (the last and the first included) must be at least \
                         {WATER_POINT_GAP_MIN_M} m apart"
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// One water area of a world in world axes: what the simulation, the queries and the renderer
/// read.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterArea {
    /// The entity with the `water` component.
    pub entity: EntityId,
    /// The height of the surface, metres.
    pub level: f64,
    /// The outline in world axes; empty: everywhere.
    pub outline: Vec<[f64; 2]>,
}

impl WaterArea {
    /// Whether the area covers the point (`x`, `z`) of the ground plane: everywhere without an
    /// outline, else by the even-odd rule over the outline ([`inside_outline`]). An outline
    /// with fewer than three usable points covers nothing.
    pub fn covers(&self, x: f64, z: f64) -> bool {
        if x.is_nan() || z.is_nan() {
            return false;
        }
        self.outline.is_empty() || inside_outline([x, z], &self.outline)
    }

    /// The depth of the water over a point of the ground `feet` high at (`x`, `z`): `None`
    /// when the area does not cover the point or the ground is at or above the surface.
    pub fn depth_over(&self, x: f64, z: f64, feet: f64) -> Option<f64> {
        if !self.covers(x, z) || !feet.is_finite() {
            return None;
        }
        let depth = self.level - feet;
        (depth > 0.0).then_some(depth)
    }

    /// The area enclosed by the outline in square metres (0 for water everywhere: it has no
    /// end, and the callers treat it apart), by the shoelace formula; its sign says which way
    /// the outline runs.
    pub fn signed_area_m2(&self) -> f64 {
        let n = self.outline.len();
        let mut sum = 0.0;
        for i in 0..n {
            let Some(a) = self.outline.get(i) else {
                continue;
            };
            let Some(b) = self.outline.get((i + 1) % n.max(1)) else {
                continue;
            };
            sum += a[0] * b[1] - b[0] * a[1];
        }
        sum / 2.0
    }
}

/// Every water area of `world`, in id order of their entities, each with its outline in world
/// axes. Entities whose surface is not a usable height are left out.
pub fn water_areas(world: &World) -> Vec<WaterArea> {
    world
        .entities()
        .filter_map(|e| {
            let water = e.water()?;
            let level = water.surface_y(e)?;
            Some(WaterArea {
                entity: e.id,
                level,
                outline: water.outline_in_world(e),
            })
        })
        .collect()
}

/// Of `areas`, the one whose surface counts at (`x`, `z`): the highest level among those that
/// cover the point. `None` on dry land.
pub fn surface_at(areas: &[WaterArea], x: f64, z: f64) -> Option<&WaterArea> {
    let mut best: Option<&WaterArea> = None;
    for area in areas.iter().filter(|a| a.covers(x, z)) {
        if best.is_none_or(|b| area.level > b.level) {
            best = Some(area);
        }
    }
    best
}

/// The depth of the water over a point of the ground `feet` high at (`x`, `z`): that of the
/// surface that counts there ([`surface_at`]); `None` on dry land.
pub fn depth_at(areas: &[WaterArea], x: f64, z: f64, feet: f64) -> Option<f64> {
    surface_at(areas, x, z).and_then(|a| a.depth_over(x, z, feet))
}

/// Whether the point `p` (`[x, z]`) lies inside the closed polygon `outline` (its last point
/// joined to its first), by the even-odd rule: a ray from the point crosses the outline's
/// edges an odd number of times. A point exactly on an edge counts as the rounding decides,
/// alike on every platform (no trigonometry, one guarded division). Fewer than three points:
/// never inside.
pub fn inside_outline(p: [f64; 2], outline: &[[f64; 2]]) -> bool {
    let n = outline.len();
    if n < 3 || p.iter().any(|v| !v.is_finite()) {
        return false;
    }
    let mut inside = false;
    for i in 0..n {
        let (Some(a), Some(b)) = (outline.get(i), outline.get((i + 1) % n)) else {
            continue;
        };
        // An edge whose ends lie on different sides of the point's z crosses the ray the
        // point sends towards +x somewhere: inside when that is to the right of the point.
        // The two z values differ here, so the division is safe.
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let crossing = (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0];
            if crossing.is_finite() && p[0] < crossing {
                inside = !inside;
            }
        }
    }
    inside
}
