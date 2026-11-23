//! Layering packs into one world: prefabs and entities by id, `world.json` field by field.

use crate::ProjectError;
use crate::fields::{PROJECT_KEYS, merge};
use crate::layer::{Entry, LayerEntity};
use crate::pack::{Pack, PrefabEntry};
use crate::paths::layer_path;
use gamecraft_world::{EntityId, Prefab, TerrainEdit, World, WorldError};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// An entity of the resolved world, with where it came from.
#[derive(Clone, Debug)]
pub(crate) struct Slot {
    pub key: String,
    pub pack: String,
    pub layer: String,
    pub file: String,
    pub entity: LayerEntity,
}

/// What a list of packs resolves to for one world.
#[derive(Clone, Debug, Default)]
pub(crate) struct Resolved {
    /// Whether any of the packs has a folder for the world.
    pub has_world: bool,
    /// The merged `world.json` fields (with the project's own keys).
    pub fields: Map<String, Value>,
    /// Which pack set each top-level field last.
    pub field_packs: BTreeMap<String, String>,
    /// The prefabs by id, with the pack each came from.
    pub prefabs: BTreeMap<String, (Prefab, String)>,
    /// The entities, in the order they were first defined.
    pub slots: Vec<Slot>,
    /// Removals of ids no earlier pack defines.
    pub warnings: Vec<String>,
    /// The terrain edits of the last pack with a `terrain/edits.json`, and that file.
    pub edits: Option<(Vec<TerrainEdit>, String)>,
}

impl Resolved {
    /// The default layer the merged fields name, else `main`.
    pub(crate) fn default_layer(&self) -> String {
        self.fields
            .get("default_layer")
            .and_then(Value::as_str)
            .unwrap_or("main")
            .to_string()
    }

    /// The cell size the merged fields give, when the layers are split into cells.
    pub(crate) fn cell_size(&self) -> Option<f64> {
        self.fields.get("cell_size_m").and_then(Value::as_f64)
    }

    /// The order of the layers the merged fields give.
    pub(crate) fn layer_order(&self) -> Vec<String> {
        self.fields
            .get("layers")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The layer names of a world folder in load order: those `order` lists first, in its order,
/// then the others by name.
pub(crate) fn ordered<'a>(
    names: impl Iterator<Item = &'a String>,
    order: &[String],
) -> Vec<String> {
    let names: Vec<&String> = names.collect();
    let mut out: Vec<String> = order
        .iter()
        .filter(|o| names.contains(o))
        .cloned()
        .collect();
    let mut rest: Vec<String> = names
        .into_iter()
        .filter(|n| !order.contains(n))
        .cloned()
        .collect();
    rest.sort();
    out.extend(rest);
    out
}

/// Resolves `world` over `packs`, in load order.
pub(crate) fn resolve(packs: &[&Pack], world: &str) -> Result<Resolved, ProjectError> {
    let mut r = Resolved::default();
    for pack in packs {
        for (id, entry) in &pack.prefabs {
            match entry {
                PrefabEntry::Prefab(p) => {
                    r.prefabs
                        .insert(id.clone(), (p.clone(), pack.manifest.id.clone()));
                }
                PrefabEntry::Removed => {
                    if r.prefabs.remove(id).is_none() {
                        r.warnings.push(format!(
                            "`{}` removes the prefab `{id}`, which no earlier pack has",
                            pack.file(&crate::paths::prefab_path(id))
                        ));
                    }
                }
            }
        }
        let Some(dir) = pack.worlds.get(world) else {
            continue;
        };
        r.has_world = true;
        if let Some(edits) = &dir.edits {
            r.edits = Some((edits.clone(), pack.file(&crate::terrain::edits_path(world))));
        }
        if let Some(fields) = &dir.fields {
            merge(&mut r.fields, fields);
            for key in fields.keys() {
                r.field_packs.insert(key.clone(), pack.manifest.id.clone());
            }
        }
    }
    let order = r.layer_order();
    let mut slots: Vec<Option<Slot>> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for pack in packs {
        let Some(dir) = pack.worlds.get(world) else {
            continue;
        };
        let mut seen: BTreeMap<&str, String> = BTreeMap::new();
        for layer_name in ordered(dir.layers.keys(), &order) {
            let Some(layer) = dir.layers.get(&layer_name) else {
                continue;
            };
            let file = pack.file(&layer_path(world, &layer_name));
            if layer.replace {
                for slot in &mut slots {
                    if slot.as_ref().is_some_and(|s| s.layer == layer_name)
                        && let Some(s) = slot.take()
                    {
                        index.remove(&s.key);
                    }
                }
            }
            for entry in &layer.entries {
                let key = entry.key();
                if let Some(earlier) = seen.insert(key, file.clone()) {
                    return Err(ProjectError::new(format!(
                        "the entity id `{key}` is used twice in the pack `{}`: in `{earlier}` \
                         and in `{file}`; an id is unique within its pack",
                        pack.manifest.id
                    )));
                }
                match entry {
                    Entry::Removed(_) => match index.remove(key) {
                        Some(i) => {
                            if let Some(slot) = slots.get_mut(i) {
                                *slot = None;
                            }
                        }
                        None => r.warnings.push(format!(
                            "`{file}` removes the entity `{key}`, which no earlier pack has"
                        )),
                    },
                    Entry::Entity(e) => {
                        let slot = Slot {
                            key: key.to_string(),
                            pack: pack.manifest.id.clone(),
                            layer: layer_name.clone(),
                            file: file.clone(),
                            entity: e.clone(),
                        };
                        match index.get(key).and_then(|i| slots.get_mut(*i)) {
                            Some(existing) => *existing = Some(slot),
                            None => {
                                index.insert(key.to_string(), slots.len());
                                slots.push(Some(slot));
                            }
                        }
                    }
                }
            }
        }
    }
    r.slots = slots.into_iter().flatten().collect();
    Ok(r)
}

/// The world `r` describes: the slots get runtime ids 1, 2, ... in their order. Every rule of
/// the world document is checked; an error names the entity's string id and file.
pub(crate) fn build_world(r: &Resolved, world_name: &str) -> Result<World, ProjectError> {
    if r.slots.len() > gamecraft_world::limits::MAX_ENTITIES {
        return Err(ProjectError::new(format!(
            "the world `{world_name}` has {} entities; a world holds at most {}",
            r.slots.len(),
            gamecraft_world::limits::MAX_ENTITIES
        )));
    }
    let ids: BTreeMap<&str, u64> = r
        .slots
        .iter()
        .enumerate()
        .map(|(i, s)| (s.key.as_str(), i as u64 + 1))
        .collect();
    let mut entities = Vec::with_capacity(r.slots.len());
    for (i, slot) in r.slots.iter().enumerate() {
        let parent = match &slot.entity.parent {
            None => None,
            Some(p) => Some(*ids.get(p.as_str()).ok_or_else(|| {
                ProjectError::in_file(
                    &slot.file,
                    format!(
                        "the entity `{}` names the parent `{p}`, which no layer of the world \
                         `{world_name}` defines",
                        slot.key
                    ),
                )
            })?),
        };
        entities.push(slot.entity.to_world_json(i as u64 + 1, parent, &slot.layer));
    }
    let mut doc = r.fields.clone();
    for key in PROJECT_KEYS {
        doc.remove(key);
    }
    if let Some((edits, file)) = &r.edits {
        match doc.get_mut("terrain") {
            Some(Value::Object(terrain)) => {
                let list = serde_json::to_value(edits).map_err(|e| {
                    ProjectError::in_file(file, format!("cannot read the edits: {e}"))
                })?;
                terrain.insert("edits".into(), list);
            }
            _ if edits.is_empty() => {}
            _ => {
                return Err(ProjectError::in_file(
                    file,
                    format!(
                        "the world `{world_name}` has no terrain, so it has no terrain edits;                          add a terrain to its world.json or remove this file"
                    ),
                ));
            }
        }
    }
    doc.entry("name")
        .or_insert_with(|| Value::String(world_name.to_string()));
    doc.insert("gamecraft".into(), json!(gamecraft_world::FORMAT_VERSION));
    let prefabs: Vec<&Prefab> = r.prefabs.values().map(|(p, _)| p).collect();
    doc.insert("prefabs".into(), json!(prefabs));
    doc.insert("next_entity_id".into(), json!(r.slots.len() as u64 + 1));
    doc.insert("entities".into(), Value::Array(entities));
    let text = Value::Object(doc).to_string();
    World::from_json(&text).map_err(|e| {
        let packs: Vec<&str> = r.field_packs.values().map(String::as_str).collect();
        let edits_file = r
            .edits
            .as_ref()
            .filter(|_| is_edit_error(&e))
            .map(|(_, f)| f);
        match entity_of(&e).and_then(|id| r.slots.get(id.0.saturating_sub(1) as usize)) {
            Some(slot) => {
                ProjectError::in_file(&slot.file, format!("the entity `{}`: {e}", slot.key))
            }
            None if edits_file.is_some() => ProjectError::in_file(
                edits_file.map_or("", String::as_str),
                format!("the world `{world_name}`: {e}"),
            ),
            None if packs.is_empty() => {
                ProjectError::new(format!("the world `{world_name}` is not valid: {e}"))
            }
            None => ProjectError::new(format!(
                "the world `{world_name}` (its world.json fields from {}) is not valid: {e}",
                packs.join(", ")
            )),
        }
    })
}

/// Whether an error of the world document is about a terrain edit.
fn is_edit_error(e: &WorldError) -> bool {
    matches!(e, WorldError::Invalid { field, .. } if field.starts_with("terrain.edits"))
}

/// The entity an error of the world document is about.
fn entity_of(e: &WorldError) -> Option<EntityId> {
    match e {
        WorldError::Invalid { entity, .. } => *entity,
        WorldError::DuplicateEntityId { id } | WorldError::EntityIdNotBelowNext { id, .. } => {
            Some(*id)
        }
        WorldError::DuplicateComponent { entity, .. } => Some(*entity),
        _ => None,
    }
}
