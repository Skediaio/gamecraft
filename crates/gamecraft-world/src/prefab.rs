//! Prefabs: an entity described once and placed many times (`docs/design/journal.md`,
//! *Prefabs*).
//!
//! A [`Prefab`] is a set of components with optional children (entities placed relative to the
//! parent). [`crate::World::spawn_prefab`] places one: the new entity refers to the prefab by
//! id, and its components are the prefab's with its overrides replacing components of the same
//! type. Those effective components are **materialised**: they are what
//! [`crate::Entity::components`] holds in memory, so the simulation and the renderer read an
//! instance like any other entity. Only the file keeps the difference: an instance is written
//! as its prefab and its overrides ([`Prefab::overrides_of`]), and reading it back merges them
//! again ([`Prefab::effective`]).

use crate::error::Ctx;
use crate::limits::{MAX_COORDINATE_M, MAX_PREFAB_CHILDREN, MAX_PREFAB_NESTING};
use crate::{Component, EntityId, ModelBounds, ShapeKind, WorldError};
use gamecraft_math::{Portable, deg_to_rad};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A reusable description of an entity: components and children.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prefab {
    /// Unique among the world's prefabs; at most 200 characters, not empty.
    pub id: String,
    /// The name an instance gets when it is spawned without one; at most 200 characters.
    /// Default empty.
    #[serde(default)]
    pub name: String,
    /// The components of every instance, at most one of each type. Default none.
    #[serde(default)]
    pub components: Vec<Component>,
    /// Entities placed with every instance, relative to it, at most 64. Default none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<PrefabChild>,
}

/// An entity a prefab places with each instance, relative to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrefabChild {
    /// The child's name, at most 200 characters. Default empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Where the child's base stands relative to the parent's, in the parent's frame
    /// (metres; -Z is the parent's forward, +X its right), before the parent's scale: the
    /// instance's scale applies to it. Default the parent's base.
    #[serde(default)]
    pub offset: [f64; 3],
    /// The child's turn relative to the parent's, degrees counter-clockwise seen from above.
    /// Default 0.
    #[serde(default)]
    pub rotation_y_deg: f64,
    /// The prefab the child is itself an instance of, by id: a prefab inside a prefab. Its
    /// `components` then replace that prefab's components of the same type, as an instance's
    /// overrides do, and it places that prefab's children in turn, at most
    /// [`crate::limits::MAX_PREFAB_NESTING`] levels deep. Default none: a plain entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefab: Option<String>,
    /// The child's components, at most one of each type (with `prefab`: its overrides).
    /// Default none.
    #[serde(default)]
    pub components: Vec<Component>,
}

/// How to place a prefab: [`crate::World::spawn_prefab`].
#[derive(Clone, Debug, PartialEq)]
pub struct Spawn {
    /// The instance's name; the prefab's name when `None`.
    pub name: Option<String>,
    /// Where the instance's base stands, metres.
    pub position: [f64; 3],
    /// The instance's turn about +Y, degrees.
    pub rotation_y_deg: f64,
    /// The instance's scale, 0.001 to 1000. Its children follow it: each stands at its offset
    /// times the scale, at the same scale. Default 1.
    pub scale: f64,
    /// Components that replace the prefab's components of the same type (or add a type the
    /// prefab lacks), at most one of each type.
    pub overrides: Vec<Component>,
    /// The layer of the instance and its children; `None` is the world's default layer.
    pub layer: Option<String>,
}

impl Default for Spawn {
    /// At the origin, unturned, scale 1, without overrides or a layer.
    fn default() -> Self {
        Spawn {
            name: None,
            position: [0.0; 3],
            rotation_y_deg: 0.0,
            scale: 1.0,
            overrides: Vec::new(),
            layer: None,
        }
    }
}

/// What [`crate::World::spawn_prefab`] placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spawned {
    /// The instance.
    pub id: EntityId,
    /// Its children, in the prefab's order.
    pub children: Vec<EntityId>,
}

impl Prefab {
    /// The prefab's component of type `kind`, if any.
    pub fn component(&self, kind: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.kind() == kind)
    }

    /// The effective components of an instance with `overrides`: the prefab's components in
    /// their order, each replaced by the override of the same type, then the overrides of
    /// types the prefab lacks, in their order.
    pub fn effective(&self, overrides: &[Component]) -> Vec<Component> {
        let mut out: Vec<Component> = self
            .components
            .iter()
            .map(|c| {
                overrides
                    .iter()
                    .find(|o| o.kind() == c.kind())
                    .unwrap_or(c)
                    .clone()
            })
            .collect();
        for o in overrides {
            if self.component(o.kind()).is_none() && !out.iter().any(|c| c.kind() == o.kind()) {
                out.push(o.clone());
            }
        }
        out
    }

    /// The overrides that turn this prefab into `components`: every component that differs
    /// from the prefab's of the same type, or whose type the prefab lacks, in their order.
    /// [`Prefab::effective`] of the answer gives `components` back when they hold every type
    /// of the prefab.
    pub fn overrides_of(&self, components: &[Component]) -> Vec<Component> {
        components
            .iter()
            .filter(|c| self.component(c.kind()) != Some(*c))
            .cloned()
            .collect()
    }

    /// Checks the prefab on its own: its id and name, its components (valid, at most one of
    /// each type) and its children (at most 64, finite offsets within 1e6 m, finite turns,
    /// valid components). The error names the prefab and the field.
    pub fn validate(&self) -> Result<(), WorldError> {
        let ctx = Ctx::WORLD;
        let wrap = |e: WorldError| WorldError::Prefab {
            prefab: self.id.clone(),
            message: e.to_string(),
        };
        ctx.id("id", &self.id).map_err(wrap)?;
        ctx.name("name", &self.name).map_err(wrap)?;
        check_components(&self.components, "components").map_err(wrap)?;
        if self.children.len() > MAX_PREFAB_CHILDREN {
            return Err(wrap(ctx.invalid(
                "children",
                format!("a list of {}", self.children.len()),
                format!("a prefab has at most {MAX_PREFAB_CHILDREN} children"),
            )));
        }
        for (i, child) in self.children.iter().enumerate() {
            let at = format!("children[{i}]");
            ctx.name(format!("{at}.name"), &child.name).map_err(wrap)?;
            for (axis, &v) in child.offset.iter().enumerate() {
                ctx.range(
                    format!("{at}.offset[{axis}]"),
                    v,
                    -MAX_COORDINATE_M,
                    MAX_COORDINATE_M,
                    " metres",
                )
                .map_err(wrap)?;
            }
            ctx.finite(format!("{at}.rotation_y_deg"), child.rotation_y_deg)
                .map_err(wrap)?;
            if let Some(id) = &child.prefab {
                ctx.id(format!("{at}.prefab"), id).map_err(wrap)?;
            }
            check_components(&child.components, &format!("{at}.components")).map_err(wrap)?;
        }
        Ok(())
    }
}

/// Every component valid, at most one of each type; errors name `field`.
pub(crate) fn check_components(components: &[Component], field: &str) -> Result<(), WorldError> {
    for (i, c) in components.iter().enumerate() {
        if components.iter().take(i).any(|e| e.kind() == c.kind()) {
            return Err(Ctx::WORLD.invalid(
                field,
                format!("a list with more than one `{}`", c.kind()),
                "it holds at most one component of each type",
            ));
        }
        c.validate(Ctx::WORLD).map_err(|e| match e {
            WorldError::Invalid {
                entity,
                field: inner,
                value,
                expected,
            } => WorldError::Invalid {
                entity,
                field: format!("{field}.{inner}"),
                value,
                expected,
            },
            other => other,
        })?;
    }
    Ok(())
}

/// `offset` (in a parent's frame) turned by the parent's yaw, in world axes: a turn about +Y,
/// counter-clockwise seen from above, so the local north (-Z) becomes
/// [`gamecraft_math::yaw_forward`].
pub(crate) fn turn(offset: [f64; 3], yaw_deg: f64) -> [f64; 3] {
    let (s, c) = deg_to_rad(yaw_deg).psin_cos();
    let [x, y, z] = offset;
    [x * c + z * s, y, -x * s + z * c]
}

/// The world placement of a child at `offset` and `rotation_y_deg` relative to a parent at
/// `position` turned by `yaw_deg` and of scale `scale`: the offset is scaled in the parent's
/// own frame, then turned, so an instance's children follow its scale. A scale of 1 leaves
/// the offset as it is, to the bit.
pub(crate) fn place_child(
    position: [f64; 3],
    yaw_deg: f64,
    scale: f64,
    offset: [f64; 3],
    rotation_y_deg: f64,
) -> ([f64; 3], f64) {
    let [x, y, z] = offset;
    let [dx, dy, dz] = turn([x * scale, y * scale, z * scale], yaw_deg);
    let [px, py, pz] = position;
    ([px + dx, py + dy, pz + dz], yaw_deg + rotation_y_deg)
}

/// Where a child is relative to its parent: the inverse of [`place_child`] for a parent of
/// scale 1. The offset is in world metres, turned into the parent's frame but not divided by
/// the parent's scale.
pub(crate) fn relative(
    parent_position: [f64; 3],
    parent_yaw_deg: f64,
    position: [f64; 3],
    yaw_deg: f64,
) -> ([f64; 3], f64) {
    let [px, py, pz] = parent_position;
    let [x, y, z] = position;
    let offset = turn([x - px, y - py, z - pz], -parent_yaw_deg);
    (offset, yaw_deg - parent_yaw_deg)
}

/// A box on the ground plane of some frame: its smallest and largest `[x, z]`.
pub type GroundBox = ([f64; 2], [f64; 2]);

/// Grows `a` to hold `b` (either may be nothing).
fn union(a: Option<GroundBox>, b: Option<GroundBox>) -> Option<GroundBox> {
    match (a, b) {
        (None, other) | (other, None) => other,
        (Some((alo, ahi)), Some((blo, bhi))) => Some((
            [alo[0].min(blo[0]), alo[1].min(blo[1])],
            [ahi[0].max(bhi[0]), ahi[1].max(bhi[1])],
        )),
    }
}

/// The smallest box around the shapes and models of `components` at the entity's scale 1, in
/// the entity's own frame: a box by its size, a cylinder or a sphere by its diameter, a model
/// by its asset's bounding box ([`ModelBounds`]) times the model's own `scale`. `None` when
/// there is nothing to measure: no shape, and no model whose asset has bounds.
pub fn components_box(
    components: &[Component],
    bounds: &BTreeMap<String, ModelBounds>,
) -> Option<GroundBox> {
    let mut out: Option<GroundBox> = None;
    for component in components {
        let mine = match component {
            Component::Shape(shape) => {
                let [sx, _, sz] = shape.size;
                let (hx, hz) = match shape.kind {
                    ShapeKind::Box => (sx / 2.0, sz / 2.0),
                    // A cylinder or a sphere is as wide along z as along x.
                    ShapeKind::Cylinder | ShapeKind::Sphere => (sx / 2.0, sx / 2.0),
                };
                Some(([-hx, -hz], [hx, hz]))
            }
            Component::Model(model) => bounds.get(&model.asset).map(|b| {
                let s = model.scale;
                let mid = |lo: f64, hi: f64| (lo + hi) / 2.0 * s;
                let half = |lo: f64, hi: f64| (hi - lo) / 2.0 * s;
                let (cx, cz) = (mid(b.min[0], b.max[0]), mid(b.min[2], b.max[2]));
                let (hx, hz) = (half(b.min[0], b.max[0]), half(b.min[2], b.max[2]));
                ([cx - hx, cz - hz], [cx + hx, cz + hz])
            }),
            _ => None,
        };
        out = union(out, mine);
    }
    // A box with a number that is not finite measures nothing.
    out.filter(|(lo, hi)| lo.iter().chain(hi).all(|v| v.is_finite()))
}

/// [`components_box`] of `prefab` at `depth` (1 for the prefab asked about), with every
/// child's box turned by the child's rotation and moved to its offset: a child that places a
/// prefab adds that prefab's effective components and, [`MAX_PREFAB_NESTING`] levels deep,
/// its own children.
fn prefab_box_at(
    prefabs: &[Prefab],
    prefab: &Prefab,
    bounds: &BTreeMap<String, ModelBounds>,
    depth: usize,
) -> Option<GroundBox> {
    let mut out = components_box(&prefab.components, bounds);
    for child in &prefab.children {
        let nested = child
            .prefab
            .as_deref()
            .and_then(|id| prefabs.iter().find(|p| p.id == id));
        let mine = match nested {
            Some(inner) => {
                let below = (depth < MAX_PREFAB_NESTING)
                    .then(|| prefab_box_at(prefabs, inner, bounds, depth.saturating_add(1)))
                    .flatten();
                union(
                    components_box(&inner.effective(&child.components), bounds),
                    below,
                )
            }
            None => components_box(&child.components, bounds),
        };
        out = union(
            out,
            mine.map(|b| moved_box(b, child.rotation_y_deg, child.offset)),
        );
    }
    out
}

/// `b` turned by `yaw_deg` about its centre, then moved by the x and z of `offset`: the
/// smallest box around it.
fn moved_box(b: GroundBox, yaw_deg: f64, offset: [f64; 3]) -> GroundBox {
    let (s, c) = deg_to_rad(yaw_deg).psin_cos();
    let (s, c) = (s.abs(), c.abs());
    let (hx, hz) = ((b.1[0] - b.0[0]) / 2.0, (b.1[1] - b.0[1]) / 2.0);
    let (cx, cz) = (
        (b.0[0] + b.1[0]) / 2.0 + offset[0],
        (b.0[1] + b.1[1]) / 2.0 + offset[2],
    );
    // The extent of a box turned by an angle along each axis of the frame.
    let (ex, ez) = (hx * c + hz * s, hx * s + hz * c);
    ([cx - ex, cz - ez], [cx + ex, cz + ez])
}

/// The smallest box around what an instance of `prefab` draws and collides with at scale 1,
/// in the instance's own frame: [`components_box`] of its own components and of every child's
/// (a child that places a prefab adds that prefab's, nested included), each turned by the
/// child's rotation and moved to its offset. A child without a shape or a model with bounds
/// adds nothing. `None` when nothing can be measured.
pub fn prefab_box(
    prefabs: &[Prefab],
    prefab: &Prefab,
    bounds: &BTreeMap<String, ModelBounds>,
) -> Option<GroundBox> {
    prefab_box_at(prefabs, prefab, bounds, 1)
}

impl crate::World {
    /// [`prefab_box`] of `prefab` among this world's prefabs and model bounds: the smallest
    /// box, in an instance's own frame at scale 1, around what an instance of it draws and
    /// collides with. `None` when nothing can be measured.
    pub fn prefab_box(&self, prefab: &Prefab) -> Option<GroundBox> {
        prefab_box(&self.prefabs, prefab, self.all_model_bounds())
    }
}
