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
use crate::limits::{MAX_COORDINATE_M, MAX_PREFAB_CHILDREN};
use crate::{Component, EntityId, WorldError};
use gamecraft_math::{Portable, deg_to_rad};
use serde::{Deserialize, Serialize};

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
    /// (metres; -Z is the parent's forward, +X its right). Default the parent's base.
    #[serde(default)]
    pub offset: [f64; 3],
    /// The child's turn relative to the parent's, degrees counter-clockwise seen from above.
    /// Default 0.
    #[serde(default)]
    pub rotation_y_deg: f64,
    /// The child's components, at most one of each type. Default none.
    #[serde(default)]
    pub components: Vec<Component>,
}

/// How to place a prefab: [`crate::World::spawn_prefab`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spawn {
    /// The instance's name; the prefab's name when `None`.
    pub name: Option<String>,
    /// Where the instance's base stands, metres.
    pub position: [f64; 3],
    /// The instance's turn about +Y, degrees.
    pub rotation_y_deg: f64,
    /// Components that replace the prefab's components of the same type (or add a type the
    /// prefab lacks), at most one of each type.
    pub overrides: Vec<Component>,
    /// The layer of the instance and its children; `None` is the world's default layer.
    pub layer: Option<String>,
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
/// `position` turned by `yaw_deg`.
pub(crate) fn place_child(
    position: [f64; 3],
    yaw_deg: f64,
    offset: [f64; 3],
    rotation_y_deg: f64,
) -> ([f64; 3], f64) {
    let [dx, dy, dz] = turn(offset, yaw_deg);
    let [px, py, pz] = position;
    ([px + dx, py + dy, pz + dz], yaw_deg + rotation_y_deg)
}

/// Where a child is relative to its parent: the inverse of [`place_child`].
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
