//! Generators: a forest or a scatter of things as one recipe entry of the world document
//! (`docs/design/generators.md`).
//!
//! A [`Generator`] names an area of the ground plane, a seed, a density and the items it places
//! with their weights. What it places is derived ([`crate::Generated`]): rebuilt from the recipe
//! and the terrain, never written to a file, and the same seed gives the same instances in the
//! same order on every platform.

use crate::error::Ctx;
use crate::limits::{MAX_COORDINATE_M, SCALE_MAX, SCALE_MIN};
use crate::{Component, Entity, EntityId, Prefab, WorldError};
use serde::{Deserialize, Serialize};

/// Most generators in one world.
pub const MAX_GENERATORS: usize = 256;
/// Most characters of a generator id.
pub const MAX_GENERATOR_ID_CHARS: usize = 64;
/// Most items of one generator.
pub const MAX_GENERATOR_ITEMS: usize = 64;
/// Most corners of a polygon area.
pub const MAX_POLYGON_POINTS: usize = 1024;
/// Most areas a generator keeps clear.
pub const MAX_GENERATOR_EXCLUDES: usize = 256;
/// Densest a generator places, instances per hectare (one per square metre).
pub const DENSITY_MAX_PER_HA: f64 = 10_000.0;
/// Most instances one generator places (its density times its area, before the terrain).
pub const MAX_GENERATED_PER_GENERATOR: u64 = 250_000;
/// Most instances all the generators of one world place together.
pub const MAX_GENERATED_TOTAL: u64 = 1_000_000;
/// Largest radius of a circle area, metres.
pub const AREA_RADIUS_MAX_M: f64 = 100_000.0;
/// Largest least distance between two instances, metres.
pub const MIN_DISTANCE_MAX_M: f64 = 1000.0;
/// Most plant seeds (shapes) of one forest item: every instance takes one of them, so the
/// renderer grows a few meshes per item, not one per tree.
pub const MAX_PLANT_VARIANTS: u32 = 16;
/// Plant seeds of one item when `variants` is left out.
pub const DEFAULT_PLANT_VARIANTS: u32 = 3;
/// The growth classes a `growth_range` is cut into: every instance takes the middle of one,
/// so a forest of mixed ages is a few meshes per seed.
pub const GROWTH_CLASSES: u32 = 3;
/// The trunk of a forest item that gives none: radius and height at scale 1 and growth 1,
/// metres. `generator.add` fills in the species' own numbers instead.
pub const DEFAULT_TRUNK: Trunk = Trunk {
    radius_m: 0.2,
    height_m: 6.0,
};
/// Thickest trunk, metres of radius.
pub const TRUNK_RADIUS_MAX_M: f64 = 20.0;
/// Tallest trunk, metres.
pub const TRUNK_HEIGHT_MAX_M: f64 = 200.0;

/// What a generator places.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GeneratorKind {
    /// Plants: every item has a `plant` component, and every tree a trunk that blocks.
    Forest,
    /// Anything: rocks, crates, bushes, each as its components say.
    Scatter,
}

impl GeneratorKind {
    /// The name as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            GeneratorKind::Forest => "forest",
            GeneratorKind::Scatter => "scatter",
        }
    }
}

/// A circle of the ground plane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Circle {
    /// Its centre `[x, z]`, metres.
    pub center: [f64; 2],
    /// Its radius, metres, above 0.
    pub radius_m: f64,
}

/// An area of the ground plane: `{"circle": {"center", "radius_m"}}` or `{"polygon": [[x, z],
/// ...]}` (corners in order, either way round; the even-odd rule decides what is inside).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Area {
    /// A circle.
    Circle(Circle),
    /// A polygon of 3 to [`MAX_POLYGON_POINTS`] corners.
    Polygon(Vec<[f64; 2]>),
}

impl Area {
    /// Whether `[x, z]` lies inside (a circle's rim counts as outside).
    pub fn contains(&self, [x, z]: [f64; 2]) -> bool {
        match self {
            Area::Circle(c) => {
                let (dx, dz) = (x - c.center[0], z - c.center[1]);
                dx * dx + dz * dz < c.radius_m * c.radius_m
            }
            Area::Polygon(points) => {
                let mut inside = false;
                let mut prev = match points.last() {
                    Some(p) => *p,
                    None => return false,
                };
                for &[px, pz] in points {
                    let [qx, qz] = prev;
                    if (pz > z) != (qz > z) {
                        let t = (z - pz) / (qz - pz);
                        if x < px + t * (qx - px) {
                            inside = !inside;
                        }
                    }
                    prev = [px, pz];
                }
                inside
            }
        }
    }

    /// The area in square metres (a polygon's by the shoelace formula, which a polygon that
    /// crosses itself gets wrong).
    pub fn area_m2(&self) -> f64 {
        match self {
            Area::Circle(c) => std::f64::consts::PI * c.radius_m * c.radius_m,
            Area::Polygon(points) => {
                let mut twice = 0.0;
                let mut prev = match points.last() {
                    Some(p) => *p,
                    None => return 0.0,
                };
                for &[x, z] in points {
                    twice += prev[0] * z - x * prev[1];
                    prev = [x, z];
                }
                (twice / 2.0).abs()
            }
        }
    }

    /// The bounding rectangle `([x0, z0], [x1, z1])`.
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        match self {
            Area::Circle(c) => {
                let [cx, cz] = c.center;
                let r = c.radius_m;
                ([cx - r, cz - r], [cx + r, cz + r])
            }
            Area::Polygon(points) => {
                let mut lo = [f64::INFINITY; 2];
                let mut hi = [f64::NEG_INFINITY; 2];
                for &[x, z] in points {
                    lo = [lo[0].min(x), lo[1].min(z)];
                    hi = [hi[0].max(x), hi[1].max(z)];
                }
                (lo, hi)
            }
        }
    }

    fn validate(&self, g: &str, field: &str) -> Result<(), WorldError> {
        let bad = |message: String| WorldError::Generator {
            generator: g.to_string(),
            message: format!("{field}: {message}"),
        };
        match self {
            Area::Circle(c) => {
                if !c
                    .center
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE_M)
                {
                    return Err(bad(format!(
                        "circle.center is {:?}; it must be a point [x, z] of finite metres \
                         within {MAX_COORDINATE_M}",
                        c.center
                    )));
                }
                if !(c.radius_m.is_finite() && c.radius_m > 0.0 && c.radius_m <= AREA_RADIUS_MAX_M)
                {
                    return Err(bad(format!(
                        "circle.radius_m is {}; it must be above 0 and at most \
                         {AREA_RADIUS_MAX_M} metres",
                        c.radius_m
                    )));
                }
            }
            Area::Polygon(points) => {
                if points.len() < 3 || points.len() > MAX_POLYGON_POINTS {
                    return Err(bad(format!(
                        "polygon has {} corners; it needs 3 to {MAX_POLYGON_POINTS}",
                        points.len()
                    )));
                }
                if let Some((i, p)) = points.iter().enumerate().find(|(_, p)| {
                    !p.iter()
                        .all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE_M)
                }) {
                    return Err(bad(format!(
                        "polygon[{i}] is {p:?}; a corner is a point [x, z] of finite metres \
                         within {MAX_COORDINATE_M}"
                    )));
                }
                if self.area_m2().partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                    return Err(bad(
                        "polygon encloses no area; give at least three corners that are not \
                         on one line"
                            .to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// The part of a tree that blocks characters and projectiles: an upright cylinder standing on
/// the tree's base, sized by the instance's scale and growth.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trunk {
    /// Radius at scale 1 and growth 1, metres; 0 for no trunk.
    pub radius_m: f64,
    /// Height at scale 1 and growth 1, metres.
    pub height_m: f64,
}

/// One kind of thing a generator places, with its share.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorItem {
    /// Its share of the instances, relative to the other items' weights; above 0.
    pub weight: f64,
    /// The components of every instance, at most one of each type (or give `prefab`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<Component>,
    /// A prefab of the world whose components every instance takes (its children are not
    /// placed); instead of `components`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefab: Option<String>,
    /// `[min, max]` of an instance's uniform scale, drawn evenly. Default `[1, 1]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_range: Option<[f64; 2]>,
    /// Turn every instance about +Y by a random angle. Default true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub random_rotation: Option<bool>,
    /// `[min, max]` of a plant's growth, in (0, 1], cut into [`GROWTH_CLASSES`] classes.
    /// Default: the plant component's growth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub growth_range: Option<[f64; 2]>,
    /// How many plant seeds the instances share (the plant component's seed and the ones
    /// after it), 1 to [`MAX_PLANT_VARIANTS`]. Default [`DEFAULT_PLANT_VARIANTS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variants: Option<u32>,
    /// The trunk that blocks. Default for a forest [`DEFAULT_TRUNK`], for a scatter none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trunk: Option<Trunk>,
}

impl GeneratorItem {
    /// An item of `weight` with `components` and nothing else set.
    pub fn of(weight: f64, components: Vec<Component>) -> GeneratorItem {
        GeneratorItem {
            weight,
            components,
            prefab: None,
            scale_range: None,
            random_rotation: None,
            growth_range: None,
            variants: None,
            trunk: None,
        }
    }

    /// The scale range, `[1, 1]` when not given.
    pub fn scales(&self) -> [f64; 2] {
        self.scale_range.unwrap_or([1.0, 1.0])
    }

    /// The number of plant seeds the instances share.
    pub fn variant_count(&self) -> u32 {
        self.variants
            .unwrap_or(DEFAULT_PLANT_VARIANTS)
            .clamp(1, MAX_PLANT_VARIANTS)
    }

    /// The components of an instance before its plant's seed and growth: its own, or its
    /// prefab's in `prefabs` (none when the prefab does not exist).
    pub fn base_components(&self, prefabs: &[Prefab]) -> Vec<Component> {
        match &self.prefab {
            Some(id) => prefabs
                .iter()
                .find(|p| p.id == *id)
                .map(|p| p.effective(&[]))
                .unwrap_or_default(),
            None => self.components.clone(),
        }
    }

    /// The growth of growth class `class` (the middle of its share of `growth_range`), `None`
    /// without a range.
    pub fn class_growth(&self, class: u32) -> Option<f64> {
        let [lo, hi] = self.growth_range?;
        let n = f64::from(GROWTH_CLASSES);
        let k = f64::from(class.min(GROWTH_CLASSES - 1));
        Some((lo + (hi - lo) * (k + 0.5) / n).clamp(f64::MIN_POSITIVE, 1.0))
    }
}

/// One generator entry of a world: an area, a seed, a density and the items it places.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generator {
    /// Unique among the world's generators: 1 to 64 ASCII letters, digits, `-` and `_`.
    /// Its instances are called `<id>#<n>`.
    pub id: String,
    /// `forest` or `scatter`.
    pub kind: GeneratorKind,
    /// Where instances stand.
    pub area: Area,
    /// The seed of the placement.
    #[serde(default)]
    pub seed: u64,
    /// Instances per hectare (10 000 m2) of the area, above 0 and at most
    /// [`DENSITY_MAX_PER_HA`]; fewer stand where the terrain, the excludes or the spacing turn
    /// points away.
    pub density_per_ha: f64,
    /// Least horizontal distance between two instances, metres. Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_distance_m: Option<f64>,
    /// Steepest ground an instance stands on, degrees. Default 90 (any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slope_max_deg: Option<f64>,
    /// `[min, max]` of the ground height an instance stands on, metres. Default any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_range_m: Option<[f64; 2]>,
    /// Areas kept clear (a road, a clearing, a camp). Default none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<Area>,
    /// What it places, 1 to [`MAX_GENERATOR_ITEMS`].
    pub items: Vec<GeneratorItem>,
}

/// Whether `id` is a valid generator id: 1 to [`MAX_GENERATOR_ID_CHARS`] ASCII letters,
/// digits, `-` and `_` (so `<id>#<n>` names an instance and `<id>` is a file-safe name).
pub fn is_generator_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_GENERATOR_ID_CHARS
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Generator {
    /// How many instances the density asks for over the whole area, before the terrain, the
    /// excludes and the spacing (rounded; a positive density asks for at least one).
    pub fn requested(&self) -> u64 {
        let wanted = self.density_per_ha * self.area.area_m2() / 10_000.0;
        if !(wanted.is_finite() && wanted > 0.0) {
            return 0;
        }
        // A saturating cast: anything beyond u64 is refused by `validate` anyway.
        ((wanted + 0.5).floor() as u64).max(1)
    }

    /// The trunk of `item` (none for a scatter item without one).
    pub fn trunk_of(&self, item: &GeneratorItem) -> Option<Trunk> {
        match (item.trunk, self.kind) {
            (Some(t), _) => (t.radius_m > 0.0 && t.height_m > 0.0).then_some(t),
            (None, GeneratorKind::Forest) => Some(DEFAULT_TRUNK),
            (None, GeneratorKind::Scatter) => None,
        }
    }

    fn bad(&self, message: impl Into<String>) -> WorldError {
        WorldError::Generator {
            generator: self.id.clone(),
            message: message.into(),
        }
    }

    /// Checks every rule of the entry on its own and against the world's `prefabs`: the id,
    /// the area and the excludes, the numbers, and every item (its components a valid entity,
    /// a prefab that exists, a forest item with a `plant`). The error names the generator and
    /// the field.
    pub fn validate(&self, prefabs: &[Prefab]) -> Result<(), WorldError> {
        if !is_generator_id(&self.id) {
            let shown: String = self.id.chars().take(MAX_GENERATOR_ID_CHARS + 8).collect();
            return Err(Ctx::WORLD.invalid(
                "generators.id",
                format!("{shown:?}"),
                format!(
                    "a generator id is 1 to {MAX_GENERATOR_ID_CHARS} ASCII letters, digits, `-` \
                     and `_`, such as \"forest-north\""
                ),
            ));
        }
        self.area.validate(&self.id, "area")?;
        if self.exclude.len() > MAX_GENERATOR_EXCLUDES {
            return Err(self.bad(format!(
                "exclude holds {} areas; at most {MAX_GENERATOR_EXCLUDES}",
                self.exclude.len()
            )));
        }
        for (i, a) in self.exclude.iter().enumerate() {
            a.validate(&self.id, &format!("exclude[{i}]"))?;
        }
        if !(self.density_per_ha.is_finite()
            && self.density_per_ha > 0.0
            && self.density_per_ha <= DENSITY_MAX_PER_HA)
        {
            return Err(self.bad(format!(
                "density_per_ha is {}; it must be above 0 and at most {DENSITY_MAX_PER_HA} \
                 instances per hectare",
                self.density_per_ha
            )));
        }
        let requested = self.requested();
        if requested > MAX_GENERATED_PER_GENERATOR {
            return Err(self.bad(format!(
                "density_per_ha {} over an area of {:.0} m2 asks for {requested} instances; one \
                 generator places at most {MAX_GENERATED_PER_GENERATOR}: lower the density, \
                 make the area smaller or split it into several generators",
                self.density_per_ha,
                self.area.area_m2()
            )));
        }
        if let Some(d) = self.min_distance_m
            && !(d.is_finite() && (0.0..=MIN_DISTANCE_MAX_M).contains(&d))
        {
            return Err(self.bad(format!(
                "min_distance_m is {d}; it must be from 0 to {MIN_DISTANCE_MAX_M} metres"
            )));
        }
        if let Some(s) = self.slope_max_deg
            && !(s.is_finite() && (0.0..=90.0).contains(&s))
        {
            return Err(self.bad(format!(
                "slope_max_deg is {s}; it must be from 0 to 90 degrees"
            )));
        }
        if let Some([lo, hi]) = self.height_range_m
            && !(lo.is_finite() && hi.is_finite() && lo <= hi)
        {
            return Err(self.bad(format!(
                "height_range_m is [{lo}, {hi}]; it must be [min, max] of finite metres with \
                 min <= max"
            )));
        }
        if self.items.is_empty() || self.items.len() > MAX_GENERATOR_ITEMS {
            return Err(self.bad(format!(
                "items holds {} items; a generator places 1 to {MAX_GENERATOR_ITEMS} kinds of \
                 thing",
                self.items.len()
            )));
        }
        for (i, item) in self.items.iter().enumerate() {
            self.check_item(i, item, prefabs)?;
        }
        Ok(())
    }

    fn check_item(
        &self,
        i: usize,
        item: &GeneratorItem,
        prefabs: &[Prefab],
    ) -> Result<(), WorldError> {
        let at = format!("items[{i}]");
        if !(item.weight.is_finite() && item.weight > 0.0 && item.weight <= 1e9) {
            return Err(self.bad(format!(
                "{at}.weight is {}; it must be above 0 (and at most 1e9)",
                item.weight
            )));
        }
        match (&item.prefab, item.components.is_empty()) {
            (Some(_), false) => {
                return Err(self.bad(format!(
                    "{at} gives both `prefab` and `components`; give one of them"
                )));
            }
            (None, true) => {
                return Err(self.bad(format!(
                    "{at} gives neither `components` nor `prefab`; an item needs what its \
                     instances are made of"
                )));
            }
            (Some(id), true) => {
                if !prefabs.iter().any(|p| p.id == *id) {
                    let known: Vec<&str> = prefabs.iter().take(10).map(|p| p.id.as_str()).collect();
                    return Err(self.bad(format!(
                        "{at}.prefab is {id:?}; no prefab has that id (the prefabs: {})",
                        if known.is_empty() {
                            "none".to_string()
                        } else {
                            known.join(", ")
                        }
                    )));
                }
            }
            (None, false) => {}
        }
        let components = item.base_components(prefabs);
        let probe = Entity {
            id: EntityId(0),
            name: String::new(),
            position: [0.0; 3],
            rotation_y_deg: 0.0,
            scale: 1.0,
            prefab: None,
            parent: None,
            layer: None,
            components,
        };
        if let Err(e) = probe.validate() {
            let text = e.to_string();
            let text = text.strip_prefix("entity 0: ").unwrap_or(&text);
            return Err(self.bad(format!("{at}.components: {text}")));
        }
        if probe.character().is_some() {
            return Err(self.bad(format!(
                "{at} has a `character`; generated instances do not move, so place characters \
                 as entities"
            )));
        }
        let plant = probe.plant();
        if self.kind == GeneratorKind::Forest && plant.is_none() {
            return Err(self.bad(format!(
                "{at} has no `plant` component; every item of a forest is a plant (use kind \
                 \"scatter\" for other things)"
            )));
        }
        if let Some([lo, hi]) = item.scale_range
            && !(SCALE_MIN <= lo && lo <= hi && hi <= SCALE_MAX)
        {
            return Err(self.bad(format!(
                "{at}.scale_range is [{lo}, {hi}]; it must be [min, max] with {SCALE_MIN} <= \
                 min <= max <= {SCALE_MAX}"
            )));
        }
        if let Some([lo, hi]) = item.growth_range {
            if plant.is_none() {
                return Err(self.bad(format!(
                    "{at}.growth_range is given but the item has no `plant` component to grow"
                )));
            }
            if !(lo.is_finite() && lo > 0.0 && lo <= hi && hi <= 1.0) {
                return Err(self.bad(format!(
                    "{at}.growth_range is [{lo}, {hi}]; it must be [min, max] with 0 < min <= \
                     max <= 1"
                )));
            }
        }
        if let Some(v) = item.variants
            && !(1..=MAX_PLANT_VARIANTS).contains(&v)
        {
            return Err(self.bad(format!(
                "{at}.variants is {v}; it must be from 1 to {MAX_PLANT_VARIANTS}"
            )));
        }
        if let Some(t) = item.trunk {
            let ok = t.radius_m.is_finite()
                && (0.0..=TRUNK_RADIUS_MAX_M).contains(&t.radius_m)
                && t.height_m.is_finite()
                && t.height_m > 0.0
                && t.height_m <= TRUNK_HEIGHT_MAX_M;
            if !ok {
                return Err(self.bad(format!(
                    "{at}.trunk is {{radius_m: {}, height_m: {}}}; the radius must be from 0 \
                     (no trunk) to {TRUNK_RADIUS_MAX_M} m and the height above 0 and at most \
                     {TRUNK_HEIGHT_MAX_M} m",
                    t.radius_m, t.height_m
                )));
            }
        }
        Ok(())
    }
}
