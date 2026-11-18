//! What every command says about itself: its parameters with types, units, defaults and allowed
//! values, its result, a runnable example and its scope. The command list for people, the CLI,
//! the JSON-lines server and the MCP server is built from these; a command is checked against
//! its spec before its handler runs.
//!
//! Ported from VisionCraft (MIT), `crates/visioncraft-core/src/spec.rs`, and adapted: the
//! parameter types are GameCraft's (`Entity` instead of VisionCraft's places and outlines),
//! every spec has a [`Scope`], `open` is called `open_params`, the messages leave the command
//! id out (the session puts it in front), the edit distance needs no indexing, and
//! [`CommandSpec::notation`] comes from VisionCraft's `visioncraft-automation/src/docs.rs`.

use serde_json::{Map, Value};

/// The kind of value a parameter takes, as JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamType {
    /// A JSON string.
    String,
    /// Any JSON number.
    Number,
    /// A whole JSON number (no fraction part).
    Integer,
    /// `true` or `false`.
    Boolean,
    /// `[x, z]` in metres on the ground plane.
    Point2,
    /// `[x, y, z]` in metres, Y up.
    Point3,
    /// `[r, g, b]`, linear, 0 to 1.
    Color,
    /// An entity: its id as a number, or its name as a string (matched without regard to case).
    Entity,
    /// A JSON object.
    Object,
    /// A JSON array.
    Array,
    /// Any JSON value.
    Any,
}

impl ParamType {
    /// The type as written in documentation and in the `params` notation.
    pub fn describe(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Point2 => "[x, z]",
            Self::Point3 => "[x, y, z]",
            Self::Color => "[r, g, b]",
            Self::Entity => "id or name",
            Self::Object => "object",
            Self::Array => "array",
            Self::Any => "any",
        }
    }

    /// The type as one lowercase word, for structured listings (`point3`, `entity`).
    pub fn name(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Point2 => "point2",
            Self::Point3 => "point3",
            Self::Color => "color",
            Self::Entity => "entity",
            Self::Object => "object",
            Self::Array => "array",
            Self::Any => "any",
        }
    }

    /// Whether `v` has this type (null is the caller's business).
    pub fn accepts(self, v: &Value) -> bool {
        let numbers = |v: &Value, n: usize| {
            v.as_array()
                .is_some_and(|a| a.len() == n && a.iter().all(|x| x.as_f64().is_some()))
        };
        match self {
            Self::String => v.is_string(),
            Self::Number => v.is_number(),
            Self::Integer => v.is_i64() || v.is_u64(),
            Self::Boolean => v.is_boolean(),
            Self::Point2 => numbers(v, 2),
            Self::Point3 => numbers(v, 3),
            Self::Color => numbers(v, 3),
            Self::Entity => v.is_u64() || v.is_string(),
            Self::Object => v.is_object(),
            Self::Array => v.is_array(),
            Self::Any => true,
        }
    }
}

/// One parameter of a command.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSpec {
    /// The key in the params object.
    pub name: String,
    /// The JSON type it takes.
    pub ty: ParamType,
    /// `m`, `deg`, `h`, `mps`, `s`, ...
    pub unit: Option<String>,
    /// Whether it must be given.
    pub required: bool,
    /// The value used when the parameter is left out, when that is a plain constant.
    pub default: Option<Value>,
    /// The only values accepted, when it is an enumeration.
    pub allowed: Option<Vec<Value>>,
    /// One or two sentences.
    pub doc: String,
}

/// An optional parameter; chain [`ParamSpec::required`], [`ParamSpec::unit`],
/// [`ParamSpec::default`] and [`ParamSpec::allowed`].
pub fn param(name: &str, ty: ParamType, doc: &str) -> ParamSpec {
    ParamSpec {
        name: name.into(),
        ty,
        unit: None,
        required: false,
        default: None,
        allowed: None,
        doc: doc.into(),
    }
}

impl ParamSpec {
    /// The parameter must be given.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// The unit of the value.
    pub fn unit(mut self, unit: &str) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// The value used when the parameter is left out.
    pub fn default(mut self, value: Value) -> Self {
        self.default = Some(value);
        self
    }

    /// The only string values accepted.
    pub fn allowed(mut self, values: &[&str]) -> Self {
        self.allowed = Some(values.iter().map(|v| Value::from(*v)).collect());
        self
    }
}

/// What a command works on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Changes the world: the document while editing (one undo step), the running world while
    /// playing.
    World,
    /// Reads only: never changes anything.
    Query,
    /// Starts, stops, steps or feeds the simulation.
    Play,
}

impl Scope {
    /// The scope as one lowercase word (`world`, `query`, `play`).
    pub fn name(self) -> &'static str {
        match self {
            Scope::World => "world",
            Scope::Query => "query",
            Scope::Play => "play",
        }
    }
}

/// Everything a command says about itself.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandSpec {
    /// The command id, `area.verb` (`entity.add`).
    pub name: String,
    /// A short title, as a menu would show it.
    pub label: String,
    /// One or a few sentences; the first one stands alone.
    pub summary: String,
    /// The parameters, in the order they are documented.
    pub params: Vec<ParamSpec>,
    /// Keys that are not listed are passed on to the handler instead of being refused.
    pub open_params: bool,
    /// The shape of the result, `null` when there is none.
    pub result: String,
    /// A params object that runs on [`crate::example_world`].
    pub example: Value,
    /// Whether a successful run can be one undo step of the document.
    pub undoable: bool,
    /// What the command works on.
    pub scope: Scope,
}

/// A spec for a command that changes the world (scope [`Scope::World`], undoable).
pub fn spec(
    name: &str,
    label: &str,
    summary: &str,
    params: Vec<ParamSpec>,
    result: &str,
    example: Value,
) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        label: label.into(),
        summary: summary.into(),
        params,
        open_params: false,
        result: result.into(),
        example,
        undoable: true,
        scope: Scope::World,
    }
}

impl CommandSpec {
    /// Unlisted keys pass.
    pub fn open(mut self) -> Self {
        self.open_params = true;
        self
    }

    /// A query: reads only, never changes the world or its history.
    pub fn query(mut self) -> Self {
        self.scope = Scope::Query;
        self.undoable = false;
        self
    }

    /// A play command: works on the simulation, not an undo step unless marked
    /// [`CommandSpec::undoable`].
    pub fn play(mut self) -> Self {
        self.scope = Scope::Play;
        self.undoable = false;
        self
    }

    /// A successful run can be one undo step of the document.
    pub fn undoable(mut self) -> Self {
        self.undoable = true;
        self
    }

    /// The parameter called `name`.
    pub fn param(&self, name: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.name == name)
    }

    /// Checks `params` against the spec: an object (or null, which is no parameters), no
    /// unknown keys (unless open), every required key present, every given value of its type
    /// and among the allowed ones. A null value counts as left out for an optional parameter.
    /// Returns the message of the first problem, without the command id.
    pub fn check<'a>(&self, params: &'a Value) -> Result<Option<&'a Map<String, Value>>, String> {
        let given = match params {
            Value::Null => return self.check_map(None).map(|()| None),
            Value::Object(m) => m,
            other => {
                return Err(format!(
                    "the parameters must be an object, got {}",
                    kind_of(other)
                ));
            }
        };
        self.check_map(Some(given)).map(|()| Some(given))
    }

    fn check_map(&self, given: Option<&Map<String, Value>>) -> Result<(), String> {
        let empty = Map::new();
        let given = given.unwrap_or(&empty);
        if !self.open_params {
            for key in given.keys() {
                if self.param(key).is_none() {
                    return Err(self.unknown_key(key));
                }
            }
        }
        for p in &self.params {
            match given.get(&p.name) {
                None | Some(Value::Null) if p.required => {
                    return Err(format!(
                        "missing `{}` ({}); {}",
                        p.name,
                        p.ty.describe(),
                        p.doc
                    ));
                }
                Some(v) if !v.is_null() => {
                    if !p.ty.accepts(v) {
                        return Err(format!(
                            "`{}` must be {}, got {}",
                            p.name,
                            p.ty.describe(),
                            kind_of(v)
                        ));
                    }
                    if let Some(allowed) = &p.allowed
                        && !allowed.contains(v)
                    {
                        let list: Vec<String> = allowed.iter().map(Value::to_string).collect();
                        return Err(format!(
                            "`{}` must be one of {}, got {}",
                            p.name,
                            list.join(", "),
                            excerpt(&v.to_string())
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn unknown_key(&self, key: &str) -> String {
        let key_text = excerpt(key);
        if self.params.is_empty() {
            return format!("unknown parameter `{key_text}`; this command takes no parameters");
        }
        let names: Vec<&str> = self.params.iter().map(|p| p.name.as_str()).collect();
        let list = names.join(", ");
        match nearest(key, names.iter().copied()) {
            Some(near) => format!(
                "unknown parameter `{key_text}`; did you mean `{near}`? The parameters are: {list}"
            ),
            None => format!("unknown parameter `{key_text}`; the parameters are: {list}"),
        }
    }

    /// The params in 3DCraft's notation: `{"name": type (unit) = default, ...}`; an optional
    /// parameter without a constant default reads `= not set`, an enumeration lists its values
    /// as `"a"|"b"`, an open spec ends with `...`.
    pub fn notation(&self) -> String {
        let mut parts: Vec<String> = self
            .params
            .iter()
            .map(|p| {
                let ty = match &p.allowed {
                    Some(values) if !values.is_empty() => values
                        .iter()
                        .map(Value::to_string)
                        .collect::<Vec<_>>()
                        .join("|"),
                    _ => p.ty.describe().to_string(),
                };
                let unit = p
                    .unit
                    .as_ref()
                    .map(|u| format!(" ({u})"))
                    .unwrap_or_default();
                let default = match (&p.default, p.required) {
                    (Some(d), _) => format!(" = {d}"),
                    (None, false) => " = not set".into(),
                    (None, true) => String::new(),
                };
                format!("\"{}\": {ty}{unit}{default}", p.name)
            })
            .collect();
        if self.open_params {
            parts.push("...".into());
        }
        format!("{{{}}}", parts.join(", "))
    }
}

/// Longest excerpt of a given value quoted in an error, in characters.
const MAX_EXCERPT_CHARS: usize = 60;

/// `text`, cut to [`MAX_EXCERPT_CHARS`] characters with `...` when longer.
pub(crate) fn excerpt(text: &str) -> String {
    if text.chars().count() <= MAX_EXCERPT_CHARS {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(MAX_EXCERPT_CHARS).collect();
    cut.push_str("...");
    cut
}

/// A value's kind, as the end of the sentence "must be ..., got ...".
pub(crate) fn kind_of(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(_) => "a boolean".into(),
        Value::Number(n) => format!("the number {n}"),
        Value::String(s) => format!("the string {:?}", excerpt(s)),
        Value::Array(a) => format!("an array of {} elements", a.len()),
        Value::Object(_) => "an object".into(),
    }
}

/// Longest word the edit distance is computed for; longer words are no misspelling of a name.
const MAX_COMPARED_CHARS: usize = 100;

/// The candidate closest to `key`, when it is close enough to be a misspelling or an
/// abbreviation of it: within a third of the longer word by edit distance (at least 2 edits),
/// or one holding the other (`cloud` for `cloud_cover`). Among the close ones the smallest
/// edit distance wins, the first on a tie. `None` when nothing is close (`lights` is not
/// `mist`).
///
/// Adapted from VisionCraft, which looked only at the candidate with the smallest distance, so
/// an abbreviation lost to a short unrelated word.
pub fn nearest<'a>(key: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let k = key.to_lowercase();
    if k.chars().count() > MAX_COMPARED_CHARS {
        return None;
    }
    candidates
        .filter_map(|c| {
            let b = c.to_lowercase();
            let d = edit_distance(&k, &b);
            let limit = (k.chars().count().max(b.chars().count()) / 3).max(2);
            let contains = k.len() >= 3 && (b.contains(&k) || k.contains(&b));
            (d <= limit || contains).then_some((d, c))
        })
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// The Levenshtein distance between two words, in characters.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut row = Vec::with_capacity(prev.len());
        row.push(i + 1);
        for (pair, cb) in prev.windows(2).zip(&b) {
            if let [diagonal, up] = pair {
                let left = row.last().copied().unwrap_or(usize::MAX - 1);
                let cost = diagonal + usize::from(ca != *cb);
                row.push(cost.min(up + 1).min(left + 1));
            }
        }
        prev = row;
    }
    prev.last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nearest_is_a_misspelling_not_just_the_least_far_word() {
        let words = ["mist", "haze", "rain", "shafts"];
        assert_eq!(nearest("lights", words.into_iter()), None);
        assert_eq!(nearest("mits", words.into_iter()), Some("mist"));
        assert_eq!(nearest("shaft", words.into_iter()), Some("shafts"));
        assert_eq!(
            nearest("cloud", ["mist", "rain", "cloud_cover"].into_iter()),
            Some("cloud_cover")
        );
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", ""), 3);
    }

    fn demo() -> CommandSpec {
        spec(
            "demo.run",
            "Demo",
            "Demo.",
            vec![
                param("position", ParamType::Point3, "Where.")
                    .required()
                    .unit("m"),
                param("mode", ParamType::String, "How.")
                    .allowed(&["a", "b"])
                    .default(json!("a")),
                param("who", ParamType::Entity, "Which entity."),
                param("tint", ParamType::Color, "Colour."),
            ],
            "null",
            json!({}),
        )
    }

    #[test]
    fn nearest_key_and_types() {
        assert_eq!(
            nearest("lenght", ["length", "area", "kind"].into_iter()),
            Some("length")
        );
        let s = demo();
        assert!(s.check(&json!({"position": [0.0, 1.0, 2]})).is_ok());
        let e = s.check(&json!({"positon": [0, 0, 0]})).unwrap_err();
        assert!(e.contains("did you mean `position`"), "{e}");
        assert!(e.contains("position, mode, who, tint"), "{e}");
        let e = s.check(&json!({})).unwrap_err();
        assert!(e.contains("missing `position`"), "{e}");
        let e = s.check(&json!({"position": null})).unwrap_err();
        assert!(e.contains("missing `position`"), "{e}");
        let e = s.check(&json!({"position": [0.0]})).unwrap_err();
        assert!(e.contains("`position` must be [x, y, z]"), "{e}");
        let e = s
            .check(&json!({"position": [0, 0, 0], "mode": "c"}))
            .unwrap_err();
        assert!(e.contains("`mode` must be one of \"a\", \"b\""), "{e}");
        assert!(
            s.check(&json!({"position": [0, 0, 0], "mode": null}))
                .is_ok()
        );
        assert!(s.check(&json!([1])).is_err());
        assert!(s.check(&Value::Null).is_err());
    }

    #[test]
    fn type_checks() {
        use ParamType as T;
        assert!(T::Integer.accepts(&json!(3)) && !T::Integer.accepts(&json!(3.5)));
        assert!(T::Number.accepts(&json!(3.5)) && !T::Number.accepts(&json!("3")));
        assert!(T::Entity.accepts(&json!(4)) && T::Entity.accepts(&json!("Scout")));
        assert!(!T::Entity.accepts(&json!(-4)) && !T::Entity.accepts(&json!(1.5)));
        assert!(T::Point2.accepts(&json!([1, 2])) && !T::Point2.accepts(&json!([1, 2, 3])));
        assert!(T::Color.accepts(&json!([1, 0.5, 0])) && !T::Color.accepts(&json!([1, 0, 0, 1])));
        assert!(!T::Point3.accepts(&json!([1, "a", 3])));
        assert!(T::Any.accepts(&Value::Null));
        assert!(T::Object.accepts(&json!({})) && !T::Object.accepts(&json!([])));
        assert!(T::Array.accepts(&json!([])) && T::Boolean.accepts(&json!(false)));
    }

    #[test]
    fn open_specs_let_unknown_keys_pass() {
        let s = demo().open();
        assert!(s.check(&json!({"position": [0, 0, 0], "extra": 1})).is_ok());
        assert!(s.notation().ends_with(", ...}"));
    }

    #[test]
    fn a_spec_without_parameters_says_so() {
        let s = spec("demo.none", "None", "None.", vec![], "null", json!({}));
        assert!(s.check(&Value::Null).is_ok());
        let e = s.check(&json!({"x": 1})).unwrap_err();
        assert!(e.contains("takes no parameters"), "{e}");
    }

    #[test]
    fn the_documented_notation() {
        assert_eq!(
            demo().notation(),
            "{\"position\": [x, y, z] (m), \"mode\": \"a\"|\"b\" = \"a\", \
             \"who\": id or name = not set, \"tint\": [r, g, b] = not set}"
        );
    }

    #[test]
    fn scopes() {
        let s = demo();
        assert_eq!((s.scope, s.undoable), (Scope::World, true));
        let q = demo().query();
        assert_eq!((q.scope, q.undoable), (Scope::Query, false));
        let p = demo().play();
        assert_eq!((p.scope, p.undoable), (Scope::Play, false));
        assert!(demo().play().undoable().undoable);
    }

    #[test]
    fn long_keys_are_cut_in_messages() {
        let long = "k".repeat(10_000);
        let e = demo()
            .check(&json!({ long.clone(): 1, "position": [0, 0, 0] }))
            .unwrap_err();
        assert!(e.len() < 400, "{}", e.len());
    }
}
