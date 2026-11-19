//! The text of the files a project writes: pretty, in a stable order, a newline at the end,
//! one entity per line in layer files.

use crate::ProjectError;
use serde::Serialize;
use serde_json::{Map, Value};

/// `value` pretty-printed (two spaces), fields in the order of its type, with a newline.
pub(crate) fn pretty_typed<T: Serialize>(value: &T) -> Result<String, ProjectError> {
    serde_json::to_string_pretty(value)
        .map(|t| t + "\n")
        .map_err(|e| ProjectError::new(format!("cannot write JSON: {e}")))
}

/// `value` with the keys of every object in sorted order, whatever order the map keeps.
pub(crate) fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                if let Some(v) = map.get(k) {
                    out.insert(k.clone(), sorted(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// An object written with its top-level keys in the order of `order` (keys not listed after
/// them, sorted) and every nested object's keys sorted; pretty, with a newline.
pub(crate) fn pretty_ordered(
    map: &Map<String, Value>,
    order: &[&str],
) -> Result<String, ProjectError> {
    let mut keys: Vec<&str> = order
        .iter()
        .copied()
        .filter(|k| map.contains_key(*k))
        .collect();
    let mut rest: Vec<&str> = map
        .keys()
        .map(String::as_str)
        .filter(|k| !order.contains(k))
        .collect();
    rest.sort_unstable();
    keys.extend(rest);
    if keys.is_empty() {
        return Ok("{}\n".to_string());
    }
    let mut out = String::from("{\n");
    for (i, key) in keys.iter().enumerate() {
        let Some(value) = map.get(*key) else {
            continue;
        };
        let name = serde_json::to_string(key)
            .map_err(|e| ProjectError::new(format!("cannot write JSON: {e}")))?;
        let body = serde_json::to_string_pretty(&sorted(value))
            .map_err(|e| ProjectError::new(format!("cannot write JSON: {e}")))?;
        out.push_str("  ");
        out.push_str(&name);
        out.push_str(": ");
        out.push_str(&body.replace('\n', "\n  "));
        if i + 1 < keys.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("}\n");
    Ok(out)
}
