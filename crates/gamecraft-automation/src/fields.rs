//! Typed access to the JSON object of a request (method params, tool arguments, one batch
//! step, a nested object of a host command), with errors that name the key. Unknown keys are
//! refused before anything runs.
//!
//! Ported from VisionCraft's `crates/visioncraft-automation/src/fields.rs` (MIT), itself from
//! 3DCraft's `crates/automation/src/fields.rs`. Adapted: only the accessors GameCraft uses,
//! and `point3`.

use serde_json::{Map, Value};

/// A value's kind, as the end of the sentence "must be ..., got ...".
pub(crate) fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// The keys of one JSON object, checked against what is allowed there.
pub(crate) struct Fields<'a> {
    owner: String,
    map: Option<&'a Map<String, Value>>,
}

impl<'a> Fields<'a> {
    /// `value` must be an object, or absent or null for an empty one, and all its keys must be
    /// in `allowed`. `noun` is what a key is called here (`parameter`, `argument`).
    pub fn new(
        owner: &str,
        noun: &str,
        value: Option<&'a Value>,
        allowed: &[&str],
    ) -> Result<Self, String> {
        let map = match value {
            None | Some(Value::Null) => None,
            Some(Value::Object(map)) => Some(map),
            Some(other) => {
                return Err(format!(
                    "{owner}: {noun}s must be a JSON object, got {}",
                    describe(other)
                ));
            }
        };
        if let Some(unknown) = map
            .into_iter()
            .flat_map(Map::keys)
            .find(|key| !allowed.contains(&key.as_str()))
        {
            let expected = if allowed.is_empty() {
                format!("it takes no {noun}s")
            } else {
                format!("its {noun}s are: {}", allowed.join(", "))
            };
            return Err(format!(
                "{owner}: unknown {noun} `{}`; {expected}",
                shorten(unknown)
            ));
        }
        Ok(Self {
            owner: owner.to_string(),
            map,
        })
    }

    /// Who owns these keys, as errors name it.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    fn wrong(&self, key: &str, expected: &str, got: &Value) -> String {
        format!(
            "{}: `{key}` must be {expected}, got {}",
            self.owner,
            describe(got)
        )
    }

    /// The error of a key that must be given.
    pub fn missing(&self, key: &str) -> String {
        format!("{}: missing `{key}`", self.owner)
    }

    /// The raw value, with JSON null counting as absent.
    pub fn get(&self, key: &str) -> Option<&'a Value> {
        self.map?.get(key).filter(|value| !value.is_null())
    }

    /// A string.
    pub fn str(&self, key: &str) -> Result<Option<&'a str>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(other) => Err(self.wrong(key, "a string", other)),
        }
    }

    /// A string that must be given (only `file.open` asks for one, and the browser has no files).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn required_str(&self, key: &str) -> Result<&'a str, String> {
        self.str(key)?.ok_or_else(|| self.missing(key))
    }

    /// `true` or `false`.
    pub fn bool(&self, key: &str) -> Result<Option<bool>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(other) => Err(self.wrong(key, "true or false", other)),
        }
    }

    /// A finite number.
    pub fn number(&self, key: &str) -> Result<Option<f64>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Number(n)) => match n.as_f64().filter(|v| v.is_finite()) {
                Some(v) => Ok(Some(v)),
                None => Err(self.wrong(key, "a finite number", &Value::Number(n.clone()))),
            },
            Some(other) => Err(self.wrong(key, "a number", other)),
        }
    }

    /// `[x, y, z]` of finite numbers.
    pub fn point3(&self, key: &str) -> Result<Option<[f64; 3]>, String> {
        let Some(value) = self.get(key) else {
            return Ok(None);
        };
        let numbers: Option<Vec<f64>> = value.as_array().and_then(|items| {
            items
                .iter()
                .map(|v| v.as_f64().filter(|f| f.is_finite()))
                .collect()
        });
        match numbers.as_deref() {
            Some(&[x, y, z]) => Ok(Some([x, y, z])),
            _ => Err(format!(
                "{}: `{key}` must be [x, y, z] in metres, three finite numbers, got {}",
                self.owner,
                shorten(&value.to_string())
            )),
        }
    }

    /// A JSON object to pass on as it is. Absent or null gives an empty object.
    pub fn object(&self, key: &str) -> Result<Value, String> {
        match self.get(key) {
            None => Ok(Value::Object(Map::new())),
            Some(value @ Value::Object(_)) => Ok(value.clone()),
            Some(other) => Err(self.wrong(key, "a JSON object", other)),
        }
    }

    /// An array.
    pub fn array(&self, key: &str) -> Result<Option<&'a [Value]>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Array(items)) => Ok(Some(items)),
            Some(other) => Err(self.wrong(key, "an array", other)),
        }
    }
}

/// Text from a request, cut to a length that is safe to repeat in an error message.
pub(crate) fn shorten(text: &str) -> String {
    const MAX_CHARS: usize = 80;
    if text.chars().count() <= MAX_CHARS {
        text.to_string()
    } else {
        let mut cut: String = text.chars().take(MAX_CHARS).collect();
        cut.push('…');
        cut
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_checked_and_named() {
        let v = json!({"a": 1, "b": "x", "c": null, "p": [1, 2, 3]});
        let f = Fields::new("demo", "argument", Some(&v), &["a", "b", "c", "p"]).unwrap();
        assert_eq!(f.number("a").unwrap(), Some(1.0));
        assert_eq!(f.str("b").unwrap(), Some("x"));
        assert_eq!(f.str("c").unwrap(), None);
        assert_eq!(f.point3("p").unwrap(), Some([1.0, 2.0, 3.0]));
        assert!(f.str("a").unwrap_err().contains("`a` must be a string"));
        assert!(f.point3("b").unwrap_err().contains("[x, y, z]"));
        assert_eq!(f.required_str("zz").unwrap_err(), "demo: missing `zz`");
        let e = Fields::new("demo", "argument", Some(&v), &["a"])
            .err()
            .unwrap();
        assert!(
            e.contains("unknown argument `b`; its arguments are: a"),
            "{e}"
        );
        let e = Fields::new("demo", "parameter", Some(&json!(1)), &[])
            .err()
            .unwrap();
        assert!(e.contains("parameters must be a JSON object"), "{e}");
        let e = Fields::new("demo", "parameter", Some(&json!({"x": 1})), &[])
            .err()
            .unwrap();
        assert!(e.contains("it takes no parameters"), "{e}");
        assert!(Fields::new("demo", "parameter", None, &[]).is_ok());
    }

    #[test]
    fn long_text_is_cut() {
        assert_eq!(shorten("short"), "short");
        assert_eq!(shorten(&"x".repeat(500)).chars().count(), 81);
    }
}
