//! Tree trunks as the simulation sees them: derived collision data, never part of the world
//! document.
//!
//! A trunk is an upright round column standing on a plant entity's position. The engine
//! (which knows the plant library) works the columns out from the same placement the renderer
//! draws and hands them over as a [`Trunks`] through a [`TrunkSource`]; the simulation
//! rebuilds them whenever the running world is replaced. They block characters (the same push
//! as a solid cylinder), stop rays and projectiles ([`crate::ray::cast`]), and give cover.
//!
//! A uniform grid of [`TRUNK_CELL_M`] cells keeps thousands of trunks cheap: a trunk is listed
//! in every cell its bounding square touches, a character looks at the cells around it, a ray
//! walks the cells it crosses (at most [`MAX_RAY_CELLS`] of them, so a ray reaches at least
//! [`MAX_RAY_CELLS`] times [`TRUNK_CELL_M`] metres, 32 km, horizontally).

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

use gamecraft_math::StateHasher;
use gamecraft_world::{EntityId, World};

use crate::ray::{RayHit, normalize};
use crate::solid::Solid;

/// Side of a grid cell, metres.
pub const TRUNK_CELL_M: f64 = 8.0;
/// Most trunks one world has; more are left out.
pub const MAX_TRUNKS: usize = 1_000_000;
/// The largest trunk radius, metres; a larger one is cut to it.
pub const MAX_TRUNK_RADIUS_M: f64 = 8.0;
/// Most grid cells one ray walks.
pub const MAX_RAY_CELLS: usize = 4096;

/// One trunk, as the engine describes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trunk {
    /// The plant entity it belongs to.
    pub entity: EntityId,
    /// World position of the centre of its base.
    pub position: [f64; 3],
    /// Radius, metres.
    pub radius_m: f64,
    /// Height above the base, metres.
    pub height_m: f64,
}

/// Where the trunks of a world come from: the engine implements it with the plant library.
pub trait TrunkSource: Send + Sync + Debug {
    /// The trunks of `world`: derived, deterministic, in a stable order.
    fn trunks_of(&self, world: &World) -> Arc<Trunks>;
}

type Cell = (i64, i64);

/// The trunks of a world with their grid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trunks {
    /// In entity id order.
    solids: Vec<Solid>,
    cells: BTreeMap<Cell, Vec<u32>>,
    /// A hash of every trunk, for the simulation's state hash; 0 without trunks.
    digest: u64,
}

fn cell_of(v: f64) -> i64 {
    // Positions are bounded by the world document; the clamp keeps the cast defined anyway.
    (v / TRUNK_CELL_M).floor().clamp(-1e12, 1e12) as i64
}

impl Trunks {
    /// The trunks in `list`: those with a finite position and a positive radius and height,
    /// sorted by entity id (the first of equal ids wins), at most [`MAX_TRUNKS`].
    pub fn new(list: impl IntoIterator<Item = Trunk>) -> Trunks {
        let mut solids: Vec<Solid> = list
            .into_iter()
            .filter_map(|t| {
                Solid::column(
                    t.entity,
                    t.position,
                    // NaN.min(x) is x: a radius that is not a number must stay one.
                    if t.radius_m.is_nan() {
                        t.radius_m
                    } else {
                        t.radius_m.min(MAX_TRUNK_RADIUS_M)
                    },
                    t.height_m,
                )
            })
            .collect();
        solids.sort_by_key(|s| s.entity);
        solids.dedup_by_key(|s| s.entity);
        solids.truncate(MAX_TRUNKS);
        let mut cells: BTreeMap<Cell, Vec<u32>> = BTreeMap::new();
        for (i, s) in solids.iter().enumerate() {
            let [x, z] = s.centre();
            let r = s.reach();
            for cx in cell_of(x - r)..=cell_of(x + r) {
                for cz in cell_of(z - r)..=cell_of(z + r) {
                    cells.entry((cx, cz)).or_default().push(i as u32);
                }
            }
        }
        let mut hasher = StateHasher::new();
        let mut digest = 0;
        if !solids.is_empty() {
            for t in solids.iter().filter_map(Self::describe) {
                hasher.write_u64(t.entity.0);
                for v in t.position.into_iter().chain([t.radius_m, t.height_m]) {
                    hasher.write_f64(v);
                }
            }
            digest = hasher.finish();
        }
        Trunks {
            solids,
            cells,
            digest,
        }
    }

    /// A hash of every trunk (entity, position, radius, height); 0 without trunks.
    pub fn digest(&self) -> u64 {
        self.digest
    }

    /// How many trunks there are.
    pub fn len(&self) -> usize {
        self.solids.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.solids.is_empty()
    }

    /// Every trunk, in entity id order.
    pub fn iter(&self) -> impl Iterator<Item = Trunk> + '_ {
        self.solids.iter().filter_map(Self::describe)
    }

    fn describe(s: &Solid) -> Option<Trunk> {
        let (radius_m, height_m) = s.column_size()?;
        let [x, z] = s.centre();
        Some(Trunk {
            entity: s.entity,
            position: [x, s.base(), z],
            radius_m,
            height_m,
        })
    }

    /// The trunk of plant entity `id`.
    pub fn get(&self, id: EntityId) -> Option<Trunk> {
        self.find(id).and_then(Self::describe)
    }

    pub(crate) fn find(&self, id: EntityId) -> Option<&Solid> {
        self.solids
            .binary_search_by_key(&id, |s| s.entity)
            .ok()
            .and_then(|i| self.solids.get(i))
    }

    /// Whether `id` is a plant entity with a trunk.
    pub fn contains(&self, id: EntityId) -> bool {
        self.find(id).is_some()
    }

    fn at(&self, indices: Vec<u32>) -> Vec<&Solid> {
        indices
            .into_iter()
            .filter_map(|i| self.solids.get(i as usize))
            .collect()
    }

    /// The trunks listed in the cells that touch the square of half side `reach` around
    /// (`x`, `z`): a superset of those within `reach`, in id order. The broad phase.
    pub(crate) fn near(&self, x: f64, z: f64, reach: f64) -> Vec<&Solid> {
        if self.is_empty() || !(x.is_finite() && z.is_finite() && reach.is_finite()) {
            return Vec::new();
        }
        let reach = reach.max(0.0);
        let mut found: Vec<u32> = Vec::new();
        for cx in cell_of(x - reach)..=cell_of(x + reach) {
            for cz in cell_of(z - reach)..=cell_of(z + reach) {
                if let Some(list) = self.cells.get(&(cx, cz)) {
                    found.extend_from_slice(list);
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        self.at(found)
    }

    /// How many trunks the broad phase looks at for (`x`, `z`) and `reach`: what a query costs.
    pub fn candidates_near(&self, x: f64, z: f64, reach: f64) -> usize {
        self.near(x, z, reach).len()
    }

    /// The trunks whose surface is within `radius_m` of (`x`, `z`) horizontally, nearest
    /// first (equal distances in id order), with their distance from the surface (0 when the
    /// point is inside).
    pub fn within(&self, x: f64, z: f64, radius_m: f64) -> Vec<(Trunk, f64)> {
        let mut out: Vec<(Trunk, f64)> = self
            .near(x, z, radius_m + MAX_TRUNK_RADIUS_M)
            .into_iter()
            .filter_map(|s| {
                let trunk = Self::describe(s)?;
                let [cx, cz] = s.centre();
                let (dx, dz) = (cx - x, cz - z);
                let d = (dx * dx + dz * dz).sqrt() - trunk.radius_m;
                (d.is_finite() && d <= radius_m).then_some((trunk, d.max(0.0)))
            })
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        out
    }

    /// Where a character's circle (centre `p`, `radius`) with its feet at `feet` and `height`
    /// tall must go to leave the trunks it overlaps, in one pass over the trunks near it;
    /// `None` when none blocks it.
    pub(crate) fn push(
        &self,
        p: [f64; 2],
        radius: f64,
        feet: f64,
        height: f64,
    ) -> Option<[f64; 2]> {
        let mut at = p;
        let mut moved = false;
        for s in self.near(p[0], p[1], radius) {
            if let Some(q) = s.push(at, radius, feet, height) {
                at = q;
                moved = true;
            }
        }
        moved.then_some(at)
    }

    /// The nearest trunk a ray meets within `max_m`, `skip` naming the entities to ignore. The
    /// direction need not be a unit vector.
    pub fn cast(
        &self,
        origin: [f64; 3],
        dir: [f64; 3],
        max_m: f64,
        skip: impl Fn(EntityId) -> bool,
    ) -> Option<(EntityId, RayHit)> {
        if self.is_empty() || !(max_m.is_finite() && max_m >= 0.0) {
            return None;
        }
        let dir = normalize(dir)?;
        let mut found: Vec<u32> = Vec::new();
        for cell in ray_cells(origin, dir, max_m) {
            if let Some(list) = self.cells.get(&cell) {
                found.extend_from_slice(list);
            }
        }
        found.sort_unstable();
        found.dedup();
        let mut best: Option<(EntityId, RayHit)> = None;
        for s in self.at(found) {
            if skip(s.entity) {
                continue;
            }
            let Some(hit) = s.column_ray(origin, dir) else {
                continue;
            };
            if hit.distance_m <= max_m && best.is_none_or(|(_, b)| hit.distance_m < b.distance_m) {
                best = Some((s.entity, hit));
            }
        }
        best
    }
}

/// The grid cells a ray from `origin` along the unit `dir` crosses in its first `max_m`
/// metres, in order (Amanatides and Woo), at most [`MAX_RAY_CELLS`].
fn ray_cells(origin: [f64; 3], dir: [f64; 3], max_m: f64) -> Vec<Cell> {
    let [ox, _, oz] = origin;
    let [dx, _, dz] = dir;
    let mut out = Vec::new();
    if !(ox.is_finite() && oz.is_finite()) {
        return out;
    }
    let (mut cx, mut cz) = (cell_of(ox), cell_of(oz));
    let axis = |o: f64, d: f64, c: i64| -> (i64, f64, f64) {
        if d > 0.0 {
            (1, TRUNK_CELL_M / d, ((c + 1) as f64 * TRUNK_CELL_M - o) / d)
        } else if d < 0.0 {
            (-1, TRUNK_CELL_M / -d, (c as f64 * TRUNK_CELL_M - o) / d)
        } else {
            (0, f64::INFINITY, f64::INFINITY)
        }
    };
    let (step_x, delta_x, mut next_x) = axis(ox, dx, cx);
    let (step_z, delta_z, mut next_z) = axis(oz, dz, cz);
    while out.len() < MAX_RAY_CELLS {
        out.push((cx, cz));
        if next_x.min(next_z) > max_m {
            break;
        }
        if next_x <= next_z {
            cx += step_x;
            next_x += delta_x;
        } else {
            cz += step_z;
            next_z += delta_z;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trunk(id: u64, x: f64, z: f64) -> Trunk {
        Trunk {
            entity: EntityId(id),
            position: [x, 0.0, z],
            radius_m: 0.3,
            height_m: 4.0,
        }
    }

    #[test]
    fn a_ray_walks_the_cells_it_crosses() {
        let cells = ray_cells([1.0, 0.0, 1.0], [1.0, 0.0, 0.0], 20.0);
        assert_eq!(cells, vec![(0, 0), (1, 0), (2, 0)]);
        let up = ray_cells([1.0, 0.0, 1.0], [0.0, 1.0, 0.0], 20.0);
        assert_eq!(up, vec![(0, 0)]);
    }

    #[test]
    fn bad_trunks_are_dropped_and_ids_are_sorted() {
        let mut bad = trunk(9, 0.0, 0.0);
        bad.radius_m = f64::NAN;
        let t = Trunks::new([
            trunk(3, 5.0, 0.0),
            bad,
            trunk(1, 0.0, 0.0),
            trunk(1, 9.0, 9.0),
        ]);
        let ids: Vec<u64> = t.iter().map(|t| t.entity.0).collect();
        assert_eq!(ids, vec![1, 3]);
    }

    #[test]
    fn a_trunk_on_a_cell_border_is_found_from_both_sides() {
        let t = Trunks::new([trunk(1, 8.0, 0.0)]);
        assert_eq!(t.candidates_near(7.9, 0.1, 0.0), 1);
        assert_eq!(t.candidates_near(8.1, 0.1, 0.0), 1);
        let hit = t.cast([-5.0, 1.0, 7.9], [1.0, 0.0, 0.0], 30.0, |_| false);
        assert!(hit.is_none(), "passes 7.9 m beside a trunk of 0.3 m");
        let hit = t.cast([-5.0, 1.0, 0.1], [1.0, 0.0, 0.0], 30.0, |_| false);
        assert!(hit.is_some());
    }
}
