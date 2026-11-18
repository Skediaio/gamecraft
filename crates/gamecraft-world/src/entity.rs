//! Entities: named things in the world, each with a stable id and a set of components.

use crate::error::Ctx;
use crate::limits::{MAX_COORDINATE_M, SCALE_MAX, SCALE_MIN};
use crate::{Character, Component, Interactable, Light, Plant, Shape, WorldError, Zone};
use serde::{Deserialize, Serialize};
use std::fmt;

/// The stable id of an entity. It survives save, load and undo, and is never reused within a
/// document. Serialised as a plain number; displayed as one (`12`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub u64);

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

fn default_scale() -> f64 {
    1.0
}

/// An entity of the world: a name, a placement and components.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    /// Stable id, unique in the world and below its `next_entity_id`.
    pub id: EntityId,
    /// Name, at most 200 characters; need not be unique. Default empty.
    #[serde(default)]
    pub name: String,
    /// World position of the entity's base (the point it stands on), metres. Default origin.
    #[serde(default)]
    pub position: [f64; 3],
    /// Rotation about +Y in degrees, counter-clockwise seen from above. Default 0.
    #[serde(default)]
    pub rotation_y_deg: f64,
    /// Uniform scale, 0.001 to 1000. Default 1.
    #[serde(default = "default_scale")]
    pub scale: f64,
    /// Components, at most one of each type. Default none.
    #[serde(default)]
    pub components: Vec<Component>,
}

/// Everything needed to add an entity except its id, which the world allocates.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityDraft {
    /// Name, at most 200 characters.
    pub name: String,
    /// World position of the base, metres.
    pub position: [f64; 3],
    /// Rotation about +Y in degrees.
    pub rotation_y_deg: f64,
    /// Uniform scale.
    pub scale: f64,
    /// Components, at most one of each type.
    pub components: Vec<Component>,
}

impl Default for EntityDraft {
    /// An unnamed entity at the origin, unrotated, scale 1, without components.
    fn default() -> Self {
        EntityDraft {
            name: String::new(),
            position: [0.0; 3],
            rotation_y_deg: 0.0,
            scale: 1.0,
            components: Vec::new(),
        }
    }
}

impl EntityDraft {
    /// The entity this draft becomes under `id`.
    pub(crate) fn into_entity(self, id: EntityId) -> Entity {
        Entity {
            id,
            name: self.name,
            position: self.position,
            rotation_y_deg: self.rotation_y_deg,
            scale: self.scale,
            components: self.components,
        }
    }
}

impl Entity {
    /// The shape component, if any.
    pub fn shape(&self) -> Option<&Shape> {
        self.components.iter().find_map(|c| match c {
            Component::Shape(s) => Some(s),
            _ => None,
        })
    }

    /// The plant component, if any.
    pub fn plant(&self) -> Option<&Plant> {
        self.components.iter().find_map(|c| match c {
            Component::Plant(p) => Some(p),
            _ => None,
        })
    }

    /// The character component, if any.
    pub fn character(&self) -> Option<&Character> {
        self.components.iter().find_map(|c| match c {
            Component::Character(ch) => Some(ch),
            _ => None,
        })
    }

    /// The zone component, if any.
    pub fn zone(&self) -> Option<&Zone> {
        self.components.iter().find_map(|c| match c {
            Component::Zone(z) => Some(z),
            _ => None,
        })
    }

    /// The interactable component, if any.
    pub fn interactable(&self) -> Option<&Interactable> {
        self.components.iter().find_map(|c| match c {
            Component::Interactable(i) => Some(i),
            _ => None,
        })
    }

    /// The light component, if any.
    pub fn light(&self) -> Option<&Light> {
        self.components.iter().find_map(|c| match c {
            Component::Light(l) => Some(l),
            _ => None,
        })
    }

    /// The entity's tags, empty without a `tags` component.
    pub fn tags(&self) -> &[String] {
        self.components
            .iter()
            .find_map(|c| match c {
                Component::Tags(t) => Some(t.tags.as_slice()),
                _ => None,
            })
            .unwrap_or(&[])
    }

    /// Whether the entity carries the tag `tag` (matched exactly).
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags().iter().any(|t| t == tag)
    }

    /// Adds `component`, replacing the existing component of the same type in place.
    pub fn set_component(&mut self, component: Component) {
        let kind = component.kind();
        match self.components.iter_mut().find(|c| c.kind() == kind) {
            Some(slot) => *slot = component,
            None => self.components.push(component),
        }
    }

    /// Removes and returns the component of type `kind` (one of [`Component::KINDS`]).
    pub fn remove_component(&mut self, kind: &str) -> Option<Component> {
        let index = self.components.iter().position(|c| c.kind() == kind)?;
        Some(self.components.remove(index))
    }

    /// Checks every rule of the entity on its own: name length, finite position within
    /// 1e6 m per axis, finite rotation, scale range, at most one component of each type, and
    /// each component's rules. The error names the entity id and the field.
    pub fn validate(&self) -> Result<(), WorldError> {
        let ctx = Ctx::entity(self.id);
        ctx.name("name", &self.name)?;
        for (axis, &v) in self.position.iter().enumerate() {
            ctx.range(
                format!("position[{axis}]"),
                v,
                -MAX_COORDINATE_M,
                MAX_COORDINATE_M,
                " metres",
            )?;
        }
        ctx.finite("rotation_y_deg", self.rotation_y_deg)?;
        ctx.range("scale", self.scale, SCALE_MIN, SCALE_MAX, "")?;
        for (i, c) in self.components.iter().enumerate() {
            let kind = c.kind();
            if self
                .components
                .iter()
                .take(i)
                .any(|earlier| earlier.kind() == kind)
            {
                return Err(WorldError::DuplicateComponent {
                    entity: self.id,
                    kind: kind.to_string(),
                });
            }
        }
        for c in &self.components {
            c.validate(ctx)?;
        }
        Ok(())
    }
}
