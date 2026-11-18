//! Scenarios: `var.set`, `var.remove`, `objective.add`, `objective.set`, `objective.remove`,
//! `trigger.add`, `trigger.set`, `trigger.remove`, `message.show`, and the queries
//! `scenario.inspect` and `objective.list` (`docs/design/scenarios.md`).
//!
//! Every World command here works on a running world too: that is how a game master, a
//! person or an agent, steers a game (AGENTS.md rule 6).

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Edit, Handler, Registration, Session, registered, registered_names};
use crate::spec::{CommandSpec, ParamType as T, Scope, excerpt, nearest, param, spec};
use crate::triggers::{MAX_MESSAGES, MESSAGE_COMMAND, Message, TARGET, WHO};
use gamecraft_sim::TICK_HZ;
use gamecraft_world::limits::MAX_TEXT_CHARS;
use gamecraft_world::{
    Condition, Objective, ObjectiveStatus, Step, Trigger, VarValue, When, World,
};
use serde_json::{Map, Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: var_set_spec,
        handler: Handler::World(var_set),
    },
    Registration {
        spec: var_remove_spec,
        handler: Handler::World(var_remove),
    },
    Registration {
        spec: objective_add_spec,
        handler: Handler::World(objective_add),
    },
    Registration {
        spec: objective_set_spec,
        handler: Handler::World(objective_set),
    },
    Registration {
        spec: objective_remove_spec,
        handler: Handler::World(objective_remove),
    },
    Registration {
        spec: objective_list_spec,
        handler: Handler::Query(objective_list),
    },
    Registration {
        spec: trigger_add_spec,
        handler: Handler::World(trigger_add),
    },
    Registration {
        spec: trigger_set_spec,
        handler: Handler::World(trigger_set),
    },
    Registration {
        spec: trigger_remove_spec,
        handler: Handler::World(trigger_remove),
    },
    Registration {
        spec: message_show_spec,
        handler: Handler::Play(message_show),
    },
    Registration {
        spec: inspect_spec,
        handler: Handler::Query(inspect),
    },
];

/// Longest a message shows, seconds.
const MAX_MESSAGE_S: f64 = 3600.0;

/// Validates the scenario part that a command writes, on an otherwise empty world, and
/// refuses a problem as bad parameters.
fn probe(set: impl FnOnce(&mut World)) -> Result<(), CommandError> {
    let mut w = World::new("", 0);
    set(&mut w);
    w.validate().map_err(|e| bad(e.to_string()))
}

fn required_str<'a>(p: &Params<'a>, name: &str) -> Result<&'a str, CommandError> {
    p.str(name)?
        .ok_or_else(|| bad(format!("missing `{name}` (string)")))
}

// ---------------------------------------------------------------------------------------------
// Variables

fn var_set_spec() -> CommandSpec {
    spec(
        "var.set",
        "Set Variable",
        "Sets a scenario variable to a number, a boolean or a string (`value`), or adds a \
         number to it (`add`; a missing variable counts from 0). While playing, a change runs \
         the triggers waiting for this variable.",
        vec![
            param(
                "name",
                T::String,
                "The variable's name, at most 200 characters.",
            )
            .required(),
            param(
                "value",
                T::Any,
                "The new value: a number, a boolean or a string; give this or `add`.",
            ),
            param(
                "add",
                T::Number,
                "A number added to the variable's number; give this or `value`.",
            ),
        ],
        "{name, value, previous (null when it did not exist)}",
        json!({"name": "score", "add": 1}),
    )
}

fn var_set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let name = required_str(&p, "name")?;
    let previous = edit.world.vars.get(name).cloned();
    let value = match (p.value("value"), p.f64("add")?) {
        (Some(_), Some(_)) => return Err(bad("give `value` or `add`, not both")),
        (None, None) => return Err(bad("give `value` or `add`")),
        (Some(v), None) => VarValue::from_json(v).map_err(|why| bad(format!("`value`: {why}")))?,
        (None, Some(add)) => match &previous {
            None => VarValue::Number(add),
            Some(VarValue::Number(x)) => VarValue::Number(x + add),
            Some(other) => {
                let kind = match other {
                    VarValue::Bool(_) => "boolean",
                    _ => "string",
                };
                return Err(bad(format!(
                    "`add` adds to a number, but variable \"{}\" holds the {kind} {other}; set \
                     it with `value`",
                    excerpt(name)
                )));
            }
        },
    };
    let checked = value.clone();
    probe(|w| {
        w.vars.insert(name.to_string(), checked);
    })?;
    let result = json!({
        "name": name,
        "value": value.to_json(),
        "previous": previous.as_ref().map_or(Value::Null, VarValue::to_json),
    });
    edit.world.vars.insert(name.to_string(), value);
    Ok(result)
}

fn var_remove_spec() -> CommandSpec {
    spec(
        "var.remove",
        "Remove Variable",
        "Removes a scenario variable. While playing, the change runs the triggers waiting for \
         it.",
        vec![param("name", T::String, "The variable's name.").required()],
        "{name, previous}",
        json!({"name": "score"}),
    )
}

fn list_or(names: Vec<&str>, none: &str) -> String {
    if names.is_empty() {
        none.to_string()
    } else {
        let shown: Vec<String> = names.iter().take(20).map(|n| excerpt(n)).collect();
        shown.join(", ")
    }
}

fn var_remove(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let name = required_str(&p, "name")?;
    let Some(previous) = edit.world.vars.remove(name) else {
        let names: Vec<&str> = edit.world.vars.keys().map(String::as_str).collect();
        return Err(failed(format!(
            "no variable is called \"{}\"; the variables are: {}",
            excerpt(name),
            list_or(names, "none")
        )));
    };
    Ok(json!({"name": name, "previous": previous.to_json()}))
}

// ---------------------------------------------------------------------------------------------
// Objectives

fn status_param(doc: &str) -> crate::spec::ParamSpec {
    param("status", T::String, doc).allowed(&ObjectiveStatus::NAMES)
}

fn objective_add_spec() -> CommandSpec {
    spec(
        "objective.add",
        "Add Objective",
        "Adds an objective the players are to achieve, at the end of the list.",
        vec![
            param(
                "id",
                T::String,
                "Its id, unique among the objectives, at most 200 characters.",
            )
            .required(),
            param(
                "title",
                T::String,
                "What to do, as the players read it, at most 2000 characters.",
            )
            .required(),
            status_param("Where it stands.").default(json!("open")),
            param(
                "hidden",
                T::Boolean,
                "Kept from the players; the game master still sees it.",
            )
            .default(json!(false)),
        ],
        "{id, title, status, hidden}",
        json!({"id": "reach-crate", "title": "Reach the crate"}),
    )
}

fn status_of(p: &Params<'_>) -> Result<Option<ObjectiveStatus>, CommandError> {
    p.str("status")?
        .map(|s| {
            ObjectiveStatus::from_name(s).ok_or_else(|| {
                bad(format!(
                    "`status` must be one of {}",
                    ObjectiveStatus::NAMES.join(", ")
                ))
            })
        })
        .transpose()
}

fn check_objective(o: &Objective) -> Result<(), CommandError> {
    let o = o.clone();
    probe(|w| w.objectives.push(o))
}

fn objective_json(o: &Objective) -> Result<Value, CommandError> {
    serde_json::to_value(o).map_err(failed)
}

fn objective_add(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = required_str(&p, "id")?;
    let objective = Objective {
        id: id.to_string(),
        title: required_str(&p, "title")?.to_string(),
        status: status_of(&p)?.unwrap_or_default(),
        hidden: p.bool("hidden")?.unwrap_or(false),
    };
    check_objective(&objective)?;
    if edit.world.objective(id).is_some() {
        return Err(failed(format!(
            "an objective with the id \"{}\" exists; change it with objective.set",
            excerpt(id)
        )));
    }
    let result = objective_json(&objective)?;
    edit.world.objectives.push(objective);
    Ok(result)
}

fn no_objective(world: &World, id: &str) -> CommandError {
    let ids: Vec<&str> = world.objectives.iter().map(|o| o.id.as_str()).collect();
    failed(format!(
        "no objective has the id \"{}\"; the objectives are: {}",
        excerpt(id),
        list_or(ids, "none (add one with objective.add)")
    ))
}

fn objective_set_spec() -> CommandSpec {
    spec(
        "objective.set",
        "Set Objective",
        "Changes an objective: its title, status or visibility; only the fields given change.",
        vec![
            param("id", T::String, "The objective's id.").required(),
            param("title", T::String, "What to do, at most 2000 characters."),
            status_param("Where it stands."),
            param("hidden", T::Boolean, "Kept from the players."),
        ],
        "{id, title, status, hidden}",
        json!({"id": "find-oak", "status": "done"}),
    )
}

fn objective_set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = required_str(&p, "id")?;
    let title = p.str("title")?;
    let status = status_of(&p)?;
    let hidden = p.bool("hidden")?;
    let world = &mut edit.world;
    let Some(index) = world.objectives.iter().position(|o| o.id == id) else {
        return Err(no_objective(world, id));
    };
    let Some(objective) = world.objectives.get_mut(index) else {
        return Err(no_objective(world, id));
    };
    if let Some(title) = title {
        objective.title = title.to_string();
    }
    if let Some(status) = status {
        objective.status = status;
    }
    if let Some(hidden) = hidden {
        objective.hidden = hidden;
    }
    check_objective(objective)?;
    objective_json(objective)
}

fn objective_remove_spec() -> CommandSpec {
    spec(
        "objective.remove",
        "Remove Objective",
        "Removes an objective.",
        vec![param("id", T::String, "The objective's id.").required()],
        "null",
        json!({"id": "find-oak"}),
    )
}

fn objective_remove(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = required_str(&p, "id")?;
    let Some(index) = edit.world.objectives.iter().position(|o| o.id == id) else {
        return Err(no_objective(&edit.world, id));
    };
    edit.world.objectives.remove(index);
    Ok(Value::Null)
}

fn objective_list_spec() -> CommandSpec {
    spec(
        "objective.list",
        "List Objectives",
        "Lists the objectives of the current world (the running world while playing), hidden \
         ones included, with how many are open, done and failed.",
        vec![],
        "{objectives: [{id, title, status, hidden}], open, done, failed}",
        json!({}),
    )
    .query()
}

fn objective_list(session: &Session, _p: Params<'_>) -> Result<Value, CommandError> {
    let world = session.current_world();
    let count = |s: ObjectiveStatus| world.objectives.iter().filter(|o| o.status == s).count();
    Ok(json!({
        "objectives": serde_json::to_value(&world.objectives).map_err(failed)?,
        "open": count(ObjectiveStatus::Open),
        "done": count(ObjectiveStatus::Done),
        "failed": count(ObjectiveStatus::Failed),
    }))
}

// ---------------------------------------------------------------------------------------------
// Triggers

const WHEN_DOC: &str = "The event: {\"event\": \"enter\"|\"leave\", \"zone\", \"who\"?}, \
    {\"event\": \"interact\", \"target\"?, \"who\"?}, {\"event\": \"time\", \"at_s\"}, \
    {\"event\": \"var\", \"name\"}, {\"event\": \"start\"}, or {\"event\": \"hit\"|\"down\", \
    \"who\"? (the shooter), \"target\"? (the entity hit or downed)} and {\"event\": \"heard\", \
    \"who\"? (the listener), \"target\"? (the shooter)}. `zone`, `target` and `who` are entity \
    ids or names, or \"tag:<tag>\".";
const IF_DOC: &str = "A condition over the variables: {\"var\", \"op\": \
    \"==\"|\"!=\"|\"<\"|\"<=\"|\">\"|\">=\", \"value\"}, {\"all\": [...]} or {\"any\": [...]}.";
const DO_DOC: &str = "The commands to run, in order, as one transaction: [{\"command\", \
    \"params\"?}], at most 64; world commands and message.show. A parameter that takes an entity \
    may be \"$who\" (the entity that caused the event) or \"$target\" (the zone or the target; \
    for hit, down and heard the event's `target`).";

fn trigger_add_spec() -> CommandSpec {
    spec(
        "trigger.add",
        "Add Trigger",
        "Adds a trigger at the end of the list: when its event happens while playing and its \
         condition holds, it runs its commands on the running world. Every command is checked \
         against its spec now; entity references are resolved when it runs.",
        vec![
            param(
                "id",
                T::String,
                "Its id, unique among the triggers, at most 200 characters.",
            )
            .required(),
            param("when", T::Object, WHEN_DOC).required(),
            param("if", T::Object, IF_DOC),
            param("do", T::Array, DO_DOC).required(),
            param("once", T::Boolean, "Disabled once it has run successfully.")
                .default(json!(true)),
            param("enabled", T::Boolean, "Only an enabled trigger runs.").default(json!(true)),
        ],
        "{id, when, if?, do, once, enabled}",
        json!({
            "id": "crate-found",
            "when": {"event": "enter", "zone": "Crate", "who": "Scout"},
            "do": [
                {"command": "var.set", "params": {"name": "score", "add": 1}},
                {"command": "message.show", "params": {"text": "You found the crate.", "to": "$who"}}
            ]
        }),
    )
}

fn parse_when(v: &Value) -> Result<When, CommandError> {
    serde_json::from_value(v.clone()).map_err(|e| bad(format!("`when`: {e}")))
}

fn parse_if(v: &Value) -> Result<Condition, CommandError> {
    serde_json::from_value(v.clone()).map_err(|e| bad(format!("`if`: {e}")))
}

fn parse_do(v: &Value) -> Result<Vec<Step>, CommandError> {
    let Some(items) = v.as_array() else {
        return Err(bad("`do` must be an array of {\"command\", \"params\"?}"));
    };
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            serde_json::from_value(item.clone()).map_err(|e| {
                bad(format!(
                    "`do[{i}]` must be {{\"command\", \"params\"?}}: {e}"
                ))
            })
        })
        .collect()
}

/// Whether `v` holds the string `$who` or `$target` anywhere below its top level.
fn nested_placeholder(v: &Value, depth: usize) -> Option<&'static str> {
    const MAX_DEPTH: usize = 64;
    if depth > MAX_DEPTH {
        return None;
    }
    match v {
        Value::String(s) if s == WHO => Some(WHO),
        Value::String(s) if s == TARGET => Some(TARGET),
        Value::Array(items) => items.iter().find_map(|x| nested_placeholder(x, depth + 1)),
        Value::Object(m) => m.values().find_map(|x| nested_placeholder(x, depth + 1)),
        _ => None,
    }
}

/// Checks every command of a trigger's `do` against its spec: the command exists, may run in
/// a trigger, its parameters fit, and `$who` / `$target` stand as the whole value of an entity
/// parameter of an event that has them.
fn check_steps(id: &str, when: &When, steps: &[Step]) -> Result<(), CommandError> {
    for (i, step) in steps.iter().enumerate() {
        let command = step.command.as_str();
        let at = format!(
            "trigger \"{}\", step {} (`{}`)",
            excerpt(id),
            i + 1,
            excerpt(command)
        );
        let Some((spec, scope)) = registered(command) else {
            let names = registered_names();
            let hint = nearest(command, names.iter().map(String::as_str))
                .map(|n| format!("; did you mean `{n}`?"))
                .unwrap_or_default();
            return Err(bad(format!("{at}: unknown command{hint}")));
        };
        if scope != Scope::World && command != MESSAGE_COMMAND {
            return Err(bad(format!(
                "{at}: a {} command cannot run in a trigger; a trigger runs world commands and \
                 {MESSAGE_COMMAND}",
                scope.name()
            )));
        }
        spec.check(&step.params)
            .map_err(|why| bad(format!("{at}: bad parameters: {why}")))?;
        let Value::Object(params) = &step.params else {
            continue;
        };
        for (key, value) in params {
            let placeholder = match value.as_str() {
                Some(WHO) => WHO,
                Some(TARGET) => TARGET,
                _ => {
                    if let Some(p) = nested_placeholder(value, 1) {
                        return Err(bad(format!(
                            "{at}: `{p}` inside `{}` is not replaced; `{p}` stands only as the \
                             whole value of an entity parameter (such as `id`)",
                            excerpt(key)
                        )));
                    }
                    continue;
                }
            };
            let is_entity = spec.param(key).is_some_and(|p| p.ty == T::Entity);
            if !is_entity {
                let ty = spec.param(key).map_or("any value", |p| p.ty.describe());
                return Err(bad(format!(
                    "{at}: `{placeholder}` stands for an entity, but `{}` takes {ty}, not an \
                     entity",
                    excerpt(key)
                )));
            }
            let has = if placeholder == WHO {
                when.has_who()
            } else {
                when.has_target()
            };
            if !has {
                return Err(bad(format!(
                    "{at}: `{placeholder}` has no value: a `{}` event has no entity for it; \
                     `{WHO}` and `{TARGET}` belong to enter, leave, interact, hit, down and heard",
                    when.event()
                )));
            }
        }
    }
    Ok(())
}

/// Checks a whole trigger: its shape (as the world validates it) and its steps.
fn check_trigger(trigger: &Trigger) -> Result<(), CommandError> {
    let copy = trigger.clone();
    probe(|w| w.triggers.push(copy))?;
    check_steps(&trigger.id, &trigger.when, &trigger.actions)
}

fn trigger_json(t: &Trigger) -> Result<Value, CommandError> {
    serde_json::to_value(t).map_err(failed)
}

fn trigger_add(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = required_str(&p, "id")?;
    let when = parse_when(
        p.value("when")
            .ok_or_else(|| bad("missing `when` (object)"))?,
    )?;
    let condition = p.value("if").map(parse_if).transpose()?;
    let actions = parse_do(p.value("do").ok_or_else(|| bad("missing `do` (array)"))?)?;
    let trigger = Trigger {
        id: id.to_string(),
        when,
        condition,
        actions,
        once: p.bool("once")?.unwrap_or(true),
        enabled: p.bool("enabled")?.unwrap_or(true),
    };
    check_trigger(&trigger)?;
    if edit.world.trigger(id).is_some() {
        return Err(failed(format!(
            "a trigger with the id \"{}\" exists; change it with trigger.set",
            excerpt(id)
        )));
    }
    let result = trigger_json(&trigger)?;
    edit.world.triggers.push(trigger);
    Ok(result)
}

fn trigger_set_spec() -> CommandSpec {
    spec(
        "trigger.set",
        "Set Trigger",
        "Changes a trigger; only the fields given change, and the whole trigger is checked \
         again (an `if` of {\"all\": []} always holds). Enabling a used-up `once` trigger \
         arms it again.",
        vec![
            param("id", T::String, "The trigger's id.").required(),
            param("when", T::Object, WHEN_DOC),
            param("if", T::Object, IF_DOC),
            param("do", T::Array, DO_DOC),
            param("once", T::Boolean, "Disabled once it has run successfully."),
            param("enabled", T::Boolean, "Only an enabled trigger runs."),
        ],
        "{id, when, if?, do, once, enabled}",
        json!({"id": "oak-found", "enabled": false}),
    )
}

fn no_trigger(world: &World, id: &str) -> CommandError {
    let ids: Vec<&str> = world.triggers.iter().map(|t| t.id.as_str()).collect();
    failed(format!(
        "no trigger has the id \"{}\"; the triggers are: {}",
        excerpt(id),
        list_or(ids, "none (add one with trigger.add)")
    ))
}

fn trigger_set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = required_str(&p, "id")?;
    let Some(index) = edit.world.triggers.iter().position(|t| t.id == id) else {
        return Err(no_trigger(&edit.world, id));
    };
    let Some(slot) = edit.world.triggers.get_mut(index) else {
        return Err(no_trigger(&edit.world, id));
    };
    let mut trigger = slot.clone();
    if let Some(v) = p.value("when") {
        trigger.when = parse_when(v)?;
    }
    if let Some(v) = p.value("if") {
        trigger.condition = Some(parse_if(v)?);
    }
    if let Some(v) = p.value("do") {
        trigger.actions = parse_do(v)?;
    }
    if let Some(once) = p.bool("once")? {
        trigger.once = once;
    }
    if let Some(enabled) = p.bool("enabled")? {
        trigger.enabled = enabled;
    }
    check_trigger(&trigger)?;
    let result = trigger_json(&trigger)?;
    *slot = trigger;
    Ok(result)
}

fn trigger_remove_spec() -> CommandSpec {
    spec(
        "trigger.remove",
        "Remove Trigger",
        "Removes a trigger.",
        vec![param("id", T::String, "The trigger's id.").required()],
        "null",
        json!({"id": "oak-found"}),
    )
}

fn trigger_remove(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = required_str(&p, "id")?;
    let Some(index) = edit.world.triggers.iter().position(|t| t.id == id) else {
        return Err(no_trigger(&edit.world, id));
    };
    edit.world.triggers.remove(index);
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------------------------
// Messages

fn message_show_spec() -> CommandSpec {
    spec(
        "message.show",
        "Show Message",
        "Shows a line to the players of the running world, for `seconds` of simulated time; \
         it is listed in sim.state. Only while playing, since messages are for players and \
         there are none while editing.",
        vec![
            param("text", T::String, "The line, 1 to 2000 characters.").required(),
            param(
                "to",
                T::Entity,
                "The character it is for (its id or name); everyone when left out.",
            ),
            param(
                "seconds",
                T::Number,
                "How long it shows, above 0 and at most 3600.",
            )
            .unit("s")
            .default(json!(5)),
        ],
        "{text, to (null for everyone), shown_at_s, until_s}",
        json!({"text": "Find the oak.", "to": "Scout", "seconds": 5}),
    )
    .play()
}

fn message_show(session: &mut Session, p: Params<'_>) -> Result<Value, CommandError> {
    let Some(sim) = session.live() else {
        return Err(failed(
            "no simulation is running; messages are shown to players while playing; start \
             one with sim.start",
        ));
    };
    let text = required_str(&p, "text")?;
    let chars = text.chars().count();
    if text.trim().is_empty() || chars > MAX_TEXT_CHARS {
        return Err(bad(format!(
            "`text` is {chars} characters long; it must hold 1 to {MAX_TEXT_CHARS} characters, \
             not only spaces"
        )));
    }
    let seconds = p.f64("seconds")?.unwrap_or(5.0);
    if !(seconds > 0.0 && seconds <= MAX_MESSAGE_S) {
        return Err(bad(format!(
            "`seconds` is {seconds}; it must be above 0 and at most {MAX_MESSAGE_S}"
        )));
    }
    let to = p.entity("to", sim.world())?;
    let tick = sim.tick();
    let ticks = ((seconds * f64::from(TICK_HZ)).round() as u64).max(1);
    let shown_at_s = sim.time_s();
    let message = Message {
        text: text.to_string(),
        to,
        shown_at_s,
        until_s: shown_at_s + seconds,
        until_tick: tick.saturating_add(ticks),
    };
    let result = message.to_json();
    let messages = &mut session.play_mut().messages;
    messages.push(message);
    while messages.len() > MAX_MESSAGES {
        messages.remove(0);
    }
    Ok(result)
}

// ---------------------------------------------------------------------------------------------
// The scenario as a whole

fn inspect_spec() -> CommandSpec {
    spec(
        "scenario.inspect",
        "Inspect Scenario",
        "Describes the scenario of the current world (the running world while playing): its \
         variables, objectives and triggers with their state, and while playing the messages \
         showing and the last trigger errors.",
        vec![],
        "{playing, tick (null when not playing), vars: {name: value}, objectives: [{id, \
         title, status, hidden}], triggers: [{id, when, if?, do, once, enabled, fired}], \
         messages: [{text, to, shown_at_s, until_s}], trigger_errors: [{trigger, tick, \
         error}]}",
        json!({}),
    )
    .query()
}

fn inspect(session: &Session, _p: Params<'_>) -> Result<Value, CommandError> {
    let world = session.current_world();
    let play = session.play();
    let mut triggers = Vec::new();
    for t in &world.triggers {
        let mut v = trigger_json(t)?;
        if let Value::Object(m) = &mut v {
            m.insert(
                "fired".into(),
                json!(play.fired_counts.get(&t.id).copied().unwrap_or(0)),
            );
        }
        triggers.push(v);
    }
    let mut out = Map::new();
    out.insert("playing".into(), json!(session.is_playing()));
    out.insert("tick".into(), json!(session.live().map(|s| s.tick())));
    out.insert(
        "vars".into(),
        serde_json::to_value(&world.vars).map_err(failed)?,
    );
    out.insert(
        "objectives".into(),
        serde_json::to_value(&world.objectives).map_err(failed)?,
    );
    out.insert("triggers".into(), Value::Array(triggers));
    out.insert("messages".into(), messages_json(session));
    out.insert(
        "trigger_errors".into(),
        Value::Array(play.errors.iter().cloned().collect()),
    );
    Ok(Value::Object(out))
}

/// The messages showing, as JSON.
pub(crate) fn messages_json(session: &Session) -> Value {
    Value::Array(
        session
            .play()
            .messages
            .iter()
            .map(Message::to_json)
            .collect(),
    )
}
