//! `world.json`: the fields of a world other than its entities and prefabs, merged field by
//! field across packs (a JSON merge patch, RFC 7386: objects merge key by key, `null` removes
//! a key, anything else replaces).

use crate::ProjectError;
use crate::paths::is_plain_name;
use gamecraft_world::World;
use serde_json::{Map, Value};

/// The version of the `world.json` format (`gamecraft_world`).
pub const WORLD_FORMAT: u32 = 1;

/// The keys a `world.json` may hold, in the order they are written.
pub(crate) const WORLD_KEYS: [&str; 11] = [
    "gamecraft_world",
    "name",
    "seed",
    "default_layer",
    "layers",
    "cell_size_m",
    "terrain",
    "environment",
    "vars",
    "objectives",
    "triggers",
];

/// The keys that belong to the project, not to the [`World`]: the default layer, the order
/// of the layers and the size of the cells the layers are split into (`cells.rs`).
pub(crate) const PROJECT_KEYS: [&str; 3] = ["default_layer", "layers", "cell_size_m"];

/// Reads and checks a `world.json` (`file` names it in errors) and returns its fields without
/// `gamecraft_world`.
pub(crate) fn parse_world(text: &str, file: &str) -> Result<Map<String, Value>, ProjectError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| ProjectError::in_file(file, format!("not valid JSON: {e}")))?;
    let Value::Object(mut map) = value else {
        return Err(ProjectError::in_file(file, "a world.json is an object"));
    };
    if map.get("gamecraft_world").and_then(Value::as_u64) != Some(u64::from(WORLD_FORMAT)) {
        return Err(ProjectError::in_file(
            file,
            format!("`gamecraft_world` must be {WORLD_FORMAT}, the world format this engine reads"),
        ));
    }
    map.remove("gamecraft_world");
    if let Some(key) = map.keys().find(|k| !WORLD_KEYS.contains(&k.as_str())) {
        return Err(ProjectError::in_file(
            file,
            format!(
                "unknown field `{key}`; a world.json holds {}",
                WORLD_KEYS.join(", ")
            ),
        ));
    }
    match map.get("cell_size_m") {
        None | Some(Value::Null) => {}
        Some(Value::Number(n)) => {
            let size = n.as_f64().unwrap_or(f64::NAN);
            crate::cells::check_cell_size(size, "`cell_size_m`")
                .map_err(|e| ProjectError::in_file(file, e.to_string()))?;
        }
        Some(other) => {
            return Err(ProjectError::in_file(
                file,
                format!(
                    "`cell_size_m` {other} is not a number; it is the side of a cell in metres \
                     (512 is usual), or leave it out for one file per layer"
                ),
            ));
        }
    }
    match map.get("default_layer") {
        None | Some(Value::Null) => {}
        Some(Value::String(l)) if is_plain_name(l) => {}
        Some(other) => {
            return Err(ProjectError::in_file(
                file,
                format!(
                    "`default_layer` {other} is not a layer name: 1 to 64 ASCII letters, digits, \
                     `-` and `_`"
                ),
            ));
        }
    }
    match map.get("layers") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items))
            if items.iter().all(|i| i.as_str().is_some_and(is_plain_name)) => {}
        Some(_) => {
            return Err(ProjectError::in_file(
                file,
                "`layers` is the order of the layers: a list of layer names (1 to 64 ASCII \
                 letters, digits, `-` and `_`)",
            ));
        }
    }
    Ok(map)
}

/// Applies the merge patch `patch` to `target`.
pub(crate) fn merge(target: &mut Map<String, Value>, patch: &Map<String, Value>) {
    for (key, value) in patch {
        match value {
            Value::Null => {
                target.remove(key);
            }
            Value::Object(inner) => {
                let slot = target
                    .entry(key.clone())
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Value::Object(existing) = slot {
                    merge(existing, inner);
                } else {
                    let mut fresh = Map::new();
                    merge(&mut fresh, inner);
                    *slot = Value::Object(fresh);
                }
            }
            other => {
                target.insert(key.clone(), other.clone());
            }
        }
    }
}

/// The merge patch that turns `base` into `current` (both objects): the keys whose values
/// differ, objects compared key by key, `null` for a key `current` lacks.
pub(crate) fn diff(base: &Map<String, Value>, current: &Map<String, Value>) -> Map<String, Value> {
    let mut patch = Map::new();
    for (key, value) in current {
        match (base.get(key), value) {
            (Some(b), v) if b == v => {}
            (Some(Value::Object(b)), Value::Object(c)) => {
                patch.insert(key.clone(), Value::Object(diff(b, c)));
            }
            (_, v) => {
                patch.insert(key.clone(), v.clone());
            }
        }
    }
    for key in base.keys() {
        if !current.contains_key(key) {
            patch.insert(key.clone(), Value::Null);
        }
    }
    patch
}

/// The fields of `world` that a `world.json` holds, as the `.gamecraft` format writes them,
/// without the terrain's edits (they have a file of their own).
pub(crate) fn world_fields(world: &World) -> Result<Map<String, Value>, ProjectError> {
    let to = |v: Result<Value, serde_json::Error>| {
        v.map_err(|e| ProjectError::new(format!("cannot write the world's fields: {e}")))
    };
    let mut map = Map::new();
    map.insert("name".into(), Value::String(world.name.clone()));
    map.insert("seed".into(), Value::from(world.seed));
    // The edits are saved to `terrain/edits.json`, not to world.json.
    let mut terrain = to(serde_json::to_value(&world.terrain))?;
    if let Value::Object(t) = &mut terrain {
        t.remove("edits");
    }
    map.insert("terrain".into(), terrain);
    map.insert(
        "environment".into(),
        to(serde_json::to_value(&world.environment))?,
    );
    map.insert("vars".into(), to(serde_json::to_value(&world.vars))?);
    map.insert(
        "objectives".into(),
        to(serde_json::to_value(&world.objectives))?,
    );
    map.insert(
        "triggers".into(),
        to(serde_json::to_value(&world.triggers))?,
    );
    Ok(map)
}
