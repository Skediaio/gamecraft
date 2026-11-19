//! One pack's files, parsed: its manifest, prefab files and worlds with their layers.

use crate::ProjectError;
use crate::assets::{PackAsset, check_assets};
use crate::fields::parse_world;
use crate::layer::Layer;
use crate::manifest::Manifest;
use crate::paths::{FileKind, classify};
use gamecraft_world::{Prefab, TerrainEdit};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Most layer files in one world of one pack.
pub const MAX_LAYERS: usize = 1024;

/// The files of one pack as the host read them: paths relative to the pack's folder (`/`
/// between parts) to their text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackSource {
    /// Where the pack is, for people (a folder path); errors name files as `location/path`.
    pub location: String,
    /// Path to text: `pack.json`, `prefabs/<id>.json`, `worlds/<world>/world.json`,
    /// `worlds/<world>/layers/<layer>.json`, `worlds/<world>/terrain/edits.json`. Other paths
    /// are not read. Also the licence records (`assets/**.license.json`) and the `.gltf`
    /// documents under `assets/`, which [`crate::check_assets`] reads.
    pub files: BTreeMap<String, String>,
    /// Every file under `assets/` (pack-relative path, `assets/...`) with its size in bytes:
    /// the assets, the files `.gltf` models refer to and the licence records.
    pub assets: BTreeMap<String, u64>,
}

/// A prefab file: a prefab, or a marker that removes the prefab of an earlier pack.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PrefabEntry {
    Prefab(Prefab),
    Removed,
}

/// One world folder of a pack.
#[derive(Clone, Debug, Default)]
pub(crate) struct WorldDir {
    /// `world.json` without its version, if the pack has one.
    pub fields: Option<Map<String, Value>>,
    /// The layer files by name.
    pub layers: BTreeMap<String, Layer>,
    /// `terrain/edits.json`, if the pack has one.
    pub edits: Option<Vec<TerrainEdit>>,
}

/// A parsed pack.
#[derive(Clone, Debug)]
pub(crate) struct Pack {
    pub manifest: Manifest,
    pub location: String,
    pub prefabs: BTreeMap<String, PrefabEntry>,
    pub worlds: BTreeMap<String, WorldDir>,
    /// The pack's assets with their records, checked.
    pub assets: Vec<PackAsset>,
}

impl Pack {
    /// `location/path`, how errors name a file of this pack.
    pub(crate) fn file(&self, path: &str) -> String {
        format!("{}/{path}", self.location)
    }
}

/// The manifest of a pack's files.
pub(crate) fn manifest_of(source: &PackSource) -> Result<Manifest, ProjectError> {
    let file = format!("{}/pack.json", source.location);
    let text = source.files.get("pack.json").ok_or_else(|| {
        ProjectError::in_file(
            &file,
            "missing; a pack is a folder with a pack.json: {\"gamecraft_pack\": 1, \"id\": ..., \
             \"name\": ...}",
        )
    })?;
    Manifest::parse(text, &file)
}

/// Parses every file of `source`.
pub(crate) fn parse_pack(source: &PackSource) -> Result<Pack, ProjectError> {
    let manifest = manifest_of(source)?;
    let mut pack = Pack {
        manifest,
        location: source.location.clone(),
        prefabs: BTreeMap::new(),
        worlds: BTreeMap::new(),
        assets: check_assets(source)?,
    };
    for (path, text) in &source.files {
        let file = pack.file(path);
        if path.starts_with("assets/") {
            continue;
        }
        match classify(path, &source.location)? {
            FileKind::Manifest | FileKind::Other => {}
            FileKind::Prefab(id) => {
                let entry = prefab_entry(text, &file, &id)?;
                if pack.prefabs.len() >= gamecraft_world::limits::MAX_PREFABS {
                    return Err(ProjectError::in_file(
                        &file,
                        format!(
                            "a pack holds at most {} prefabs",
                            gamecraft_world::limits::MAX_PREFABS
                        ),
                    ));
                }
                pack.prefabs.insert(id, entry);
            }
            FileKind::World(world) => {
                let fields = parse_world(text, &file)?;
                pack.worlds.entry(world).or_default().fields = Some(fields);
            }
            FileKind::TerrainEdits(world) => {
                let edits = crate::terrain::parse_edits(text, &file)?;
                pack.worlds.entry(world).or_default().edits = Some(edits);
            }
            FileKind::Layer(world, layer) => {
                let parsed = Layer::parse(text, &file)?;
                let dir = pack.worlds.entry(world).or_default();
                if dir.layers.len() >= MAX_LAYERS {
                    return Err(ProjectError::in_file(
                        &file,
                        format!("a world of a pack holds at most {MAX_LAYERS} layer files"),
                    ));
                }
                dir.layers.insert(layer, parsed);
            }
        }
    }
    Ok(pack)
}

/// A prefab file whose name says `id`.
fn prefab_entry(text: &str, file: &str, id: &str) -> Result<PrefabEntry, ProjectError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| ProjectError::in_file(file, format!("not valid JSON: {e}")))?;
    if let Some(removed) = value.get("removed") {
        let only_id = value.as_object().is_some_and(|m| m.len() == 2)
            && value.get("id").and_then(Value::as_str) == Some(id);
        if removed != &Value::Bool(true) || !only_id {
            return Err(ProjectError::in_file(
                file,
                format!("a removal is exactly {{\"id\": \"{id}\", \"removed\": true}}"),
            ));
        }
        return Ok(PrefabEntry::Removed);
    }
    let prefab: Prefab = serde_json::from_value(value)
        .map_err(|e| ProjectError::in_file(file, format!("not a valid prefab: {e}")))?;
    if prefab.id != id {
        return Err(ProjectError::in_file(
            file,
            format!(
                "the prefab's id is {:?}; a prefab file is named after its id, so it must be \
                 {id:?} (or rename the file)",
                prefab.id
            ),
        ));
    }
    prefab
        .validate()
        .map_err(|e| ProjectError::in_file(file, e))?;
    Ok(PrefabEntry::Prefab(prefab))
}
