//! The world document and its `.gamecraft` JSON format.

use crate::error::Ctx;
use crate::limits::MAX_ENTITIES;
use crate::{Entity, EntityDraft, EntityId, Environment, Terrain, WorldError};
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
    entities: BTreeMap<EntityId, Entity>,
    next_entity_id: EntityId,
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
    /// Absent: one more than the highest entity id.
    #[serde(default)]
    next_entity_id: Option<EntityId>,
    #[serde(default)]
    entities: Vec<Entity>,
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
            entities: BTreeMap::new(),
            next_entity_id: EntityId(1),
        }
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

    /// Removes an entity and returns it. Its id is not handed out again.
    pub fn remove_entity(&mut self, id: EntityId) -> Option<Entity> {
        self.entities.remove(&id)
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
    /// terrain, the environment, the entity count, `next_entity_id`, and every entity
    /// (in id order) with its components.
    pub fn validate(&self) -> Result<(), WorldError> {
        let ctx = Ctx::WORLD;
        ctx.name("name", &self.name)?;
        if let Some(terrain) = &self.terrain {
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
        }
        Ok(())
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
            next_entity_id: Some(self.next_entity_id),
            entities: self.entities.values().cloned().collect(),
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
            entities,
            next_entity_id,
        };
        world.validate()?;
        Ok(world)
    }
}

/// An entity as a candidate in an error: `id "name"`.
fn describe(e: &Entity) -> String {
    format!("{} \"{}\"", e.id, e.name)
}
