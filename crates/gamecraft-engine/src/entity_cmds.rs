//! Entities: `entity.add`, `entity.set`, `entity.remove`, `entity.set_component`,
//! `entity.remove_component`, `entity.snap_to_ground`.

use crate::error::{CommandError, bad, failed};
use crate::params::{Params, component};
use crate::session::{Edit, Handler, Registration};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use gamecraft_world::{Component, Entity, EntityDraft, EntityId};
use serde_json::{Value, json};

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

fn position_param() -> crate::spec::ParamSpec {
    param(
        "position",
        T::Point3,
        "Where the entity's base stands, [x, y, z]; give this or `at`, not both.",
    )
    .unit("m")
}

fn at_param() -> crate::spec::ParamSpec {
    param(
        "at",
        T::Point2,
        "Puts the base on the ground at [x, z]; an error where there is no ground. Give this or \
         `position`, not both.",
    )
    .unit("m")
}

/// The base position from `position` or `at`; `None` when neither is given.
pub(crate) fn placement(edit: &Edit<'_>, p: Params<'_>) -> Result<Option<[f64; 3]>, CommandError> {
    match (p.point3("position")?, p.point2("at")?) {
        (Some(_), Some(_)) => Err(bad(
            "give `position` or `at`, not both: `at` puts the base on the ground",
        )),
        (Some(position), None) => Ok(Some(position)),
        (None, Some([x, z])) => Ok(Some([x, edit.ground(x, z)?, z])),
        (None, None) => Ok(None),
    }
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
        "Adds an entity with a name, a placement and components. Give `position` or `at`.",
        vec![
            param(
                "name",
                T::String,
                "Its name, at most 200 characters; need not be unique.",
            )
            .default(json!("")),
            position_param(),
            at_param(),
            param(
                "rotation_y_deg",
                T::Number,
                "Turn about +Y, counter-clockwise seen from above; 0 faces north (-Z).",
            )
            .unit("deg")
            .default(json!(0)),
            param("scale", T::Number, "Uniform scale, 0.001 to 1000.").default(json!(1)),
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
        "{id, position}",
        json!({
            "name": "Rock",
            "at": [10, 5],
            "components": [{"type": "shape", "kind": "sphere", "size": [1.5, 1.5, 1.5]}]
        }),
    )
}

fn add(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let position =
        placement(edit, p)?.ok_or_else(|| bad("missing `position` or `at`; give one of them"))?;
    let mut components = Vec::new();
    if let Some(list) = p.value("components").and_then(Value::as_array) {
        for (i, v) in list.iter().enumerate() {
            components.push(component(v, &format!("components[{i}]"))?);
        }
    }
    let draft = EntityDraft {
        name: p.str("name")?.unwrap_or("").to_string(),
        position,
        rotation_y_deg: p.f64("rotation_y_deg")?.unwrap_or(0.0),
        scale: p.f64("scale")?.unwrap_or(1.0),
        components,
        layer: layer_of(p)?.flatten(),
    };
    let id = edit
        .world
        .add_entity(draft)
        .map_err(|e| bad(e.to_string()))?;
    Ok(json!({"id": id.0, "position": position}))
}

fn set_spec() -> CommandSpec {
    spec(
        "entity.set",
        "Edit Entity",
        "Renames, moves, turns, scales an entity or puts it in another layer; only the fields \
         given change. The entities that belong to it (a prefab's children) move and turn with \
         it, and go to the same layer.",
        vec![
            param("id", T::Entity, ID_DOC).required(),
            param("name", T::String, "Its new name, at most 200 characters."),
            position_param(),
            at_param(),
            param(
                "rotation_y_deg",
                T::Number,
                "Turn about +Y, counter-clockwise seen from above; 0 faces north (-Z).",
            )
            .unit("deg"),
            param("scale", T::Number, "Uniform scale, 0.001 to 1000."),
            layer_param("The layer it goes to, with the entities that belong to it."),
        ],
        "{id, position}",
        json!({"id": "Crate", "at": [4, -6], "rotation_y_deg": 45}),
    )
}

fn set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = p.required_entity("id", &edit.world)?;
    let position = placement(edit, p)?;
    let name = p.str("name")?;
    let rotation = p.f64("rotation_y_deg")?;
    let scale = p.f64("scale")?;
    let layer = layer_of(p)?;
    if position.is_some() || rotation.is_some() {
        let entity = entity_mut(edit, id)?;
        let to = position.unwrap_or(entity.position);
        let turn = rotation.unwrap_or(entity.rotation_y_deg);
        edit.world
            .place_entity(id, to, turn)
            .map_err(|e| bad(e.to_string()))?;
    }
    let entity = entity_mut(edit, id)?;
    if let Some(name) = name {
        entity.name = name.to_string();
    }
    if let Some(s) = scale {
        entity.scale = s;
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
