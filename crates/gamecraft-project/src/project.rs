//! A project: a root pack and its dependencies, layered into one world, with where everything
//! came from, so that a changed world is written back to the files it belongs to.

use crate::ProjectError;
use crate::assets::PackAsset;
use crate::cells::{Cell, cell_path, check_cell_size};
use crate::fields::{PROJECT_KEYS, WORLD_FORMAT, WORLD_KEYS, diff, world_fields};
use crate::layer::{Entry, Layer, LayerEntity};
use crate::manifest::{Manifest, dependency_order};
use crate::pack::{Pack, PackSource, manifest_of, parse_pack};
use crate::paths::{is_plain_name, is_prefab_file_id, layer_path, prefab_path, world_path};
use crate::resolve::{Resolved, Slot, build_world, ordered, resolve};
use crate::terrain::{edits_path, edits_text};
use crate::text::{pretty_ordered, pretty_typed};
use gamecraft_world::{Entity, EntityId, TerrainEdit, World};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The layer of a world when its `world.json` names none.
pub const DEFAULT_LAYER: &str = "main";
/// Longest string id generated for a new entity before its number, in characters.
const MAX_GENERATED_KEY_CHARS: usize = 48;

/// Where an entity of the resolved world came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntitySource {
    /// Its string id, unique within its pack.
    pub key: String,
    /// The pack whose file defined it last.
    pub pack: String,
    /// The layer file it is in (`layers/<layer>.json`).
    pub layer: String,
}

/// Where everything of the resolved world came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provenance {
    /// Every entity loaded or saved, by runtime id.
    pub entities: BTreeMap<EntityId, EntitySource>,
    /// The pack each prefab came from, by prefab id.
    pub prefabs: BTreeMap<String, String>,
    /// The pack that set each top-level `world.json` field last.
    pub world_fields: BTreeMap<String, String>,
}

/// A pack of the project, in load order.
#[derive(Clone, Debug, PartialEq)]
pub struct PackInfo {
    /// Its manifest.
    pub manifest: Manifest,
    /// Where it is, as the host named it.
    pub location: String,
}

/// An asset of the project: the pack that gives it (the last one in load order with that path)
/// and the asset with its record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectAsset {
    /// The id of the pack it comes from.
    pub pack: String,
    /// Where that pack is, as the host named it.
    pub location: String,
    /// The asset, its record and its files.
    pub asset: PackAsset,
}

/// What a save writes, paths relative to the root pack's folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavePlan {
    /// Files to write (created or replaced), path to text.
    pub writes: BTreeMap<String, String>,
    /// Files to delete (a prefab of the root pack that was removed).
    pub removes: Vec<String>,
}

impl SavePlan {
    /// Whether the save writes nothing.
    pub fn is_empty(&self) -> bool {
        self.writes.is_empty() && self.removes.is_empty()
    }
}

/// A planned save: what to write, and the state the project is in once it is written
/// ([`Project::finish_save`]).
#[derive(Clone, Debug)]
pub struct Saving {
    /// The files.
    pub plan: SavePlan,
    next: Next,
}

#[derive(Clone, Debug)]
struct Next {
    provenance: Provenance,
    own: Own,
    saved: World,
    saved_fields: Map<String, Value>,
}

/// Most levels of parents followed to find the entity whose position places a line in a cell.
const MAX_ROOT_STEPS: usize = gamecraft_world::limits::MAX_HIERARCHY_DEPTH + 1;

/// What the dependencies alone resolve to: what the root pack's files are a difference from.
#[derive(Clone, Debug, Default)]
struct Base {
    has_world: bool,
    fields: Map<String, Value>,
    /// The terrain edits the dependencies give.
    edits: Vec<TerrainEdit>,
    entities: BTreeMap<String, (String, LayerEntity)>,
    prefabs: BTreeSet<String>,
}

/// The root pack's own files of the world, as they are on disk (normalised).
#[derive(Clone, Debug, Default)]
struct Own {
    /// `world.json` without its version; `None` when the root pack has none.
    fields: Option<Map<String, Value>>,
    layers: BTreeMap<String, Layer>,
    /// The layers that are split into cells, each cell's file (normalised); the other layers
    /// of `layers` are one file each.
    cells: BTreeMap<String, BTreeMap<Cell, Layer>>,
    prefab_files: BTreeSet<String>,
    /// `terrain/edits.json`; `None` when the root pack has none.
    edits: Option<Vec<TerrainEdit>>,
}

/// An open project: the root pack, its dependencies in load order, the world being edited and
/// where every entity, prefab and world field came from.
#[derive(Clone, Debug)]
pub struct Project {
    root: String,
    world: String,
    packs: Vec<PackInfo>,
    worlds: Vec<String>,
    default_layer: String,
    layer_order: Vec<String>,
    provenance: Provenance,
    base: Base,
    own: Own,
    saved: World,
    saved_fields: Map<String, Value>,
    warnings: Vec<String>,
    assets: BTreeMap<String, ProjectAsset>,
    /// The side of the cells the root pack's layers are saved in; `None`: one file per layer.
    cell_size: Option<f64>,
    /// The cells it was loaded with, when only some were: such a project is not saved.
    partial: Option<BTreeSet<Cell>>,
}

/// The manifests of `packs`, by id; each key must be its pack's id.
pub fn manifests(
    packs: &BTreeMap<String, PackSource>,
) -> Result<BTreeMap<String, Manifest>, ProjectError> {
    let mut out = BTreeMap::new();
    for (id, source) in packs {
        let manifest = manifest_of(source)?;
        if &manifest.id != id {
            return Err(ProjectError::in_file(
                &format!("{}/pack.json", source.location),
                format!(
                    "the pack's id is `{}`, but it was found as the pack `{id}`; a dependency \
                     names a pack by the id in its pack.json",
                    manifest.id
                ),
            ));
        }
        out.insert(id.clone(), manifest);
    }
    Ok(out)
}

impl Project {
    /// Loads the project whose root pack is `root`: orders `packs` by their dependencies,
    /// layers them and resolves `world` (by default the first world of the root pack, else of
    /// the nearest dependency that lists one). Returns the project and the world.
    pub fn load(
        root: &str,
        packs: &BTreeMap<String, PackSource>,
        world: Option<&str>,
    ) -> Result<(Project, World), ProjectError> {
        Project::load_cells(root, packs, world, None)
    }

    /// [`Project::load`] of only the entities in `cells` of layers split into cells (every
    /// entity of a layer that is one file is loaded): what a streamer loads near a player.
    /// Such a project refuses to save, since the other cells are not in its world.
    pub fn load_cells(
        root: &str,
        packs: &BTreeMap<String, PackSource>,
        world: Option<&str>,
        cells: Option<&BTreeSet<Cell>>,
    ) -> Result<(Project, World), ProjectError> {
        let manifests = manifests(packs)?;
        let order = dependency_order(root, &manifests)?;
        let mut parsed = Vec::with_capacity(order.len());
        for id in &order {
            let source = packs
                .get(id)
                .ok_or_else(|| ProjectError::new(format!("the pack `{id}` is missing")))?;
            parsed.push(parse_pack(source, cells)?);
        }
        let mut worlds: Vec<String> = Vec::new();
        for pack in &parsed {
            for w in &pack.manifest.worlds {
                if !worlds.contains(w) {
                    worlds.push(w.clone());
                }
            }
        }
        for pack in &parsed {
            if let Some(w) = pack.worlds.keys().find(|w| !worlds.contains(w)) {
                return Err(ProjectError::in_file(
                    &pack.file(&format!("worlds/{w}")),
                    format!(
                        "the world `{w}` is not listed in the `worlds` of any pack; add it to \
                         the `worlds` of `{}/pack.json`",
                        pack.location
                    ),
                ));
            }
        }
        let chosen = match world {
            Some(w) if worlds.iter().any(|x| x == w) => w.to_string(),
            Some(w) => {
                return Err(ProjectError::new(format!(
                    "no pack of the project has the world `{w}`; the worlds are: {}",
                    list(&worlds)
                )));
            }
            None => parsed
                .iter()
                .rev()
                .find_map(|p| p.manifest.worlds.first().cloned())
                .ok_or_else(|| {
                    ProjectError::new(format!(
                        "neither the pack `{root}` nor its dependencies list a world in `worlds`; \
                         a game is a pack with worlds"
                    ))
                })?,
        };
        let all: Vec<&Pack> = parsed.iter().collect();
        let resolved = resolve(&all, &chosen)?;
        let world = build_world(&resolved, &chosen)?;
        let (deps, root_pack) = match all.split_last() {
            Some((last, deps)) => (deps, *last),
            None => return Err(ProjectError::new("the project has no packs")),
        };
        let base = base_of(deps, &chosen)?;
        let mut project = Project {
            root: root.to_string(),
            world: chosen.clone(),
            packs: parsed
                .iter()
                .map(|p| PackInfo {
                    manifest: p.manifest.clone(),
                    location: p.location.clone(),
                })
                .collect(),
            worlds,
            default_layer: resolved.default_layer(),
            layer_order: resolved.layer_order(),
            cell_size: resolved.cell_size(),
            partial: cells.cloned(),
            provenance: provenance_of(&resolved, &world),
            base,
            own: Own::default(),
            saved_fields: world_fields(&world)?,
            saved: world.clone(),
            warnings: resolved.warnings.clone(),
            assets: parsed
                .iter()
                .flat_map(|p| {
                    p.assets.iter().map(|a| {
                        (
                            a.path.clone(),
                            ProjectAsset {
                                pack: p.manifest.id.clone(),
                                location: p.location.clone(),
                                asset: a.clone(),
                            },
                        )
                    })
                })
                .collect(),
        };
        project.own = project.own_of(root_pack, &world);
        Ok((project, world))
    }

    /// A new project of one pack `id` (named `name`) holding `world` as its world
    /// `world_name`, every entity in its layer (the default layer `main` when it has none).
    /// Returns the project and the save that writes all of its files, `pack.json` included;
    /// call [`Project::finish_save`] once they are written.
    pub fn create(
        id: &str,
        name: &str,
        world_name: &str,
        world: &World,
    ) -> Result<(Project, Saving), ProjectError> {
        Project::create_cells(id, name, world_name, world, None)
    }

    /// [`Project::create`] with the layers split into cells of `cell_size_m` metres
    /// (`cells.rs`); `None` is one file per layer.
    pub fn create_cells(
        id: &str,
        name: &str,
        world_name: &str,
        world: &World,
        cell_size_m: Option<f64>,
    ) -> Result<(Project, Saving), ProjectError> {
        let manifest = Manifest::new(id, name, vec![world_name.to_string()]);
        manifest.validate("pack.json")?;
        if !is_plain_name(world_name) {
            return Err(ProjectError::new(format!(
                "the world name {world_name:?} is not a plain name: 1 to 64 ASCII letters, \
                 digits, `-` and `_`"
            )));
        }
        world
            .validate()
            .map_err(|e| ProjectError::new(format!("the world is not valid: {e}")))?;
        if let Some(size) = cell_size_m {
            check_cell_size(size, "`cell_size_m`")?;
        }
        let project = Project {
            root: id.to_string(),
            world: world_name.to_string(),
            packs: vec![PackInfo {
                manifest: manifest.clone(),
                location: String::new(),
            }],
            worlds: vec![world_name.to_string()],
            default_layer: DEFAULT_LAYER.to_string(),
            layer_order: Vec::new(),
            provenance: Provenance::default(),
            base: Base::default(),
            own: Own::default(),
            saved: World::new("", 0),
            saved_fields: Map::new(),
            warnings: Vec::new(),
            assets: BTreeMap::new(),
            cell_size: cell_size_m,
            partial: None,
        };
        let mut saving = project.plan_save(world)?;
        saving
            .plan
            .writes
            .insert("pack.json".into(), manifest.to_text()?);
        Ok((project, saving))
    }

    /// The root pack's id.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// The world being edited.
    pub fn world(&self) -> &str {
        &self.world
    }

    /// The worlds the packs list, in load order.
    pub fn worlds(&self) -> &[String] {
        &self.worlds
    }

    /// The assets of every pack by path inside `assets/`: a later pack's asset replaces an
    /// earlier one's of the same path.
    pub fn assets(&self) -> &BTreeMap<String, ProjectAsset> {
        &self.assets
    }

    /// The packs, in load order (the root pack last).
    pub fn packs(&self) -> &[PackInfo] {
        &self.packs
    }

    /// The root pack's manifest.
    pub fn manifest(&self) -> Option<&Manifest> {
        self.packs.last().map(|p| &p.manifest)
    }

    /// The layer entities without a layer are saved to.
    pub fn default_layer(&self) -> &str {
        &self.default_layer
    }

    /// Where the loaded and saved entities, the prefabs and the world fields came from.
    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    /// Problems that did not stop the load (a removal of an id no earlier pack has).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The world as it was opened or last saved.
    pub fn saved_world(&self) -> &World {
        &self.saved
    }

    /// The layer entity `e` is in: its own, or the world's default layer.
    pub fn layer_of<'a>(&'a self, e: &'a Entity) -> &'a str {
        e.layer.as_deref().unwrap_or(&self.default_layer)
    }

    /// The root pack's files of the world as they are, normalised against the loaded world.
    fn own_of(&self, root: &Pack, world: &World) -> Own {
        let mut own = Own {
            fields: None,
            layers: BTreeMap::new(),
            cells: BTreeMap::new(),
            prefab_files: root.prefabs.keys().cloned().collect(),
            edits: None,
        };
        let by_key: BTreeMap<&str, EntityId> = self
            .provenance
            .entities
            .iter()
            .filter(|(_, s)| s.pack == self.root)
            .map(|(id, s)| (s.key.as_str(), *id))
            .collect();
        let Some(dir) = root.worlds.get(&self.world) else {
            return own;
        };
        own.fields.clone_from(&dir.fields);
        own.edits.clone_from(&dir.edits);
        let normalised = |layer: &Layer| Layer {
            replace: layer.replace,
            entries: layer
                .entries
                .iter()
                .map(|entry| match entry {
                    Entry::Entity(raw) => by_key
                        .get(raw.id.as_str())
                        .and_then(|id| world.entity(*id))
                        .map_or_else(
                            || entry.clone(),
                            |e| Entry::Entity(form(&self.provenance, world, e, &raw.id)),
                        ),
                    Entry::Removed(_) => entry.clone(),
                })
                .collect(),
        };
        for (name, layer) in &dir.layers {
            own.layers.insert(name.clone(), normalised(layer));
        }
        for (name, cells) in &dir.cells {
            own.cells.insert(
                name.clone(),
                cells.iter().map(|(c, l)| (*c, normalised(l))).collect(),
            );
        }
        own
    }

    /// Plans the save of `world`: the minimal set of file texts, each change in the file it
    /// belongs to in the root pack; a change to something a dependency brought is written to
    /// the root pack as an override (or a removal), never to the dependency. A world that did
    /// not change writes nothing. Nothing is changed until [`Project::finish_save`].
    pub fn plan_save(&self, world: &World) -> Result<Saving, ProjectError> {
        if let Some(cells) = &self.partial {
            return Err(ProjectError::new(format!(
                "the project was opened with {} of its cells only, so its world lacks the \
                 others; open it whole (project.open without `cells`) to save",
                cells.len()
            )));
        }
        world
            .validate()
            .map_err(|e| ProjectError::new(format!("the world is not valid: {e}")))?;
        let mut plan = SavePlan::default();
        let mut provenance = self.provenance.clone();
        let mut own = self.own.clone();
        let saved_fields = world_fields(world)?;

        // String ids for entities that have none yet. Taken are the ids the dependencies
        // define (a removed one still gets its removal line) and those of the entities that
        // exist, in the provenance and in the root pack's files as they are. The id of an
        // entity of the root pack that is gone is free again: its line leaves the file and
        // nothing refers to it, so a script that replaces a layer gives its entities the same
        // ids as before (a bake of the same script writes the same files).
        let root_keys: BTreeMap<&str, EntityId> = provenance
            .entities
            .iter()
            .filter(|(_, s)| s.pack == self.root)
            .map(|(id, s)| (s.key.as_str(), *id))
            .collect();
        let gone = |key: &str| {
            root_keys
                .get(key)
                .is_some_and(|id| world.entity(*id).is_none())
        };
        let mut used: BTreeSet<String> = self.base.entities.keys().cloned().collect();
        used.extend(
            provenance
                .entities
                .iter()
                .filter(|(id, _)| world.entity(**id).is_some())
                .map(|(_, s)| s.key.clone()),
        );
        for layer in own.layers.values() {
            used.extend(
                layer
                    .entries
                    .iter()
                    .map(|e| e.key())
                    .filter(|key| !gone(key))
                    .map(str::to_string),
            );
        }
        let mut numbered: BTreeMap<String, u64> = BTreeMap::new();
        for e in world.entities() {
            if let std::collections::btree_map::Entry::Vacant(slot) =
                provenance.entities.entry(e.id)
            {
                slot.insert(EntitySource {
                    key: new_key(&e.name, &mut used, &mut numbered),
                    pack: self.root.clone(),
                    layer: self.layer_of(e).to_string(),
                });
            }
        }

        // Which entities the root pack's files must hold, and their lines.
        let mut wanted: BTreeMap<String, (String, LayerEntity)> = BTreeMap::new();
        let mut present: BTreeSet<String> = BTreeSet::new();
        let mut moved: Vec<(EntityId, String)> = Vec::new();
        for e in world.entities() {
            let Some(source) = provenance.entities.get(&e.id) else {
                continue;
            };
            let layer = e
                .layer
                .clone()
                .unwrap_or_else(|| self.default_layer.clone());
            let form = form(&provenance, world, e, &source.key);
            present.insert(source.key.clone());
            let needed = source.pack == self.root
                || self
                    .base
                    .entities
                    .get(&source.key)
                    .is_none_or(|(l, f)| *l != layer || *f != form);
            if needed {
                moved.push((e.id, layer.clone()));
                wanted.insert(source.key.clone(), (layer, form));
            }
        }
        for (id, layer) in moved {
            if let Some(source) = provenance.entities.get_mut(&id) {
                source.pack.clone_from(&self.root);
                source.layer = layer;
            }
        }
        let mut layers: BTreeMap<String, Layer> = BTreeMap::new();
        let mut placed: BTreeSet<String> = BTreeSet::new();
        for (name, layer) in &own.layers {
            let mut entries = Vec::new();
            for entry in &layer.entries {
                let key = entry.key();
                match entry {
                    Entry::Removed(_) if !present.contains(key) => {
                        entries.push(entry.clone());
                        placed.insert(key.to_string());
                    }
                    Entry::Removed(_) => {}
                    Entry::Entity(_) => {
                        if let Some((l, form)) = wanted.get(key)
                            && l == name
                        {
                            entries.push(Entry::Entity(form.clone()));
                            placed.insert(key.to_string());
                        }
                    }
                }
            }
            layers.insert(
                name.clone(),
                Layer {
                    replace: layer.replace,
                    entries,
                },
            );
        }
        for e in world.entities() {
            let Some(source) = provenance.entities.get(&e.id) else {
                continue;
            };
            if placed.contains(&source.key) {
                continue;
            }
            if let Some((layer, form)) = wanted.get(&source.key) {
                layers
                    .entry(layer.clone())
                    .or_default()
                    .entries
                    .push(Entry::Entity(form.clone()));
                placed.insert(source.key.clone());
            }
        }
        for (key, (layer, _)) in &self.base.entities {
            if present.contains(key) || placed.contains(key) {
                continue;
            }
            if own.layers.get(layer).is_some_and(|l| l.replace) {
                continue;
            }
            layers
                .entry(layer.clone())
                .or_default()
                .entries
                .push(Entry::Removed(key.clone()));
        }
        let mut cells_now: BTreeMap<String, BTreeMap<Cell, Layer>> = BTreeMap::new();
        let places = match self.cell_size {
            Some(_) => self.places(&layers),
            None => BTreeMap::new(),
        };
        for (name, layer) in &layers {
            let on_disk = own.cells.get(name);
            let Some(size) = self.cell_size else {
                if own.layers.get(name) != Some(layer) || on_disk.is_some() {
                    plan.writes
                        .insert(layer_path(&self.world, name), layer.to_text()?);
                }
                for cell in on_disk.into_iter().flat_map(BTreeMap::keys) {
                    plan.removes.push(cell_path(&self.world, name, *cell));
                }
                continue;
            };
            let split = split_cells(layer, size, &places);
            for (cell, part) in &split {
                if on_disk.and_then(|d| d.get(cell)) != Some(part) {
                    plan.writes
                        .insert(cell_path(&self.world, name, *cell), part.to_text()?);
                }
            }
            for cell in on_disk.into_iter().flat_map(BTreeMap::keys) {
                if !split.contains_key(cell) {
                    plan.removes.push(cell_path(&self.world, name, *cell));
                }
            }
            if self.own.layers.contains_key(name) && on_disk.is_none() {
                plan.removes.push(layer_path(&self.world, name));
            }
            cells_now.insert(name.clone(), split);
        }
        own.layers = layers;
        own.cells = cells_now;

        // world.json: a merge patch over what the dependencies give.
        let owns_world = !self.base.has_world;
        let cell_key = self.cell_key();
        let cells_changed =
            own.fields.as_ref().and_then(|f| f.get("cell_size_m")) != cell_key.as_ref();
        if saved_fields != self.saved_fields
            || (own.fields.is_none() && owns_world)
            || cells_changed
        {
            let mut patch = diff(&self.base.fields, &saved_fields);
            let mut keep = own.fields.clone().unwrap_or_default();
            keep.retain(|k, _| PROJECT_KEYS.contains(&k.as_str()) && k != "cell_size_m");
            if let Some(v) = cell_key {
                keep.insert("cell_size_m".into(), v);
            }
            if own.fields.is_none() && owns_world {
                keep.insert(
                    "default_layer".into(),
                    Value::String(self.default_layer.clone()),
                );
            }
            patch.extend(keep);
            for key in patch.keys() {
                provenance
                    .world_fields
                    .insert(key.clone(), self.root.clone());
            }
            let mut file = Map::new();
            file.insert("gamecraft_world".into(), json!(WORLD_FORMAT));
            file.extend(patch.clone());
            plan.writes
                .insert(world_path(&self.world), pretty_ordered(&file, &WORLD_KEYS)?);
            own.fields = Some(patch);
        }

        // Terrain edits: the whole list to the root pack's file when it differs from what
        // the files give.
        let edits = world
            .terrain
            .as_ref()
            .map(|t| t.edits.clone())
            .unwrap_or_default();
        let given = own.edits.as_ref().unwrap_or(&self.base.edits);
        if edits != *given {
            plan.writes
                .insert(edits_path(&self.world), edits_text(&edits)?);
            own.edits = Some(edits);
        }

        // Prefabs: a changed or new one is written whole; a removed one is deleted, or
        // removed by a marker when a dependency brings it.
        for prefab in &world.prefabs {
            if self.saved.prefab(&prefab.id) == Some(prefab) {
                continue;
            }
            if !is_prefab_file_id(&prefab.id) {
                return Err(ProjectError::new(format!(
                    "the prefab `{}` cannot be saved: a prefab file is named after its id, and \
                     an id saved to a file is ASCII letters, digits, `-`, `_` and `.`; give it \
                     such an id",
                    prefab.id
                )));
            }
            plan.writes
                .insert(prefab_path(&prefab.id), pretty_typed(prefab)?);
            own.prefab_files.insert(prefab.id.clone());
            provenance
                .prefabs
                .insert(prefab.id.clone(), self.root.clone());
        }
        for prefab in &self.saved.prefabs {
            if world.prefab(&prefab.id).is_some() {
                continue;
            }
            provenance.prefabs.remove(&prefab.id);
            if self.base.prefabs.contains(&prefab.id) {
                plan.writes.insert(
                    prefab_path(&prefab.id),
                    pretty_typed(&json!({"id": prefab.id, "removed": true}))?,
                );
                own.prefab_files.insert(prefab.id.clone());
            } else if own.prefab_files.remove(&prefab.id) {
                plan.removes.push(prefab_path(&prefab.id));
            }
        }

        Ok(Saving {
            plan,
            next: Next {
                provenance,
                own,
                saved: world.clone(),
                saved_fields,
            },
        })
    }

    /// The side of the cells the world's layers are saved in, metres; `None`: one file per
    /// layer.
    pub fn cell_size(&self) -> Option<f64> {
        self.cell_size
    }

    /// The cells the project was loaded with, when only some were ([`Project::load_cells`]).
    pub fn partial(&self) -> Option<&BTreeSet<Cell>> {
        self.partial.as_ref()
    }

    /// Saves the world's layers in cells of `size_m` metres from the next save on (the root
    /// pack's `world.json` gets `cell_size_m`), or in one file per layer with `None`. The old
    /// files are removed by that save.
    pub fn set_cell_size(&mut self, size_m: Option<f64>) -> Result<(), ProjectError> {
        if let Some(size) = size_m {
            check_cell_size(size, "`cell_size_m`")?;
        }
        self.cell_size = size_m;
        Ok(())
    }

    /// What the root pack's `world.json` holds as `cell_size_m`: nothing when the dependencies
    /// give the same, `null` to drop a dependency's.
    fn cell_key(&self) -> Option<Value> {
        let inherited = self.base.fields.get("cell_size_m").and_then(Value::as_f64);
        match self.cell_size {
            Some(v) if inherited == Some(v) => None,
            Some(v) => Some(json!(v)),
            None if inherited.is_some() => Some(Value::Null),
            None => None,
        }
    }

    /// Where each line of the root pack's `layers` stands for its cell: the position of the
    /// entity at the top of its chain of parents, by string id (the dependencies' entities
    /// too, for a removal or a parent they bring).
    fn places(&self, layers: &BTreeMap<String, Layer>) -> BTreeMap<String, [f64; 3]> {
        let mut known: BTreeMap<&str, ([f64; 3], Option<&str>)> = self
            .base
            .entities
            .iter()
            .map(|(k, (_, e))| (k.as_str(), (e.position, e.parent.as_deref())))
            .collect();
        for layer in layers.values() {
            for entry in &layer.entries {
                if let Entry::Entity(e) = entry {
                    known.insert(e.id.as_str(), (e.position, e.parent.as_deref()));
                }
            }
        }
        let root_of = |key: &str| -> Option<[f64; 3]> {
            let mut at = key;
            let mut position = known.get(at)?.0;
            for _ in 0..MAX_ROOT_STEPS {
                let Some((p, parent)) = known.get(at) else {
                    break;
                };
                position = *p;
                match parent {
                    Some(up) if known.contains_key(up) => at = up,
                    _ => break,
                }
            }
            Some(position)
        };
        layers
            .values()
            .flat_map(|l| l.entries.iter())
            .filter_map(|entry| {
                let key = entry.key();
                root_of(key).map(|p| (key.to_string(), p))
            })
            .collect()
    }

    /// Takes the state of a save whose files were written.
    pub fn finish_save(&mut self, saving: Saving) {
        self.provenance = saving.next.provenance;
        self.own = saving.next.own;
        self.saved = saving.next.saved;
        self.saved_fields = saving.next.saved_fields;
    }

    /// What the project holds, as JSON: packs, worlds, the layers of the current world with
    /// their entity counts, prefabs with their packs, and what changed since it was opened or
    /// last saved (the files a save would write).
    pub fn info(&self, world: &World) -> Result<Value, ProjectError> {
        let packs: Vec<Value> = self
            .packs
            .iter()
            .map(|p| {
                json!({
                    "id": p.manifest.id,
                    "name": p.manifest.name,
                    "version": p.manifest.version,
                    "location": p.location,
                    "depends": p.manifest.depends.iter().map(|d| d.id.clone()).collect::<Vec<_>>(),
                    "worlds": p.manifest.worlds,
                    "root": p.manifest.id == self.root,
                })
            })
            .collect();
        let mut counts: BTreeMap<String, u64> = BTreeMap::new();
        for e in world.entities() {
            *counts.entry(self.layer_of(e).to_string()).or_insert(0) += 1;
        }
        for layer in self.own.layers.keys() {
            counts.entry(layer.clone()).or_insert(0);
        }
        for (layer, _) in self.base.entities.values() {
            counts.entry(layer.clone()).or_insert(0);
        }
        let names = ordered(counts.keys(), &self.layer_order);
        let layers: Vec<Value> = names
            .iter()
            .map(|name| {
                json!({
                    "name": name,
                    "entities": counts.get(name).copied().unwrap_or(0),
                    "default": *name == self.default_layer,
                })
            })
            .collect();
        let prefabs: Vec<Value> = world
            .prefabs
            .iter()
            .map(|p| {
                json!({
                    "id": p.id,
                    "pack": self.provenance.prefabs.get(&p.id),
                    "instances": world.prefab_instances(&p.id).len(),
                })
            })
            .collect();
        // A project loaded in part is not saved, so it has no files to write.
        let plan = match &self.partial {
            Some(_) => SavePlan::default(),
            None => self.plan_save(world)?.plan,
        };
        let (mut added, mut changed, mut removed) = (0u64, 0u64, 0u64);
        for e in world.entities() {
            match self.saved.entity(e.id) {
                None => added += 1,
                Some(old) if old != e => changed += 1,
                Some(_) => {}
            }
        }
        for e in self.saved.entities() {
            if world.entity(e.id).is_none() {
                removed += 1;
            }
        }
        let prefabs_changed: Vec<&str> = world
            .prefabs
            .iter()
            .filter(|p| self.saved.prefab(&p.id) != Some(*p))
            .map(|p| p.id.as_str())
            .chain(
                self.saved
                    .prefabs
                    .iter()
                    .filter(|p| world.prefab(&p.id).is_none())
                    .map(|p| p.id.as_str()),
            )
            .collect();
        Ok(json!({
            "root": self.root,
            "world": self.world,
            "worlds": self.worlds,
            "default_layer": self.default_layer,
            "packs": packs,
            "layers": layers,
            "prefabs": prefabs,
            "entities": world.entity_count(),
            "changes": {
                "entities_added": added,
                "entities_changed": changed,
                "entities_removed": removed,
                "prefabs_changed": prefabs_changed,
                "world_fields_changed": world_fields(world)? != self.saved_fields,
                "terrain_edits_changed": world.terrain.as_ref().map(|t| &t.edits)
                    != self.saved.terrain.as_ref().map(|t| &t.edits),
                "files": plan.writes.keys().collect::<Vec<_>>(),
                "removes": plan.removes,
            },
            "cell_size_m": self.cell_size,
            "cells": self.partial.as_ref().map(|c| c.iter().map(|c| [c.x, c.z]).collect::<Vec<_>>()),
            "warnings": self.warnings,
        }))
    }
}

/// What the dependencies alone resolve to for `world`, normalised through a world document.
fn base_of(deps: &[&Pack], world: &str) -> Result<Base, ProjectError> {
    let resolved = resolve(deps, world)?;
    let mut base = Base {
        has_world: resolved.has_world,
        prefabs: resolved.prefabs.keys().cloned().collect(),
        ..Base::default()
    };
    if !resolved.has_world {
        return Ok(base);
    }
    let built = build_world(&resolved, world)?;
    base.fields = world_fields(&built)?;
    base.edits = built
        .terrain
        .as_ref()
        .map(|t| t.edits.clone())
        .unwrap_or_default();
    let keys: BTreeMap<EntityId, &str> = resolved
        .slots
        .iter()
        .enumerate()
        .map(|(i, s)| (EntityId(i as u64 + 1), s.key.as_str()))
        .collect();
    for (e, slot) in built.entities().zip(&resolved.slots) {
        let parent = e
            .parent
            .and_then(|p| keys.get(&p))
            .map(|k| (*k).to_string());
        base.entities.insert(
            slot.key.clone(),
            (
                slot.layer.clone(),
                LayerEntity::of(&built, e, &slot.key, parent),
            ),
        );
    }
    Ok(base)
}

/// `layer` split into cells of `size_m` by `places` (a line without a place goes to cell
/// `0_0`), each cell's lines in the layer's order. A layer with no lines that replaces the
/// one of earlier packs keeps that as an empty cell `0_0`.
fn split_cells(
    layer: &Layer,
    size_m: f64,
    places: &BTreeMap<String, [f64; 3]>,
) -> BTreeMap<Cell, Layer> {
    let mut out: BTreeMap<Cell, Layer> = BTreeMap::new();
    for entry in &layer.entries {
        let cell = places
            .get(entry.key())
            .map_or_else(Cell::default, |p| Cell::of(p[0], p[2], size_m));
        out.entry(cell)
            .or_insert_with(|| Layer {
                replace: layer.replace,
                entries: Vec::new(),
            })
            .entries
            .push(entry.clone());
    }
    if out.is_empty() && layer.replace {
        out.insert(
            Cell::default(),
            Layer {
                replace: true,
                entries: Vec::new(),
            },
        );
    }
    out
}

/// The file form of `e` under `key`, its parent by the parent's string id in `provenance`.
fn form(provenance: &Provenance, world: &World, e: &Entity, key: &str) -> LayerEntity {
    let parent = e.parent.map(|p| {
        provenance
            .entities
            .get(&p)
            .map_or_else(|| format!("entity-{}", p.0), |s| s.key.clone())
    });
    LayerEntity::of(world, e, key, parent)
}

/// Where everything of a resolved and built world came from.
fn provenance_of(r: &Resolved, world: &World) -> Provenance {
    let entities = world
        .entities()
        .zip(&r.slots)
        .map(|(e, s): (&Entity, &Slot)| {
            (
                e.id,
                EntitySource {
                    key: s.key.clone(),
                    pack: s.pack.clone(),
                    layer: s.layer.clone(),
                },
            )
        })
        .collect();
    Provenance {
        entities,
        prefabs: r
            .prefabs
            .iter()
            .map(|(id, (_, pack))| (id.clone(), pack.clone()))
            .collect(),
        world_fields: r.field_packs.clone(),
    }
}

/// A new string id from an entity's name (`Oak 3` -> `oak-3`), not yet in `used`, which it
/// joins. `numbered` remembers per stem the next number to try, so naming many entities alike
/// costs one try each, not one per entity named so before (44 000 walls of one name took
/// hours).
fn new_key(
    name: &str,
    used: &mut BTreeSet<String>,
    numbered: &mut BTreeMap<String, u64>,
) -> String {
    let mut stem = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            stem.push(c);
        } else if !stem.ends_with('-') && !stem.is_empty() {
            stem.push('-');
        }
        if stem.len() >= MAX_GENERATED_KEY_CHARS {
            break;
        }
    }
    let stem = stem.trim_end_matches('-');
    let stem = if stem.is_empty() { "entity" } else { stem };
    let mut key = stem.to_string();
    let next = numbered.entry(stem.to_string()).or_insert(2);
    while used.contains(&key) {
        key = format!("{stem}-{next}");
        *next = next.saturating_add(1);
    }
    used.insert(key.clone());
    key
}

/// `a, b, c`, or `none`.
fn list(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}
