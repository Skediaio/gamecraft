//! A project: a root pack and its dependencies, layered into one world, with where everything
//! came from, so that a changed world is written back to the files it belongs to.

use crate::ProjectError;
use crate::assets::PackAsset;
use crate::fields::{PROJECT_KEYS, WORLD_FORMAT, WORLD_KEYS, diff, world_fields};
use crate::generators::{GeneratorEntry, difference, generators_path, generators_text, layered};
use crate::layer::{Entry, Layer, LayerEntity};
use crate::manifest::{Manifest, dependency_order};
use crate::pack::{Pack, PackSource, manifest_of, parse_pack};
use crate::paths::{is_plain_name, is_prefab_file_id, layer_path, prefab_path, world_path};
use crate::resolve::{Resolved, Slot, build_world, ordered, resolve};
use crate::terrain::{edits_path, edits_text};
use crate::text::{pretty_ordered, pretty_typed};
use gamecraft_world::{Entity, EntityId, Generator, TerrainEdit, World};
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

/// What the dependencies alone resolve to: what the root pack's files are a difference from.
#[derive(Clone, Debug, Default)]
struct Base {
    has_world: bool,
    fields: Map<String, Value>,
    /// The terrain edits the dependencies give.
    edits: Vec<TerrainEdit>,
    /// The generators the dependencies give.
    generators: Vec<Generator>,
    entities: BTreeMap<String, (String, LayerEntity)>,
    prefabs: BTreeSet<String>,
}

/// The root pack's own files of the world, as they are on disk (normalised).
#[derive(Clone, Debug, Default)]
struct Own {
    /// `world.json` without its version; `None` when the root pack has none.
    fields: Option<Map<String, Value>>,
    layers: BTreeMap<String, Layer>,
    prefab_files: BTreeSet<String>,
    /// `terrain/edits.json`; `None` when the root pack has none.
    edits: Option<Vec<TerrainEdit>>,
    /// `generators.json`; `None` when the root pack has none.
    generators: Option<Vec<GeneratorEntry>>,
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
        let manifests = manifests(packs)?;
        let order = dependency_order(root, &manifests)?;
        let mut parsed = Vec::with_capacity(order.len());
        for id in &order {
            let source = packs
                .get(id)
                .ok_or_else(|| ProjectError::new(format!("the pack `{id}` is missing")))?;
            parsed.push(parse_pack(source)?);
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
            prefab_files: root.prefabs.keys().cloned().collect(),
            edits: None,
            generators: None,
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
        own.generators.clone_from(&dir.generators);
        for (name, layer) in &dir.layers {
            let entries = layer
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
                .collect();
            own.layers.insert(
                name.clone(),
                Layer {
                    replace: layer.replace,
                    entries,
                },
            );
        }
        own
    }

    /// Plans the save of `world`: the minimal set of file texts, each change in the file it
    /// belongs to in the root pack; a change to something a dependency brought is written to
    /// the root pack as an override (or a removal), never to the dependency. A world that did
    /// not change writes nothing. Nothing is changed until [`Project::finish_save`].
    pub fn plan_save(&self, world: &World) -> Result<Saving, ProjectError> {
        world
            .validate()
            .map_err(|e| ProjectError::new(format!("the world is not valid: {e}")))?;
        let mut plan = SavePlan::default();
        let mut provenance = self.provenance.clone();
        let mut own = self.own.clone();
        let saved_fields = world_fields(world)?;

        // String ids for entities that have none yet.
        let mut used: BTreeSet<String> = self.base.entities.keys().cloned().collect();
        used.extend(provenance.entities.values().map(|s| s.key.clone()));
        for layer in own.layers.values() {
            used.extend(layer.entries.iter().map(|e| e.key().to_string()));
        }
        for e in world.entities() {
            if let std::collections::btree_map::Entry::Vacant(slot) =
                provenance.entities.entry(e.id)
            {
                slot.insert(EntitySource {
                    key: new_key(&e.name, &mut used),
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
        for (name, layer) in &layers {
            if own.layers.get(name) != Some(layer) {
                plan.writes
                    .insert(layer_path(&self.world, name), layer.to_text()?);
            }
        }
        own.layers = layers;

        // world.json: a merge patch over what the dependencies give.
        let owns_world = !self.base.has_world;
        if saved_fields != self.saved_fields || (own.fields.is_none() && owns_world) {
            let mut patch = diff(&self.base.fields, &saved_fields);
            let mut keep = own.fields.clone().unwrap_or_default();
            keep.retain(|k, _| PROJECT_KEYS.contains(&k.as_str()));
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

        // Generators: the root pack's file holds what differs from the dependencies; it is
        // written when the world's generators differ from what the files give.
        let given = match &own.generators {
            Some(lines) => {
                let mut ignored = Vec::new();
                let base_lines: Vec<GeneratorEntry> = self
                    .base
                    .generators
                    .iter()
                    .cloned()
                    .map(GeneratorEntry::Entry)
                    .collect();
                layered(
                    [
                        (base_lines.as_slice(), String::new()),
                        (lines.as_slice(), String::new()),
                    ]
                    .into_iter(),
                    &mut ignored,
                )
                .0
            }
            None => self.base.generators.clone(),
        };
        if world.generators != given {
            let lines = difference(&self.base.generators, &world.generators);
            plan.writes
                .insert(generators_path(&self.world), generators_text(&lines)?);
            own.generators = Some(lines);
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
        let saving = self.plan_save(world)?;
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
                "generators_changed": world.generators != self.saved.generators,
                "files": saving.plan.writes.keys().collect::<Vec<_>>(),
                "removes": saving.plan.removes,
            },
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
    base.generators = built.generators.clone();
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
/// joins.
fn new_key(name: &str, used: &mut BTreeSet<String>) -> String {
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
    let mut n: u64 = 2;
    while used.contains(&key) {
        key = format!("{stem}-{n}");
        n = n.saturating_add(1);
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
