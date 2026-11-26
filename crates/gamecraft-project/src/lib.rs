//! Projects and packs (layer L2, `docs/design/projects.md`): how a game lives on disk as a
//! folder of text files, and how mods change it.
//!
//! A **pack** is a folder: a manifest (`pack.json`), prefab files (`prefabs/<id>.json`, one
//! prefab each) and worlds (`worlds/<world>/world.json` for the terrain recipe, environment
//! and scenario, and `worlds/<world>/layers/<layer>.json`, each a list of entities with string
//! ids, one entity per line). A game is a pack with worlds; a mod is a pack that depends on
//! the game. A project is a root pack and its dependencies.
//!
//! Packs load in dependency order ([`dependency_order`]) and a later pack changes an earlier
//! one by id: a prefab file with the same id replaces the prefab (`{"id", "removed": true}`
//! removes it); an entity in a later layer with an id that exists replaces that entity (in
//! the later layer) and `{"id", "removed": true}` removes it; a layer file with
//! `"replace": true` first drops the layer of the same name of earlier packs; a new layer
//! name adds a layer; `world.json` fields merge field by field (a JSON merge patch).
//!
//! [`Project::load`] turns the packs' files (maps of path to text, [`PackSource`]) into one
//! [`gamecraft_world::World`] and remembers where every entity, prefab and world field came
//! from ([`Provenance`]). [`Project::plan_save`] goes the other way: from a changed world to
//! the minimal set of file texts to write, each change in the file it belongs to in the root
//! pack; a change to something a dependency brought becomes an override in the root pack, so
//! editing a game you depend on makes a mod and never edits the game. Output is pretty and
//! stable, and saving an unchanged world writes nothing.
//!
//! A pack's **assets** live under `assets/`, each with a licence record beside it
//! ([`assets`]: CC0 or CC-BY only); [`Project::assets`] gives the assets of all packs, a later
//! pack's asset replacing an earlier one's of the same path. [`model::inspect`] reads a glTF
//! model's triangle count and bounding box from its bytes, and [`model::resolve_uri`] names the
//! file a model refers to inside `assets/`. Its **levels of detail** ([`lod`]: the
//! `_LOD<n>` names and the `MSFT_lod` extension of glTF, and the distances the levels change
//! at) are found there by the same walk for the host and the renderer. The images the models share are the pack's
//! **texture store** ([`textures`]): an import extracts them into `assets/textures/<hash>.<ext>`.
//! A model's **collision parts** ([`model::Collider`]): the boxes a `mesh` collider collides as,
//! from the file's `*_collider` nodes or from its meshes, measured by the same walk.
//!
//! Pure: no files, no clock. The host reads and writes the files (`gamecraft-automation`).

pub mod assets;
pub mod bake;
pub mod cells;
mod error;
mod fields;
pub mod heightmap;
mod layer;
pub mod lod;
mod manifest;
pub mod model;
mod pack;
mod paths;
mod project;
mod resolve;
mod terrain;
mod text;
pub mod textures;

pub use assets::{
    ASSETS_DIR, AssetRecord, License, PackAsset, RECORD_SUFFIX, check_assets, credits, record_path,
};
pub use bake::{BAKE_FILE, BAKE_FORMAT, BakeRecord};
pub use cells::{Cell, DEFAULT_CELL_SIZE_M, cells_in_area};
pub use error::ProjectError;
pub use fields::WORLD_FORMAT;
pub use heightmap::{
    ElevationFormat, HEIGHTMAP_DIR, MAX_ELEVATION_BYTES, TILE_BYTES, TILE_HEADER,
    TILE_HEADER_BYTES, Tile, decode_tile, encode_tile, encode_tiles, read_elevation,
    tile_file_name,
};
pub use layer::LAYER_FORMAT;
pub use lod::{
    DRAW_COVERAGE, Levels, LodDistances, LodOverride, LodSource, MIN_MODEL_SIZE_M,
    REFERENCE_FOV_Y_DEG, REFERENCE_HEIGHT_PX, SWITCH_COVERAGE, check_coverage, coarser_check,
    extend_bounds, follows_scale, from_coverage, levels_of, lod_name, rule, size_across,
};
pub use manifest::{Dependency, MAX_PACKS, Manifest, PACK_FORMAT, dependency_order};
pub use model::{
    COLLIDER_SUFFIXES, Collider, ColliderSource, MAX_MODEL_PARTS, ModelSummary, inspect,
    is_collider_name,
};
pub use pack::{MAX_LAYERS, PackSource};
pub use paths::{
    MAX_ENTITY_KEY_CHARS, MAX_PLAIN_NAME_CHARS, MAX_PREFAB_FILE_CHARS, is_entity_key,
    is_plain_name, is_prefab_file_id,
};
pub use project::{
    DEFAULT_LAYER, EntitySource, PackInfo, Project, ProjectAsset, Provenance, SavePlan, Saving,
    manifests, pack_assets,
};
pub use terrain::TERRAIN_EDITS_FORMAT;
pub use textures::{TEXTURES_DIR, content_hash, is_texture};
