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
//! terrain recipe (its source, then its sculpt and paint edits, [`sculpt`]) into a
//! [`HeightField`] that answers ground height, normal and ray queries over exactly the
//! triangles the renderer draws, with the painted layers' weights; [`Terrain::rebuild`] redoes
//! only the tiles a change touches and [`tiles`] keeps resolved tiles as binary files.
//!
//! A `road` component is a centre line with a width lying on the terrain; its surface is
//! derived ([`Road::drape`]): the footprint clipped against the terrain's triangles and lifted
//! a few centimetres, so it follows the ground exactly ([`road`]).
//!
//! A `model` component names a glTF asset of a pack; the asset's bounding box is derived data
//! that the host computes and hands over with [`World::set_model_bounds`] ([`ModelBounds`]).
//!
//! No GPU, no files, no clock: everything here is deterministic. Conventions: metres, Y up,
//! the ground plane is x and z, north is -Z and east is +X; an entity's position is its base.

mod combat;
mod component;
pub mod cooked;
mod entity;
mod environment;
mod error;
mod grid_heights;
mod instance;
pub mod limits;
mod model;
mod prefab;
pub mod road;
mod scenario;
pub mod sculpt;
mod store;
mod terrain;
pub mod tiles;
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
pub use grid_heights::GridHeights;
pub use model::{
    MAX_ASSET_PATH_CHARS, MAX_MODEL_LOD_LEVELS, MODEL_EXTENSIONS, Model, ModelBounds, ModelBox,
    ModelCollider, is_asset_path, is_model_path,
};
pub use prefab::{Prefab, PrefabChild, Spawn, Spawned};
pub use road::{
    DrapeStats, DrapedPolygon, Retiered, Road, RoadEnds, RoadMesh, free_road_tier, raycast_roads,
    retier_roads,
};
pub use scenario::{
    CompareOp, Condition, EntityRef, Objective, ObjectiveStatus, Step, TAG_PREFIX, Trigger,
    VarValue, When,
};
pub use sculpt::{PaintRule, TerrainEdit, TerrainLayer, is_terrain_layer_name};
pub use terrain::{HeightField, Rebuilt, Terrain, TerrainSource};
pub use tiles::CachedBuild;
pub use world::{FORMAT_VERSION, World};
