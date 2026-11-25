//! Command documentation: what `engine.commands` and the MCP tool `command_list` give an agent.
//!
//! Ported from VisionCraft's `crates/visioncraft-automation/src/docs.rs` (MIT). One shape,
//! [`CommandDoc`], for the engine's commands and the host's own: both are
//! [`gamecraft_engine::CommandSpec`]s, mapped one to one by [`from_spec`]. Adapted: every entry
//! has a `scope` (`world`, `query`, `play` or `host`), a parameter's `type` is the one-word name
//! of its [`gamecraft_engine::ParamType`], and the params notation comes from the spec itself.

use gamecraft_engine::CommandSpec;
use serde_json::{Value, json};

/// Longest first sentence of a summary in a brief listing, in characters.
const MAX_FIRST_SENTENCE_CHARS: usize = 200;

/// One parameter of a command.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamDoc {
    /// The key in the params object.
    pub name: String,
    /// `string`, `number`, `integer`, `boolean`, `point2`, `point3`, `color`, `entity`,
    /// `object`, `array` or `any`.
    pub ty: String,
    /// `m`, `deg`, `s`, ...
    pub unit: Option<String>,
    /// Whether it must be given.
    pub required: bool,
    /// The default as JSON text (`0.5`, `true`, `"noise"`), when it is a constant.
    pub default: Option<String>,
    /// The only values accepted, when it is an enumeration (strings as they are, other values
    /// as JSON text).
    pub allowed: Vec<String>,
    /// One or two sentences.
    pub doc: String,
}

/// The documentation of one command.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandDoc {
    /// The command id, `area.verb`.
    pub id: String,
    /// A short name, as a menu would show it.
    pub label: String,
    /// One or a few sentences; the first one stands alone.
    pub summary: String,
    /// The parameters, in the order they are documented.
    pub params: Vec<ParamDoc>,
    /// The params in 3DCraft's notation: `{"name": type (unit) = default, ...}`.
    pub notation: String,
    /// The params may hold keys that are not listed.
    pub open: bool,
    /// The result's shape; `None` when there is none.
    pub result: Option<String>,
    /// A params object that runs.
    pub example: Value,
    /// Whether a successful run that changes the document is one undo step.
    pub undoable: bool,
    /// `world`, `query`, `play` (the engine's scopes) or `host`.
    pub scope: String,
}

/// A command's spec, as documentation.
pub fn from_spec(s: &CommandSpec) -> CommandDoc {
    let text = |v: &Value| v.as_str().map_or_else(|| v.to_string(), str::to_string);
    CommandDoc {
        id: s.name.clone(),
        label: s.label.clone(),
        summary: s.summary.clone(),
        params: s
            .params
            .iter()
            .map(|p| ParamDoc {
                name: p.name.clone(),
                ty: p.ty.name().to_string(),
                unit: p.unit.clone(),
                required: p.required,
                default: p.default.as_ref().map(Value::to_string),
                allowed: p.allowed.iter().flatten().map(text).collect(),
                doc: p.doc.clone(),
            })
            .collect(),
        notation: s.notation(),
        open: s.open_params,
        result: (s.result != "null").then(|| s.result.clone()),
        example: s.example.clone(),
        undoable: s.undoable,
        scope: s.scope.name().to_string(),
    }
}

impl CommandDoc {
    /// The entry `engine.commands` gives: brief or full. `enabled` says whether the command
    /// can run now, and why not.
    pub fn to_json(&self, brief: bool, enabled: Result<(), String>) -> Value {
        if brief {
            return json!({
                "id": self.id,
                "params": self.notation,
                "summary": first_sentence(&self.summary),
                "enabled": enabled.is_ok(),
            });
        }
        let parameters: Vec<Value> = self
            .params
            .iter()
            .map(|p| {
                json!({
                    "name": p.name, "type": p.ty, "unit": p.unit, "required": p.required,
                    "default": p.default, "allowed": p.allowed, "doc": p.doc,
                })
            })
            .collect();
        json!({
            "id": self.id,
            "label": self.label,
            "params": self.notation,
            "summary": self.summary,
            "parameters": parameters,
            "openParams": self.open,
            "result": self.result,
            "example": self.example,
            "undoable": self.undoable,
            "scope": self.scope,
            "enabled": enabled.is_ok(),
            "disabledReason": enabled.err(),
        })
    }
}

/// The first sentence of `summary`, at most 200 characters (3DCraft's `first_sentence`,
/// simplified): it ends at the first full stop outside brackets that ends the text or is
/// followed by a space.
pub fn first_sentence(summary: &str) -> String {
    let mut depth = 0i32;
    let mut out = String::new();
    let mut chars = summary.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            '.' if depth == 0 && chars.peek().is_none_or(|n| *n == ' ') => break,
            _ => {}
        }
    }
    if out.chars().count() <= MAX_FIRST_SENTENCE_CHARS {
        out
    } else {
        let cut: String = out.chars().take(MAX_FIRST_SENTENCE_CHARS - 1).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamecraft_engine::{Session, example_world};

    #[test]
    fn an_engine_spec_maps_one_to_one() {
        let s = Session::new(example_world());
        let d = from_spec(s.spec("entity.add").unwrap());
        assert_eq!(d.id, "entity.add");
        assert_eq!(d.scope, "world");
        assert!(d.undoable);
        let position = d.params.iter().find(|p| p.name == "position").unwrap();
        assert_eq!(
            (position.ty.as_str(), position.unit.as_deref()),
            ("point3", Some("m"))
        );
        assert_eq!(d.notation, s.spec("entity.add").unwrap().notation());
        assert!(d.example.is_object());
        let terrain = from_spec(s.spec("terrain.set").unwrap());
        let kind = terrain.params.iter().find(|p| p.name == "kind").unwrap();
        assert_eq!(kind.allowed, ["flat", "noise", "grid", "heightmap"]);
        assert_eq!(kind.default.as_deref(), Some("\"noise\""));
        let inspect = from_spec(s.spec("world.inspect").unwrap());
        assert_eq!((inspect.scope.as_str(), inspect.undoable), ("query", false));
        let none = from_spec(s.spec("terrain.clear").unwrap());
        assert_eq!(none.result, None);
    }

    #[test]
    fn the_json_entry_brief_and_full() {
        let s = Session::new(example_world());
        let d = from_spec(s.spec("sim.step").unwrap());
        let brief = d.to_json(true, Ok(()));
        assert_eq!(brief["id"], "sim.step");
        assert_eq!(brief["enabled"], true);
        assert!(brief.get("parameters").is_none());
        let full = d.to_json(false, Err("no simulation is running".into()));
        assert_eq!(full["scope"], "play");
        assert_eq!(full["enabled"], false);
        assert_eq!(full["disabledReason"], "no simulation is running");
        assert_eq!(full["parameters"][0]["name"], "ticks");
        assert_eq!(full["parameters"][0]["type"], "integer");
    }

    #[test]
    fn the_first_sentence() {
        assert_eq!(first_sentence("One. Two."), "One.");
        assert_eq!(first_sentence("A (b. c) d. E."), "A (b. c) d.");
        assert_eq!(first_sentence("No full stop"), "No full stop");
        assert_eq!(first_sentence("1.5 m. Then"), "1.5 m.");
        assert_eq!(first_sentence(&"x".repeat(500)).chars().count(), 200);
    }
}
