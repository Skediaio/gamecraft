//! The cooked form of a world: derived data, a compact binary file that loads faster than the
//! text it was made from (`docs/decisions.md`, *A cooked binary cache*).
//!
//! The `.gamecraft` text stays the source. A cooked world is made from a world read from that
//! text and carries a hash of the text's bytes ([`source_hash`]); reading it back with another
//! hash, another format version or any damage is an error, and the host then reads the text
//! (it never fails because of a cache). The world comes back exactly: every entity with its
//! materialised components, the children of prefab instances included (nothing is derived
//! again), the same `next_entity_id`, the same text when written again.
//!
//! A child of a prefab instance that is exactly what its prefab places is stored as its id,
//! its parent and its index in the prefab, and placed again on reading, as the text does.
//!
//! The encoding is postcard. Components are internally tagged in serde, which a format that
//! does not describe itself cannot read, so each distinct component is kept once as its JSON
//! text and entities refer to it by index: a city's thousands of identical walls cost one
//! component. Names, prefab ids and layers are kept once in a string table. The world's other
//! fields (terrain recipe, environment, prefabs, scenario) are its JSON text without entities.

use crate::instance::{Lookup, derive, diff};
use crate::limits::MAX_ENTITIES;
use crate::store::EntityMap;
use crate::{Component, Entity, EntityId, World, WorldError};
use gamecraft_math::StateHasher;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The version of the cooked format; a cooked world of another version is not read. 2: the
/// `road` component.
pub const COOKED_FORMAT: u32 = 2;

/// The first bytes of a cooked world.
const MAGIC: [u8; 8] = *b"GCCOOKED";

/// The hash of a world's source bytes that a cooked world is checked against: FNV-1a over a
/// tag, the length and the bytes.
pub fn source_hash(source: &[u8]) -> u64 {
    let mut h = StateHasher::new();
    h.write_bytes(b"gamecraft-source");
    h.write_u64(source.len() as u64);
    h.write_bytes(source);
    h.finish()
}

#[derive(Serialize, Deserialize)]
struct Cooked {
    magic: [u8; 8],
    format: u32,
    world_format: u32,
    source: u64,
    /// The world without entities, as `.gamecraft` JSON.
    head: String,
    next_entity_id: u64,
    strings: Vec<String>,
    /// Each distinct component once, as JSON.
    components: Vec<String>,
    entities: Vec<CookedEntity>,
}

#[derive(Serialize, Deserialize)]
enum CookedEntity {
    /// An entity in full.
    Own(OwnEntity),
    /// A child exactly as its parent's prefab places it.
    Derived { id: u64, parent: u64, slot: u32 },
}

#[derive(Serialize, Deserialize)]
struct OwnEntity {
    id: u64,
    name: u32,
    position: [f64; 3],
    rotation_y_deg: f64,
    scale: f64,
    prefab: Option<u32>,
    parent: Option<u64>,
    layer: Option<u32>,
    prefab_child: Option<u32>,
    components: Vec<u32>,
}

/// Values kept once, by index.
struct Table {
    items: Vec<String>,
    index: BTreeMap<String, u32>,
}

impl Table {
    fn new() -> Table {
        Table {
            items: Vec::new(),
            index: BTreeMap::new(),
        }
    }

    fn of(&mut self, item: &str) -> Result<u32, WorldError> {
        if let Some(i) = self.index.get(item) {
            return Ok(*i);
        }
        let i = u32::try_from(self.items.len()).map_err(|_| cooked("too many distinct values"))?;
        self.items.push(item.to_string());
        self.index.insert(item.to_string(), i);
        Ok(i)
    }
}

fn cooked(message: impl Into<String>) -> WorldError {
    WorldError::Cooked {
        message: message.into(),
    }
}

impl World {
    /// Whether `e` is a child exactly as its parent's prefab places it (to the bit, components
    /// in the same order): its parent and its index in the prefab.
    fn as_derived(&self, lookup: &Lookup<'_>, e: &Entity) -> Option<(EntityId, u32)> {
        let (slot, parent_id) = (e.prefab_child?, e.parent?);
        let parent = self.entity(parent_id)?;
        let def = lookup.of(parent)?.children.get(slot as usize)?;
        let derived = derive(lookup, parent, slot as usize, def, e.id);
        (diff(slot, &derived, e).is_empty()
            && derived.components == e.components
            && derived.prefab == e.prefab)
            .then_some((parent_id, slot))
    }

    /// The cooked form of this world, made from source bytes whose [`source_hash`] is
    /// `source`. The world is validated first.
    pub fn to_cooked(&self, source: u64) -> Result<Vec<u8>, WorldError> {
        let mut head = self.clone();
        head.entities = EntityMap::default();
        let head_text = head.to_json()?;
        self.validate()?;
        let lookup = Lookup::new(&self.prefabs)?;
        let (mut strings, mut components) = (Table::new(), Table::new());
        let mut entities = Vec::with_capacity(self.entity_count());
        for e in self.entities() {
            if let Some((parent, slot)) = self.as_derived(&lookup, e) {
                entities.push(CookedEntity::Derived {
                    id: e.id.0,
                    parent: parent.0,
                    slot,
                });
                continue;
            }
            let mut refs = Vec::with_capacity(e.components.len());
            for c in &e.components {
                let text = serde_json::to_string(c).map_err(|e| cooked(e.to_string()))?;
                refs.push(components.of(&text)?);
            }
            entities.push(CookedEntity::Own(OwnEntity {
                id: e.id.0,
                name: strings.of(&e.name)?,
                position: e.position,
                rotation_y_deg: e.rotation_y_deg,
                scale: e.scale,
                prefab: e.prefab.as_deref().map(|p| strings.of(p)).transpose()?,
                parent: e.parent.map(|p| p.0),
                layer: e.layer.as_deref().map(|l| strings.of(l)).transpose()?,
                prefab_child: e.prefab_child,
                components: refs,
            }));
        }
        let file = Cooked {
            magic: MAGIC,
            format: COOKED_FORMAT,
            world_format: crate::FORMAT_VERSION,
            source,
            head: head_text,
            next_entity_id: self.next_entity_id.0,
            strings: strings.items,
            components: components.items,
            entities,
        };
        postcard::to_allocvec(&file).map_err(|e| cooked(e.to_string()))
    }

    /// The world a cooked form holds, when it was made from source bytes whose
    /// [`source_hash`] is `source` by this format; anything else (another source, another
    /// version, damaged bytes, a world that does not validate) is an error, never a panic.
    pub fn from_cooked(bytes: &[u8], source: u64) -> Result<World, WorldError> {
        let file: Cooked =
            postcard::from_bytes(bytes).map_err(|e| cooked(format!("not readable: {e}")))?;
        if file.magic != MAGIC || file.format != COOKED_FORMAT {
            return Err(cooked(format!(
                "of another format (version {}, this engine reads {COOKED_FORMAT})",
                file.format
            )));
        }
        if file.world_format != crate::FORMAT_VERSION {
            return Err(cooked("made for another version of the world format"));
        }
        if file.source != source {
            return Err(cooked(
                "made from another source: the world's text changed since it was cooked",
            ));
        }
        if file.entities.len() > MAX_ENTITIES {
            return Err(WorldError::TooManyEntities {
                count: file.entities.len(),
                max: MAX_ENTITIES,
            });
        }
        let mut world = World::from_json(&file.head)?;
        let parsed: Vec<Component> = file
            .components
            .iter()
            .map(|t| serde_json::from_str(t).map_err(|e| cooked(format!("a component: {e}"))))
            .collect::<Result<_, _>>()?;
        let string = |i: u32| {
            file.strings
                .get(i as usize)
                .cloned()
                .ok_or_else(|| cooked(format!("string {i} is missing")))
        };
        let lookup = Lookup::new(&world.prefabs)?;
        let mut entities: BTreeMap<EntityId, Entity> = BTreeMap::new();
        for e in &file.entities {
            let e = match e {
                CookedEntity::Own(e) => e,
                CookedEntity::Derived { id, parent, slot } => {
                    let parent = entities
                        .get(&EntityId(*parent))
                        .ok_or_else(|| cooked(format!("entity {id} has no parent before it")))?;
                    let def = lookup
                        .of(parent)
                        .and_then(|p| p.children.get(*slot as usize))
                        .ok_or_else(|| cooked(format!("entity {id} is no child of its prefab")))?;
                    let child = derive(&lookup, parent, *slot as usize, def, EntityId(*id));
                    if entities.insert(child.id, child).is_some() {
                        return Err(WorldError::DuplicateEntityId { id: EntityId(*id) });
                    }
                    continue;
                }
            };
            let components = e
                .components
                .iter()
                .map(|i| {
                    parsed
                        .get(*i as usize)
                        .cloned()
                        .ok_or_else(|| cooked(format!("component {i} is missing")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let entity = Entity {
                id: EntityId(e.id),
                name: string(e.name)?,
                position: e.position,
                rotation_y_deg: e.rotation_y_deg,
                scale: e.scale,
                prefab: e.prefab.map(string).transpose()?,
                parent: e.parent.map(EntityId),
                layer: e.layer.map(string).transpose()?,
                prefab_child: e.prefab_child,
                components,
            };
            if entities.insert(entity.id, entity).is_some() {
                return Err(WorldError::DuplicateEntityId { id: EntityId(e.id) });
            }
        }
        world.entities = entities.into_iter().collect();
        world.next_entity_id = EntityId(file.next_entity_id);
        world.validate()?;
        Ok(world)
    }
}
