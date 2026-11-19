//! Scenario data of a world: variables, objectives and triggers whose actions are commands.
//!
//! These are data, not code: a trigger names an event pattern ([`When`]), an optional
//! [`Condition`] over the variables and objectives and a list of command [`Step`]s, exactly as an agent sends
//! them. The world checks their shape only; entity references and commands are resolved and
//! checked by whoever runs them (the engine), when they run.

use crate::WorldError;
use crate::error::Ctx;
use crate::limits::{
    MAX_CONDITION_DEPTH, MAX_CONDITION_ITEMS, MAX_NAME_CHARS, MAX_OBJECTIVES, MAX_TEXT_CHARS,
    MAX_TRIGGER_STEPS, MAX_TRIGGERS, MAX_VARS, TRIGGER_TIME_MAX_S,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;

/// The value of a variable: a number, a boolean or a string.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum VarValue {
    /// `true` or `false`.
    Bool(bool),
    /// A finite number.
    Number(f64),
    /// A string of at most [`MAX_TEXT_CHARS`] characters.
    Text(String),
}

impl VarValue {
    /// Reads a JSON value; anything but a number, a boolean or a string is an error that says
    /// so.
    pub fn from_json(v: &Value) -> Result<VarValue, String> {
        match v {
            Value::Bool(b) => Ok(VarValue::Bool(*b)),
            Value::Number(n) => n
                .as_f64()
                .filter(|x| x.is_finite())
                .map(VarValue::Number)
                .ok_or_else(|| format!("the number {n} is not a finite number")),
            Value::String(s) => Ok(VarValue::Text(s.clone())),
            Value::Null => Err("a value must be a number, a boolean or a string, got null".into()),
            Value::Array(_) => {
                Err("a value must be a number, a boolean or a string, got an array".into())
            }
            Value::Object(_) => {
                Err("a value must be a number, a boolean or a string, got an object".into())
            }
        }
    }

    /// The value as JSON.
    pub fn to_json(&self) -> Value {
        match self {
            VarValue::Bool(b) => Value::Bool(*b),
            VarValue::Number(x) => serde_json::Number::from_f64(*x)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            VarValue::Text(s) => Value::String(s.clone()),
        }
    }

    fn validate(&self, ctx: Ctx, field: &str) -> Result<(), WorldError> {
        match self {
            VarValue::Bool(_) => Ok(()),
            VarValue::Number(x) => ctx.finite(field, *x),
            VarValue::Text(s) => ctx.text(field, s, MAX_TEXT_CHARS),
        }
    }
}

impl fmt::Display for VarValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VarValue::Bool(b) => write!(f, "{b}"),
            VarValue::Number(x) => write!(f, "{x}"),
            VarValue::Text(s) => write!(f, "{s:?}"),
        }
    }
}

impl<'de> Deserialize<'de> for VarValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        VarValue::from_json(&v).map_err(serde::de::Error::custom)
    }
}

/// Reads the `vars` object, naming the variable whose value is wrong.
pub(crate) fn deserialize_vars<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, VarValue>, D::Error> {
    let raw: BTreeMap<String, Value> = BTreeMap::deserialize(d)?;
    raw.into_iter()
        .map(|(name, v)| match VarValue::from_json(&v) {
            Ok(value) => Ok((name, value)),
            Err(why) => Err(serde::de::Error::custom(format!(
                "vars.{}: {why}",
                excerpt(&name)
            ))),
        })
        .collect()
}

/// The state of an objective.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectiveStatus {
    /// Still to do.
    #[default]
    Open,
    /// Achieved.
    Done,
    /// Can no longer be achieved.
    Failed,
}

impl ObjectiveStatus {
    /// Every status name, as in JSON.
    pub const NAMES: [&'static str; 3] = ["open", "done", "failed"];

    /// The name as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            ObjectiveStatus::Open => "open",
            ObjectiveStatus::Done => "done",
            ObjectiveStatus::Failed => "failed",
        }
    }

    /// The status called `name`, `None` for another word.
    pub fn from_name(name: &str) -> Option<ObjectiveStatus> {
        match name {
            "open" => Some(ObjectiveStatus::Open),
            "done" => Some(ObjectiveStatus::Done),
            "failed" => Some(ObjectiveStatus::Failed),
            _ => None,
        }
    }
}

/// Something the players are to achieve. Players and the game master read objectives;
/// triggers and commands change them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Objective {
    /// Unique among the world's objectives, at most 200 characters.
    pub id: String,
    /// What to do, as the players read it, at most 2000 characters.
    pub title: String,
    /// `open`, `done` or `failed`. Default `open`.
    #[serde(default)]
    pub status: ObjectiveStatus,
    /// Kept from the players (the game master still sees it). Default false.
    #[serde(default)]
    pub hidden: bool,
}

/// A reference to an entity inside a trigger: its id or its name, resolved when the trigger
/// runs. Written as a string; a whole number in a file is read as that id. In a `who` filter
/// it may also be `tag:<tag>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Value", into = "String")]
pub struct EntityRef(pub String);

/// The prefix of a `who` filter that matches by tag.
pub const TAG_PREFIX: &str = "tag:";

impl EntityRef {
    /// The tag of a `tag:<tag>` filter, `None` for an id or a name.
    pub fn tag(&self) -> Option<&str> {
        self.0.strip_prefix(TAG_PREFIX)
    }

    fn validate(&self, ctx: Ctx, field: &str) -> Result<(), WorldError> {
        ctx.id(field, &self.0)?;
        if let Some(tag) = self.tag()
            && tag.trim().is_empty()
        {
            return Err(ctx.invalid(
                field,
                format!("{:?}", self.0),
                "a tag filter names a tag after `tag:`, such as \"tag:player\"",
            ));
        }
        Ok(())
    }
}

impl TryFrom<Value> for EntityRef {
    type Error = String;

    fn try_from(v: Value) -> Result<Self, Self::Error> {
        match v {
            Value::String(s) => Ok(EntityRef(s)),
            Value::Number(n) if n.as_u64().is_some() => Ok(EntityRef(n.to_string())),
            other => Err(format!(
                "an entity reference must be an id or name (a whole number or a string), got {}",
                excerpt(&other.to_string())
            )),
        }
    }
}

impl From<EntityRef> for String {
    fn from(r: EntityRef) -> String {
        r.0
    }
}

impl fmt::Display for EntityRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The event a trigger waits for. In JSON an object whose `event` field names the kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum When {
    /// A character entered the zone of the entity `zone`.
    Enter {
        /// The entity with the `zone` component.
        zone: EntityRef,
        /// Only this character, or characters with this tag (`tag:player`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<EntityRef>,
    },
    /// A character left the zone of the entity `zone`.
    Leave {
        /// The entity with the `zone` component.
        zone: EntityRef,
        /// Only this character, or characters with this tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<EntityRef>,
    },
    /// A character interacted with an `interactable` entity.
    Interact {
        /// Only this entity (any interactable when left out).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<EntityRef>,
        /// Only this character, or characters with this tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<EntityRef>,
    },
    /// Simulated time since the start reached `at_s`.
    Time {
        /// Seconds of simulated time, 0 to 1e9.
        at_s: f64,
    },
    /// The variable `name` changed (was set to another value, created or removed).
    Var {
        /// The variable.
        name: String,
    },
    /// The simulation started.
    Start,
    /// A projectile hit something. `who` is the shooter, `target` the entity hit (a character
    /// or a shape; a hit on the terrain has none).
    Hit {
        /// Only this shooter, or shooters with this tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<EntityRef>,
        /// Only this entity hit, or entities with this tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<EntityRef>,
    },
    /// A hit used up a character's health. `who` is the shooter, `target` the character that
    /// went down.
    Down {
        /// Only this shooter, or shooters with this tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<EntityRef>,
        /// Only this character, or characters with this tag (`tag:enemy`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<EntityRef>,
    },
    /// A character heard a shot. `who` is the character that heard it, `target` the shooter.
    Heard {
        /// Only this listener, or listeners with this tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<EntityRef>,
        /// Only this shooter, or shooters with this tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<EntityRef>,
    },
}

impl When {
    /// The event name, as in JSON.
    pub fn event(&self) -> &'static str {
        match self {
            When::Enter { .. } => "enter",
            When::Leave { .. } => "leave",
            When::Interact { .. } => "interact",
            When::Time { .. } => "time",
            When::Var { .. } => "var",
            When::Start => "start",
            When::Hit { .. } => "hit",
            When::Down { .. } => "down",
            When::Heard { .. } => "heard",
        }
    }

    /// Whether events of this kind have an acting entity (`$who`).
    pub fn has_who(&self) -> bool {
        matches!(
            self,
            When::Enter { .. }
                | When::Leave { .. }
                | When::Interact { .. }
                | When::Hit { .. }
                | When::Down { .. }
                | When::Heard { .. }
        )
    }

    /// Whether events of this kind are about an entity (`$target`): the zone or the target.
    pub fn has_target(&self) -> bool {
        self.has_who()
    }

    fn validate(&self, ctx: Ctx, at: &str) -> Result<(), WorldError> {
        let field = |name: &str| format!("{at}.when.{name}");
        match self {
            When::Enter { zone, who } | When::Leave { zone, who } => {
                zone.validate(ctx, &field("zone"))?;
                if let Some(who) = who {
                    who.validate(ctx, &field("who"))?;
                }
            }
            When::Interact { target, who }
            | When::Hit { who, target }
            | When::Down { who, target }
            | When::Heard { who, target } => {
                if let Some(target) = target {
                    target.validate(ctx, &field("target"))?;
                }
                if let Some(who) = who {
                    who.validate(ctx, &field("who"))?;
                }
            }
            When::Time { at_s } => {
                ctx.range(field("at_s"), *at_s, 0.0, TRIGGER_TIME_MAX_S, " seconds")?;
            }
            When::Var { name } => ctx.id(field("name"), name)?,
            When::Start => {}
        }
        Ok(())
    }
}

/// How a condition compares a variable with a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompareOp {
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
}

impl CompareOp {
    /// Every operator, as in JSON.
    pub const SYMBOLS: [&'static str; 6] = ["==", "!=", "<", "<=", ">", ">="];

    /// The operator as in JSON.
    pub fn symbol(self) -> &'static str {
        match self {
            CompareOp::Eq => "==",
            CompareOp::Ne => "!=",
            CompareOp::Lt => "<",
            CompareOp::Le => "<=",
            CompareOp::Gt => ">",
            CompareOp::Ge => ">=",
        }
    }

    /// The operator written `symbol`.
    pub fn from_symbol(symbol: &str) -> Option<CompareOp> {
        match symbol {
            "==" => Some(CompareOp::Eq),
            "!=" => Some(CompareOp::Ne),
            "<" => Some(CompareOp::Lt),
            "<=" => Some(CompareOp::Le),
            ">" => Some(CompareOp::Gt),
            ">=" => Some(CompareOp::Ge),
            _ => None,
        }
    }
}

/// A condition over the world's variables and objectives: `{var, op, value}`,
/// `{objective, status}`, `{all: [...]}` or `{any: [...]}`, nested at most
/// [`MAX_CONDITION_DEPTH`] deep.
///
/// A comparison with a missing variable, or with a value of another type, is "not equal":
/// `!=` holds, every other operator does not. `<`, `<=`, `>` and `>=` need two numbers. An
/// objective condition holds when the objective exists and has that status. An empty `all`
/// holds, an empty `any` does not.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(try_from = "Value")]
pub enum Condition {
    /// `{var, op, value}`.
    Compare {
        /// The variable's name.
        var: String,
        /// The operator.
        op: CompareOp,
        /// What the variable is compared with.
        value: VarValue,
    },
    /// `{objective, status}`: the objective with this id has this status.
    Objective {
        /// The objective's id.
        objective: String,
        /// The status it must have.
        status: ObjectiveStatus,
    },
    /// `{all: [...]}`: every one holds.
    All(Vec<Condition>),
    /// `{any: [...]}`: at least one holds.
    Any(Vec<Condition>),
}

const CONDITION_FORMS: &str = "a condition is {\"var\", \"op\", \"value\"}, {\"objective\", \
     \"status\"}, {\"all\": [...]} or {\"any\": [...]}";

impl Condition {
    /// Whether the condition holds for `vars` and `objectives`. A condition nested deeper than
    /// [`MAX_CONDITION_DEPTH`] does not hold.
    pub fn holds(&self, vars: &BTreeMap<String, VarValue>, objectives: &[Objective]) -> bool {
        self.holds_at(vars, objectives, 1)
    }

    fn holds_at(
        &self,
        vars: &BTreeMap<String, VarValue>,
        objectives: &[Objective],
        depth: usize,
    ) -> bool {
        if depth > MAX_CONDITION_DEPTH {
            return false;
        }
        match self {
            Condition::Compare { var, op, value } => compare(vars.get(var), *op, value),
            Condition::Objective { objective, status } => objectives
                .iter()
                .any(|o| o.id == *objective && o.status == *status),
            Condition::All(items) => items
                .iter()
                .all(|c| c.holds_at(vars, objectives, depth + 1)),
            Condition::Any(items) => items
                .iter()
                .any(|c| c.holds_at(vars, objectives, depth + 1)),
        }
    }

    fn parse(v: Value, depth: usize) -> Result<Condition, String> {
        if depth > MAX_CONDITION_DEPTH {
            return Err(format!(
                "the condition is nested too deep; at most {MAX_CONDITION_DEPTH} levels of \
                 all/any"
            ));
        }
        let Value::Object(mut m) = v else {
            return Err(format!(
                "{CONDITION_FORMS}, got {}",
                excerpt(&v.to_string())
            ));
        };
        for key in ["all", "any"] {
            if let Some(items) = m.remove(key) {
                if let Some(extra) = m.keys().next() {
                    return Err(format!(
                        "unknown field `{}` beside `{key}`; {CONDITION_FORMS}",
                        excerpt(extra)
                    ));
                }
                let Value::Array(items) = items else {
                    return Err(format!("`{key}` must be an array of conditions"));
                };
                let parsed = items
                    .into_iter()
                    .map(|c| Condition::parse(c, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(if key == "all" {
                    Condition::All(parsed)
                } else {
                    Condition::Any(parsed)
                });
            }
        }
        if let Some(objective) = m.remove("objective") {
            return Condition::parse_objective(objective, m);
        }
        let (Some(var), Some(op), Some(value)) =
            (m.remove("var"), m.remove("op"), m.remove("value"))
        else {
            return Err(CONDITION_FORMS.to_string());
        };
        if let Some(extra) = m.keys().next() {
            return Err(format!(
                "unknown field `{}` in a comparison; {CONDITION_FORMS}",
                excerpt(extra)
            ));
        }
        let Value::String(var) = var else {
            return Err("`var` must be the name of a variable (a string)".into());
        };
        let op = op
            .as_str()
            .and_then(CompareOp::from_symbol)
            .ok_or_else(|| {
                format!(
                    "`op` is {}; it must be one of {}",
                    excerpt(&op.to_string()),
                    CompareOp::SYMBOLS.join(", ")
                )
            })?;
        let value = VarValue::from_json(&value).map_err(|why| format!("`value`: {why}"))?;
        Ok(Condition::Compare { var, op, value })
    }

    /// `{objective, status}`, with `objective` already taken out of `m`.
    fn parse_objective(objective: Value, mut m: Map<String, Value>) -> Result<Condition, String> {
        let Value::String(objective) = objective else {
            return Err("`objective` must be the id of an objective (a string)".into());
        };
        let Some(status) = m.remove("status") else {
            return Err(format!(
                "an objective condition needs `status`: one of {}",
                ObjectiveStatus::NAMES.join(", ")
            ));
        };
        if let Some(extra) = m.keys().next() {
            return Err(format!(
                "unknown field `{}` in an objective condition; {CONDITION_FORMS}",
                excerpt(extra)
            ));
        }
        let status = status
            .as_str()
            .and_then(ObjectiveStatus::from_name)
            .ok_or_else(|| {
                format!(
                    "`status` is {}; it must be one of {}",
                    excerpt(&status.to_string()),
                    ObjectiveStatus::NAMES.join(", ")
                )
            })?;
        Ok(Condition::Objective { objective, status })
    }

    /// The nesting depth (a comparison is 1), counted up to one beyond the limit.
    fn depth(&self) -> usize {
        self.depth_at(1)
    }

    fn depth_at(&self, depth: usize) -> usize {
        if depth > MAX_CONDITION_DEPTH {
            return depth;
        }
        match self {
            Condition::Compare { .. } | Condition::Objective { .. } => depth,
            Condition::All(items) | Condition::Any(items) => items
                .iter()
                .map(|c| c.depth_at(depth + 1))
                .max()
                .unwrap_or(depth),
        }
    }

    fn validate(&self, ctx: Ctx, field: &str) -> Result<(), WorldError> {
        let depth = self.depth();
        if depth > MAX_CONDITION_DEPTH {
            return Err(ctx.invalid(
                field,
                "nested too deep",
                format!("a condition nests all/any at most {MAX_CONDITION_DEPTH} levels deep"),
            ));
        }
        self.validate_items(ctx, field)
    }

    fn validate_items(&self, ctx: Ctx, field: &str) -> Result<(), WorldError> {
        match self {
            Condition::Compare { var, value, .. } => {
                ctx.id(format!("{field}.var"), var)?;
                value.validate(ctx, &format!("{field}.value"))
            }
            Condition::Objective { objective, .. } => {
                ctx.id(format!("{field}.objective"), objective)
            }
            Condition::All(items) | Condition::Any(items) => {
                let key = if matches!(self, Condition::All(_)) {
                    "all"
                } else {
                    "any"
                };
                if items.len() > MAX_CONDITION_ITEMS {
                    return Err(ctx.invalid(
                        format!("{field}.{key}"),
                        format!("{} conditions", items.len()),
                        format!("it holds at most {MAX_CONDITION_ITEMS} conditions"),
                    ));
                }
                for (i, c) in items.iter().enumerate() {
                    c.validate_items(ctx, &format!("{field}.{key}[{i}]"))?;
                }
                Ok(())
            }
        }
    }
}

fn compare(have: Option<&VarValue>, op: CompareOp, want: &VarValue) -> bool {
    let equal = have == Some(want);
    match op {
        CompareOp::Eq => equal,
        CompareOp::Ne => !equal,
        _ => match (have, want) {
            (Some(VarValue::Number(a)), VarValue::Number(b)) => match op {
                CompareOp::Lt => a < b,
                CompareOp::Le => a <= b,
                CompareOp::Gt => a > b,
                _ => a >= b,
            },
            _ => false,
        },
    }
}

impl TryFrom<Value> for Condition {
    type Error = String;

    fn try_from(v: Value) -> Result<Self, Self::Error> {
        Condition::parse(v, 1)
    }
}

impl Serialize for Condition {
    /// `{var, op, value}` in that order, or `{all: [...]}` / `{any: [...]}`.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Condition::Compare { var, op, value } => {
                let mut m = s.serialize_map(Some(3))?;
                m.serialize_entry("var", var)?;
                m.serialize_entry("op", op.symbol())?;
                m.serialize_entry("value", value)?;
                m.end()
            }
            Condition::Objective { objective, status } => {
                let mut m = s.serialize_map(Some(2))?;
                m.serialize_entry("objective", objective)?;
                m.serialize_entry("status", status.name())?;
                m.end()
            }
            Condition::All(items) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("all", items)?;
                m.end()
            }
            Condition::Any(items) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("any", items)?;
                m.end()
            }
        }
    }
}

/// One action of a trigger: a command exactly as an agent sends it. Inside `params` the strings
/// `$who` and `$target` stand for the entity that caused the event and the entity it was about.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// The command id (`environment.set`).
    pub command: String,
    /// Its parameters: an object, or null for none.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

fn default_true() -> bool {
    true
}

/// When something happens, and a condition holds, run commands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    /// Unique among the world's triggers, at most 200 characters.
    pub id: String,
    /// The event it waits for.
    pub when: When,
    /// A condition over the variables and objectives that must hold when the event happens.
    /// Default none.
    #[serde(rename = "if", default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
    /// The commands it runs, in order, as one all-or-nothing transaction; at most 64.
    #[serde(rename = "do")]
    pub actions: Vec<Step>,
    /// Disabled once it has run successfully. Default true.
    #[serde(default = "default_true")]
    pub once: bool,
    /// Only an enabled trigger runs. Default true.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Trigger {
    fn validate(&self, ctx: Ctx, at: &str) -> Result<(), WorldError> {
        self.when.validate(ctx, at)?;
        if let Some(c) = &self.condition {
            c.validate(ctx, &format!("{at}.if"))?;
        }
        if self.actions.len() > MAX_TRIGGER_STEPS {
            return Err(ctx.invalid(
                format!("{at}.do"),
                format!("{} commands", self.actions.len()),
                format!("a trigger runs at most {MAX_TRIGGER_STEPS} commands"),
            ));
        }
        for (i, step) in self.actions.iter().enumerate() {
            let field = format!("{at}.do[{i}]");
            ctx.id(format!("{field}.command"), &step.command)?;
            if !(step.params.is_object() || step.params.is_null()) {
                return Err(ctx.invalid(
                    format!("{field}.params"),
                    excerpt(&step.params.to_string()),
                    "it must be an object of parameters (or left out)",
                ));
            }
        }
        Ok(())
    }
}

/// Checks the scenario part of a world: counts, names, ids unique, every trigger's shape.
pub(crate) fn validate(
    vars: &BTreeMap<String, VarValue>,
    objectives: &[Objective],
    triggers: &[Trigger],
) -> Result<(), WorldError> {
    let ctx = Ctx::WORLD;
    if vars.len() > MAX_VARS {
        return Err(ctx.invalid(
            "vars",
            format!("{} variables", vars.len()),
            format!("a world holds at most {MAX_VARS} variables"),
        ));
    }
    for (name, value) in vars {
        if name.trim().is_empty() {
            return Err(ctx.invalid(
                "vars",
                "a variable with an empty name",
                "a variable's name holds at least one character other than a space",
            ));
        }
        ctx.name(format!("vars.{}", excerpt(name)), name)?;
        value.validate(ctx, &format!("vars.{}", excerpt(name)))?;
    }
    if objectives.len() > MAX_OBJECTIVES {
        return Err(ctx.invalid(
            "objectives",
            format!("{} objectives", objectives.len()),
            format!("a world holds at most {MAX_OBJECTIVES} objectives"),
        ));
    }
    for (i, o) in objectives.iter().enumerate() {
        ctx.id(format!("objectives[{i}].id"), &o.id)?;
        if objectives.iter().take(i).any(|earlier| earlier.id == o.id) {
            return Err(ctx.invalid(
                format!("objectives[{i}].id"),
                format!("{:?}", o.id),
                "the id is used more than once; every objective needs its own id",
            ));
        }
        ctx.text(format!("objectives[{i}].title"), &o.title, MAX_TEXT_CHARS)?;
    }
    if triggers.len() > MAX_TRIGGERS {
        return Err(ctx.invalid(
            "triggers",
            format!("{} triggers", triggers.len()),
            format!("a world holds at most {MAX_TRIGGERS} triggers"),
        ));
    }
    for (i, t) in triggers.iter().enumerate() {
        ctx.id(format!("triggers[{i}].id"), &t.id)?;
        if triggers.iter().take(i).any(|earlier| earlier.id == t.id) {
            return Err(ctx.invalid(
                format!("triggers[{i}].id"),
                format!("{:?}", t.id),
                "the id is used more than once; every trigger needs its own id",
            ));
        }
        t.validate(ctx, &format!("triggers[{i}] ({:?})", excerpt(&t.id)))?;
    }
    Ok(())
}

/// Longest excerpt of a name or value quoted in an error, in characters.
const MAX_EXCERPT_CHARS: usize = 60;

pub(crate) fn excerpt(text: &str) -> String {
    if text.chars().count() <= MAX_EXCERPT_CHARS.min(MAX_NAME_CHARS) {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(MAX_EXCERPT_CHARS).collect();
    cut.push_str("...");
    cut
}
