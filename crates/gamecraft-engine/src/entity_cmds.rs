//! Entities: `entity.add`, `entity.set`, `entity.remove`, `entity.remove_many`,
//! `entity.set_component`, `entity.remove_component`, `entity.snap_to_ground`.
//!
//! `entity.add` places its entity by `position`, by `at` (the ground under it, with an
//! `offset_m`) or by `span`, a deck between two ground points (`span.rs`).

use crate::error::{CommandError, bad, failed};
use crate::params::{Params, component};
use crate::session::{Edit, Handler, Registration};
use crate::span::{Measured, Span};
use crate::spec::{CommandSpec, ParamType as T, kind_of, param, spec};
use gamecraft_world::limits::{MAX_COORDINATE_M, SCALE_MAX, SCALE_MIN};
use gamecraft_world::{Component, Entity, EntityDraft, EntityId};
use serde_json::{Map, Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: add_spec,
        handler: Handler::World(add),
    },
    Registration {
        spec: set_spec,
        handler: Handler::World(set),
    },
    Registration {
        spec: remove_spec,
        handler: Handler::World(remove),
    },
    Registration {
        spec: remove_many_spec,
        handler: Handler::World(remove_many),
    },
    Registration {
        spec: set_component_spec,
        handler: Handler::World(set_component),
    },
    Registration {
        spec: remove_component_spec,
        handler: Handler::World(remove_component),
    },
    Registration {
        spec: snap_spec,
        handler: Handler::World(snap_to_ground),
    },
];

const ID_DOC: &str = "The entity: its id as a number, or its name as a string (any case).";
/// Most entities `entity.remove_many` takes at once: as many as a world holds.
const MAX_REMOVED: usize = gamecraft_world::limits::MAX_ENTITIES;

pub(crate) fn position_param() -> crate::spec::ParamSpec {
    param(
        "position",
        T::Point3,
        "Where the entity's base stands, [x, y, z]; give this or `at`, not both.",
    )
    .unit("m")
}

pub(crate) fn at_param() -> crate::spec::ParamSpec {
    param(
        "at",
        T::Point2,
        "Puts the base on the ground at [x, z]; an error where there is no ground. Give this or \
         `position`, not both.",
    )
    .unit("m")
}

/// The `offset_m` parameter: how high above the ground of `at` the base stands.
pub(crate) fn offset_param() -> crate::spec::ParamSpec {
    param(
        "offset_m",
        T::Number,
        "With `at`: how high the base stands above the ground under it (negative: below it). \
         Give it with `at` only.",
    )
    .unit("m")
}

/// The offset over the ground, 0 when left out; checked against the coordinate limit.
fn offset_of(p: &Params<'_>) -> Result<f64, CommandError> {
    let offset = p.f64("offset_m")?.unwrap_or(0.0);
    if !(offset.is_finite() && offset.abs() <= MAX_COORDINATE_M) {
        return Err(bad(format!(
            "`offset_m` is {offset:?}; it must be a finite number of metres from \
             -{MAX_COORDINATE_M} to {MAX_COORDINATE_M}"
        )));
    }
    Ok(offset)
}

/// Why `offset_m` was refused: it goes with `at`.
const OFFSET_NEEDS_AT: &str =
    "`offset_m` goes with `at`: it says how high the base stands above the ground `at` names";

/// The base position from `position` or `at` (with `offset_m`); `None` when neither is given.
pub(crate) fn placement(edit: &Edit<'_>, p: &Params<'_>) -> Result<Option<[f64; 3]>, CommandError> {
    match (p.point3("position")?, p.point2("at")?) {
        (Some(_), Some(_)) => Err(bad(
            "give `position` or `at`, not both: `at` puts the base on the ground",
        )),
        (Some(_), None) if p.has("offset_m") => Err(bad(OFFSET_NEEDS_AT)),
        (Some(position), None) => Ok(Some(position)),
        (None, Some([x, z])) => {
            let ground = edit.ground(x, z)?;
            // Without an offset the base is exactly on the ground, as before.
            let y = match p.f64("offset_m")? {
                Some(offset) if offset != 0.0 => ground + offset_of(p)?,
                _ => ground,
            };
            Ok(Some([x, y, z]))
        }
        (None, None) if p.has("offset_m") => Err(bad(OFFSET_NEEDS_AT)),
        (None, None) => Ok(None),
    }
}

/// The `span` parameter of a command that places one thing, an entity or an instance.
pub(crate) fn span_param() -> crate::spec::ParamSpec {
    param(
        "span",
        T::Object,
        "Places it as a deck between two points of the ground: {\"from\": [x, z], \"to\": \
         [x, z], \"offset_m\"? = 0, \"length_m\"?}. It stands level at the midpoint, its base on \
         the higher end's ground plus `offset_m` (what a sloped span leaves above the lower \
         ground is in the answer, `ends`), faces from `from` to `to` (its front, -Z, at `to`), \
         and is scaled so that its length along its forward axis spans the two ends. Give this \
         or `position` or `at`, not both; not with `scale`, `rotation_y_deg` or `offset_m`, \
         which it sets itself.",
    )
}

/// Refuses the parameters a `span` cannot go with; `thing` is "entity" or "instance".
pub(crate) fn span_alone(p: &Params<'_>, thing: &str) -> Result<(), CommandError> {
    for key in ["position", "at"] {
        if p.has(key) {
            return Err(bad(format!(
                "`span` places the {thing} between two ground points; give `span` or `{key}`, \
                 not both"
            )));
        }
    }
    if p.has("offset_m") {
        return Err(bad(
            "`offset_m` goes with `at`; a span has its own, `span.offset_m`",
        ));
    }
    for key in ["scale", "rotation_y_deg"] {
        if p.has(key) {
            return Err(bad(format!(
                "`span` sets the scale and the turn itself; leave out `{key}`"
            )));
        }
    }
    Ok(())
}

/// The `layer` parameter: what it says each command does with it.
pub(crate) fn layer_param(what: &str) -> crate::spec::ParamSpec {
    param(
        "layer",
        T::String,
        &format!(
            "{what} A plain file name of 1 to 64 letters, digits, `-` and `_` (in a project the \
             layer file `layers/<layer>.json`); \"\" or left out is the world's default layer."
        ),
    )
}

/// The layer a command asks for: `None` when left out, `Some(None)` for the default layer
/// (`""`), else the name. The name is checked with the entity it lands on.
pub(crate) fn layer_of(p: Params<'_>) -> Result<Option<Option<String>>, CommandError> {
    Ok(p.str("layer")?
        .map(|l| (!l.is_empty()).then(|| l.to_string())))
}

/// The entity `id`, to change.
fn entity_mut<'w>(edit: &'w mut Edit<'_>, id: EntityId) -> Result<&'w mut Entity, CommandError> {
    edit.world
        .entity_mut(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))
}

fn add_spec() -> CommandSpec {
    spec(
        "entity.add",
        "Add Entity",
        "Adds an entity with a name, a placement and components. Give `position`, `at` or \
         `span`.",
        vec![
            param(
                "name",
                T::String,
                "Its name, at most 200 characters; need not be unique.",
            )
            .default(json!("")),
            position_param(),
            at_param(),
            offset_param(),
            param(
                "rotation_y_deg",
                T::Number,
                "Turn about +Y, counter-clockwise seen from above; 0 faces north (-Z).",
            )
            .unit("deg")
            .default(json!(0)),
            param("scale", T::Number, "Uniform scale, 0.001 to 1000.").default(json!(1)),
            span_param(),
            layer_param("The layer it belongs to."),
            param(
                "components",
                T::Array,
                "Components as in the file format, at most one of each type: {\"type\": \
                 \"shape\", \"kind\": \"box\"|\"sphere\"|\"cylinder\", \"size\", \"color\", \
                 \"solid\"}, {\"type\": \"plant\", \"species\", \"seed\", \"growth\"}, \
                 {\"type\": \"character\", \"height_m\", ...}; fields left out take their \
                 defaults.",
            )
            .default(json!([])),
        ],
        "{id, position, span?}: with `span`, what it placed: {from, to, length_m, scale, ends: \
         [{point, ground_m, gap_m}, {point, ground_m, gap_m}]}: where each end of the deck \
         stands, the ground under it and how far above it (0 on level ground)",
        json!({
            "name": "Rock",
            "at": [10, 5],
            "components": [{"type": "shape", "kind": "sphere", "size": [1.5, 1.5, 1.5]}]
        }),
    )
}

/// The components of the `components` parameter (`components[i]` in errors).
fn components_of(p: &Params<'_>) -> Result<Vec<Component>, CommandError> {
    let mut components = Vec::new();
    if let Some(list) = p.value("components").and_then(Value::as_array) {
        for (i, v) in list.iter().enumerate() {
            components.push(component(v, &format!("components[{i}]"))?);
        }
    }
    Ok(components)
}

/// The `scale` parameter: `None` when left out, else within an entity's limits (the sentence
/// names the parameter, where the world's would name the entity).
pub(crate) fn scale_of(p: &Params<'_>) -> Result<Option<f64>, CommandError> {
    match p.f64("scale")? {
        None => Ok(None),
        Some(scale) if (SCALE_MIN..=SCALE_MAX).contains(&scale) => Ok(Some(scale)),
        Some(scale) => Err(bad(format!(
            "`scale` is {scale:?}; it must be a number from {SCALE_MIN} to {SCALE_MAX}"
        ))),
    }
}

fn add(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let components = components_of(&p)?;
    let (position, rotation_y_deg, scale, span) = if p.has("span") {
        span_alone(&p, "entity")?;
        let measured = gamecraft_world::components_box(&components, edit.world.all_model_bounds())
            .map(Measured::of_box);
        let Span {
            position,
            rotation_y_deg,
            scale,
            report,
        } = crate::span::span(
            edit,
            &p,
            measured,
            "the entity",
            "has no shape to measure its length from",
        )?;
        (position, rotation_y_deg, scale, Some(report))
    } else {
        (
            placement(edit, &p)?
                .ok_or_else(|| bad("missing `position` or `at` (or `span`, a deck between two ground points); give one of them"))?,
            p.f64("rotation_y_deg")?.unwrap_or(0.0),
            scale_of(&p)?.unwrap_or(1.0),
            None,
        )
    };
    let draft = EntityDraft {
        name: p.str("name")?.unwrap_or("").to_string(),
        position,
        rotation_y_deg,
        scale,
        components,
        layer: layer_of(p)?.flatten(),
    };
    let id = edit
        .world
        .add_entity(draft)
        .map_err(|e| bad(e.to_string()))?;
    let mut out = Map::new();
    out.insert("id".into(), json!(id.0));
    out.insert("position".into(), json!(position));
    if let Some(report) = span {
        out.insert("span".into(), report);
    }
    Ok(Value::Object(out))
}

fn set_spec() -> CommandSpec {
    spec(
        "entity.set",
        "Edit Entity",
        "Renames, moves, turns, scales an entity or puts it in another layer; only the fields \
         given change. The entities that belong to it (a prefab's children) move, turn and \
         scale with it, and go to the same layer.",
        vec![
            param("id", T::Entity, ID_DOC).required(),
            param("name", T::String, "Its new name, at most 200 characters."),
            position_param(),
            at_param(),
            offset_param(),
            param(
                "rotation_y_deg",
                T::Number,
                "Turn about +Y, counter-clockwise seen from above; 0 faces north (-Z).",
            )
            .unit("deg"),
            param(
                "scale",
                T::Number,
                "Uniform scale, 0.001 to 1000. A prefab's children follow it: each stands at \
                 its offset times the scale, at that scale.",
            ),
            layer_param("The layer it goes to, with the entities that belong to it."),
        ],
        "{id, position}",
        json!({"id": "Crate", "at": [4, -6], "rotation_y_deg": 45}),
    )
}

fn set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = p.required_entity("id", &edit.world)?;
    let position = placement(edit, &p)?;
    let name = p.str("name")?;
    let rotation = p.f64("rotation_y_deg")?;
    let layer = layer_of(p)?;
    if position.is_some() || rotation.is_some() {
        let entity = entity_mut(edit, id)?;
        let to = position.unwrap_or(entity.position);
        let turn = rotation.unwrap_or(entity.rotation_y_deg);
        edit.world
            .place_entity(id, to, turn)
            .map_err(|e| bad(e.to_string()))?;
    }
    if let Some(s) = scale_of(&p)? {
        edit.world
            .set_scale(id, s)
            .map_err(|e| bad(e.to_string()))?;
    }
    let entity = entity_mut(edit, id)?;
    if let Some(name) = name {
        entity.name = name.to_string();
    }
    entity.validate().map_err(|e| bad(e.to_string()))?;
    let position = entity.position;
    if let Some(layer) = layer {
        for e in std::iter::once(id).chain(edit.world.descendants(id)) {
            let entity = entity_mut(edit, e)?;
            entity.layer.clone_from(&layer);
            entity.validate().map_err(|e| bad(e.to_string()))?;
        }
    }
    Ok(json!({"id": id.0, "position": position}))
}

fn remove_spec() -> CommandSpec {
    spec(
        "entity.remove",
        "Remove Entity",
        "Removes an entity, and the entities that belong to it (a prefab's children); their \
         ids are never handed out again.",
        vec![param("id", T::Entity, ID_DOC).required()],
        "null",
        json!({"id": 1}),
    )
}

fn remove(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = p.required_entity("id", &edit.world)?;
    edit.world.remove_entity(id);
    Ok(Value::Null)
}

/// Most absent ids `entity.remove_many` names in its error; the rest are counted.
const MAX_NAMED_ABSENT: usize = 10;

fn remove_many_spec() -> CommandSpec {
    spec(
        "entity.remove_many",
        "Remove Entities",
        "Removes every entity of `ids`, and the entities that belong to them (a prefab's \
         children); an id or a name that no entity has is an error naming it, and nothing \
         goes. One undo step, checked once, whatever the number: for a layer, a selection, a \
         generated set, where entity.remove one by one walks the world once per entity.",
        vec![
            param(
                "ids",
                T::Array,
                "[entity, ...]: each an entity's id as a number (a whole number from 1), or its \
                 name as a string; 1 to 1000000 of them.",
            )
            .required(),
        ],
        "{removed}",
        json!({"ids": [1, "Oak"]}),
    )
}

fn remove_many(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let items = p
        .value("ids")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`ids` must be a list of entities: [3, 4, \"Gate\"]"))?;
    if items.is_empty() || items.len() > MAX_REMOVED {
        return Err(bad(format!(
            "`ids` holds {} entities; give 1 to {MAX_REMOVED}",
            items.len()
        )));
    }
    let mut ids = Vec::with_capacity(items.len());
    let mut absent: Vec<String> = Vec::new();
    for item in items {
        match item {
            // An id, like a name: it must be an entity's.
            Value::Number(n) => match n.as_u64().filter(|id| *id >= 1) {
                Some(id) => {
                    if edit.world.entity(EntityId(id)).is_some() {
                        ids.push(EntityId(id));
                    } else {
                        absent.push(id.to_string());
                    }
                }
                None => {
                    return Err(bad(format!(
                        "`ids` must be a list of entities: each an id (a whole number from 1) or \
                         a name (a string), got {}",
                        kind_of(item)
                    )));
                }
            },
            Value::String(name) => {
                ids.push(
                    edit.world
                        .find(name)
                        .map_err(|e| bad(format!("`ids`: {e}")))?,
                );
            }
            other => {
                let kind = match other {
                    Value::Null => "null",
                    Value::Bool(_) => "a boolean",
                    Value::Array(_) => "a list",
                    Value::Object(_) => "an object",
                    _ => "something else",
                };
                return Err(bad(format!(
                    "`ids` must be a list of entities: each an id (a number) or a name (a \
                     string), got {kind}"
                )));
            }
        }
    }
    if !absent.is_empty() {
        let (named, more) = absent.split_at(absent.len().min(MAX_NAMED_ABSENT));
        let list = named.join(", ");
        let more = (!more.is_empty()).then(|| format!(" and {} more", more.len()));
        return Err(bad(format!(
            "`ids`: no entity has the {}{list}{}; every id must be one of the world's (entity.get, \
             world.inspect list them)",
            if absent.len() == 1 { "id " } else { "ids " },
            more.unwrap_or_default()
        )));
    }
    let removed = edit.world.remove_entities(&ids);
    Ok(json!({"removed": removed}))
}

fn set_component_spec() -> CommandSpec {
    spec(
        "entity.set_component",
        "Set Component",
        "Adds a component to an entity, or replaces the one of the same type; fields left out \
         take their defaults.",
        vec![
            param("id", T::Entity, ID_DOC).required(),
            param(
                "component",
                T::Object,
                "The component as in the file format: {\"type\": \"shape\"|\"plant\"|\
                 \"character\", ...}.",
            )
            .required(),
        ],
        "null",
        json!({"id": "Crate", "component": {"type": "shape", "kind": "cylinder", "size": [1, 2, 1], "color": [0.6, 0.3, 0.1]}}),
    )
}

fn set_component(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = p.required_entity("id", &edit.world)?;
    let value = p
        .value("component")
        .ok_or_else(|| bad("missing `component`"))?;
    let c = component(value, "component")?;
    let entity = entity_mut(edit, id)?;
    entity.set_component(c);
    entity.validate().map_err(|e| bad(e.to_string()))?;
    Ok(Value::Null)
}

fn remove_component_spec() -> CommandSpec {
    spec(
        "entity.remove_component",
        "Remove Component",
        "Removes the component of one type from an entity; an error when it has none.",
        vec![
            param("id", T::Entity, ID_DOC).required(),
            param("type", T::String, "The component type.")
                .required()
                .allowed(&Component::KINDS),
        ],
        "null",
        json!({"id": "Oak", "type": "plant"}),
    )
}

fn remove_component(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = p.required_entity("id", &edit.world)?;
    let kind = p.str("type")?.ok_or_else(|| bad("missing `type`"))?;
    let entity = entity_mut(edit, id)?;
    if entity.remove_component(kind).is_none() {
        let has: Vec<&str> = entity.components.iter().map(Component::kind).collect();
        let list = if has.is_empty() {
            "none".to_string()
        } else {
            has.join(", ")
        };
        return Err(failed(format!(
            "entity {id} has no `{kind}` component; its components: {list}"
        )));
    }
    Ok(Value::Null)
}

fn snap_spec() -> CommandSpec {
    spec(
        "entity.snap_to_ground",
        "Snap to Ground",
        "Puts the base of one entity, or of every entity when `id` is left out, on the ground \
         beneath it. Entities with no ground beneath are skipped and counted.",
        vec![param(
            "id",
            T::Entity,
            "The entity; every entity when left out.",
        )],
        "{moved, skipped}: entities now standing on the ground, and entities without ground \
         beneath",
        json!({"id": "Crate"}),
    )
}

fn snap_to_ground(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let ids: Vec<EntityId> = match p.entity("id", &edit.world)? {
        Some(id) => vec![id],
        None => edit.world.entities().map(|e| e.id).collect(),
    };
    let (mut moved, mut skipped) = (0u64, 0u64);
    for id in ids {
        let Some([x, _, z]) = edit.world.entity(id).map(|e| e.position) else {
            continue;
        };
        match edit.ground_or_none(x, z) {
            Some(y) => {
                if let Some(e) = edit.world.entity_mut(id) {
                    e.position = [x, y, z];
                }
                moved += 1;
            }
            None => skipped += 1,
        }
    }
    Ok(json!({"moved": moved, "skipped": skipped}))
}
