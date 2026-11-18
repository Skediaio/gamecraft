//! Typed access to a command's parameters after the spec has checked them.
//!
//! The spec check already refused wrong JSON types, so these getters mostly convert; they
//! still never panic and refuse, naming the parameter, what the spec cannot say (a negative
//! integer where an id or a count is meant, a number too large for its type).

use crate::error::{CommandError, bad, failed};
use crate::spec::kind_of;
use gamecraft_world::{Component, EntityId, World};
use serde_json::{Map, Value};

/// The parameters of one call: an object, possibly empty. A null value is a parameter left
/// out.
#[derive(Clone, Copy, Debug)]
pub struct Params<'a> {
    map: Option<&'a Map<String, Value>>,
}

impl<'a> Params<'a> {
    /// The parameters held by `map`; `None` is no parameters.
    pub(crate) fn new(map: Option<&'a Map<String, Value>>) -> Params<'a> {
        Params { map }
    }

    /// The value of `name`, `None` when left out or null.
    pub fn value(&self, name: &str) -> Option<&'a Value> {
        self.map?.get(name).filter(|v| !v.is_null())
    }

    /// Whether `name` is given (not null).
    pub fn has(&self, name: &str) -> bool {
        self.value(name).is_some()
    }

    /// A number.
    pub fn f64(&self, name: &str) -> Result<Option<f64>, CommandError> {
        self.value(name)
            .map(|v| {
                v.as_f64()
                    .filter(|x| x.is_finite())
                    .ok_or_else(|| bad(format!("`{name}` must be a number, got {}", kind_of(v))))
            })
            .transpose()
    }

    /// A whole number from 0 to `u64::MAX`.
    pub fn u64(&self, name: &str) -> Result<Option<u64>, CommandError> {
        self.value(name)
            .map(|v| {
                v.as_u64().ok_or_else(|| {
                    bad(format!(
                        "`{name}` must be a whole number from 0 to {}, got {}",
                        u64::MAX,
                        kind_of(v)
                    ))
                })
            })
            .transpose()
    }

    /// A whole number from 0 to `u32::MAX`.
    pub fn u32(&self, name: &str) -> Result<Option<u32>, CommandError> {
        self.value(name)
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| {
                        bad(format!(
                            "`{name}` must be a whole number from 0 to {}, got {}",
                            u32::MAX,
                            kind_of(v)
                        ))
                    })
            })
            .transpose()
    }

    /// `true` or `false`.
    pub fn bool(&self, name: &str) -> Result<Option<bool>, CommandError> {
        self.value(name)
            .map(|v| {
                v.as_bool()
                    .ok_or_else(|| bad(format!("`{name}` must be a boolean, got {}", kind_of(v))))
            })
            .transpose()
    }

    /// A string.
    pub fn str(&self, name: &str) -> Result<Option<&'a str>, CommandError> {
        self.value(name)
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| bad(format!("`{name}` must be a string, got {}", kind_of(v))))
            })
            .transpose()
    }

    /// `N` finite numbers in an array.
    fn numbers<const N: usize>(
        &self,
        name: &str,
        shape: &str,
    ) -> Result<Option<[f64; N]>, CommandError> {
        let Some(v) = self.value(name) else {
            return Ok(None);
        };
        let wrong = || bad(format!("`{name}` must be {shape}, got {}", kind_of(v)));
        let items = v.as_array().ok_or_else(wrong)?;
        if items.len() != N {
            return Err(wrong());
        }
        let mut out = [0.0; N];
        for (slot, item) in out.iter_mut().zip(items) {
            *slot = item.as_f64().filter(|x| x.is_finite()).ok_or_else(wrong)?;
        }
        Ok(Some(out))
    }

    /// `[x, z]`.
    pub fn point2(&self, name: &str) -> Result<Option<[f64; 2]>, CommandError> {
        self.numbers::<2>(name, "[x, z]")
    }

    /// `[x, y, z]`.
    pub fn point3(&self, name: &str) -> Result<Option<[f64; 3]>, CommandError> {
        self.numbers::<3>(name, "[x, y, z]")
    }

    /// An entity of `world`: a number is its id, a string its name (or id) as
    /// [`World::find`] matches it. Not found and ambiguous names are
    /// [`CommandError::Failed`] with the world's sentence.
    pub fn entity(&self, name: &str, world: &World) -> Result<Option<EntityId>, CommandError> {
        let Some(v) = self.value(name) else {
            return Ok(None);
        };
        match v {
            Value::Number(n) => {
                let id = n.as_u64().ok_or_else(|| {
                    bad(format!(
                        "`{name}` must be an entity id (a whole number from 1) or a name, got {}",
                        kind_of(v)
                    ))
                })?;
                if world.entity(EntityId(id)).is_some() {
                    return Ok(Some(EntityId(id)));
                }
                // The world's own sentence, with candidates; a name that happens to read like
                // the number does not count, since a number is an id.
                match world.find(&id.to_string()) {
                    Err(e) => Err(failed(e)),
                    Ok(_) => Err(failed(format!(
                        "no entity has the id {id}; refer to an entity named \"{id}\" with the \
                         string \"{id}\""
                    ))),
                }
            }
            Value::String(s) => world.find(s).map(Some).map_err(failed),
            other => Err(bad(format!(
                "`{name}` must be an entity id or name, got {}",
                kind_of(other)
            ))),
        }
    }

    /// An entity that must be given.
    pub fn required_entity(&self, name: &str, world: &World) -> Result<EntityId, CommandError> {
        self.entity(name, world)?
            .ok_or_else(|| bad(format!("missing `{name}` (id or name)")))
    }
}

/// Reads a component in the file format (`{"type": "shape", ...}`); `field` names it in
/// errors (`components[0]`). Fields left out take their defaults. An unknown type is refused
/// with the nearest type, a wrong field with the component type and the field; a plant's
/// species must be in the plant library.
pub(crate) fn component(v: &Value, field: &str) -> Result<Component, CommandError> {
    let kinds = Component::KINDS.join(", ");
    let Some(object) = v.as_object() else {
        return Err(bad(format!(
            "`{field}` must be an object {{\"type\": {kinds}, ...}}, got {}",
            kind_of(v)
        )));
    };
    let kind = match object.get("type") {
        Some(Value::String(s)) => s.as_str(),
        Some(other) => {
            return Err(bad(format!(
                "`{field}.type` must be one of {kinds}, got {}",
                kind_of(other)
            )));
        }
        None => {
            return Err(bad(format!("`{field}` needs a `type`, one of {kinds}")));
        }
    };
    let Some(&kind) = Component::KINDS.iter().find(|k| **k == kind) else {
        let hint = crate::spec::nearest(kind, Component::KINDS.into_iter())
            .map(|n| format!("; did you mean `{n}`?"))
            .unwrap_or_default();
        return Err(bad(format!(
            "`{field}.type` is {:?}, which is not a component type; the types are {kinds}{hint}",
            crate::spec::excerpt(kind)
        )));
    };
    let parsed: Result<Component, _> = serde_json::from_value(v.clone());
    let component = match parsed {
        Ok(c) => c,
        Err(e) => return Err(bad(wrong_field(object, kind, field, &e.to_string()))),
    };
    if let Component::Plant(plant) = &component
        && !crate::plants::species_exists(&plant.species)
    {
        let hint = crate::plants::nearest_species(&plant.species)
            .map(|n| format!("; the nearest is \"{n}\""))
            .unwrap_or_default();
        return Err(bad(format!(
            "`{field}.species` is {:?}, which is not in the plant library{hint}",
            crate::spec::excerpt(&plant.species)
        )));
    }
    Ok(component)
}

/// The sentence for a component that did not parse: finds the field that is wrong by trying
/// each given field alone on a minimal valid component of the same type.
fn wrong_field(object: &Map<String, Value>, kind: &str, field: &str, fallback: &str) -> String {
    let base = || -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("type".into(), Value::from(kind));
        match kind {
            "shape" => {
                m.insert("kind".into(), Value::from("box"));
            }
            "plant" => {
                m.insert("species".into(), Value::from("x"));
            }
            _ => {}
        }
        m
    };
    for (key, value) in object {
        if key == "type" {
            continue;
        }
        let mut probe = base();
        probe.insert(key.clone(), value.clone());
        if let Err(e) = serde_json::from_value::<Component>(Value::Object(probe)) {
            let why = e.to_string();
            let key_text = crate::spec::excerpt(key);
            return if why.starts_with("unknown field") {
                format!("`{field}` ({kind}): {why}")
            } else {
                format!("`{field}` ({kind}): field `{key_text}`: {why}")
            };
        }
    }
    format!("`{field}` ({kind}): {fallback}")
}
