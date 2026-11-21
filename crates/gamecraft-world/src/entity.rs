//! Entities: named things in the world, each with a stable id and a set of components.

use crate::error::Ctx;
use crate::limits::{MAX_COORDINATE_M, SCALE_MAX, SCALE_MIN};
use crate::{
    Behaviour, Character, Component, Health, HeldModel, Interactable, Launcher, Light, Model,
    Plant, Shape, WorldError, Zone,
};
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

/// Most characters in a layer name.
pub const MAX_LAYER_CHARS: usize = 64;

/// Whether `layer` is a valid layer name: 1 to [`MAX_LAYER_CHARS`] ASCII letters, digits, `-`
/// and `_`, so that `layers/<layer>.json` is a file inside its world's folder.
pub fn is_layer_name(layer: &str) -> bool {
    !layer.is_empty()
        && layer.len() <= MAX_LAYER_CHARS
        && layer
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `layer` must be a valid layer name ([`is_layer_name`]).
fn check_layer(ctx: Ctx, layer: &str) -> Result<(), WorldError> {
    if is_layer_name(layer) {
        return Ok(());
    }
    let shown: String = layer.chars().take(MAX_LAYER_CHARS + 8).collect();
    Err(ctx.invalid(
        "layer",
        format!("{shown:?}"),
        format!(
            "a layer is a plain file name: 1 to {MAX_LAYER_CHARS} ASCII letters, digits, `-` \
             and `_` (no `/`, `.` or spaces), such as \"village\"; leave it out for the \
             world's default layer"
        ),
    ))
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
    /// The prefab this entity was placed from, by id; `None` for an entity of its own. Its
    /// `components` are then the prefab's with its overrides replacing those of the same type
    /// ([`crate::Prefab::effective`]); the file keeps only the overrides. Default none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefab: Option<String>,
    /// The entity this one belongs to (a prefab's child): it moves with it and is removed with
    /// it. Default none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<EntityId>,
    /// The layer of its world the entity belongs to: in a project, the layer file it is saved
    /// to (`layers/<layer>.json`). A plain file name of 1 to 64 letters, digits, `-` and `_`;
    /// `None` is the world's default layer. Default none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Components, at most one of each type: for a prefab's instance its effective
    /// components. Default none.
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
    /// The layer it belongs to; `None` is the world's default layer.
    pub layer: Option<String>,
}

impl Default for EntityDraft {
    /// An unnamed entity at the origin, unrotated, scale 1, without components, in the
    /// default layer.
    fn default() -> Self {
        EntityDraft {
            name: String::new(),
            position: [0.0; 3],
            rotation_y_deg: 0.0,
            scale: 1.0,
            components: Vec::new(),
            layer: None,
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
            prefab: None,
            parent: None,
            layer: self.layer,
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

    /// The launcher component, if any.
    pub fn launcher(&self) -> Option<&Launcher> {
        self.components.iter().find_map(|c| match c {
            Component::Launcher(l) => Some(l),
            _ => None,
        })
    }

    /// The health component, if any.
    pub fn health(&self) -> Option<&Health> {
        self.components.iter().find_map(|c| match c {
            Component::Health(h) => Some(h),
            _ => None,
        })
    }

    /// The health component, if any, to change.
    pub fn health_mut(&mut self) -> Option<&mut Health> {
        self.components.iter_mut().find_map(|c| match c {
            Component::Health(h) => Some(h),
            _ => None,
        })
    }

    /// The behaviour component, if any.
    pub fn behaviour(&self) -> Option<&Behaviour> {
        self.components.iter().find_map(|c| match c {
            Component::Behaviour(b) => Some(b),
            _ => None,
        })
    }

    /// The model component, if any.
    pub fn model(&self) -> Option<&Model> {
        self.components.iter().find_map(|c| match c {
            Component::Model(m) => Some(m),
            _ => None,
        })
    }

    /// The held_model component, if any.
    pub fn held_model(&self) -> Option<&HeldModel> {
        self.components.iter().find_map(|c| match c {
            Component::HeldModel(h) => Some(h),
            _ => None,
        })
    }

    /// Whether the entity has a `health` component that is used up.
    pub fn is_down(&self) -> bool {
        self.health().is_some_and(Health::is_down)
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
        if let Some(layer) = &self.layer {
            check_layer(ctx, layer)?;
        }
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
