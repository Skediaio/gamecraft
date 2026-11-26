//! The world document and its `.gamecraft` JSON format.

use crate::error::Ctx;
use crate::instance::{ChildEdit, Lookup, Pending, Written, mark_written, place_derived};
use crate::limits::{MAX_ENTITIES, MAX_HIERARCHY_DEPTH, MAX_PREFABS};
use crate::prefab::{check_components, place_child, relative};
use crate::store::EntityMap;
use crate::{
    Component, Entity, EntityDraft, EntityId, Environment, Heightmap, ModelBounds, ModelBox,
    Objective, Prefab, Scale, Spawn, Spawned, Terrain, Trigger, VarValue, WorldError,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The version of the `.gamecraft` file format this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// Most candidates listed in a [`WorldError::NotFound`] or [`WorldError::Ambiguous`].
const MAX_CANDIDATES: usize = 10;

/// Longest excerpt of a wrong `gamecraft` value quoted in an error, in characters.
const MAX_VERSION_EXCERPT_CHARS: usize = 40;

/// A world: terrain recipe, environment and entities. The source of truth of a game world.
///
/// Entities live in id order; ids are handed out by [`World::add_entity`] from a counter that
/// only grows, so an id is never reused within a document, not even after
/// [`World::remove_entity`] or a save and load.
#[derive(Clone, Debug, PartialEq)]
pub struct World {
    /// Name of the world, at most 200 characters.
    pub name: String,
    /// The world seed, from which every generator derives its own random stream.
    pub seed: u64,
    /// The terrain recipe; `None` means no ground.
    pub terrain: Option<Terrain>,
    /// Time, place and weather.
    pub environment: Environment,
    /// The scenario's variables, by name: numbers, booleans or strings. Commands set them;
    /// trigger conditions read them.
    pub vars: BTreeMap<String, VarValue>,
    /// The scenario's objectives, in the order given; ids unique.
    pub objectives: Vec<Objective>,
    /// The scenario's triggers, in the order given (the order they run in); ids unique.
    pub triggers: Vec<Trigger>,
    /// The prefabs entities are placed from, in the order given; ids unique. Change them
    /// with [`World::put_prefab`] and [`World::remove_prefab`], which keep the instances in
    /// step.
    pub prefabs: Vec<Prefab>,
    /// Shared between copies of the world: copying a world, or changing one entity of a
    /// copy, costs O(log n), not O(n) (`store.rs`).
    pub(crate) entities: EntityMap,
    pub(crate) next_entity_id: EntityId,
    /// The local bounding boxes of model assets, by asset path: derived from the assets by the
    /// host, never written to the file.
    model_bounds: BTreeMap<String, ModelBounds>,
}

/// The file layout, in the order the fields are written.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldFile {
    gamecraft: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    seed: u64,
    #[serde(default)]
    terrain: Option<Terrain>,
    #[serde(default)]
    environment: Environment,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    prefabs: Vec<Prefab>,
    /// Absent: one more than the highest entity id.
    #[serde(default)]
    next_entity_id: Option<EntityId>,
    /// True when the children of prefab instances are derived from their prefabs and only
    /// their differences are written (`instance.rs`); absent in files of earlier engines,
    /// which write every child in full.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    derived_children: bool,
    #[serde(default)]
    entities: Vec<EntityFile>,
    #[serde(
        default,
        deserialize_with = "crate::scenario::deserialize_vars",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    vars: BTreeMap<String, VarValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    objectives: Vec<Objective>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    triggers: Vec<Trigger>,
}

fn default_scale() -> Scale {
    Scale::ONE
}

/// An entity as the file holds it: an instance of a prefab keeps its prefab and overrides
/// instead of its components.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntityFile {
    id: EntityId,
    #[serde(default)]
    name: String,
    #[serde(default)]
    position: [f64; 3],
    #[serde(default)]
    rotation_y_deg: f64,
    #[serde(default = "default_scale")]
    scale: Scale,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prefab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    components: Option<Vec<Component>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    overrides: Option<Vec<Component>>,
    /// An instance's children by prefab child (`null`: removed), when they are not the ids
    /// it was spawned with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    children: Option<Vec<Option<EntityId>>>,
    /// What differs in an instance's children from what its prefab places.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    child_edits: Option<Vec<ChildEdit>>,
}

impl EntityFile {
    /// The file form of `e`; `prefab` is the prefab it names, if it names one.
    fn of(e: &Entity, prefab: Option<&Prefab>) -> EntityFile {
        let (components, overrides) = match prefab {
            Some(p) => {
                let overrides = p.overrides_of(&e.components);
                (None, (!overrides.is_empty()).then_some(overrides))
            }
            None => (Some(e.components.clone()), None),
        };
        EntityFile {
            id: e.id,
            name: e.name.clone(),
            position: e.position,
            rotation_y_deg: e.rotation_y_deg,
            scale: e.scale,
            prefab: e.prefab.clone(),
            parent: e.parent,
            layer: e.layer.clone(),
            components,
            overrides,
            children: None,
            child_edits: None,
        }
    }

    /// The entity, its components materialised from its prefab in `prefabs`.
    fn into_entity(self, prefabs: &[Prefab]) -> Result<Entity, WorldError> {
        let ctx = Ctx::entity(self.id);
        let components = match &self.prefab {
            Some(id) => {
                if self.components.is_some() {
                    return Err(ctx.invalid(
                        "components",
                        "given",
                        format!(
                            "an entity placed from prefab \"{id}\" takes its prefab's \
                             components; give its own as `overrides`, which replace the \
                             prefab's components of the same type"
                        ),
                    ));
                }
                let prefab = prefabs
                    .iter()
                    .find(|p| p.id == *id)
                    .ok_or_else(|| unknown_prefab(ctx, id, prefabs))?;
                let overrides = self.overrides.unwrap_or_default();
                check_components(&overrides, "overrides").map_err(|e| in_entity(e, self.id))?;
                prefab.effective(&overrides)
            }
            None => {
                if self.overrides.is_some() {
                    return Err(ctx.invalid(
                        "overrides",
                        "given",
                        "overrides belong to an entity placed from a prefab; give `prefab`, \
                         or put the components in `components`",
                    ));
                }
                self.components.unwrap_or_default()
            }
        };
        Ok(Entity {
            id: self.id,
            name: self.name,
            position: self.position,
            rotation_y_deg: self.rotation_y_deg,
            scale: self.scale,
            prefab: self.prefab,
            parent: self.parent,
            layer: self.layer,
            prefab_child: None,
            components,
        })
    }
}

/// The error of an entity that names a prefab that does not exist.
fn unknown_prefab(ctx: Ctx, id: &str, prefabs: &[Prefab]) -> WorldError {
    let known: Vec<&str> = prefabs
        .iter()
        .take(MAX_CANDIDATES)
        .map(|p| p.id.as_str())
        .collect();
    let expected = if known.is_empty() {
        "no prefab has that id; the world has no prefabs".to_string()
    } else {
        format!(
            "no prefab has that id; the prefabs are: {}",
            known.join(", ")
        )
    };
    ctx.invalid("prefab", format!("{id:?}"), expected)
}

/// `e` as an error of entity `id`.
fn in_entity(e: WorldError, id: EntityId) -> WorldError {
    match e {
        WorldError::Invalid {
            field,
            value,
            expected,
            ..
        } => WorldError::Invalid {
            entity: Some(id),
            field,
            value,
            expected,
        },
        other => other,
    }
}

/// Reads only the format version, ignoring everything else, so a document of another
/// version is reported as such rather than as a list of unknown fields.
#[derive(Deserialize)]
struct VersionProbe {
    #[serde(default)]
    gamecraft: Option<serde_json::Value>,
}

/// An entity's placement: its base position, its turn about +Y in degrees and its scale.
type Pose = ([f64; 3], f64, Scale);

/// Where an entity stood before a move of its tree and where it stands after it.
#[derive(Clone, Copy)]
struct Moved {
    from: Pose,
    to: Pose,
}

impl World {
    /// An empty world: no terrain, the default environment, no entities, next id 1.
    pub fn new(name: &str, seed: u64) -> World {
        World {
            name: name.to_string(),
            seed,
            terrain: None,
            environment: Environment::default(),
            vars: BTreeMap::new(),
            objectives: Vec::new(),
            triggers: Vec::new(),
            prefabs: Vec::new(),
            entities: EntityMap::default(),
            next_entity_id: EntityId(1),
            model_bounds: BTreeMap::new(),
        }
    }

    /// The bounding box of the model asset `asset` (a path inside a pack's `assets/` folder),
    /// as the host derived it; `None` when the host gave none.
    pub fn model_bounds(&self, asset: &str) -> Option<&ModelBounds> {
        self.model_bounds.get(asset)
    }

    /// Every asset's bounding box the host gave, by path.
    pub fn all_model_bounds(&self) -> &BTreeMap<String, ModelBounds> {
        &self.model_bounds
    }

    /// Replaces the derived bounding boxes of the model assets (by asset path). Derived data:
    /// it is not written to the file and does not change the document's text. Bounds with a
    /// number that is not finite, or a minimum above the maximum, are left out.
    pub fn set_model_bounds(&mut self, bounds: BTreeMap<String, ModelBounds>) {
        self.model_bounds = bounds.into_iter().filter(|(_, b)| b.is_valid()).collect();
    }

    /// The samples of the terrain's heightmap when its source is one; `None` otherwise.
    pub fn heightmap(&self) -> Option<&Heightmap> {
        self.terrain.as_ref().and_then(Terrain::heightmap)
    }

    /// Hands over the samples of the terrain's heightmap (the host reads them from the
    /// project's `terrain/heightmap/` folder). The document's text does not change (the
    /// source names them by a hash of their content), and only what the document names is
    /// taken ([`Terrain::set_heightmap`]). An error without a terrain.
    pub fn set_heightmap(&mut self, tiles: Heightmap) -> Result<(), WorldError> {
        match self.terrain.as_mut() {
            Some(terrain) => terrain.set_heightmap(tiles),
            None => Err(WorldError::Invalid {
                entity: None,
                field: "terrain".into(),
                value: "none".into(),
                expected: "a terrain whose source is a heightmap, to hand its samples over; \
                          terrain.set with kind \"heightmap\" makes one"
                    .into(),
            }),
        }
    }

    /// The collider box of `entity`'s model in this world: `None` without a model, with the
    /// collider `none`, or when the asset has no bounds.
    pub fn model_box(&self, entity: &Entity) -> Option<ModelBox> {
        let model = entity.model()?;
        ModelBox::of(entity, self.model_bounds.get(&model.asset)?)
    }

    /// The id the next [`World::add_entity`] will hand out.
    pub fn next_entity_id(&self) -> EntityId {
        self.next_entity_id
    }

    /// All entities, in id order.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    /// How many entities the world holds.
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// The entity with this id.
    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(id)
    }

    /// The entity with this id, to edit. Changing its `id` makes the world invalid.
    pub fn entity_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(id)
    }

    /// Adds an entity under the next free id and returns that id.
    ///
    /// The draft is validated first; an invalid draft is an error and uses up no id. Fails
    /// when the world already holds [`MAX_ENTITIES`] entities or the id counter is exhausted.
    pub fn add_entity(&mut self, draft: EntityDraft) -> Result<EntityId, WorldError> {
        if self.entities.len() >= MAX_ENTITIES {
            return Err(WorldError::TooManyEntities {
                count: self.entities.len().saturating_add(1),
                max: MAX_ENTITIES,
            });
        }
        let id = self.next_entity_id;
        let next = id.0.checked_add(1).ok_or(WorldError::IdsExhausted)?;
        let entity = draft.into_entity(id);
        entity.validate()?;
        self.entities.insert(id, entity);
        self.next_entity_id = EntityId(next);
        Ok(id)
    }

    /// Removes an entity, and the entities that belong to it (its children, theirs, ...),
    /// and returns it. Its id is not handed out again.
    pub fn remove_entity(&mut self, id: EntityId) -> Option<Entity> {
        let removed = self.entities.remove(id)?;
        for child in self.descendants(id) {
            self.entities.remove(child);
        }
        Some(removed)
    }

    /// Removes every entity of `ids` and the entities that belong to them (their children,
    /// theirs, ...), and says how many went. An id that does not exist is skipped; the ids
    /// are never handed out again. One pass over the entities per level of the deepest chain,
    /// whatever the number of `ids`: for removing many at once (a layer, a selection), where
    /// [`World::remove_entity`] one by one would walk the world once per entity.
    pub fn remove_entities(&mut self, ids: &[EntityId]) -> usize {
        let mut doomed: BTreeSet<EntityId> = ids
            .iter()
            .copied()
            .filter(|id| self.entities.contains_key(*id))
            .collect();
        for _ in 0..MAX_HIERARCHY_DEPTH {
            let next: Vec<EntityId> = self
                .entities
                .values()
                .filter(|e| e.parent.is_some_and(|p| doomed.contains(&p)))
                .map(|e| e.id)
                .filter(|c| !doomed.contains(c))
                .collect();
            if next.is_empty() {
                break;
            }
            doomed.extend(next);
        }
        let count = doomed.len();
        for id in doomed {
            self.entities.remove(id);
        }
        count
    }

    /// The entities whose parent is `id`, in id order.
    pub fn children(&self, id: EntityId) -> Vec<EntityId> {
        self.entities
            .values()
            .filter(|e| e.parent == Some(id))
            .map(|e| e.id)
            .collect()
    }

    /// Everything below `id`: its children, their children, ..., at most
    /// [`MAX_HIERARCHY_DEPTH`] levels deep, level by level in id order.
    pub fn descendants(&self, id: EntityId) -> Vec<EntityId> {
        let mut out = Vec::new();
        let mut level = vec![id];
        for _ in 0..MAX_HIERARCHY_DEPTH {
            let next: Vec<EntityId> = self
                .entities
                .values()
                .filter(|e| e.parent.is_some_and(|p| level.contains(&p)))
                .map(|e| e.id)
                .filter(|c| *c != id && !out.contains(c))
                .collect();
            if next.is_empty() {
                break;
            }
            out.extend(next.iter().copied());
            level = next;
        }
        out
    }

    /// Moves and turns entity `id`, and carries everything below it along rigidly (children
    /// keep their place relative to it, at their scale). A prefab's child that stands where
    /// its prefab puts it is placed where the prefab puts it again, to the bit, so it stays
    /// derived data (`instance.rs`). Nothing changes when a moved entity would be invalid (a
    /// coordinate beyond 1e6 m, a non-finite number); the error names it.
    pub fn place_entity(
        &mut self,
        id: EntityId,
        position: [f64; 3],
        rotation_y_deg: f64,
    ) -> Result<(), WorldError> {
        let below = self.descendants(id);
        let scale = self.placement_of(id)?.2;
        self.place_tree(id, position, rotation_y_deg, scale, &below)
    }

    /// Sets the scale of entity `id` (0.001 to 1000 on every axis), where it stands:
    /// everything below it scales with it. A prefab's child that is as its prefab places it
    /// stands at its offset times the new scale, in the instance's frame, to the bit; any other
    /// entity below keeps its place relative to `id` in world metres, grown by the ratio of the
    /// scales axis by axis. A child at the scale the entity had (a prefab's, unedited) takes the
    /// new scale; one with a scale of its own keeps it, multiplied by the ratio in its own
    /// frame. Nothing changes when the result would be invalid; the error names it.
    pub fn set_scale(&mut self, id: EntityId, scale: Scale) -> Result<(), WorldError> {
        let below = self.descendants(id);
        let (position, rotation_y_deg, _) = self.placement_of(id)?;
        self.place_tree(id, position, rotation_y_deg, scale, &below)
    }

    /// Where entity `id` stands, how it is turned and how large it is ([`WorldError::NotFound`]
    /// without it).
    fn placement_of(&self, id: EntityId) -> Result<Pose, WorldError> {
        let entity = self.entities.get(id).ok_or_else(|| WorldError::NotFound {
            reference: id.to_string(),
            candidates: Vec::new(),
        })?;
        Ok((entity.position, entity.rotation_y_deg, entity.scale))
    }

    /// [`World::place_entity`] and [`World::set_scale`] in one pass: entity `id` at
    /// `position`, turned by `rotation_y_deg` and of `scale`, with the entities below `id`
    /// given, level by level.
    pub(crate) fn place_tree(
        &mut self,
        id: EntityId,
        position: [f64; 3],
        rotation_y_deg: f64,
        scale: Scale,
        below: &[EntityId],
    ) -> Result<(), WorldError> {
        let entity = self.entities.get(id).ok_or_else(|| WorldError::NotFound {
            reference: id.to_string(),
            candidates: Vec::new(),
        })?;
        let lookup = Lookup::new(&self.prefabs)?;
        let from = (entity.position, entity.rotation_y_deg, entity.scale);
        let (from_pos, from_yaw, from_scale) = from;
        // How much everything below grows, axis by axis: 1 (exact) when the scale stays.
        let ratio = scale.over(from_scale);
        let mut moved = Vec::new();
        // Old and new placement of every moved entity, for the children below it.
        let mut placements: BTreeMap<EntityId, Moved> = BTreeMap::new();
        let mut root = entity.clone();
        root.position = position;
        root.rotation_y_deg = rotation_y_deg;
        root.scale = scale;
        root.validate()?;
        placements.insert(
            id,
            Moved {
                from,
                to: (position, rotation_y_deg, scale),
            },
        );
        moved.push(root);
        for &child in below {
            let Some(e) = self.entities.get(child) else {
                continue;
            };
            let exact = e.prefab_child.zip(e.parent).and_then(|(slot, parent)| {
                let moved = placements.get(&parent).copied()?;
                let (p0, y0, s0) = moved.from;
                let (p1, y1, s1) = moved.to;
                let def = lookup
                    .of(self.entities.get(parent)?)?
                    .children
                    .get(slot as usize)?;
                // What the prefab gives this child, at the old and at the new placement.
                let (was, was_yaw) = place_child(p0, y0, s0, def.offset, def.rotation_y_deg);
                let untouched = was
                    .iter()
                    .zip(&e.position)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
                    && was_yaw.to_bits() == e.rotation_y_deg.to_bits()
                    && e.scale == s0.in_turned_frame(def.rotation_y_deg);
                untouched.then(|| {
                    (
                        place_child(p1, y1, s1, def.offset, def.rotation_y_deg),
                        s1.in_turned_frame(def.rotation_y_deg),
                    )
                })
            });
            let ((p, yaw), child_scale) = exact.unwrap_or_else(|| {
                // Relative to `id` as it stood, in metres of its frame: the offset grows by the
                // ratio as it is. The entity's own scale is in its own frame, turned by `turn`,
                // so there the ratio is turned too.
                let (offset, turn) = relative(from_pos, from_yaw, e.position, e.rotation_y_deg);
                let own = if e.scale == from_scale {
                    scale
                } else {
                    e.scale.by(ratio.in_turned_frame(turn))
                };
                (
                    place_child(position, rotation_y_deg, ratio, offset, turn),
                    own,
                )
            });
            placements.insert(
                child,
                Moved {
                    from: (e.position, e.rotation_y_deg, e.scale),
                    to: (p, yaw, child_scale),
                },
            );
            let mut e = e.clone();
            e.position = p;
            e.rotation_y_deg = yaw;
            e.scale = child_scale;
            e.validate()?;
            moved.push(e);
        }
        for e in moved {
            self.entities.insert(e.id, e);
        }
        Ok(())
    }

    /// The prefab with this id.
    pub fn prefab(&self, id: &str) -> Option<&Prefab> {
        self.prefabs.iter().find(|p| p.id == id)
    }

    fn prefab_not_found(&self, id: &str) -> WorldError {
        WorldError::PrefabNotFound {
            prefab: id.to_string(),
            known: self
                .prefabs
                .iter()
                .take(MAX_CANDIDATES)
                .map(|p| p.id.clone())
                .collect(),
        }
    }

    /// The entities placed from prefab `id`, in id order.
    pub fn prefab_instances(&self, id: &str) -> Vec<EntityId> {
        self.entities
            .values()
            .filter(|e| e.prefab.as_deref() == Some(id))
            .map(|e| e.id)
            .collect()
    }

    /// The overrides of entity `id`: its components that differ from its prefab's (none for
    /// an entity without a prefab, or one that does not exist).
    pub fn overrides_of(&self, id: EntityId) -> Vec<Component> {
        let Some(e) = self.entities.get(id) else {
            return Vec::new();
        };
        match e.prefab.as_deref().and_then(|p| self.prefab(p)) {
            Some(prefab) => prefab.overrides_of(&e.components),
            None => Vec::new(),
        }
    }

    /// Adds `prefab`, or replaces the prefab with its id. Replacing one changes its
    /// instances, wherever they are (prefabs inside prefabs too): each keeps its overrides
    /// (its components that differed from the old prefab) and takes the rest from the new
    /// one, and its children follow the same rule, child by child in the prefab's order
    /// (`instance.rs`, [`World::follow_prefab`]). The prefab is validated first, and the
    /// prefabs as a whole (every prefab a child names exists, no loops, nesting and instance
    /// size within their limits); an invalid one, or a world too full for the children it
    /// adds, changes nothing.
    pub fn put_prefab(&mut self, prefab: Prefab) -> Result<(), WorldError> {
        prefab.validate()?;
        let mut prefabs = self.prefabs.clone();
        let old = match prefabs.iter().position(|p| p.id == prefab.id) {
            Some(index) => {
                let Some(slot) = prefabs.get_mut(index) else {
                    return Err(self.prefab_not_found(&prefab.id));
                };
                Some(std::mem::replace(slot, prefab.clone()))
            }
            None => {
                if prefabs.len() >= MAX_PREFABS {
                    return Err(Ctx::WORLD.invalid(
                        "prefabs",
                        format!("a list of {}", prefabs.len()),
                        format!("a world holds at most {MAX_PREFABS} prefabs"),
                    ));
                }
                prefabs.push(prefab.clone());
                None
            }
        };
        Lookup::new(&prefabs)?;
        let mut next = self.clone();
        let before = std::mem::replace(&mut next.prefabs, prefabs);
        if let Some(old) = old {
            next.follow_prefab(&before, &old, &prefab)?;
        }
        *self = next;
        Ok(())
    }

    /// Removes prefab `id` and returns it; refused while entities are placed from it (the
    /// error names them).
    pub fn remove_prefab(&mut self, id: &str) -> Result<Prefab, WorldError> {
        let index = self
            .prefabs
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| self.prefab_not_found(id))?;
        if let Some((user, i)) = self.prefabs.iter().find_map(|p| {
            p.children
                .iter()
                .position(|c| c.prefab.as_deref() == Some(id))
                .map(|i| (p, i))
        }) {
            return Err(WorldError::Prefab {
                prefab: id.to_string(),
                message: format!(
                    "prefab \"{}\" places it as its child {i}; remove that child (prefab.set) \
                     or that prefab first",
                    user.id
                ),
            });
        }
        let placed = self.prefab_instances(id);
        if !placed.is_empty() {
            return Err(WorldError::PrefabInUse {
                prefab: id.to_string(),
                count: placed.len(),
                entities: placed
                    .iter()
                    .take(MAX_CANDIDATES)
                    .filter_map(|i| self.entities.get(*i))
                    .map(describe)
                    .collect(),
            });
        }
        Ok(self.prefabs.remove(index))
    }

    /// Places prefab `prefab`: an entity that refers to it, with its components and the
    /// overrides of `spawn`, and one entity per child of the prefab, placed relative to it
    /// and linked to it as their parent (a child that places a prefab places that prefab's
    /// children in turn). The ids are consecutive: the instance's, then its children's depth
    /// first in the prefab's order, which is what lets the file leave them out. All or
    /// nothing: an invalid placement, override or child, or a world too full for them all,
    /// changes nothing.
    pub fn spawn_prefab(&mut self, prefab: &str, spawn: Spawn) -> Result<Spawned, WorldError> {
        let lookup = Lookup::new(&self.prefabs)?;
        let p = lookup
            .get(prefab)
            .ok_or_else(|| self.prefab_not_found(prefab))?;
        check_components(&spawn.overrides, "overrides")?;
        let total = lookup.size(&p.id);
        if self.entities.len().saturating_add(total) > MAX_ENTITIES {
            return Err(WorldError::TooManyEntities {
                count: self.entities.len().saturating_add(total),
                max: MAX_ENTITIES,
            });
        }
        let first = self.next_entity_id.0;
        first
            .checked_add(total as u64)
            .ok_or(WorldError::IdsExhausted)?;
        let root_id = EntityId(first);
        let root = Entity {
            id: root_id,
            name: spawn.name.unwrap_or_else(|| p.name.clone()),
            position: spawn.position,
            rotation_y_deg: spawn.rotation_y_deg,
            scale: spawn.scale,
            prefab: Some(p.id.clone()),
            parent: None,
            layer: spawn.layer.clone(),
            prefab_child: None,
            components: p.effective(&spawn.overrides),
        };
        root.validate()?;
        let mut next = first.saturating_add(1);
        let mut placed = vec![root.clone()];
        crate::instance::spawn_children(&lookup, &root, &mut next, &mut placed, 1)?;
        let children = placed
            .iter()
            .filter(|e| e.parent == Some(root_id))
            .map(|e| e.id)
            .collect();
        for e in placed {
            self.entities.insert(e.id, e);
        }
        self.next_entity_id = EntityId(next);
        Ok(Spawned {
            id: root_id,
            children,
        })
    }

    /// Resolves a reference to an entity: an id (`"12"`) or a name, matched without regard to
    /// case and surrounding spaces. An id wins over a name.
    ///
    /// No match is [`WorldError::NotFound`], listing entities whose names contain the
    /// reference (or the first entities); several name matches are [`WorldError::Ambiguous`],
    /// listing them. At most 10 candidates are listed, as `id "name"`.
    pub fn find(&self, reference: &str) -> Result<EntityId, WorldError> {
        let wanted = reference.trim();
        if let Ok(n) = wanted.parse::<u64>()
            && self.entities.contains_key(EntityId(n))
        {
            return Ok(EntityId(n));
        }
        let lower = wanted.to_lowercase();
        let matches: Vec<&Entity> = self
            .entities
            .values()
            .filter(|e| e.name.trim().to_lowercase() == lower)
            .collect();
        match matches.as_slice() {
            [one] => Ok(one.id),
            [] => {
                let close: Vec<String> = self
                    .entities
                    .values()
                    .filter(|e| !lower.is_empty() && e.name.to_lowercase().contains(&lower))
                    .take(MAX_CANDIDATES)
                    .map(describe)
                    .collect();
                let candidates = if close.is_empty() {
                    self.entities
                        .values()
                        .take(MAX_CANDIDATES)
                        .map(describe)
                        .collect()
                } else {
                    close
                };
                Err(WorldError::NotFound {
                    reference: reference.to_string(),
                    candidates,
                })
            }
            many => Err(WorldError::Ambiguous {
                reference: reference.to_string(),
                candidates: many
                    .iter()
                    .take(MAX_CANDIDATES)
                    .map(|e| describe(e))
                    .collect(),
            }),
        }
    }

    /// Checks every invariant of the document and reports the first violation, naming the
    /// entity (if any), the field, the value and what is allowed: the world name, the
    /// terrain, the environment, the entity count, `next_entity_id`, every entity (in id
    /// order) with its components, and the scenario: variables, objectives and triggers.
    pub fn validate(&self) -> Result<(), WorldError> {
        self.check(None).map(drop)
    }

    /// [`World::validate`] of a world changed from `before`, which must be valid: the same
    /// result and the same first error, found with less work. The entities are checked only
    /// where they are not the very ones of `before` (in id order), and the terrain only when
    /// it differs. Every entity is checked as by `validate` when the change could make an
    /// entity it did not touch invalid: a prefab changed, an entity was removed or got
    /// another parent, `next_entity_id` went down. Returns how many entities it checked, a
    /// measure of the work (AGENTS.md rule 18).
    ///
    /// When `before` is not valid, an error of `validate` may go unreported; the session
    /// calls this only with a world that passed.
    pub fn validate_change(&self, before: &World) -> Result<usize, WorldError> {
        self.check(Some(before))
    }

    /// The entities of this world that are not the very ones of `before`: new or changed, in
    /// id order, when they can be listed; `None` when every entity must be looked at (one of
    /// `before` is gone, the prefabs differ, an entity got another parent). For what is
    /// derived from the entities and kept up to date in place, as a simulation's spatial
    /// index is.
    pub fn changed_entities_since(&self, before: &World) -> Option<Vec<EntityId>> {
        self.changed_entities(before)
    }

    /// The keys of the entities that differ from `before`'s, when they are all that needs
    /// checking (see [`World::validate_change`]); `None` when every entity must be.
    fn changed_entities(&self, before: &World) -> Option<Vec<EntityId>> {
        if self.prefabs != before.prefabs || self.next_entity_id < before.next_entity_id {
            return None;
        }
        let changes = self.entities.changes(&before.entities)?;
        if changes.removed {
            return None;
        }
        let reparented = changes.changed.iter().any(|&key| {
            match (self.entities.get(key), before.entities.get(key)) {
                (Some(now), Some(then)) => now.parent != then.parent,
                _ => false,
            }
        });
        (!reparented).then_some(changes.changed)
    }

    /// Checks the document (see [`World::validate`]), or the change from a valid `before`
    /// (see [`World::validate_change`]); the number of entities checked.
    fn check(&self, before: Option<&World>) -> Result<usize, WorldError> {
        let ctx = Ctx::WORLD;
        ctx.name("name", &self.name)?;
        if let Some(terrain) = &self.terrain
            && before.is_none_or(|b| b.terrain.as_ref() != Some(terrain))
        {
            terrain.validate()?;
        }
        self.environment.validate()?;
        if self.entities.len() > MAX_ENTITIES {
            return Err(WorldError::TooManyEntities {
                count: self.entities.len(),
                max: MAX_ENTITIES,
            });
        }
        if self.next_entity_id.0 == 0 {
            return Err(ctx.invalid(
                "next_entity_id",
                0,
                "it must be at least 1, and above every entity id",
            ));
        }
        if self.prefabs.len() > MAX_PREFABS {
            return Err(ctx.invalid(
                "prefabs",
                format!("a list of {}", self.prefabs.len()),
                format!("a world holds at most {MAX_PREFABS} prefabs"),
            ));
        }
        let mut prefabs: BTreeMap<&str, &Prefab> = BTreeMap::new();
        for prefab in &self.prefabs {
            prefab.validate()?;
            if prefabs.insert(prefab.id.as_str(), prefab).is_some() {
                return Err(WorldError::Prefab {
                    prefab: prefab.id.clone(),
                    message: "the id is used by more than one prefab; every prefab needs its \
                              own id"
                        .to_string(),
                });
            }
        }
        Lookup::new(&self.prefabs)?;
        let checked = match before.and_then(|b| self.changed_entities(b)) {
            Some(keys) => {
                for &key in &keys {
                    if let Some(entity) = self.entities.get(key) {
                        self.check_entity(key, entity, &prefabs)?;
                    }
                }
                keys.len()
            }
            None => {
                for (key, entity) in self.entities.iter() {
                    self.check_entity(key, entity, &prefabs)?;
                }
                self.entities.len()
            }
        };
        crate::scenario::validate(&self.vars, &self.objectives, &self.triggers)?;
        Ok(checked)
    }

    /// The checks of one entity stored under `key`: its id, its own fields and components,
    /// its prefab and its parents.
    fn check_entity(
        &self,
        key: EntityId,
        entity: &Entity,
        prefabs: &BTreeMap<&str, &Prefab>,
    ) -> Result<(), WorldError> {
        let ectx = Ctx::entity(key);
        if entity.id != key {
            return Err(ectx.invalid(
                "id",
                entity.id,
                format!("it must stay {key}, the id the entity is stored under"),
            ));
        }
        if key.0 == 0 {
            return Err(ectx.invalid("id", 0, "entity ids start at 1"));
        }
        if key >= self.next_entity_id {
            return Err(WorldError::EntityIdNotBelowNext {
                id: key,
                next: self.next_entity_id,
            });
        }
        entity.validate()?;
        if let Some(id) = &entity.prefab {
            let prefab = prefabs
                .get(id.as_str())
                .ok_or_else(|| unknown_prefab(ectx, id, &self.prefabs))?;
            if let Some(missing) = prefab
                .components
                .iter()
                .find(|c| entity.components.iter().all(|e| e.kind() != c.kind()))
            {
                return Err(ectx.invalid(
                    "components",
                    format!("without `{}`", missing.kind()),
                    format!(
                        "an entity placed from prefab \"{id}\" keeps every component type \
                         of its prefab; override the `{}` instead of removing it, or \
                         remove it from the prefab",
                        missing.kind()
                    ),
                ));
            }
        }
        self.check_parent(entity)?;
        Ok(())
    }

    /// An entity's parent exists, is not the entity itself, and the chain above it ends
    /// within [`MAX_HIERARCHY_DEPTH`] steps (no loop).
    fn check_parent(&self, entity: &Entity) -> Result<(), WorldError> {
        let ctx = Ctx::entity(entity.id);
        let mut above = entity.parent;
        for _ in 0..=MAX_HIERARCHY_DEPTH {
            let Some(parent) = above else {
                return Ok(());
            };
            if parent == entity.id {
                return Err(ctx.invalid(
                    "parent",
                    parent,
                    "an entity cannot be its own parent, nor below itself",
                ));
            }
            let Some(p) = self.entities.get(parent) else {
                return Err(ctx.invalid(
                    "parent",
                    parent,
                    "no entity has that id; a parent must exist (or leave `parent` out)",
                ));
            };
            above = p.parent;
        }
        Err(ctx.invalid(
            "parent",
            entity.parent.map_or(0, |p| p.0),
            format!("the chain of parents above it is deeper than {MAX_HIERARCHY_DEPTH} or loops"),
        ))
    }

    /// The objective with this id.
    pub fn objective(&self, id: &str) -> Option<&Objective> {
        self.objectives.iter().find(|o| o.id == id)
    }

    /// The trigger with this id.
    pub fn trigger(&self, id: &str) -> Option<&Trigger> {
        self.triggers.iter().find(|t| t.id == id)
    }

    /// The document as pretty-printed JSON in the `.gamecraft` format: fields in a fixed
    /// order, entities sorted by id, numbers written so they read back bit for bit.
    ///
    /// The world is validated first, so only a valid document is ever written.
    pub fn to_json(&self) -> Result<String, WorldError> {
        self.validate()?;
        let lookup = Lookup::new(&self.prefabs)?;
        let written = Written::plan(&lookup, &self.entities);
        let mut entities =
            Vec::with_capacity(self.entities.len().saturating_sub(written.absorbed.len()));
        for e in self.entities.values() {
            if written.absorbed.contains(&e.id) {
                continue;
            }
            let mut file = EntityFile::of(e, lookup.of(e));
            let (children, child_edits) = written.of_instance(&lookup, &self.entities, e, 1);
            file.children = children;
            file.child_edits = child_edits;
            entities.push(file);
        }
        let file = WorldFile {
            gamecraft: FORMAT_VERSION,
            name: self.name.clone(),
            seed: self.seed,
            terrain: self.terrain.clone(),
            environment: self.environment.clone(),
            prefabs: self.prefabs.clone(),
            next_entity_id: Some(self.next_entity_id),
            derived_children: written.derived,
            entities,
            vars: self.vars.clone(),
            objectives: self.objectives.clone(),
            triggers: self.triggers.clone(),
        };
        serde_json::to_string_pretty(&file).map_err(|e| WorldError::Serialize {
            message: e.to_string(),
        })
    }

    /// Reads a `.gamecraft` document: parses it, checks the format version, rejects unknown
    /// fields, duplicate entity ids and ids at or above `next_entity_id`, and validates the
    /// result. Every problem is an error that says which; no input makes it panic (nesting
    /// deeper than serde_json's recursion limit is an error too).
    ///
    /// Optional fields take their defaults; without `next_entity_id` the next id is one more
    /// than the highest entity id.
    pub fn from_json(text: &str) -> Result<World, WorldError> {
        let probe: VersionProbe = serde_json::from_str(text)?;
        match probe.gamecraft {
            Some(serde_json::Value::Number(n)) if n.as_u64() == Some(u64::from(FORMAT_VERSION)) => {
            }
            Some(other) => {
                return Err(WorldError::Version {
                    found: other
                        .to_string()
                        .chars()
                        .take(MAX_VERSION_EXCERPT_CHARS)
                        .collect(),
                });
            }
            None => {
                return Err(WorldError::Version {
                    found: "missing".to_string(),
                });
            }
        }
        let file: WorldFile = serde_json::from_str(text)?;
        if file.entities.len() > MAX_ENTITIES {
            return Err(WorldError::TooManyEntities {
                count: file.entities.len(),
                max: MAX_ENTITIES,
            });
        }
        let lookup = Lookup::new(&file.prefabs)?;
        let mut entities = BTreeMap::new();
        let mut pending = Vec::new();
        for mut entity in file.entities {
            let (children, edits) = (entity.children.take(), entity.child_edits.take());
            if !file.derived_children && (children.is_some() || edits.is_some()) {
                return Err(Ctx::entity(entity.id).invalid(
                    if children.is_some() {
                        "children"
                    } else {
                        "child_edits"
                    },
                    "given",
                    "an instance's `children` and `child_edits` are read only from a file \
                     with `\"derived_children\": true`; add it, or write the children in full",
                ));
            }
            let entity = entity.into_entity(&file.prefabs)?;
            let id = entity.id;
            if entity.prefab.is_some() {
                pending.push(Pending {
                    id,
                    children,
                    edits: edits.unwrap_or_default(),
                });
            }
            if entities.insert(id, entity).is_some() {
                return Err(WorldError::DuplicateEntityId { id });
            }
        }
        if file.derived_children {
            place_derived(&lookup, &mut entities, pending)?;
        } else {
            mark_written(&lookup, &mut entities);
        }
        let next_entity_id = match file.next_entity_id {
            Some(next) => next,
            None => {
                let highest = entities.keys().next_back().map_or(0, |id| id.0);
                EntityId(highest.checked_add(1).ok_or(WorldError::IdsExhausted)?)
            }
        };
        let world = World {
            name: file.name,
            seed: file.seed,
            terrain: file.terrain,
            environment: file.environment,
            vars: file.vars,
            objectives: file.objectives,
            triggers: file.triggers,
            prefabs: file.prefabs,
            entities: entities.into_iter().collect(),
            next_entity_id,
            model_bounds: BTreeMap::new(),
        };
        world.validate()?;
        Ok(world)
    }
}

/// An entity as a candidate in an error: `id "name"`.
fn describe(e: &Entity) -> String {
    format!("{} \"{}\"", e.id, e.name)
}
