//! What the generators of a world place: derived from the recipe and the terrain, held in
//! memory, never written to a file (`docs/design/generators.md`).
//!
//! Placing runs in two stages, so that a change to the terrain or to the excludes moves only
//! the instances it touches:
//!
//! 1. **Candidates**, from the generator's area, seed, density, spacing and items only:
//!    `requested` points drawn evenly over the area by a seeded stream (each kept only when it
//!    is at least `min_distance_m` from those kept before it, 30 draws per point at most), each
//!    with its item (by weight), plant seed, growth class, scale and turn from a second stream.
//!    Candidate `n` is the instance `<id>#<n>`, whatever happens to the others.
//! 2. **Placing**, a test of each candidate on its own: not inside an exclude, ground under it,
//!    no steeper than `slope_max_deg`, within `height_range_m`; it stands on the ground.
//!
//! So editing the terrain under a forest keeps every other tree where it stood, and keeping a
//! spot clear (or baking it into entities) leaves the rest with their numbers.

use crate::generator::{GROWTH_CLASSES, Generator, GeneratorItem};
use crate::{Component, Entity, EntityId, HeightField, ModelBounds, Prefab, Terrain};
use gamecraft_math::{Portable, Rng, StateHasher, rad_to_deg};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;
use std::fmt;
use std::sync::Arc;

/// Draws per wanted candidate before a generator gives up on the rest (a spacing too wide for
/// the density, a polygon thin in its bounding box).
const TRIES_PER_CANDIDATE: u64 = 30;
/// Most draws one generator makes.
const MAX_TRIES: u64 = 50_000_000;
/// Side of a cell of the grid that finds the instances near a point or a ray, metres.
pub const GRID_CELL_M: f64 = 16.0;
/// Most grid cells a ray visits in one generator (a ray longer than the area is cut to it).
const MAX_RAY_CELLS: usize = 200_000;
/// The stream labels of the two stages.
const STREAM_POINTS: u64 = 0x9E3C_0001;
const STREAM_ITEMS: u64 = 0x9E3C_0002;

/// One placed instance of a generator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    /// Its number `n`, from 1: the instance is `<generator id>#<n>`. Numbers are the
    /// candidates' and have gaps where the terrain or an exclude turned a candidate away.
    pub number: u32,
    /// The index of its item in the generator's `items`.
    pub item: u16,
    /// Which of the item's plant seeds it takes (0 without a plant).
    pub variant: u8,
    /// Which growth class of the item's `growth_range` it takes (0 without one).
    pub growth_class: u8,
    /// Its base on the ground, world metres.
    pub position: [f64; 3],
    /// Its turn about +Y, degrees.
    pub rotation_y_deg: f64,
    /// Its uniform scale.
    pub scale: f64,
}

/// A candidate of the first stage: everything of an instance but its height.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Candidate {
    number: u32,
    item: u16,
    variant: u8,
    growth_class: u8,
    point: [f64; 2],
    rotation_y_deg: f64,
    scale: f64,
}

/// A tree's trunk where it stands: an upright cylinder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrunkAt {
    /// The centre of its base, world metres.
    pub base: [f64; 3],
    /// Its radius, metres.
    pub radius_m: f64,
    /// Its height, metres.
    pub height_m: f64,
}

/// What an item's instances are, for the simulation and the rays.
#[derive(Clone, Debug, PartialEq)]
struct ItemBody {
    /// The components of an instance before its plant's seed and growth.
    components: Vec<Component>,
    /// Whether the components hold a shape or a model (something besides the trunk to hit).
    has_body: bool,
    /// The farthest a part of an instance at scale 1 reaches from its base, horizontally.
    reach_m: f64,
}

/// The placed instances of one generator, with the recipe and terrain they came from.
#[derive(Clone, PartialEq)]
pub struct GeneratedSet {
    generator: Generator,
    terrain: Option<Terrain>,
    candidates: Arc<Vec<Candidate>>,
    instances: Vec<Instance>,
    items: Vec<ItemBody>,
    /// Cells of [`GRID_CELL_M`] to the indices of the instances whose base is in them.
    grid: BTreeMap<(i64, i64), Vec<u32>>,
    /// The farthest any instance reaches from its base horizontally, metres.
    reach_m: f64,
    stamp: u64,
}

impl fmt::Debug for GeneratedSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedSet")
            .field("id", &self.generator.id)
            .field("candidates", &self.candidates.len())
            .field("instances", &self.instances.len())
            .finish()
    }
}

/// A count and the bounding rectangle of a set of instances.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footprint {
    /// The smallest corner `[x, z]`.
    pub min: [f64; 2],
    /// The largest corner `[x, z]`.
    pub max: [f64; 2],
}

impl GeneratedSet {
    /// The recipe the instances were placed from.
    pub fn generator(&self) -> &Generator {
        &self.generator
    }

    /// The candidates of the first stage: the instances before the terrain and the excludes.
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// The placed instances, by number.
    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    /// The instance with this number.
    pub fn instance(&self, number: u32) -> Option<&Instance> {
        let i = self
            .instances
            .binary_search_by_key(&number, |i| i.number)
            .ok()?;
        self.instances.get(i)
    }

    /// A hash of the instances (their numbers, items, seeds, places, turns and scales): equal
    /// sets have equal stamps.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// The farthest a part of any instance reaches from its base, horizontally, metres.
    pub fn reach_m(&self) -> f64 {
        self.reach_m
    }

    /// The bounding rectangle of the instances' bases, `None` without instances.
    pub fn footprint(&self) -> Option<Footprint> {
        footprint(
            self.instances
                .iter()
                .map(|i| [i.position[0], i.position[2]]),
        )
    }

    /// How many instances each item has, in the order of `items`.
    pub fn per_item(&self) -> Vec<usize> {
        let mut out = vec![0usize; self.generator.items.len()];
        for i in &self.instances {
            if let Some(n) = out.get_mut(usize::from(i.item)) {
                *n += 1;
            }
        }
        out
    }

    /// The id of an instance, `<generator id>#<n>`.
    pub fn instance_id(&self, instance: &Instance) -> String {
        format!("{}#{}", self.generator.id, instance.number)
    }

    fn item(&self, instance: &Instance) -> Option<&GeneratorItem> {
        self.generator.items.get(usize::from(instance.item))
    }

    /// The instance's components: its item's (or its item's prefab's), its plant with the
    /// seed and growth the instance took.
    pub fn components(&self, instance: &Instance) -> Vec<Component> {
        let Some(body) = self.items.get(usize::from(instance.item)) else {
            return Vec::new();
        };
        let mut components = body.components.clone();
        if let Some(item) = self.item(instance) {
            for c in &mut components {
                if let Component::Plant(p) = c {
                    p.seed = p.seed.wrapping_add(u64::from(instance.variant));
                    if let Some(g) = item.class_growth(u32::from(instance.growth_class)) {
                        p.growth = g;
                    }
                }
            }
        }
        components
    }

    /// The instance as an entity (id 0, named `<id>#<n>`, in no layer): what
    /// `generator.bake` adds, and what the rays and the simulation see of it.
    pub fn entity(&self, instance: &Instance) -> Entity {
        Entity {
            id: EntityId(0),
            name: self.instance_id(instance),
            position: instance.position,
            rotation_y_deg: instance.rotation_y_deg,
            scale: instance.scale,
            prefab: None,
            parent: None,
            layer: None,
            components: self.components(instance),
        }
    }

    /// Whether the instance has a shape or a model besides its trunk.
    pub fn has_body(&self, instance: &Instance) -> bool {
        self.items
            .get(usize::from(instance.item))
            .is_some_and(|b| b.has_body)
    }

    /// The instance's trunk, sized by its scale and growth; `None` for an item without one.
    pub fn trunk(&self, instance: &Instance) -> Option<TrunkAt> {
        let item = self.item(instance)?;
        let trunk = self.generator.trunk_of(item)?;
        let growth = item
            .class_growth(u32::from(instance.growth_class))
            .or_else(|| {
                self.items
                    .get(usize::from(instance.item))
                    .and_then(|b| plant_growth(&b.components))
            })
            .unwrap_or(1.0);
        let k = instance.scale * growth;
        let at = TrunkAt {
            base: instance.position,
            radius_m: trunk.radius_m * k,
            height_m: trunk.height_m * k,
        };
        (at.radius_m > 0.0 && at.height_m > 0.0 && at.radius_m.is_finite()).then_some(at)
    }

    /// The instances whose base lies within `radius_m` plus [`GeneratedSet::reach_m`] of
    /// `[x, z]` (so everything that can reach the circle), by number.
    pub fn near(&self, [x, z]: [f64; 2], radius_m: f64) -> Vec<&Instance> {
        let r = radius_m.max(0.0) + self.reach_m;
        if !(x.is_finite() && z.is_finite() && r.is_finite()) {
            return Vec::new();
        }
        let (Some((c0, r0)), Some((c1, r1))) = (cell_of([x - r, z - r]), cell_of([x + r, z + r]))
        else {
            return Vec::new();
        };
        let span = (c1.saturating_sub(c0)).saturating_add(1);
        let rows = (r1.saturating_sub(r0)).saturating_add(1);
        let mut slots: Vec<u32> = Vec::new();
        if span.saturating_mul(rows) as usize > self.grid.len() {
            for (&(cx, cz), v) in &self.grid {
                if (c0..=c1).contains(&cx) && (r0..=r1).contains(&cz) {
                    slots.extend(v);
                }
            }
        } else {
            for cx in c0..=c1 {
                for cz in r0..=r1 {
                    if let Some(v) = self.grid.get(&(cx, cz)) {
                        slots.extend(v);
                    }
                }
            }
        }
        slots.sort_unstable();
        let r2 = r * r;
        slots
            .into_iter()
            .filter_map(|s| self.instances.get(s as usize))
            .filter(|i| {
                let (dx, dz) = (i.position[0] - x, i.position[2] - z);
                dx * dx + dz * dz <= r2
            })
            .collect()
    }

    /// The instances that can be met by the ray from `origin` along `dir` (unit length)
    /// within `max_m`: those in the grid cells the ray crosses, widened by
    /// [`GeneratedSet::reach_m`], by number. A superset; the caller tests each.
    pub fn along(&self, origin: [f64; 3], dir: [f64; 3], max_m: f64) -> Vec<&Instance> {
        let Some(fp) = self.footprint() else {
            return Vec::new();
        };
        let pad = self.reach_m + GRID_CELL_M;
        let (lo, hi) = (
            [fp.min[0] - pad, fp.min[1] - pad],
            [fp.max[0] + pad, fp.max[1] + pad],
        );
        // Cut the ray to the rectangle of the instances.
        let [ox, _, oz] = origin;
        let [dx, _, dz] = dir;
        let mut t0: f64 = 0.0;
        let mut t1: f64 = max_m;
        for (o, d, l, h) in [(ox, dx, lo[0], hi[0]), (oz, dz, lo[1], hi[1])] {
            if d == 0.0 {
                if o < l || o > h {
                    return Vec::new();
                }
                continue;
            }
            let (a, b) = ((l - o) / d, (h - o) / d);
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            t0 = t0.max(a);
            t1 = t1.min(b);
        }
        if !(t0.is_finite() && t1.is_finite()) || t0 > t1 {
            return Vec::new();
        }
        let horizontal = (dx * dx + dz * dz).sqrt();
        let step = GRID_CELL_M / 2.0;
        let steps = if horizontal > 0.0 {
            ((t1 - t0) * horizontal / step).floor() + 1.0
        } else {
            1.0
        };
        let widen = (self.reach_m / GRID_CELL_M).floor() as i64 + 1;
        let mut cells: BTreeSet<(i64, i64)> = BTreeSet::new();
        let steps = if steps.is_finite() {
            (steps as u64).min(MAX_RAY_CELLS as u64)
        } else {
            0
        };
        for k in 0..=steps {
            let t = if steps == 0 {
                t0
            } else {
                t0 + (t1 - t0) * (k as f64 / steps as f64)
            };
            let Some((cx, cz)) = cell_of([ox + t * dx, oz + t * dz]) else {
                continue;
            };
            for ax in -widen..=widen {
                for az in -widen..=widen {
                    cells.insert((cx.saturating_add(ax), cz.saturating_add(az)));
                }
            }
            if cells.len() > MAX_RAY_CELLS {
                break;
            }
        }
        let mut slots: Vec<u32> = cells
            .iter()
            .filter_map(|c| self.grid.get(c))
            .flatten()
            .copied()
            .collect();
        slots.sort_unstable();
        slots
            .into_iter()
            .filter_map(|s| self.instances.get(s as usize))
            .collect()
    }
}

/// The growth of the plant among `components`.
fn plant_growth(components: &[Component]) -> Option<f64> {
    components.iter().find_map(|c| match c {
        Component::Plant(p) => Some(p.growth),
        _ => None,
    })
}

/// The bounding rectangle of `points`, `None` for none.
fn footprint(points: impl Iterator<Item = [f64; 2]>) -> Option<Footprint> {
    let mut out: Option<Footprint> = None;
    for [x, z] in points {
        out = Some(match out {
            None => Footprint {
                min: [x, z],
                max: [x, z],
            },
            Some(f) => Footprint {
                min: [f.min[0].min(x), f.min[1].min(z)],
                max: [f.max[0].max(x), f.max[1].max(z)],
            },
        });
    }
    out
}

/// The grid cell of `[x, z]`; `None` for a non-finite point.
fn cell_of([x, z]: [f64; 2]) -> Option<(i64, i64)> {
    if !(x.is_finite() && z.is_finite()) {
        return None;
    }
    // Saturating casts: coordinates are bounded by the document's limit anyway.
    Some((
        (x / GRID_CELL_M).floor() as i64,
        (z / GRID_CELL_M).floor() as i64,
    ))
}

/// Everything the generators of a world place: one [`GeneratedSet`] per generator, in the
/// generators' order. Derived data: cloning shares the sets, and it takes no part in a world's
/// equality (it follows from the generators and the terrain, which do).
#[derive(Clone, Default)]
pub struct Generated {
    sets: Vec<Arc<GeneratedSet>>,
}

impl PartialEq for Generated {
    /// Always equal: derived data follows from the fields of the world that are compared.
    fn eq(&self, _: &Generated) -> bool {
        true
    }
}

impl fmt::Debug for Generated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.sets.iter()).finish()
    }
}

impl Generated {
    /// The sets, in the order of the world's generators.
    pub fn sets(&self) -> impl Iterator<Item = &GeneratedSet> {
        self.sets.iter().map(|s| s.as_ref())
    }

    /// The set of generator `id`.
    pub fn set(&self, id: &str) -> Option<&GeneratedSet> {
        self.sets().find(|s| s.generator.id == id)
    }

    /// The set at `index` (the generator's place in the world's list).
    pub fn set_at(&self, index: usize) -> Option<&GeneratedSet> {
        self.sets.get(index).map(|s| s.as_ref())
    }

    /// How many instances all sets hold.
    pub fn count(&self) -> usize {
        self.sets.iter().map(|s| s.instances.len()).sum()
    }

    /// A hash of every set's stamp, in order: what a renderer compares to know the instances
    /// changed.
    pub fn stamp(&self) -> u64 {
        let mut h = StateHasher::new();
        for s in &self.sets {
            h.write_str(&s.generator.id);
            h.write_u64(s.stamp);
        }
        h.finish()
    }

    /// The instance named `<generator id>#<n>`, with its set.
    pub fn find(&self, reference: &str) -> Option<(&GeneratedSet, &Instance)> {
        let (id, n) = reference.trim().rsplit_once('#')?;
        let n: u32 = n.parse().ok()?;
        let set = self.set(id)?;
        Some((set, set.instance(n)?))
    }

    /// Whether the sets were derived from exactly these generators and this terrain.
    pub fn is_current(&self, generators: &[Generator], terrain: Option<&Terrain>) -> bool {
        self.sets.len() == generators.len()
            && self
                .sets
                .iter()
                .zip(generators)
                .all(|(s, g)| s.generator == *g && s.terrain.as_ref() == terrain)
    }

    /// The sets for `generators` over `terrain` (its field `heights`): an unchanged
    /// generator keeps its set, one whose terrain changed keeps its candidates and is placed
    /// again, a new or changed one is derived from the start. `prefabs` give the components of
    /// prefab items, `model_bounds` how far models reach.
    pub fn derive(
        &self,
        generators: &[Generator],
        prefabs: &[Prefab],
        terrain: Option<&Terrain>,
        heights: Option<&HeightField>,
        model_bounds: &BTreeMap<String, ModelBounds>,
    ) -> Generated {
        let sets = generators
            .iter()
            .map(|g| {
                let previous = self.set(&g.id).filter(|s| s.generator == *g);
                let items = item_bodies(g, prefabs, model_bounds);
                match previous {
                    Some(p) if p.terrain.as_ref() == terrain && p.items == items => self
                        .sets
                        .iter()
                        .find(|s| s.generator.id == g.id)
                        .map(Arc::clone)
                        .unwrap_or_else(|| Arc::new(p.clone())),
                    Some(p) => {
                        Arc::new(place(g, Arc::clone(&p.candidates), items, terrain, heights))
                    }
                    None => Arc::new(place(g, Arc::new(candidates(g)), items, terrain, heights)),
                }
            })
            .collect();
        Generated { sets }
    }
}

/// The bodies of a generator's items.
fn item_bodies(
    g: &Generator,
    prefabs: &[Prefab],
    model_bounds: &BTreeMap<String, ModelBounds>,
) -> Vec<ItemBody> {
    g.items
        .iter()
        .map(|item| {
            let components = item.base_components(prefabs);
            let [_, smax] = item.scales();
            let mut reach: f64 = g.trunk_of(item).map_or(0.0, |t| t.radius_m);
            let mut has_body = false;
            for c in &components {
                match c {
                    Component::Shape(s) => {
                        has_body = true;
                        let [x, _, z] = s.size;
                        reach = reach.max((x * x + z * z).sqrt() / 2.0);
                    }
                    Component::Model(m) => {
                        has_body = true;
                        if let Some(b) = model_bounds.get(&m.asset) {
                            let far = b
                                .min
                                .iter()
                                .chain(&b.max)
                                .map(|v| v.abs())
                                .fold(0.0, f64::max);
                            reach = reach.max(far * 2f64.sqrt() * m.scale);
                        }
                    }
                    _ => {}
                }
            }
            let reach_m = reach * smax;
            ItemBody {
                components,
                has_body,
                reach_m: if reach_m.is_finite() { reach_m } else { 0.0 },
            }
        })
        .collect()
}

/// The points placed so far, bucketed by cells of the least distance.
struct Spacing {
    cell: f64,
    min2: f64,
    buckets: BTreeMap<(i64, i64), Vec<[f64; 2]>>,
}

impl Spacing {
    fn new(min_distance: f64) -> Spacing {
        Spacing {
            cell: min_distance,
            min2: min_distance * min_distance,
            buckets: BTreeMap::new(),
        }
    }

    fn key(&self, [x, z]: [f64; 2]) -> (i64, i64) {
        // Saturating float-to-int casts: far or odd values share an edge bucket.
        (
            (x / self.cell).floor() as i64,
            (z / self.cell).floor() as i64,
        )
    }

    fn fits(&self, point: [f64; 2]) -> bool {
        if self.cell <= 0.0 {
            return true;
        }
        let (kx, kz) = self.key(point);
        let [x, z] = point;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let key = (kx.saturating_add(dx), kz.saturating_add(dz));
                let Some(points) = self.buckets.get(&key) else {
                    continue;
                };
                for [px, pz] in points {
                    let (ex, ez) = (x - px, z - pz);
                    if ex * ex + ez * ez < self.min2 {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn add(&mut self, point: [f64; 2]) {
        if self.cell > 0.0 {
            self.buckets.entry(self.key(point)).or_default().push(point);
        }
    }
}

/// The first stage: the candidates of `g`, from its recipe alone.
fn candidates(g: &Generator) -> Vec<Candidate> {
    let wanted = g.requested();
    let area = g.area.area_m2();
    let (lo, hi) = g.area.bounds();
    let circle = match &g.area {
        crate::Area::Circle(c) => Some((c.center, c.radius_m)),
        crate::Area::Polygon(_) => None,
    };
    let rect = (hi[0] - lo[0]) * (hi[1] - lo[1]);
    let spread = if circle.is_some() || area.is_nan() || area <= 0.0 {
        1.0
    } else {
        (rect / area).max(1.0)
    };
    let tries = (wanted as f64 * TRIES_PER_CANDIDATE as f64 * spread).min(MAX_TRIES as f64);
    let tries = if tries.is_finite() { tries as u64 } else { 0 };
    let root = Rng::new(g.seed);
    let mut points = root.fork(STREAM_POINTS);
    let mut draws = root.fork(STREAM_ITEMS);
    let weights: Vec<f64> = g.items.iter().map(|i| i.weight.max(0.0)).collect();
    let total: f64 = weights.iter().sum();
    let mut spacing = Spacing::new(g.min_distance_m.unwrap_or(0.0));
    let mut out: Vec<Candidate> =
        Vec::with_capacity(usize::try_from(wanted.min(1 << 20)).unwrap_or(0));
    for _ in 0..tries {
        if out.len() as u64 >= wanted {
            break;
        }
        let point = match circle {
            Some(([cx, cz], r)) => {
                let d = r * points.unit().sqrt();
                let (s, c) = (TAU * points.unit()).psin_cos();
                [cx + d * c, cz + d * s]
            }
            None => [points.range(lo[0], hi[0]), points.range(lo[1], hi[1])],
        };
        if !g.area.contains(point) || !spacing.fits(point) {
            continue;
        }
        spacing.add(point);
        // Five draws for every candidate, whatever its item uses, so one item's settings
        // never shift another's numbers.
        let (u_item, u_variant, u_class, u_scale, u_turn) = (
            draws.unit(),
            draws.unit(),
            draws.unit(),
            draws.unit(),
            draws.unit(),
        );
        let mut pick = u_item * total;
        let mut item = 0usize;
        for (i, w) in weights.iter().enumerate() {
            item = i;
            if pick < *w {
                break;
            }
            pick -= w;
        }
        let Some(spec) = g.items.get(item) else {
            continue;
        };
        let variants = f64::from(spec.variant_count());
        let variant = ((u_variant * variants).floor() as u32).min(spec.variant_count() - 1);
        let class = if spec.growth_range.is_some() {
            ((u_class * f64::from(GROWTH_CLASSES)).floor() as u32).min(GROWTH_CLASSES - 1)
        } else {
            0
        };
        let [s0, s1] = spec.scales();
        let scale = (s0 + (s1 - s0) * u_scale).clamp(s0, s1);
        let rotation_y_deg = if spec.random_rotation.unwrap_or(true) {
            (360.0 * u_turn) % 360.0
        } else {
            0.0
        };
        let number = u32::try_from(out.len().saturating_add(1)).unwrap_or(u32::MAX);
        out.push(Candidate {
            number,
            item: u16::try_from(item).unwrap_or(u16::MAX),
            variant: u8::try_from(variant).unwrap_or(0),
            growth_class: u8::try_from(class).unwrap_or(0),
            point,
            rotation_y_deg,
            scale,
        });
    }
    out
}

/// The slope of the ground at `[x, z]`, degrees, from the field's normal there.
fn slope_deg(heights: &HeightField, x: f64, z: f64) -> Option<f64> {
    let [nx, ny, nz] = heights.normal_at(x, z)?;
    if ny.is_nan() || ny <= 0.0 {
        return Some(90.0);
    }
    Some(rad_to_deg(((nx * nx + nz * nz).sqrt() / ny).patan()))
}

/// Whether `[x, z]` keeps the generator's ground rules, and the ground height there.
fn ground(g: &Generator, heights: &HeightField, point: [f64; 2]) -> Option<f64> {
    if g.exclude.iter().any(|a| a.contains(point)) {
        return None;
    }
    let [x, z] = point;
    let y = heights.height_at(x, z)?;
    if let Some([lo, hi]) = g.height_range_m
        && !(lo <= y && y <= hi)
    {
        return None;
    }
    if let Some(max) = g.slope_max_deg
        && max < 90.0
        && slope_deg(heights, x, z).is_none_or(|s| s > max)
    {
        return None;
    }
    Some(y)
}

/// The second stage: the candidates that keep the ground rules, standing on the ground.
fn place(
    g: &Generator,
    candidates: Arc<Vec<Candidate>>,
    items: Vec<ItemBody>,
    terrain: Option<&Terrain>,
    heights: Option<&HeightField>,
) -> GeneratedSet {
    let mut instances = Vec::new();
    if let (Some(_), Some(h)) = (terrain, heights) {
        for c in candidates.iter() {
            let Some(y) = ground(g, h, c.point) else {
                continue;
            };
            let [x, z] = c.point;
            instances.push(Instance {
                number: c.number,
                item: c.item,
                variant: c.variant,
                growth_class: c.growth_class,
                position: [x, y, z],
                rotation_y_deg: c.rotation_y_deg,
                scale: c.scale,
            });
        }
    }
    let mut grid: BTreeMap<(i64, i64), Vec<u32>> = BTreeMap::new();
    let mut hasher = StateHasher::new();
    hasher.write_str(&g.id);
    for (slot, i) in instances.iter().enumerate() {
        if let Some(cell) = cell_of([i.position[0], i.position[2]]) {
            grid.entry(cell)
                .or_default()
                .push(u32::try_from(slot).unwrap_or(u32::MAX));
        }
        hasher.write_u32(i.number);
        hasher.write_u32(u32::from(i.item));
        hasher.write_u8(i.variant);
        hasher.write_u8(i.growth_class);
        for v in i.position {
            hasher.write_f64(v);
        }
        hasher.write_f64(i.rotation_y_deg);
        hasher.write_f64(i.scale);
    }
    let reach_m = items.iter().map(|b| b.reach_m).fold(0.0, f64::max);
    GeneratedSet {
        generator: g.clone(),
        terrain: terrain.cloned(),
        candidates,
        instances,
        items,
        grid,
        reach_m,
        stamp: hasher.finish(),
    }
}
