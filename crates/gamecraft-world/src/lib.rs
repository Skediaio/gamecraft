//! The GameCraft world document (layer L1).
//!
//! A [`World`] is the source of truth of a game world: its terrain recipe, its environment
//! (time, sun, weather), its prefabs, its entities with their components, and its scenario: variables,
//! objectives and triggers whose actions are commands. It is pure data with serde:
//! [`World::to_json`] and [`World::from_json`] read and write the `.gamecraft` file format,
//! version [`FORMAT_VERSION`], with a stable field and entity order so documents diff and merge.
//! [`World::validate`] checks every invariant and names the field, the value and the allowed
//! range when one is broken.
//!
//! Derived data is rebuilt from the document, never stored in it: [`Terrain::build`] turns the
//! terrain recipe into a [`HeightField`] that answers ground height, normal and ray queries
//! over exactly the triangles the renderer draws.
//!
//! No GPU, no files, no clock: everything here is deterministic. Conventions: metres, Y up,
//! the ground plane is x and z, north is -Z and east is +X; an entity's position is its base.

mod combat;
mod component;
mod entity;
mod environment;
mod error;
pub mod limits;
mod prefab;
mod scenario;
mod terrain;
mod world;

#[cfg(test)]
mod tests;

pub use combat::{Behaviour, BehaviourMode, Health, Launcher};
pub use component::{
    Character, Component, Interactable, Light, Plant, Shape, ShapeKind, Tags, Zone, ZoneShape,
};
pub use entity::{Entity, EntityDraft, EntityId, MAX_LAYER_CHARS, is_layer_name};
pub use environment::{Environment, STANDARD_TEMPERATURE_C, SunPosition};
pub use error::WorldError;
pub use prefab::{Prefab, PrefabChild, Spawn, Spawned};
pub use scenario::{
    CompareOp, Condition, EntityRef, Objective, ObjectiveStatus, Step, TAG_PREFIX, Trigger,
    VarValue, When,
};
pub use terrain::{HeightField, Terrain, TerrainSource};
pub use world::{FORMAT_VERSION, World};
