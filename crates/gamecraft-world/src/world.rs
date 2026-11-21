//! The world document and its `.gamecraft` JSON format.

use crate::error::Ctx;
use crate::limits::{MAX_ENTITIES, MAX_HIERARCHY_DEPTH, MAX_PREFABS};
use crate::prefab::{check_components, place_child, relative};
use crate::{
    Component, Entity, EntityDraft, EntityId, Environment, ModelBounds, ModelBox, Objective,
    Prefab, Spawn, Spawned, Terrain, Trigger, VarValue, ViewRules, WorldError,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    /// Which views a player may use and how wide the player's camera sees.
    pub view: ViewRules,
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
    entities: BTreeMap<EntityId, Entity>,
    next_entity_id: EntityId,
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
    /// Written only when not the default.
    #[serde(default, skip_serializing_if = "ViewRules::is_default")]
    view: ViewRules,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    prefabs: Vec<Prefab>,
    /// Absent: one more than the highest entity id.
    #[serde(default)]
    next_entity_id: Option<EntityId>,
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

fn default_scale() -> f64 {
    1.0
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
    scale: f64,
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

impl World {
    /// An empty world: no terrain, the default environment, no entities, next id 1.
    pub fn new(name: &str, seed: u64) -> World {
        World {
            name: name.to_string(),
            seed,
            terrain: None,
            environment: Environment::default(),
            view: ViewRules::default(),
            vars: BTreeMap::new(),
            objectives: Vec::new(),
            triggers: Vec::new(),
            prefabs: Vec::new(),
            entities: BTreeMap::new(),
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
        self.entities.get(&id)
    }

    /// The entity with this id, to edit. Changing its `id` makes the world invalid.
    pub fn entity_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(&id)
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
        let removed = self.entities.remove(&id)?;
        for child in self.descendants(id) {
            self.entities.remove(&child);
        }
        Some(removed)
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
    /// keep their place relative to it). Nothing changes when a moved entity would be invalid
    /// (a coordinate beyond 1e6 m, a non-finite number); the error names it.
    pub fn place_entity(
        &mut self,
        id: EntityId,
        position: [f64; 3],
        rotation_y_deg: f64,
    ) -> Result<(), WorldError> {
        let entity = self.entities.get(&id).ok_or_else(|| WorldError::NotFound {
            reference: id.to_string(),
            candidates: Vec::new(),
        })?;
        let (from, from_yaw) = (entity.position, entity.rotation_y_deg);
        let mut moved = Vec::new();
        let mut root = entity.clone();
        root.position = position;
        root.rotation_y_deg = rotation_y_deg;
        root.validate()?;
        moved.push(root);
        for child in self.descendants(id) {
            let Some(e) = self.entities.get(&child) else {
                continue;
            };
            let (offset, turn) = relative(from, from_yaw, e.position, e.rotation_y_deg);
            let (p, yaw) = place_child(position, rotation_y_deg, offset, turn);
            let mut e = e.clone();
            e.position = p;
            e.rotation_y_deg = yaw;
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
        let Some(e) = self.entities.get(&id) else {
            return Vec::new();
        };
        match e.prefab.as_deref().and_then(|p| self.prefab(p)) {
            Some(prefab) => prefab.overrides_of(&e.components),
            None => Vec::new(),
        }
    }

    /// Adds `prefab`, or replaces the prefab with its id. Replacing one changes its
    /// instances: each keeps its overrides (its components that differed from the old
    /// prefab) and takes the rest from the new one. Children already placed stay as they are.
    /// The prefab is validated first; an invalid one changes nothing.
    pub fn put_prefab(&mut self, prefab: Prefab) -> Result<(), WorldError> {
        prefab.validate()?;
        match self.prefabs.iter().position(|p| p.id == prefab.id) {
            Some(index) => {
                let Some(slot) = self.prefabs.get_mut(index) else {
                    return Err(self.prefab_not_found(&prefab.id));
                };
                let old = std::mem::replace(slot, prefab.clone());
                for e in self.entities.values_mut() {
                    if e.prefab.as_deref() == Some(prefab.id.as_str()) {
                        let overrides = old.overrides_of(&e.components);
                        e.components = prefab.effective(&overrides);
                    }
                }
            }
            None => {
                if self.prefabs.len() >= MAX_PREFABS {
                    return Err(Ctx::WORLD.invalid(
                        "prefabs",
                        format!("a list of {}", self.prefabs.len()),
                        format!("a world holds at most {MAX_PREFABS} prefabs"),
                    ));
                }
                self.prefabs.push(prefab);
            }
        }
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
        let placed = self.prefab_instances(id);
        if !placed.is_empty() {
            return Err(WorldError::PrefabInUse {
                prefab: id.to_string(),
                count: placed.len(),
                entities: placed
                    .iter()
                    .take(MAX_CANDIDATES)
                    .filter_map(|i| self.entities.get(i))
                    .map(describe)
                    .collect(),
            });
        }
        Ok(self.prefabs.remove(index))
    }

    /// Places prefab `prefab`: an entity that refers to it, with its components and the
    /// overrides of `spawn`, and one entity per child of the prefab, placed relative to it
    /// and linked to it as their parent. All or nothing: an invalid placement, override or
    /// child, or a world too full for them all, changes nothing.
    pub fn spawn_prefab(&mut self, prefab: &str, spawn: Spawn) -> Result<Spawned, WorldError> {
        let p = self
            .prefab(prefab)
            .ok_or_else(|| self.prefab_not_found(prefab))?
            .clone();
        check_components(&spawn.overrides, "overrides")?;
        let total = 1usize.saturating_add(p.children.len());
        if self.entities.len().saturating_add(total) > MAX_ENTITIES {
            return Err(WorldError::TooManyEntities {
                count: self.entities.len().saturating_add(total),
                max: MAX_ENTITIES,
            });
        }
        let first = self.next_entity_id.0;
        let next = first
            .checked_add(total as u64)
            .ok_or(WorldError::IdsExhausted)?;
        let root_id = EntityId(first);
        let root = Entity {
            id: root_id,
            name: spawn.name.unwrap_or_else(|| p.name.clone()),
            position: spawn.position,
            rotation_y_deg: spawn.rotation_y_deg,
            scale: 1.0,
            prefab: Some(p.id.clone()),
            parent: None,
            layer: spawn.layer.clone(),
            components: p.effective(&spawn.overrides),
        };
        root.validate()?;
        let mut placed = vec![root];
        let mut children = Vec::new();
        for (i, child) in p.children.iter().enumerate() {
            let id = EntityId(first.saturating_add(1).saturating_add(i as u64));
            let (position, rotation_y_deg) = place_child(
                spawn.position,
                spawn.rotation_y_deg,
                child.offset,
                child.rotation_y_deg,
            );
            let e = Entity {
                id,
                name: child.name.clone(),
                position,
                rotation_y_deg,
                scale: 1.0,
                prefab: None,
                parent: Some(root_id),
                layer: spawn.layer.clone(),
                components: child.components.clone(),
            };
            e.validate()?;
            children.push(id);
            placed.push(e);
        }
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
            && self.entities.contains_key(&EntityId(n))
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
    /// terrain, the environment, the view rules, the entity count, `next_entity_id`, every entity (in id
    /// order) with its components, and the scenario: variables, objectives and triggers.
    pub fn validate(&self) -> Result<(), WorldError> {
        let ctx = Ctx::WORLD;
        ctx.name("name", &self.name)?;
        if let Some(terrain) = &self.terrain {
            terrain.validate()?;
        }
        self.environment.validate()?;
        self.view.validate()?;
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
        for (&key, entity) in &self.entities {
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
        }
        crate::scenario::validate(&self.vars, &self.objectives, &self.triggers)
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
            let Some(p) = self.entities.get(&parent) else {
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
        let file = WorldFile {
            gamecraft: FORMAT_VERSION,
            name: self.name.clone(),
            seed: self.seed,
            terrain: self.terrain.clone(),
            environment: self.environment.clone(),
            view: self.view.clone(),
            prefabs: self.prefabs.clone(),
            next_entity_id: Some(self.next_entity_id),
            entities: self
                .entities
                .values()
                .map(|e| EntityFile::of(e, e.prefab.as_deref().and_then(|p| self.prefab(p))))
                .collect(),
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
        let mut entities = BTreeMap::new();
        for entity in file.entities {
            let entity = entity.into_entity(&file.prefabs)?;
            let id = entity.id;
            if entities.insert(id, entity).is_some() {
                return Err(WorldError::DuplicateEntityId { id });
            }
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
            view: file.view,
            vars: file.vars,
            objectives: file.objectives,
            triggers: file.triggers,
            prefabs: file.prefabs,
            entities,
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
