//! Prefab instances and their children as derived data (`docs/design/journal.md`, *Prefabs*).
//!
//! In memory an instance's children are real entities, each marked with the index of the
//! prefab child it was placed as ([`Entity::prefab_child`]). In the file they are not written:
//! they follow from the prefab and the instance's placement, so reading the instance places
//! them again. Only what differs from the prefab is written, on the instance:
//!
//! - `children`: the ids of the children by prefab child, `null` for a removed one. Left out
//!   when they are the ids an instance gets when it is spawned: the instance's own id plus
//!   1, 2, ... in the prefab's order, depth first through prefabs inside prefabs.
//! - `child_edits`: per changed child, by its index in the prefab, only the fields that
//!   differ (name, placement, scale, layer, component overrides, removed components), and for
//!   a child that is itself an instance its own `children` and `child_edits`.
//!
//! An entity added under an instance that is not from the prefab is written as an entity of
//! its own with its parent, as before. A world file says `"derived_children": true` when it
//! stores children this way; a file without it (written by an earlier engine, or a project's
//! layers, which write every child) holds every child in full, and a child at the id its
//! instance would give it is taken as that prefab child.

use crate::error::Ctx;
use crate::limits::{MAX_ENTITIES, MAX_HIERARCHY_DEPTH, MAX_INSTANCE_ENTITIES, MAX_PREFAB_NESTING};
use crate::prefab::place_child;
use crate::store::EntityMap;
use crate::{Component, Entity, EntityId, Prefab, PrefabChild, WorldError};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The prefabs of a world by id, with the number of entities an instance of each places.
/// Building one checks the prefabs as a whole: every prefab a child names exists, no prefab
/// contains itself, nesting stays within [`MAX_PREFAB_NESTING`] levels and an instance within
/// [`MAX_INSTANCE_ENTITIES`] entities.
pub(crate) struct Lookup<'a> {
    by_id: BTreeMap<&'a str, &'a Prefab>,
    /// Entities an instance places (itself included) and the levels of prefabs it spans.
    measured: BTreeMap<&'a str, (usize, usize)>,
}

impl<'a> Lookup<'a> {
    /// The lookup of `prefabs`, or the first problem of their nesting.
    pub(crate) fn new(prefabs: &'a [Prefab]) -> Result<Lookup<'a>, WorldError> {
        let mut lookup = Lookup {
            by_id: prefabs.iter().map(|p| (p.id.as_str(), p)).collect(),
            measured: BTreeMap::new(),
        };
        let mut stack = Vec::new();
        for p in prefabs {
            lookup.measure(p, &mut stack)?;
        }
        Ok(lookup)
    }

    /// Size and levels of `prefab`, memoised; `stack` holds the prefabs being measured.
    fn measure(
        &mut self,
        prefab: &'a Prefab,
        stack: &mut Vec<&'a str>,
    ) -> Result<(usize, usize), WorldError> {
        if let Some(m) = self.measured.get(prefab.id.as_str()) {
            return Ok(*m);
        }
        let outer = stack.first().copied().unwrap_or(prefab.id.as_str());
        if stack.contains(&prefab.id.as_str()) || stack.len() >= MAX_PREFAB_NESTING {
            let mut chain: Vec<&str> = stack.clone();
            chain.push(&prefab.id);
            return Err(WorldError::Prefab {
                prefab: outer.to_string(),
                message: format!(
                    "its children place prefabs inside prefabs {} or contain the prefab \
                     itself ({}); prefabs nest at most {MAX_PREFAB_NESTING} levels deep and \
                     never inside themselves",
                    if stack.contains(&prefab.id.as_str()) {
                        "in a loop".to_string()
                    } else {
                        format!("more than {MAX_PREFAB_NESTING} levels deep")
                    },
                    chain.join(" > ")
                ),
            });
        }
        stack.push(&prefab.id);
        let (mut size, mut levels) = (1usize, 1usize);
        for (i, child) in prefab.children.iter().enumerate() {
            match &child.prefab {
                None => size = size.saturating_add(1),
                Some(id) => {
                    let Some(inner) = self.by_id.get(id.as_str()).copied() else {
                        stack.pop();
                        return Err(WorldError::Prefab {
                            prefab: prefab.id.clone(),
                            message: format!(
                                "children[{i}].prefab: no prefab has the id \"{id}\"; a child \
                                 places a prefab of the same world (add that prefab first)"
                            ),
                        });
                    };
                    let (s, l) = match self.measure(inner, stack) {
                        Ok(m) => m,
                        Err(e) => {
                            stack.pop();
                            return Err(e);
                        }
                    };
                    size = size.saturating_add(s);
                    levels = levels.max(l.saturating_add(1));
                }
            }
        }
        stack.pop();
        if levels > MAX_PREFAB_NESTING {
            return Err(WorldError::Prefab {
                prefab: prefab.id.clone(),
                message: format!(
                    "its children place prefabs inside prefabs {levels} levels deep; prefabs \
                     nest at most {MAX_PREFAB_NESTING} levels deep and never inside themselves"
                ),
            });
        }
        if size > MAX_INSTANCE_ENTITIES {
            return Err(WorldError::Prefab {
                prefab: prefab.id.clone(),
                message: format!(
                    "an instance would place {size} entities with the prefabs inside it; one \
                     instance places at most {MAX_INSTANCE_ENTITIES}"
                ),
            });
        }
        self.measured.insert(&prefab.id, (size, levels));
        Ok((size, levels))
    }

    /// The prefab with this id.
    pub(crate) fn get(&self, id: &str) -> Option<&'a Prefab> {
        self.by_id.get(id).copied()
    }

    /// The prefab `e` is an instance of, when it is one of a known prefab.
    pub(crate) fn of(&self, e: &Entity) -> Option<&'a Prefab> {
        e.prefab.as_deref().and_then(|p| self.get(p))
    }

    /// How many entities an instance of prefab `id` places, itself included (1 for an
    /// unknown id).
    pub(crate) fn size(&self, id: &str) -> usize {
        self.measured.get(id).map_or(1, |m| m.0)
    }

    /// How many entities prefab child `def` places, itself included.
    fn def_size(&self, def: &PrefabChild) -> usize {
        def.prefab.as_deref().map_or(1, |q| self.size(q))
    }

    /// The ids the children of an instance `root` of `prefab` get when it is spawned: depth
    /// first, in the prefab's order, right after the instance's own id.
    pub(crate) fn default_ids(&self, root: EntityId, prefab: &Prefab) -> Vec<EntityId> {
        let mut next = root.0.saturating_add(1);
        prefab
            .children
            .iter()
            .map(|def| {
                let id = EntityId(next);
                next = next.saturating_add(self.def_size(def) as u64);
                id
            })
            .collect()
    }
}

/// Whether two numbers are the same bits (so `-0.0` and `0.0` differ, as they do in a file).
fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits()
}

fn same3(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(&b).all(|(x, y)| same(*x, *y))
}

/// The entity prefab child `def` (index `slot`) of instance `parent` is, under `id`: placed
/// relative to the parent, in its layer, with the child's components (for a child that places
/// a prefab: that prefab's components with the child's as overrides).
pub(crate) fn derive(
    lookup: &Lookup<'_>,
    parent: &Entity,
    slot: usize,
    def: &PrefabChild,
    id: EntityId,
) -> Entity {
    let (position, rotation_y_deg) = place_child(
        parent.position,
        parent.rotation_y_deg,
        parent.scale,
        def.offset,
        def.rotation_y_deg,
    );
    let nested = def.prefab.as_deref().and_then(|q| lookup.get(q));
    let name = match nested {
        Some(q) if def.name.is_empty() => q.name.clone(),
        _ => def.name.clone(),
    };
    let components = match nested {
        Some(q) => q.effective(&def.components),
        None => def.components.clone(),
    };
    Entity {
        id,
        name,
        position,
        rotation_y_deg,
        // The instance's scale: a child of its own has none (a scale given to a child is an
        // edit of the instance, `ChildEdit::scale`).
        scale: parent.scale,
        prefab: def.prefab.clone(),
        parent: Some(parent.id),
        layer: parent.layer.clone(),
        prefab_child: u32::try_from(slot).ok(),
        components,
    }
}

/// Places the children of instance `root` and everything below them, depth first in the
/// prefab's order, under ids from `*next` on, appending them to `out`. Each is validated.
pub(crate) fn spawn_children(
    lookup: &Lookup<'_>,
    root: &Entity,
    next: &mut u64,
    out: &mut Vec<Entity>,
    depth: usize,
) -> Result<(), WorldError> {
    let Some(prefab) = lookup.of(root) else {
        return Ok(());
    };
    if depth > MAX_PREFAB_NESTING {
        return Err(WorldError::Prefab {
            prefab: prefab.id.clone(),
            message: format!("prefabs nest at most {MAX_PREFAB_NESTING} levels deep"),
        });
    }
    for (i, def) in prefab.children.iter().enumerate() {
        let id = EntityId(*next);
        *next = next.checked_add(1).ok_or(WorldError::IdsExhausted)?;
        let child = derive(lookup, root, i, def, id);
        child.validate()?;
        out.push(child.clone());
        spawn_children(lookup, &child, next, out, depth.saturating_add(1))?;
    }
    Ok(())
}

/// Reads a field that may be `null` as `Some(None)`, so an absent field and `null` differ.
fn some<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

/// What differs in one child of an instance from what its prefab places, by the child's
/// index in the prefab. Every field is optional; absent means as the prefab places it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChildEdit {
    /// The index of the child in its prefab's `children`.
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The child's own world position, metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_y_deg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// The child's layer; `null` is the world's default layer.
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub layer: Option<Option<String>>,
    /// Components that replace the child's of the same type, or that it lacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<Vec<Component>>,
    /// Component types the child no longer has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_components: Option<Vec<String>>,
    /// For a child that places a prefab: its children's ids, as on an instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<Option<EntityId>>>,
    /// For a child that places a prefab: its children's edits, as on an instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_edits: Option<Vec<ChildEdit>>,
}

impl ChildEdit {
    /// Whether it changes nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.position.is_none()
            && self.rotation_y_deg.is_none()
            && self.scale.is_none()
            && self.layer.is_none()
            && self.overrides.is_none()
            && self.removed_components.is_none()
            && self.children.is_none()
            && self.child_edits.is_none()
    }

    /// Whether it changes the child itself (not counting what is below it).
    pub(crate) fn changes_child(&self) -> bool {
        let mut own = self.clone();
        own.children = None;
        own.child_edits = None;
        !own.is_empty()
    }
}

/// What turns `derived` (what the prefab places) into `actual`, as the edit of child `index`.
/// Overrides come in a fixed order, those of the derived child's types in its order and then
/// new types in `actual`'s order, so that reading the edit back and diffing again gives the
/// same edit.
pub(crate) fn diff(index: u32, derived: &Entity, actual: &Entity) -> ChildEdit {
    let own = |kind: &str| actual.components.iter().find(|c| c.kind() == kind);
    let mut overrides: Vec<Component> = derived
        .components
        .iter()
        .filter_map(|d| own(d.kind()).filter(|a| *a != d).cloned())
        .collect();
    overrides.extend(
        actual
            .components
            .iter()
            .filter(|a| derived.components.iter().all(|d| d.kind() != a.kind()))
            .cloned(),
    );
    let removed: Vec<String> = derived
        .components
        .iter()
        .filter(|d| own(d.kind()).is_none())
        .map(|d| d.kind().to_string())
        .collect();
    ChildEdit {
        index,
        name: (derived.name != actual.name).then(|| actual.name.clone()),
        position: (!same3(derived.position, actual.position)).then_some(actual.position),
        rotation_y_deg: (!same(derived.rotation_y_deg, actual.rotation_y_deg))
            .then_some(actual.rotation_y_deg),
        scale: (!same(derived.scale, actual.scale)).then_some(actual.scale),
        layer: (derived.layer != actual.layer).then(|| actual.layer.clone()),
        overrides: (!overrides.is_empty()).then_some(overrides),
        removed_components: (!removed.is_empty()).then_some(removed),
        children: None,
        child_edits: None,
    }
}

/// `derived` with `edit` applied (not what is below it). An unknown component type in
/// `removed_components` is an error of entity `derived.id`.
pub(crate) fn apply(edit: &ChildEdit, mut derived: Entity) -> Result<Entity, WorldError> {
    let ctx = Ctx::entity(derived.id);
    if let Some(name) = &edit.name {
        derived.name.clone_from(name);
    }
    if let Some(p) = edit.position {
        derived.position = p;
    }
    if let Some(r) = edit.rotation_y_deg {
        derived.rotation_y_deg = r;
    }
    if let Some(s) = edit.scale {
        derived.scale = s;
    }
    if let Some(layer) = &edit.layer {
        derived.layer.clone_from(layer);
    }
    for kind in edit.removed_components.iter().flatten() {
        if !Component::KINDS.contains(&kind.as_str()) {
            return Err(ctx.invalid(
                "removed_components",
                format!("{kind:?}"),
                format!("a component type, one of: {}", Component::KINDS.join(", ")),
            ));
        }
        derived.remove_component(kind);
    }
    if let Some(overrides) = &edit.overrides {
        crate::prefab::check_components(overrides, "overrides").map_err(|e| match e {
            WorldError::Invalid {
                field,
                value,
                expected,
                ..
            } => WorldError::Invalid {
                entity: Some(derived.id),
                field,
                value,
                expected,
            },
            other => other,
        })?;
        for o in overrides {
            derived.set_component(o.clone());
        }
    }
    Ok(derived)
}

/// The children of every entity that has some, in id order.
pub(crate) fn child_index(entities: &EntityMap) -> BTreeMap<EntityId, Vec<EntityId>> {
    let mut index: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
    for e in entities.values() {
        if let Some(p) = e.parent {
            index.entry(p).or_default().push(e.id);
        }
    }
    index
}

/// Everything below `id` by `index`, level by level, at most [`MAX_HIERARCHY_DEPTH`] levels.
pub(crate) fn descendants_in(
    index: &BTreeMap<EntityId, Vec<EntityId>>,
    id: EntityId,
) -> Vec<EntityId> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::from([id]);
    let mut level = vec![id];
    for _ in 0..MAX_HIERARCHY_DEPTH {
        let next: Vec<EntityId> = level
            .iter()
            .filter_map(|p| index.get(p))
            .flatten()
            .copied()
            .filter(|c| seen.insert(*c))
            .collect();
        if next.is_empty() {
            break;
        }
        out.extend(next.iter().copied());
        level = next;
    }
    out
}

/// The children of instance `x` of `prefab` by prefab child: per index the child placed as
/// it (the first in id order that is marked so and places the same prefab), `None` when the
/// instance has none (it was removed).
pub(crate) fn claims(
    prefab: &Prefab,
    x: EntityId,
    index: &BTreeMap<EntityId, Vec<EntityId>>,
    entities: &EntityMap,
) -> Vec<Option<EntityId>> {
    let mut slots: Vec<Option<EntityId>> = vec![None; prefab.children.len()];
    for &c in index.get(&x).into_iter().flatten() {
        let Some(e) = entities.get(c) else {
            continue;
        };
        let Some(i) = e.prefab_child else {
            continue;
        };
        let Some(def) = prefab.children.get(i as usize) else {
            continue;
        };
        if let Some(slot) = slots.get_mut(i as usize)
            && slot.is_none()
            && e.prefab == def.prefab
        {
            *slot = Some(c);
        }
    }
    slots
}

/// What a world file holds of instances: which entities are written inside their instance
/// rather than on their own, and per instance its children's ids and edits.
pub(crate) struct Written {
    /// Children written inside their instance.
    pub absorbed: BTreeSet<EntityId>,
    /// Whether any instance has children from its prefab (the file then says so).
    pub derived: bool,
    claims: BTreeMap<EntityId, Vec<Option<EntityId>>>,
}

impl Written {
    /// The plan for `entities` with `lookup`.
    pub(crate) fn plan(lookup: &Lookup<'_>, entities: &EntityMap) -> Written {
        let index = child_index(entities);
        let mut claims_by = BTreeMap::new();
        let mut absorbed = BTreeSet::new();
        let mut derived = false;
        for e in entities.values() {
            let Some(prefab) = lookup.of(e) else {
                continue;
            };
            if prefab.children.is_empty() {
                continue;
            }
            derived = true;
            let slots = claims(prefab, e.id, &index, entities);
            absorbed.extend(slots.iter().flatten().copied());
            claims_by.insert(e.id, slots);
        }
        Written {
            absorbed,
            derived,
            claims: claims_by,
        }
    }

    /// The `children` and `child_edits` of instance `x`: `None` where they are what the
    /// prefab gives.
    pub(crate) fn of_instance(
        &self,
        lookup: &Lookup<'_>,
        entities: &EntityMap,
        x: &Entity,
        depth: usize,
    ) -> (Option<Vec<Option<EntityId>>>, Option<Vec<ChildEdit>>) {
        let (Some(prefab), Some(slots)) = (lookup.of(x), self.claims.get(&x.id)) else {
            return (None, None);
        };
        let defaults: Vec<Option<EntityId>> = lookup
            .default_ids(x.id, prefab)
            .into_iter()
            .map(Some)
            .collect();
        let children = (*slots != defaults).then(|| slots.clone());
        let mut edits = Vec::new();
        for (i, (slot, def)) in slots.iter().zip(&prefab.children).enumerate() {
            let Some(actual) = slot.and_then(|c| entities.get(c)) else {
                continue;
            };
            let index = u32::try_from(i).unwrap_or(u32::MAX);
            let mut edit = diff(index, &derive(lookup, x, i, def, actual.id), actual);
            if depth < MAX_PREFAB_NESTING {
                let (c, e) = self.of_instance(lookup, entities, actual, depth.saturating_add(1));
                edit.children = c;
                edit.child_edits = e;
            }
            if !edit.is_empty() {
                edits.push(edit);
            }
        }
        (children, (!edits.is_empty()).then_some(edits))
    }
}

/// An instance read from a file whose children are still to be placed.
pub(crate) struct Pending {
    pub id: EntityId,
    pub children: Option<Vec<Option<EntityId>>>,
    pub edits: Vec<ChildEdit>,
}

/// Places the children of every instance in `entities` (read from a file with derived
/// children): `pending` are the instances written in the file with their `children` and
/// `child_edits`. A child written in full (at the id it would get, with the instance as its
/// parent) is taken as that child instead of placing it again. Every inconsistency is an
/// error naming the instance and the field.
pub(crate) fn place_derived(
    lookup: &Lookup<'_>,
    entities: &mut BTreeMap<EntityId, Entity>,
    mut pending: Vec<Pending>,
) -> Result<(), WorldError> {
    let mut done: BTreeSet<EntityId> = BTreeSet::new();
    pending.reverse();
    while let Some(Pending {
        id,
        children,
        edits,
    }) = pending.pop()
    {
        if !done.insert(id) {
            continue;
        }
        let ctx = Ctx::entity(id);
        let Some(x) = entities.get(&id).cloned() else {
            continue;
        };
        let Some(prefab) = lookup.of(&x) else {
            if children.is_some() || !edits.is_empty() {
                return Err(ctx.invalid(
                    "children",
                    "given",
                    "`children` and `child_edits` belong to an instance of a prefab with \
                     children; give `prefab`, or leave them out",
                ));
            }
            continue;
        };
        let ids = match children {
            Some(ids) if ids.len() != prefab.children.len() => {
                return Err(ctx.invalid(
                    "children",
                    format!("a list of {}", ids.len()),
                    format!(
                        "one id (or null for a removed child) per child of prefab \"{}\", which \
                         has {}",
                        prefab.id,
                        prefab.children.len()
                    ),
                ));
            }
            Some(ids) => ids,
            None => lookup
                .default_ids(id, prefab)
                .into_iter()
                .map(Some)
                .collect(),
        };
        let mut by_slot: BTreeMap<usize, ChildEdit> = BTreeMap::new();
        for (k, edit) in edits.into_iter().enumerate() {
            let slot = edit.index as usize;
            if slot >= prefab.children.len() {
                return Err(ctx.invalid(
                    format!("child_edits[{k}].index"),
                    edit.index,
                    format!(
                        "the index of a child of prefab \"{}\": 0 to {}",
                        prefab.id,
                        prefab.children.len().saturating_sub(1)
                    ),
                ));
            }
            if by_slot.insert(slot, edit).is_some() {
                return Err(ctx.invalid(
                    format!("child_edits[{k}].index"),
                    slot,
                    "each child is edited at most once",
                ));
            }
        }
        for (slot, (child, def)) in ids.iter().zip(&prefab.children).enumerate() {
            let edit = by_slot.remove(&slot);
            let Some(cid) = *child else {
                if edit.is_some() {
                    return Err(ctx.invalid(
                        format!("child_edits (index {slot})"),
                        "given",
                        "that child is removed (null in `children`); leave out its edit",
                    ));
                }
                continue;
            };
            if let Some(written) = entities.get_mut(&cid) {
                if written.parent != Some(id) {
                    return Err(ctx.invalid(
                        format!("children[{slot}]"),
                        cid,
                        format!(
                            "the entity {cid} is not a child of this instance; give each child \
                             of the instance an id no other entity has"
                        ),
                    ));
                }
                if edit.is_some() {
                    return Err(ctx.invalid(
                        format!("child_edits (index {slot})"),
                        "given",
                        format!(
                            "the child {cid} is written in full as an entity of its own; give \
                             its changes there and leave out the edit"
                        ),
                    ));
                }
                if written.prefab == def.prefab && written.prefab_child.is_none() {
                    written.prefab_child = u32::try_from(slot).ok();
                }
                continue;
            }
            if entities.len() >= MAX_ENTITIES {
                return Err(WorldError::TooManyEntities {
                    count: entities.len().saturating_add(1),
                    max: MAX_ENTITIES,
                });
            }
            let derived = derive(lookup, &x, slot, def, cid);
            let (entity, nested) = match edit {
                Some(edit) => {
                    let nested = Pending {
                        id: cid,
                        children: edit.children.clone(),
                        edits: edit.child_edits.clone().unwrap_or_default(),
                    };
                    (apply(&edit, derived)?, nested)
                }
                None => (
                    derived,
                    Pending {
                        id: cid,
                        children: None,
                        edits: Vec::new(),
                    },
                ),
            };
            let nested_instance = entity.prefab.is_some();
            entities.insert(cid, entity);
            if nested_instance {
                pending.push(nested);
            } else if nested.children.is_some() || !nested.edits.is_empty() {
                return Err(ctx.invalid(
                    format!("child_edits (index {slot}).children"),
                    "given",
                    "only a child that places a prefab has children of its own to edit",
                ));
            }
        }
    }
    Ok(())
}

/// Marks the children of a file without derived children: a child of an instance at the id
/// the instance would give it (and placing the same prefab) is taken as that prefab child.
pub(crate) fn mark_written(lookup: &Lookup<'_>, entities: &mut BTreeMap<EntityId, Entity>) {
    let instances: Vec<(EntityId, &Prefab)> = entities
        .values()
        .filter_map(|e| lookup.of(e).map(|p| (e.id, p)))
        .filter(|(_, p)| !p.children.is_empty())
        .collect();
    for (x, prefab) in instances {
        for (slot, (cid, def)) in lookup
            .default_ids(x, prefab)
            .into_iter()
            .zip(&prefab.children)
            .enumerate()
        {
            if let Some(child) = entities.get_mut(&cid)
                && child.parent == Some(x)
                && child.prefab == def.prefab
                && child.prefab_child.is_none()
            {
                child.prefab_child = u32::try_from(slot).ok();
            }
        }
    }
}

/// Which child of `old` each child of `new` is: the child of the same name when that name is
/// unique in both prefabs, otherwise the child at the same index when it has the same name
/// and is not taken yet. `None`: a new child.
pub(crate) fn match_children(old: &Prefab, new: &Prefab) -> Vec<Option<usize>> {
    let unique = |p: &Prefab, name: &str| {
        !name.is_empty() && p.children.iter().filter(|c| c.name == name).count() == 1
    };
    let mut taken = vec![false; old.children.len()];
    let mut out: Vec<Option<usize>> = new
        .children
        .iter()
        .map(|nd| {
            if !unique(new, &nd.name) || !unique(old, &nd.name) {
                return None;
            }
            let i = old.children.iter().position(|od| od.name == nd.name)?;
            if let Some(t) = taken.get_mut(i) {
                *t = true;
            }
            Some(i)
        })
        .collect();
    for (j, slot) in out.iter_mut().enumerate() {
        if slot.is_some() {
            continue;
        }
        let (Some(nd), Some(od)) = (new.children.get(j), old.children.get(j)) else {
            continue;
        };
        if od.name == nd.name && taken.get(j) == Some(&false) && !unique(old, &od.name) {
            if let Some(t) = taken.get_mut(j) {
                *t = true;
            }
            *slot = Some(j);
        }
    }
    out
}

impl crate::World {
    /// Brings the instances of prefab `new.id` in step after it changed from `old`
    /// (`before`: the world's prefabs before the change; `self.prefabs` holds the new ones).
    /// The rule is the one of components, applied to children, which are recognised by name
    /// ([`match_children`]):
    ///
    /// - a child the prefab still has (placing the same prefab, if any) keeps what was edited
    ///   in it and takes everything else from the new prefab child, moving with what is below
    ///   it when its placement changes;
    /// - a child the prefab no longer has (or that now places another prefab) is removed
    ///   when it was not edited and nothing of its own is below it, and otherwise stays, as
    ///   an entity of its own under the instance;
    /// - a child the prefab gained is placed in every instance, under new ids;
    /// - a child removed from an instance stays removed.
    pub(crate) fn follow_prefab(
        &mut self,
        before: &[Prefab],
        old: &Prefab,
        new: &Prefab,
    ) -> Result<(), WorldError> {
        let prefabs = self.prefabs.clone();
        let lookup = Lookup::new(&prefabs)?;
        let old_lookup = Lookup::new(before)?;
        let index = child_index(&self.entities);
        let from_old = match_children(old, new);
        let mut to_new: Vec<Option<usize>> = vec![None; old.children.len()];
        for (j, i) in from_old.iter().enumerate() {
            if let Some(slot) = i.and_then(|i| to_new.get_mut(i)) {
                *slot = Some(j);
            }
        }
        let instances: Vec<EntityId> = self
            .entities
            .values()
            .filter(|e| e.prefab.as_deref() == Some(new.id.as_str()))
            .map(|e| e.id)
            .collect();
        for x_id in instances {
            let Some(mut x) = self.entities.get(x_id).cloned() else {
                continue;
            };
            let overrides = old.overrides_of(&x.components);
            x.components = new.effective(&overrides);
            x.validate()?;
            self.entities.insert(x_id, x.clone());
            let slots = claims(old, x_id, &index, &self.entities);
            for (i, od) in old.children.iter().enumerate() {
                let Some(c) = slots.get(i).copied().flatten() else {
                    continue; // removed from this instance: stays removed
                };
                let Some(actual) = self.entities.get(c).cloned() else {
                    continue;
                };
                let slot = u32::try_from(i).unwrap_or(u32::MAX);
                let edit = diff(slot, &derive(&old_lookup, &x, i, od, c), &actual);
                let below = descendants_in(&index, c);
                let kept = to_new
                    .get(i)
                    .copied()
                    .flatten()
                    .and_then(|j| new.children.get(j).map(|nd| (j, nd)))
                    .filter(|(_, nd)| nd.prefab == od.prefab);
                if let Some((j, nd)) = kept {
                    let mut e = apply(&edit, derive(&lookup, &x, j, nd, c))?;
                    let (position, yaw) = (e.position, e.rotation_y_deg);
                    e.position = actual.position;
                    e.rotation_y_deg = actual.rotation_y_deg;
                    e.validate()?;
                    self.entities.insert(c, e);
                    if !same3(position, actual.position) || !same(yaw, actual.rotation_y_deg) {
                        self.place_tree(c, position, yaw, actual.scale, &below)?;
                    }
                    continue;
                }
                let pristine = !edit.changes_child()
                    && below.iter().all(|d| {
                        self.entities
                            .get(*d)
                            .is_some_and(|e| e.prefab_child.is_some())
                    });
                if pristine {
                    self.entities.remove(c);
                    for d in below {
                        self.entities.remove(d);
                    }
                } else if let Some(e) = self.entities.get_mut(c) {
                    e.prefab_child = None;
                }
            }
            for (j, nd) in new.children.iter().enumerate() {
                let follows = from_old
                    .get(j)
                    .copied()
                    .flatten()
                    .and_then(|i| old.children.get(i))
                    .is_some_and(|od| od.prefab == nd.prefab);
                if !follows {
                    self.spawn_slot(&lookup, &x, j, nd)?;
                }
            }
        }
        Ok(())
    }

    /// Places prefab child `def` (index `slot`) of instance `x`, and what is below it, under
    /// new ids.
    fn spawn_slot(
        &mut self,
        lookup: &Lookup<'_>,
        x: &Entity,
        slot: usize,
        def: &PrefabChild,
    ) -> Result<(), WorldError> {
        let size = lookup.def_size(def);
        if self.entities.len().saturating_add(size) > MAX_ENTITIES {
            return Err(WorldError::TooManyEntities {
                count: self.entities.len().saturating_add(size),
                max: MAX_ENTITIES,
            });
        }
        let mut next = self.next_entity_id.0;
        let id = EntityId(next);
        next = next.checked_add(1).ok_or(WorldError::IdsExhausted)?;
        let child = derive(lookup, x, slot, def, id);
        child.validate()?;
        let mut placed = vec![child.clone()];
        spawn_children(lookup, &child, &mut next, &mut placed, 2)?;
        for e in placed {
            self.entities.insert(e.id, e);
        }
        self.next_entity_id = EntityId(next);
        Ok(())
    }
}
