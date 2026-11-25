//! Layer files (`worlds/<world>/layers/<layer>.json`): a list of entities with string ids, one
//! per line, and markers that remove an entity of an earlier pack.

use crate::ProjectError;
use crate::paths::{MAX_ENTITY_KEY_CHARS, is_entity_key};
use gamecraft_world::{Component, Entity, Scale, World};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The version of the layer file format (`gamecraft_layer`).
pub const LAYER_FORMAT: u32 = 1;

fn default_scale() -> Scale {
    Scale::ONE
}

/// An entity as a layer file holds it: like an entity of the `.gamecraft` format, with a
/// string id unique within its pack and its parent by string id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LayerEntity {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub position: [f64; 3],
    #[serde(default)]
    pub rotation_y_deg: f64,
    #[serde(default = "default_scale")]
    pub scale: Scale,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<Component>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<Vec<Component>>,
}

impl LayerEntity {
    /// The file form of `e` in `world` under `key`, its parent named by `parent`: an instance
    /// of a prefab keeps its prefab and its overrides instead of its components.
    pub(crate) fn of(world: &World, e: &Entity, key: &str, parent: Option<String>) -> LayerEntity {
        let (components, overrides) = match e.prefab.as_deref().and_then(|p| world.prefab(p)) {
            Some(prefab) => {
                let overrides = prefab.overrides_of(&e.components);
                (None, (!overrides.is_empty()).then_some(overrides))
            }
            None => (Some(e.components.clone()), None),
        };
        LayerEntity {
            id: key.to_string(),
            name: e.name.clone(),
            position: e.position,
            rotation_y_deg: e.rotation_y_deg,
            scale: e.scale,
            prefab: e.prefab.clone(),
            parent,
            components,
            overrides,
        }
    }

    /// The entity in the `.gamecraft` format, under the runtime id `id`.
    pub(crate) fn to_world_json(&self, id: u64, parent: Option<u64>, layer: &str) -> Value {
        let mut v = json!({
            "id": id,
            "name": self.name,
            "position": self.position,
            "rotation_y_deg": self.rotation_y_deg,
            "scale": self.scale,
            "layer": layer,
        });
        if let Some(map) = v.as_object_mut() {
            if let Some(p) = &self.prefab {
                map.insert("prefab".into(), json!(p));
            }
            if let Some(p) = parent {
                map.insert("parent".into(), json!(p));
            }
            if let Some(c) = &self.components {
                map.insert("components".into(), json!(c));
            }
            if let Some(o) = &self.overrides {
                map.insert("overrides".into(), json!(o));
            }
        }
        v
    }
}

/// One line of a layer file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Entry {
    /// An entity, new or replacing the entity of an earlier pack with the same id.
    Entity(LayerEntity),
    /// `{"id": "...", "removed": true}`: removes the entity of an earlier pack with this id.
    Removed(String),
}

impl Entry {
    /// The string id.
    pub(crate) fn key(&self) -> &str {
        match self {
            Entry::Entity(e) => &e.id,
            Entry::Removed(k) => k,
        }
    }

    /// The line, compact JSON.
    fn to_line(&self) -> Result<String, ProjectError> {
        let text = match self {
            Entry::Entity(e) => serde_json::to_string(e),
            Entry::Removed(k) => serde_json::to_string(&json!({"id": k, "removed": true})),
        };
        text.map_err(|e| ProjectError::new(format!("cannot write JSON: {e}")))
    }
}

/// A layer file.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Layer {
    /// `"replace": true`: the layer of the same name of earlier packs is dropped first.
    pub replace: bool,
    /// The lines, in file order.
    pub entries: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerFile {
    gamecraft_layer: Value,
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    entities: Vec<Value>,
}

impl Layer {
    /// Reads and checks a layer file (`file` names it in errors).
    pub(crate) fn parse(text: &str, file: &str) -> Result<Layer, ProjectError> {
        let raw: LayerFile = serde_json::from_str(text).map_err(|e| {
            ProjectError::in_file(
                file,
                format!(
                    "not a valid layer file: {e}; a layer file is {{\"gamecraft_layer\": 1, \
                     \"entities\": [...]}}"
                ),
            )
        })?;
        if raw.gamecraft_layer.as_u64() != Some(u64::from(LAYER_FORMAT)) {
            return Err(ProjectError::in_file(
                file,
                format!(
                    "`gamecraft_layer` must be {LAYER_FORMAT}, the layer format this engine reads"
                ),
            ));
        }
        let mut entries = Vec::with_capacity(raw.entities.len());
        for (i, value) in raw.entities.into_iter().enumerate() {
            entries.push(entry(value, i, file)?);
        }
        Ok(Layer {
            replace: raw.replace,
            entries,
        })
    }

    /// The file's text: one entity per line, so a diff shows exactly what changed.
    pub(crate) fn to_text(&self) -> Result<String, ProjectError> {
        let mut out = format!("{{\n  \"gamecraft_layer\": {LAYER_FORMAT},\n");
        if self.replace {
            out.push_str("  \"replace\": true,\n");
        }
        if self.entries.is_empty() {
            out.push_str("  \"entities\": []\n}\n");
            return Ok(out);
        }
        out.push_str("  \"entities\": [\n");
        for (i, e) in self.entries.iter().enumerate() {
            out.push_str("    ");
            out.push_str(&e.to_line()?);
            if i + 1 < self.entries.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  ]\n}\n");
        Ok(out)
    }
}

/// The entry at `entities[i]` of `file`.
fn entry(value: Value, i: usize, file: &str) -> Result<Entry, ProjectError> {
    let at = |message: String| ProjectError::in_file(file, format!("entities[{i}]: {message}"));
    let Some(map) = value.as_object() else {
        return Err(at("an entity is an object with a string `id`".into()));
    };
    let key = match map.get("id") {
        Some(Value::String(k)) => k.clone(),
        _ => {
            return Err(at(
                "an entity needs a string `id`, unique within its pack".into()
            ));
        }
    };
    if !is_entity_key(&key) {
        return Err(at(format!(
            "the id {key:?} is not allowed: 1 to {MAX_ENTITY_KEY_CHARS} ASCII letters, digits, \
             `-`, `_`, `.`, `/` and `:`, without `..`"
        )));
    }
    if let Some(removed) = map.get("removed") {
        if removed != &Value::Bool(true) || map.len() != 2 {
            return Err(at(format!(
                "entity `{key}`: a removal is exactly {{\"id\": \"{key}\", \"removed\": true}}"
            )));
        }
        return Ok(Entry::Removed(key));
    }
    let entity: LayerEntity =
        serde_json::from_value(value).map_err(|e| at(format!("entity `{key}`: {e}")))?;
    Ok(Entry::Entity(entity))
}
