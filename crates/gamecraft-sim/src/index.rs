//! A spatial index: a uniform grid over the ground plane of what a ray or a character can
//! meet, so that neither walks every entity of a large world.
//!
//! Every entity with a shape, a model's collider boxes or a road is a member, entered in each
//! cell of the grid its footprint touches. A ray visits the cells its ground projection
//! crosses and tests only their members; a character takes the members of the cells around it.
//! A road is entered in the cells its own segments touch ([`Entity::road`],
//! `Road::segment_boxes`), so a road that crosses a whole town is looked at only where it
//! runs. An entity whose shape or collider touches more cells than [`MAX_CELLS`], or whose
//! road more than [`MAX_ROAD_CELLS`], is tested always, as is a character (it moves every
//! tick): all three are few.
//!
//! The index changes nothing in what is found: the candidates are a superset of what a walk
//! over every entity would hit, the test of each is the same code, and ties are settled by
//! the lowest id as the walk in id order settled them. It is built once from a world and
//! updated for the entities a change touched ([`SpatialIndex::update`]), from the world's own
//! list of changed entities. Deterministic: a `BTreeMap` of cells, candidates sorted by id
//! where order matters, no hash map in anything that reaches a result.

use gamecraft_math::{Portable, deg_to_rad};
use gamecraft_world::{Entity, EntityId, World};
use std::collections::BTreeMap;

/// The side of a cell, metres. Small enough that a kilometre of ray and a character's
/// surroundings look at a few hundred entities in a district, large enough that a 8 km world
/// is a few ten thousand cells.
pub const CELL_M: f64 = 32.0;
/// Most cells the box around an entity's shape and model collider may touch before the entity
/// is tested always.
pub const MAX_CELLS: usize = 256;
/// Most cells a road is entered in (those its segments' boxes touch, each once) before it is
/// tested always: a road of the longest line the file allows (4096 points), run so that it
/// turns back through more cells than any road of a place. Length alone does not reach it: a
/// straight road of ten kilometres is some three hundred cells.
pub const MAX_ROAD_CELLS: usize = 16_384;

/// The smallest box around what an entity's shape, model collider or road occupies on the
/// ground plane: `(min [x, z], max [x, z])`, or `None` for an entity nothing can meet (no
/// shape, no model with bounds, no road with points), or with a number that is not finite.
/// Conservative: what the simulation collides with and the renderer draws lies inside. A road
/// counts by the box around its whole line here; the index enters it by segment
/// ([`MAX_ROAD_CELLS`]).
pub fn footprint(world: &World, e: &Entity) -> Option<([f64; 2], [f64; 2])> {
    let parts = parts_box(world, e);
    let road = e.road().and_then(|road| road.bounds(e));
    let box_ = match (parts, road) {
        (None, None) => return None,
        (Some(b), None) | (None, Some(b)) => b,
        (Some((plo, phi)), Some((rlo, rhi))) => (
            [plo[0].min(rlo[0]), plo[1].min(rlo[1])],
            [phi[0].max(rhi[0]), phi[1].max(rhi[1])],
        ),
    };
    let (lo, hi) = box_;
    (lo.iter().chain(&hi).all(|v| v.is_finite()) && lo[0] <= hi[0] && lo[1] <= hi[1])
        .then_some(box_)
}

/// The smallest box around an entity's shape and model collider on the ground plane:
/// `(min [x, z], max [x, z])`, or `None` when it has neither.
fn parts_box(world: &World, e: &Entity) -> Option<([f64; 2], [f64; 2])> {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut grow = |x0: f64, z0: f64, x1: f64, z1: f64| {
        lo[0] = lo[0].min(x0);
        lo[1] = lo[1].min(z0);
        hi[0] = hi[0].max(x1);
        hi[1] = hi[1].max(z1);
    };
    // A box of half extents (hx, hz) turned by `yaw` about (cx, cz).
    let turned = |cx: f64, cz: f64, hx: f64, hz: f64, yaw: f64| {
        let (s, c) = deg_to_rad(yaw).psin_cos();
        let (ex, ez) = (hx * c.abs() + hz * s.abs(), hx * s.abs() + hz * c.abs());
        (cx - ex, cz - ez, cx + ex, cz + ez)
    };
    let [px, _, pz] = e.position;
    if let Some(shape) = e.shape() {
        let (half, round) = (e.scale.times(shape.size), e.scale.middle() * shape.size[0]);
        let (x0, z0, x1, z1) = match shape.kind {
            gamecraft_world::ShapeKind::Box => {
                turned(px, pz, half[0] / 2.0, half[2] / 2.0, e.rotation_y_deg)
            }
            gamecraft_world::ShapeKind::Cylinder | gamecraft_world::ShapeKind::Sphere => (
                px - round / 2.0,
                pz - round / 2.0,
                px + round / 2.0,
                pz + round / 2.0,
            ),
        };
        grow(x0, z0, x1, z1);
    }
    // The model's collider: its bounding box, or each part of its own geometry.
    world.model_boxes(e, &mut |b| {
        let [cx, _, cz] = b.center;
        let (x0, z0, x1, z1) = turned(cx, cz, b.half[0], b.half[2], b.yaw_deg);
        grow(x0, z0, x1, z1);
    });
    let finite = lo.iter().chain(&hi).all(|v| v.is_finite()) && lo[0] <= hi[0] && lo[1] <= hi[1];
    finite.then_some((lo, hi))
}

/// The cells a footprint touches: `(cx, cz)` from, inclusive, and to, inclusive.
fn cells_of((lo, hi): ([f64; 2], [f64; 2])) -> Option<([i64; 2], [i64; 2])> {
    let at = |v: [f64; 2]| [(v[0] / CELL_M).floor(), (v[1] / CELL_M).floor()];
    let a = at(lo);
    let b = at(hi);
    let (a0, a1) = (
        i64::try_from(a[0] as i128).ok()?,
        i64::try_from(a[1] as i128).ok()?,
    );
    let (b0, b1) = (
        i64::try_from(b[0] as i128).ok()?,
        i64::try_from(b[1] as i128).ok()?,
    );
    Some(([a0, a1], [b0, b1]))
}

/// How many cells a cell range counts (saturating).
fn cell_count(from: [i64; 2], to: [i64; 2]) -> usize {
    let w = (to[0] as i128 - from[0] as i128 + 1).max(0) as u128;
    let h = (to[1] as i128 - from[1] as i128 + 1).max(0) as u128;
    usize::try_from(w.saturating_mul(h)).unwrap_or(usize::MAX)
}

/// Every cell of the range `from` to `to`, inclusive, in `cells`.
fn push_cells(cells: &mut Vec<[i64; 2]>, from: [i64; 2], to: [i64; 2]) {
    for cx in from[0]..=to[0] {
        for cz in from[1]..=to[1] {
            cells.push([cx, cz]);
        }
    }
}

/// Where an entity goes into the index.
enum Placed {
    /// Nothing a ray or a character can meet: not a member.
    Nothing,
    /// A candidate for every query.
    Everywhere,
    /// In these cells, sorted, each once.
    Cells(Vec<[i64; 2]>),
}

/// Where a member is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// In the cells its footprint touches.
    Cells,
    /// Its footprint touches too many cells (a shape's or collider's more than [`MAX_CELLS`],
    /// a road's more than [`MAX_ROAD_CELLS`]), or it is a character (which moves every tick):
    /// a candidate for every query.
    Everywhere,
}

/// One indexed entity.
#[derive(Clone, Debug)]
struct Member {
    id: EntityId,
    place: Place,
    /// The cells it is in (`Place::Cells`).
    cells: Vec<[i64; 2]>,
}

/// A uniform grid over the ground plane of what a ray or a character can meet.
#[derive(Clone, Debug, Default)]
pub struct SpatialIndex {
    cells: BTreeMap<[i64; 2], Vec<u32>>,
    members: Vec<Option<Member>>,
    /// The entity of each member, to find it again when it changes.
    by_entity: BTreeMap<EntityId, u32>,
    /// Slots of members that left, taken first by the next one.
    free: Vec<u32>,
}

impl SpatialIndex {
    /// An empty index.
    pub fn new() -> SpatialIndex {
        SpatialIndex::default()
    }

    /// The index of every entity of `world` that a ray or a character can meet.
    pub fn build(world: &World) -> SpatialIndex {
        let mut index = SpatialIndex::new();
        for e in world.entities() {
            index.insert(world, e);
        }
        index
    }

    /// Brings the index from `before` (the world it was built or last updated for) to `world`:
    /// the entities that are not the very ones of `before` are taken out and put back. When
    /// the two cannot be compared entity by entity (something was removed, the prefabs
    /// changed) it is built again.
    pub fn update(&mut self, before: &World, world: &World) {
        match world.changed_entities_since(before) {
            Some(changed) => {
                for id in changed {
                    self.remove(id);
                    if let Some(e) = world.entity(id) {
                        self.insert(world, e);
                    }
                }
            }
            None => *self = SpatialIndex::build(world),
        }
    }

    /// How many entities the index holds.
    pub fn len(&self) -> usize {
        self.by_entity.len()
    }

    /// Whether the index holds nothing.
    pub fn is_empty(&self) -> bool {
        self.by_entity.is_empty()
    }

    /// How many cells hold something.
    pub fn cells(&self) -> usize {
        self.cells.len()
    }

    /// How many members are found everywhere: the characters, and what is too large for the
    /// grid. A large world should have few: every ray and every step looks at each of them.
    pub fn found_everywhere(&self) -> usize {
        self.everywhere().count()
    }

    /// Puts `e` in (replacing what it had).
    fn insert(&mut self, world: &World, e: &Entity) {
        self.remove(e.id);
        // A character moves every tick, and what has more cells than the grid holds for it
        // has no place: both are found everywhere.
        let (place, cells) = match Self::cells_of_entity(world, e) {
            Placed::Nothing => return,
            _ if e.character().is_some() => (Place::Everywhere, Vec::new()),
            Placed::Everywhere => (Place::Everywhere, Vec::new()),
            Placed::Cells(cells) => (Place::Cells, cells),
        };
        // The world holds at most a million entities: a member always fits.
        let Some(slot) = self
            .free
            .pop()
            .or_else(|| u32::try_from(self.members.len()).ok())
        else {
            return;
        };
        if slot as usize == self.members.len() {
            self.members.push(None);
        }
        for &cell in &cells {
            self.cells.entry(cell).or_default().push(slot);
        }
        if let Some(member) = self.members.get_mut(slot as usize) {
            *member = Some(Member {
                id: e.id,
                place,
                cells,
            });
            self.by_entity.insert(e.id, slot);
        }
    }

    /// Where `e` goes: nothing to meet, everywhere, or in these cells sorted and each once.
    /// The box around its shape and model collider counts as one footprint, at most
    /// [`MAX_CELLS`] cells. A road counts by its segments: the cells each segment's box
    /// ([`gamecraft_world::Road::segment_boxes`]) touches, at most [`MAX_ROAD_CELLS`] in all.
    fn cells_of_entity(world: &World, e: &Entity) -> Placed {
        let mut cells = Vec::new();
        if let Some((lo, hi)) = parts_box(world, e) {
            let Some((from, to)) = cells_of((lo, hi)) else {
                return Placed::Everywhere;
            };
            if cell_count(from, to) > MAX_CELLS {
                return Placed::Everywhere;
            }
            push_cells(&mut cells, from, to);
        }
        let Some(road) = e.road() else {
            return if cells.is_empty() {
                Placed::Nothing
            } else {
                Placed::Cells(cells)
            };
        };
        // The work is bounded before the cells are counted once: a road of many long segments
        // is found everywhere after a few of them.
        let mut work = 0usize;
        for (lo, hi) in road.segment_boxes(e) {
            let Some((from, to)) = cells_of((lo, hi)) else {
                return Placed::Everywhere;
            };
            work = work.saturating_add(cell_count(from, to));
            if work > MAX_ROAD_CELLS.saturating_mul(4) {
                return Placed::Everywhere;
            }
            push_cells(&mut cells, from, to);
        }
        cells.sort_unstable();
        cells.dedup();
        if cells.len() > MAX_ROAD_CELLS {
            return Placed::Everywhere;
        }
        Placed::Cells(cells)
    }

    /// Takes the entity `id` out.
    fn remove(&mut self, id: EntityId) {
        let Some(slot) = self.by_entity.remove(&id) else {
            return;
        };
        if let Some(member) = self.members.get_mut(slot as usize).and_then(Option::take) {
            for cell in member.cells {
                if let Some(list) = self.cells.get_mut(&cell) {
                    list.retain(|&s| s != slot);
                    if list.is_empty() {
                        self.cells.remove(&cell);
                    }
                }
            }
        }
        self.free.push(slot);
    }

    /// The members in the cells the box `(lo, hi)` touches, as they come (an entity in two
    /// of its cells twice; [`SpatialIndex::near`] sorts and deduplicates). `None` when the
    /// box touches more cells than there are members to look at, four times over.
    fn members_in_box(&self, lo: [f64; 2], hi: [f64; 2]) -> Option<Vec<EntityId>> {
        let (from, to) = cells_of((lo, hi))?;
        if cell_count(from, to) > self.len().saturating_mul(4).max(64) {
            return None;
        }
        let mut out = Vec::new();
        for cx in from[0]..=to[0] {
            for cz in from[1]..=to[1] {
                if let Some(list) = self.cells.get(&[cx, cz]) {
                    out.extend(list.iter().map(|&s| self.entity_of(s)));
                }
            }
        }
        Some(out)
    }

    /// The entity of a member.
    fn entity_of(&self, slot: u32) -> EntityId {
        self.members
            .get(slot as usize)
            .and_then(Option::as_ref)
            .map_or(EntityId(u64::MAX), |m| m.id)
    }

    /// The candidates for a ray from `origin` along the unit `dir` up to `max_m`: every entity
    /// whose footprint could meet the segment (a superset of what it hits), sorted by id, each
    /// once. `None` when walking the cells would be more work than looking at every entity.
    pub fn ray_candidates(
        &self,
        origin: [f64; 3],
        dir: [f64; 3],
        max_m: f64,
    ) -> Option<Vec<EntityId>> {
        // The ground projection of the segment.
        let (ox, oz) = (origin[0], origin[2]);
        let (dx, dz) = (dir[0], dir[2]);
        let end = [ox + dx * max_m, oz + dz * max_m];
        if ![ox, oz, end[0], end[1]].iter().all(|v| v.is_finite()) {
            return None;
        }
        // Fewer cells along the ray than a share of the members per cell it crosses: worth it.
        let span = (((end[0] - ox) / CELL_M).abs() + ((end[1] - oz) / CELL_M).abs()).ceil();
        let cells_along = (span as usize).saturating_add(2);
        if cells_along > self.len().saturating_mul(4).max(64) {
            return None;
        }
        let at = |x: f64, z: f64| [(x / CELL_M).floor() as i128, (z / CELL_M).floor() as i128];
        let cell = |c: [i128; 2]| -> Option<[i64; 2]> {
            Some([i64::try_from(c[0]).ok()?, i64::try_from(c[1]).ok()?])
        };
        let mut out = Vec::new();
        let visit = |index: &Self, c: [i128; 2], out: &mut Vec<EntityId>| {
            if let Some(key) = cell(c)
                && let Some(list) = index.cells.get(&key)
            {
                out.extend(list.iter().map(|&s| index.entity_of(s)));
            }
        };
        // Amanatides and Woo's walk: the parameter of the next crossing of each grid line.
        let (mut t_max_x, mut t_max_z, mut t_delta_x, mut t_delta_z, mut step_x, mut step_z) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            0i128,
            0i128,
        );
        let mut c = at(ox, oz);
        if dx != 0.0 {
            step_x = if dx > 0.0 { 1 } else { -1 };
            let next = if dx > 0.0 {
                c[0].saturating_add(1) as f64 * CELL_M
            } else {
                c[0] as f64 * CELL_M
            };
            t_max_x = ((next - ox) / dx / max_m).clamp(0.0, 1.0);
            t_delta_x = (CELL_M / dx.abs() / max_m).abs();
        }
        if dz != 0.0 {
            step_z = if dz > 0.0 { 1 } else { -1 };
            let next = if dz > 0.0 {
                c[1].saturating_add(1) as f64 * CELL_M
            } else {
                c[1] as f64 * CELL_M
            };
            t_max_z = ((next - oz) / dz / max_m).clamp(0.0, 1.0);
            t_delta_z = (CELL_M / dz.abs() / max_m).abs();
        }
        // A vertical ray stays in one cell; a finite one stops at `max_m`.
        let mut guard = 0usize;
        visit(self, c, &mut out);
        // A ray that keeps to one grid row (or column) never crosses the other's lines: its
        // parameter stays infinite, and the first of the two decides.
        while t_max_x.min(t_max_z) <= 1.0 {
            if t_max_x < t_max_z {
                c[0] = c[0].saturating_add(step_x);
                t_max_x += t_delta_x;
            } else {
                c[1] = c[1].saturating_add(step_z);
                t_max_z += t_delta_z;
            }
            visit(self, c, &mut out);
            guard += 1;
            if guard > cells_along.saturating_mul(2).max(64) {
                // More cells than the span promised (a diagonal of rounding): every member.
                return None;
            }
        }
        out.extend(self.everywhere());
        out.sort_unstable();
        out.dedup();
        Some(out)
    }

    /// The members found everywhere (too large for the grid, or characters).
    fn everywhere(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.members
            .iter()
            .flatten()
            .filter(|m| m.place != Place::Cells)
            .map(|m| m.id)
    }

    /// The entities a character at `p` (x, z) with `reach` metres around it could meet in a
    /// tick: the members of the cells the square touches, and those found everywhere. Sorted
    /// by id, each once (the order in which solids are applied changes a push).
    pub fn near(&self, p: [f64; 2], reach: f64) -> Vec<EntityId> {
        let mut out = self
            .members_in_box([p[0] - reach, p[1] - reach], [p[0] + reach, p[1] + reach])
            .unwrap_or_else(|| self.members.iter().flatten().map(|m| m.id).collect());
        out.extend(self.everywhere());
        out.sort_unstable();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamecraft_world::{Component, EntityDraft, Shape, ShapeKind};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn footprints_hold_what_they_describe() {
        let mut world = World::new("w", 1);
        let add = |world: &mut World, kind: ShapeKind, size: [f64; 3], yaw: f64| {
            world
                .add_entity(EntityDraft {
                    position: [10.0, 0.0, -4.0],
                    rotation_y_deg: yaw,
                    components: vec![Component::Shape(Shape {
                        visible: true,
                        kind,
                        size,
                        ..Shape::default()
                    })],
                    ..EntityDraft::default()
                })
                .unwrap()
        };
        let mut box_ = |kind, size, yaw| {
            let id = add(&mut world, kind, size, yaw);
            footprint(&world, world.entity(id).unwrap()).unwrap()
        };
        // An axis-aligned box: exactly its extents.
        let (lo, hi) = box_(ShapeKind::Box, [2.0, 1.0, 6.0], 0.0);
        assert!(
            close(lo[0], 9.0) && close(hi[0], 11.0) && close(lo[1], -7.0) && close(hi[1], -1.0),
            "{lo:?} {hi:?}"
        );
        // Turned 90 degrees: the long side lies along x.
        let (lo, hi) = box_(ShapeKind::Box, [2.0, 1.0, 6.0], 90.0);
        assert!(
            close(lo[0], 7.0) && close(hi[0], 13.0) && close(lo[1], -5.0) && close(hi[1], -3.0),
            "{lo:?} {hi:?}"
        );
        // A sphere: as wide along z as along x.
        let (lo, hi) = box_(ShapeKind::Sphere, [2.0, 2.0, 2.0], 0.0);
        assert!(close(hi[1] - lo[1], 2.0));
        // Nothing to meet.
        let id = world.add_entity(EntityDraft::default()).unwrap();
        assert!(footprint(&world, world.entity(id).unwrap()).is_none());
    }

    #[test]
    fn hostile_rays_and_places_do_not_panic() {
        // A ray from beyond any world (its cell coordinate saturates), or with a direction
        // or a reach that is extreme: an answer, never a panic.
        let mut world = World::new("w", 1);
        world
            .add_entity(EntityDraft {
                position: [0.0, 0.0, 0.0],
                components: vec![Component::Shape(Shape {
                    visible: true,
                    kind: ShapeKind::Box,
                    size: [4.0, 3.0, 4.0],
                    ..Shape::default()
                })],
                ..EntityDraft::default()
            })
            .unwrap();
        let index = SpatialIndex::build(&world);
        let _ = index.ray_candidates(
            [f64::MAX, 1.0, f64::MIN_POSITIVE],
            [f64::MIN_POSITIVE, 0.0, -f64::MAX],
            f64::MAX,
        );
        let _ = index.ray_candidates([-f64::MAX; 3], [0.0, -1.0, 0.0], 1e300);
        let _ = index.ray_candidates([0.0; 3], [1e-300, 0.0, 1e300], 100.0);
        let _ = index.ray_candidates([0.5, 0.0, 0.5], [0.0, 0.0, 0.0], 100.0);
        let _ = index.near([f64::MAX, -f64::MAX], f64::MAX);
        let _ = index.near([0.0, 0.0], f64::NAN);
        // On the entity, the honest rays still find it.
        assert_eq!(
            index.ray_candidates([-10.0, 1.5, 0.0], [1.0, 0.0, 0.0], 20.0),
            Some(vec![EntityId(1)])
        );
    }

    #[test]
    fn a_ray_meets_the_same_as_a_walk_over_every_entity() {
        let mut world = World::new("w", 1);
        let mut ids = Vec::new();
        for i in 0..40 {
            let x = -300.0 + 15.0 * (i as f64);
            let id = world
                .add_entity(EntityDraft {
                    position: [x, 0.0, 20.0 * ((i % 3) as f64 - 1.0)],
                    components: vec![Component::Shape(Shape {
                        visible: true,
                        kind: ShapeKind::Box,
                        size: [4.0, 3.0, 4.0],
                        ..Shape::default()
                    })],
                    ..EntityDraft::default()
                })
                .unwrap();
            ids.push(id);
        }
        let index = SpatialIndex::build(&world);
        assert_eq!(index.len(), 40);
        // A ray along the row z = 0: the boxes on that row, and those 20 m beside it that
        // share its cell (a candidate is more than a hit), and none of those 20 m below.
        let c = index
            .ray_candidates([-400.0, 1.0, 0.0], [1.0, 0.0, 0.0], 1000.0)
            .unwrap();
        assert_eq!(c.len(), 26, "{c:?}");
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(c.contains(id), i % 3 != 0, "box {i}");
        }
        // The same length looking down the column x = -300: the one box in its cell, none
        // of the next column's.
        let c = index
            .ray_candidates([-300.0, 1.0, 500.0], [0.0, 0.0, -1.0], 1000.0)
            .unwrap();
        assert_eq!(c, vec![ids[0]], "{c:?}");
        // A slanted ray crosses rows and columns.
        let c = index
            .ray_candidates(
                [-320.0, 1.0, -40.0],
                [1.0, 0.0, 1.0].map(|v| v / 2f64.sqrt()),
                500.0,
            )
            .unwrap();
        assert!(
            c.iter().all(|id| ids.contains(id)) && !c.is_empty(),
            "{c:?}"
        );
        // A ray away from the row has no candidate.
        let c = index
            .ray_candidates([-400.0, 1.0, 500.0], [1.0, 0.0, 0.0], 100.0)
            .unwrap();
        assert!(c.is_empty());
        // A vertical ray sees the cell under it only: straight over the first box, it.
        let c = index
            .ray_candidates([-300.0, 50.0, -20.0], [0.0, -1.0, 0.0], 60.0)
            .unwrap();
        assert_eq!(c, vec![ids[0]], "{c:?}");
        let c = index
            .ray_candidates([-300.0, 50.0, 0.0], [0.0, -1.0, 0.0], 60.0)
            .unwrap();
        assert!(
            c.is_empty(),
            "nothing stands at z = 0 on that column: {c:?}"
        );
    }
}
