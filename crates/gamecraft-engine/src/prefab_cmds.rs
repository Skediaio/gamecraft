//! Prefabs: `prefab.add`, `prefab.set`, `prefab.remove`, `prefab.spawn`, `prefab.list`
//! (`docs/design/journal.md`, *Prefabs*).
//!
//! A prefab is an entity described once (components and children) and placed many times;
//! each instance takes the prefab's components with its overrides replacing those of the same
//! type. `prefab.spawn` is a World command, so it works on a running world too: the game
//! master's "drop a squad here".

use crate::entity_cmds::placement;
use crate::error::{CommandError, bad, failed};
use crate::params::{Params, component};
use crate::session::{Edit, Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, kind_of, nearest, param, spec};
use gamecraft_world::{Component, Prefab, PrefabChild, Spawn};
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
        spec: spawn_spec,
        handler: Handler::World(spawn),
    },
    Registration {
        spec: list_spec,
        handler: Handler::Query(list),
    },
];

/// The keys of one child in `children`.
const CHILD_KEYS: [&str; 4] = ["name", "offset", "rotation_y_deg", "components"];

const COMPONENTS_DOC: &str = "Components as in the file format, at most one of each type \
    ({\"type\": \"shape\", ...}, {\"type\": \"character\", ...}, ...); fields left out take \
    their defaults.";

const CHILDREN_DOC: &str = "Entities placed with every instance, relative to it, at most 64: \
    [{\"name\": \"\", \"offset\": [x, y, z] (metres in the parent's frame: -Z its forward, +X \
    its right), \"rotation_y_deg\": 0, \"components\": [...]}]. Each becomes an entity of its \
    own with the instance as its `parent`: it moves with it (entity.set) and is removed with it.";

const PREFAB_DOC: &str = "{id, name, components: [...], children: [{name, offset, \
    rotation_y_deg, components}], instances: [ids], instance_count}";

/// The components of the list parameter `name` (`components[2]` in errors).
fn components(p: Params<'_>, name: &str) -> Result<Option<Vec<Component>>, CommandError> {
    let Some(list) = p.value(name).and_then(Value::as_array) else {
        return Ok(None);
    };
    list.iter()
        .enumerate()
        .map(|(i, v)| component(v, &format!("{name}[{i}]")))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// A number of a child, `children[i].<key>`.
fn number(v: &Value, at: &str) -> Result<f64, CommandError> {
    v.as_f64()
        .ok_or_else(|| bad(format!("`{at}` must be a number, got {}", kind_of(v))))
}

/// The children of the `children` parameter.
fn children(p: Params<'_>) -> Result<Option<Vec<PrefabChild>>, CommandError> {
    let Some(list) = p.value("children").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for (i, v) in list.iter().enumerate() {
        let at = format!("children[{i}]");
        let Some(object) = v.as_object() else {
            return Err(bad(format!(
                "`{at}` must be an object {{\"offset\": [x, y, z], \"components\": [...]}}, \
                 got {}",
                kind_of(v)
            )));
        };
        if let Some(key) = object.keys().find(|k| !CHILD_KEYS.contains(&k.as_str())) {
            let hint = nearest(key, CHILD_KEYS.into_iter())
                .map(|n| format!("; did you mean `{n}`?"))
                .unwrap_or_default();
            return Err(bad(format!(
                "`{at}` has an unknown key `{}`; a child takes {}{hint}",
                crate::spec::excerpt(key),
                CHILD_KEYS.join(", ")
            )));
        }
        let name = match object.get("name") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(other) => {
                return Err(bad(format!(
                    "`{at}.name` must be a string, got {}",
                    kind_of(other)
                )));
            }
        };
        let offset = match object.get("offset") {
            None | Some(Value::Null) => [0.0; 3],
            Some(Value::Array(xyz)) if xyz.len() == 3 => {
                let mut o = [0.0; 3];
                for (slot, (axis, v)) in o.iter_mut().zip(xyz.iter().enumerate()) {
                    *slot = number(v, &format!("{at}.offset[{axis}]"))?;
                }
                o
            }
            Some(other) => {
                return Err(bad(format!(
                    "`{at}.offset` must be [x, y, z] in metres, got {}",
                    kind_of(other)
                )));
            }
        };
        let rotation_y_deg = match object.get("rotation_y_deg") {
            None | Some(Value::Null) => 0.0,
            Some(v) => number(v, &format!("{at}.rotation_y_deg"))?,
        };
        let components = match object.get("components") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(list)) => list
                .iter()
                .enumerate()
                .map(|(j, c)| component(c, &format!("{at}.components[{j}]")))
                .collect::<Result<Vec<_>, _>>()?,
            Some(other) => {
                return Err(bad(format!(
                    "`{at}.components` must be an array of components, got {}",
                    kind_of(other)
                )));
            }
        };
        out.push(PrefabChild {
            name,
            offset,
            rotation_y_deg,
            components,
        });
    }
    Ok(Some(out))
}

/// The required `id` of a prefab.
fn prefab_id<'a>(p: Params<'a>, key: &str) -> Result<&'a str, CommandError> {
    p.str(key)?.ok_or_else(|| bad(format!("missing `{key}`")))
}

fn add_spec() -> CommandSpec {
    spec(
        "prefab.add",
        "Add Prefab",
        "Describes an entity once, to place many times with prefab.spawn: components, and \
         children placed relative to each instance. The id must be new; prefab.set changes a \
         prefab.",
        vec![
            param(
                "id",
                T::String,
                "Its id, unique among the prefabs, at most 200 characters.",
            )
            .required(),
            param(
                "name",
                T::String,
                "The name an instance gets when spawned without one.",
            )
            .default(json!("")),
            param("components", T::Array, COMPONENTS_DOC).default(json!([])),
            param("children", T::Array, CHILDREN_DOC).default(json!([])),
        ],
        "{id}",
        json!({
            "id": "lamp-post",
            "name": "Lamp post",
            "components": [{"type": "tags", "tags": ["lamp"]}],
            "children": [
                {"name": "Post", "components": [{"type": "shape", "kind": "cylinder", "size": [0.2, 3, 0.2]}]},
                {"name": "Lamp", "offset": [0, 3, 0], "components": [{"type": "light", "intensity_cd": 300, "range_m": 15}]}
            ]
        }),
    )
}

fn add(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = prefab_id(p, "id")?;
    if edit.world.prefab(id).is_some() {
        return Err(failed(format!(
            "a prefab with the id \"{id}\" exists; change it with prefab.set, or give another id"
        )));
    }
    let prefab = Prefab {
        id: id.to_string(),
        name: p.str("name")?.unwrap_or("").to_string(),
        components: components(p, "components")?.unwrap_or_default(),
        children: children(p)?.unwrap_or_default(),
    };
    edit.world
        .put_prefab(prefab)
        .map_err(|e| bad(e.to_string()))?;
    Ok(json!({"id": id}))
}

fn set_spec() -> CommandSpec {
    spec(
        "prefab.set",
        "Edit Prefab",
        "Changes a prefab; only the fields given change, each replaced as a whole. Its \
         instances change with it: each keeps its overrides and takes the rest from the new \
         prefab. Children already placed stay as they are; new spawns get the new children.",
        vec![
            param("id", T::String, "The prefab's id.").required(),
            param("name", T::String, "Its new name."),
            param("components", T::Array, COMPONENTS_DOC),
            param("children", T::Array, CHILDREN_DOC),
        ],
        "{id, instances: [ids]}: the entities that changed with it",
        json!({"id": "marker", "components": [{"type": "shape", "kind": "cylinder", "size": [0.5, 2, 0.5], "color": [0.9, 0.2, 0.1]}]}),
    )
}

fn set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = prefab_id(p, "id")?;
    let mut prefab = edit
        .world
        .prefab(id)
        .cloned()
        .ok_or_else(|| unknown(edit, id))?;
    if let Some(name) = p.str("name")? {
        prefab.name = name.to_string();
    }
    if let Some(list) = components(p, "components")? {
        prefab.components = list;
    }
    if let Some(list) = children(p)? {
        prefab.children = list;
    }
    edit.world
        .put_prefab(prefab)
        .map_err(|e| bad(e.to_string()))?;
    let instances: Vec<u64> = edit
        .world
        .prefab_instances(id)
        .iter()
        .map(|i| i.0)
        .collect();
    Ok(json!({"id": id, "instances": instances}))
}

/// The error for a prefab id that names none.
fn unknown(edit: &Edit<'_>, id: &str) -> CommandError {
    let ids: Vec<&str> = edit.world.prefabs.iter().map(|p| p.id.as_str()).collect();
    let hint = nearest(id, ids.iter().copied())
        .map(|n| format!("; did you mean \"{n}\"?"))
        .unwrap_or_default();
    if ids.is_empty() {
        failed(format!(
            "no prefab has the id \"{}\"; the world has no prefabs (add one with prefab.add)",
            crate::spec::excerpt(id)
        ))
    } else {
        failed(format!(
            "no prefab has the id \"{}\"; prefab.list lists them{hint}",
            crate::spec::excerpt(id)
        ))
    }
}

fn remove_spec() -> CommandSpec {
    spec(
        "prefab.remove",
        "Remove Prefab",
        "Removes a prefab. Refused while entities are placed from it (the error names them): \
         remove those first.",
        vec![param("id", T::String, "The prefab's id.").required()],
        "null",
        json!({"id": "marker"}),
    )
}

fn remove(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = prefab_id(p, "id")?;
    if edit.world.prefab(id).is_none() {
        return Err(unknown(edit, id));
    }
    edit.world.remove_prefab(id).map_err(failed)?;
    Ok(Value::Null)
}

fn spawn_spec() -> CommandSpec {
    spec(
        "prefab.spawn",
        "Spawn Prefab",
        "Places a prefab: an entity that refers to it, with the prefab's components and \
         `overrides` replacing those of the same type, and its children placed relative to it \
         (each an entity with the instance as `parent`). Give `position` or `at`. Works on a \
         running world too: what is spawned takes part from the next tick.",
        vec![
            param("prefab", T::String, "The prefab's id.").required(),
            param(
                "position",
                T::Point3,
                "Where the instance's base stands, [x, y, z]; give this or `at`, not both.",
            )
            .unit("m"),
            param(
                "at",
                T::Point2,
                "Puts the base on the ground at [x, z]; an error where there is no ground. Give \
                 this or `position`, not both.",
            )
            .unit("m"),
            param(
                "rotation_y_deg",
                T::Number,
                "Turn about +Y, counter-clockwise seen from above; 0 faces north (-Z). The \
                 children turn with it.",
            )
            .unit("deg")
            .default(json!(0)),
            param(
                "name",
                T::String,
                "The instance's name; the prefab's name when left out.",
            ),
            crate::entity_cmds::layer_param("The layer of the instance and its children."),
            param(
                "overrides",
                T::Array,
                "Components that replace the prefab's of the same type (or add a type it lacks), \
                 at most one of each type, as in the file format.",
            )
            .default(json!([])),
        ],
        "{id, children: [ids], position}",
        json!({"prefab": "marker", "at": [4, 4], "name": "Marker 1", "overrides": [{"type": "tags", "tags": ["marker", "first"]}]}),
    )
}

fn spawn(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = prefab_id(p, "prefab")?;
    if edit.world.prefab(id).is_none() {
        return Err(unknown(edit, id));
    }
    let position =
        placement(edit, p)?.ok_or_else(|| bad("missing `position` or `at`; give one of them"))?;
    let spawn = Spawn {
        name: p.str("name")?.map(str::to_string),
        position,
        rotation_y_deg: p.f64("rotation_y_deg")?.unwrap_or(0.0),
        overrides: components(p, "overrides")?.unwrap_or_default(),
        layer: crate::entity_cmds::layer_of(p)?.flatten(),
    };
    let spawned = edit
        .world
        .spawn_prefab(id, spawn)
        .map_err(|e| bad(e.to_string()))?;
    Ok(json!({
        "id": spawned.id.0,
        "children": spawned.children.iter().map(|c| c.0).collect::<Vec<_>>(),
        "position": position,
    }))
}

fn list_spec() -> CommandSpec {
    spec(
        "prefab.list",
        "List Prefabs",
        "Every prefab of the current world (the running world while playing), in the order \
         given, with the entities placed from it.",
        vec![],
        &format!("{{prefabs: [{PREFAB_DOC}]}}"),
        json!({}),
    )
    .query()
}

/// A prefab as JSON, with its instances in `world`.
pub(crate) fn prefab_json(
    world: &gamecraft_world::World,
    prefab: &Prefab,
) -> Result<Value, CommandError> {
    let instances: Vec<u64> = world
        .prefab_instances(&prefab.id)
        .iter()
        .map(|i| i.0)
        .collect();
    let Value::Object(mut out) = serde_json::to_value(prefab).map_err(failed)? else {
        return Err(failed("a prefab is not an object"));
    };
    if !out.contains_key("children") {
        out.insert("children".into(), json!([]));
    }
    out.insert("instance_count".into(), json!(instances.len()));
    out.insert("instances".into(), json!(instances));
    Ok(Value::Object(out))
}

fn list(session: &Session, _p: Params<'_>) -> Result<Value, CommandError> {
    let world = session.current_world();
    let prefabs = world
        .prefabs
        .iter()
        .map(|prefab| prefab_json(world, prefab))
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Map::new();
    out.insert("prefabs".into(), Value::Array(prefabs));
    Ok(Value::Object(out))
}
