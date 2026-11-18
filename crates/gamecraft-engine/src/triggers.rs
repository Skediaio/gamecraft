//! Running a scenario's triggers while playing (`docs/design/scenarios.md`, *Running*).
//!
//! The simulation detects `enter`, `leave`, `interact` and `time` events during a tick; the
//! session adds `start` (on `sim.start`) and `var` (a variable changed). After each tick the
//! session hands every event, in order, to [`happen`]: each enabled trigger of the running
//! world that waits for it, whose references match and whose condition holds, runs its
//! commands in order as one all-or-nothing transaction on the running world. A trigger with
//! `once` is disabled when it has run successfully. Variables changed by a trigger's commands
//! are `var` events run in the same pass, depth first, at most [`MAX_TRIGGER_DEPTH`] deep.
//! A failing trigger leaves the world as it was before it and is reported, never fatal.

use crate::session::Session;
use crate::spec::Scope;
use gamecraft_sim::{Event, EventKind, Sim};
use gamecraft_world::{EntityId, EntityRef, Trigger, VarValue, When, World};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Deepest chain of triggers fired by the commands of triggers.
pub(crate) const MAX_TRIGGER_DEPTH: usize = 32;
/// Trigger errors kept for `sim.state`.
pub(crate) const MAX_TRIGGER_ERRORS: usize = 20;
/// Most events, and most fired triggers, one `sim.step` or `sim.start` answer lists; the
/// rest are counted.
pub(crate) const MAX_REPORTED: usize = 1000;
/// Most messages showing at once; the oldest goes first.
pub(crate) const MAX_MESSAGES: usize = 100;
/// The command a trigger may run besides world commands.
pub(crate) const MESSAGE_COMMAND: &str = "message.show";
/// The placeholder for the entity that caused the event.
pub(crate) const WHO: &str = "$who";
/// The placeholder for the entity the event was about.
pub(crate) const TARGET: &str = "$target";

/// A line shown to the players.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Message {
    pub(crate) text: String,
    /// The character it is for; everyone when `None`.
    pub(crate) to: Option<EntityId>,
    pub(crate) shown_at_s: f64,
    pub(crate) until_s: f64,
    /// Shown while the tick is below this.
    pub(crate) until_tick: u64,
}

impl Message {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "text": self.text,
            "to": self.to.map(|id| id.0),
            "shown_at_s": self.shown_at_s,
            "until_s": self.until_s,
        })
    }
}

/// The events and fired triggers of one `sim.step` or `sim.start`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Report {
    pub(crate) events: Vec<Value>,
    pub(crate) fired: Vec<Value>,
    pub(crate) events_omitted: u64,
    pub(crate) fired_omitted: u64,
}

impl Report {
    /// Adds `events` and `fired` (and their omitted counts when any) to an answer.
    pub(crate) fn into_answer(self, answer: &mut Map<String, Value>) {
        answer.insert("events".into(), Value::Array(self.events));
        answer.insert("fired".into(), Value::Array(self.fired));
        if self.events_omitted > 0 {
            answer.insert("events_omitted".into(), json!(self.events_omitted));
        }
        if self.fired_omitted > 0 {
            answer.insert("fired_omitted".into(), json!(self.fired_omitted));
        }
    }
}

/// What the scenario keeps while playing, outside the world.
#[derive(Clone, Debug, Default)]
pub(crate) struct PlayLog {
    /// Messages showing.
    pub(crate) messages: Vec<Message>,
    /// The last [`MAX_TRIGGER_ERRORS`] trigger errors: `{trigger, tick, error}`.
    pub(crate) errors: VecDeque<Value>,
    /// How often each trigger ran successfully.
    pub(crate) fired_counts: BTreeMap<String, u64>,
    /// Variables changed by commands since their `var` events last ran.
    pub(crate) pending_vars: Vec<String>,
    /// The answer being collected.
    pub(crate) report: Report,
}

/// Something triggers wait for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Happening {
    /// An event of the simulation.
    Sim(EventKind),
    /// A variable changed.
    Var(String),
    /// The simulation started.
    Start,
}

impl Happening {
    fn name(&self) -> &'static str {
        match self {
            Happening::Sim(kind) => kind.name(),
            Happening::Var(_) => "var",
            Happening::Start => "start",
        }
    }

    fn to_json(&self, tick: u64) -> Value {
        match self {
            Happening::Sim(kind) => serde_json::to_value(Event {
                tick,
                kind: kind.clone(),
            })
            .unwrap_or(Value::Null),
            Happening::Var(name) => json!({"tick": tick, "event": "var", "name": name}),
            Happening::Start => json!({"tick": tick, "event": "start"}),
        }
    }

    /// Whether a trigger's `when` is of this kind (references are matched later).
    fn same_kind(&self, when: &When) -> bool {
        match (self, when) {
            (Happening::Sim(EventKind::Enter { .. }), When::Enter { .. })
            | (Happening::Sim(EventKind::Leave { .. }), When::Leave { .. })
            | (Happening::Sim(EventKind::Interact { .. }), When::Interact { .. })
            | (Happening::Start, When::Start) => true,
            (Happening::Sim(EventKind::Time { at_s }), When::Time { at_s: t }) => at_s == t,
            (Happening::Var(name), When::Var { name: n }) => name == n,
            _ => false,
        }
    }
}

/// The names of the variables that differ between `before` and `after`, in name order.
pub(crate) fn changed_vars(
    before: &BTreeMap<String, VarValue>,
    after: &BTreeMap<String, VarValue>,
) -> Vec<String> {
    let names: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    names
        .into_iter()
        .filter(|name| before.get(*name) != after.get(*name))
        .cloned()
        .collect()
}

/// Starts collecting the answer of a `sim.step` or `sim.start`.
pub(crate) fn begin_report(session: &mut Session) {
    session.play_mut().report = Report::default();
}

/// Hands out the collected answer.
pub(crate) fn take_report(session: &mut Session) -> Report {
    std::mem::take(&mut session.play_mut().report)
}

/// Runs the `var` events of variables changed by commands since the last step, at `tick`.
pub(crate) fn run_pending(session: &mut Session, tick: u64) {
    let names = std::mem::take(&mut session.play_mut().pending_vars);
    let mut seen = BTreeSet::new();
    for name in names {
        if seen.insert(name.clone()) {
            happen(session, tick, Happening::Var(name));
        }
    }
}

/// Records `happening` in the answer and runs the triggers that wait for it.
pub(crate) fn happen(session: &mut Session, tick: u64, happening: Happening) {
    let mut chain = Vec::new();
    run_triggers(session, tick, &happening, 0, &mut chain);
}

/// Forgets messages whose time is over.
pub(crate) fn expire_messages(session: &mut Session, tick: u64) {
    session.play_mut().messages.retain(|m| m.until_tick > tick);
}

fn record_event(session: &mut Session, tick: u64, happening: &Happening) {
    let report = &mut session.play_mut().report;
    if report.events.len() < MAX_REPORTED {
        report.events.push(happening.to_json(tick));
    } else {
        report.events_omitted = report.events_omitted.saturating_add(1);
    }
}

fn record_fired(
    session: &mut Session,
    trigger: &str,
    tick: u64,
    happening: &Happening,
    outcome: Result<(), String>,
) {
    let play = session.play_mut();
    let mut fired = json!({
        "trigger": trigger,
        "tick": tick,
        "event": happening.name(),
        "ok": outcome.is_ok(),
    });
    match outcome {
        Ok(()) => {
            let count = play.fired_counts.entry(trigger.to_string()).or_insert(0);
            *count = count.saturating_add(1);
        }
        Err(error) => {
            if let Value::Object(m) = &mut fired {
                m.insert("error".into(), json!(error));
            }
            play.errors
                .push_back(json!({"trigger": trigger, "tick": tick, "error": error}));
            while play.errors.len() > MAX_TRIGGER_ERRORS {
                play.errors.pop_front();
            }
        }
    }
    let report = &mut play.report;
    if report.fired.len() < MAX_REPORTED {
        report.fired.push(fired);
    } else {
        report.fired_omitted = report.fired_omitted.saturating_add(1);
    }
}

fn run_triggers(
    session: &mut Session,
    tick: u64,
    happening: &Happening,
    depth: usize,
    chain: &mut Vec<String>,
) {
    record_event(session, tick, happening);
    let Some(sim) = session.live() else {
        return;
    };
    let ids: Vec<String> = sim
        .world()
        .triggers
        .iter()
        .filter(|t| t.enabled && happening.same_kind(&t.when))
        .map(|t| t.id.clone())
        .collect();
    for id in ids {
        // Read again: an earlier trigger may have changed, disabled or removed this one.
        let Some(world) = session.live().map(Sim::world) else {
            return;
        };
        let Some(trigger) = world.trigger(&id).filter(|t| t.enabled).cloned() else {
            continue;
        };
        let (who, target) = match matches(world, &trigger.when, happening) {
            Ok(Some(pair)) => pair,
            Ok(None) => continue,
            Err(why) => {
                record_fired(session, &id, tick, happening, Err(why));
                continue;
            }
        };
        if let Some(condition) = &trigger.condition
            && !condition.holds(&world.vars)
        {
            continue;
        }
        if depth >= MAX_TRIGGER_DEPTH {
            let mut names = chain.clone();
            names.push(id.clone());
            let why = format!(
                "not run: triggers fired by triggers nest at most {MAX_TRIGGER_DEPTH} deep; \
                 the chain is {}",
                names.join(" > ")
            );
            record_fired(session, &id, tick, happening, Err(why));
            continue;
        }
        match run_actions(session, &trigger, who, target) {
            Ok(changed) => {
                record_fired(session, &id, tick, happening, Ok(()));
                chain.push(id.clone());
                for name in changed {
                    run_triggers(session, tick, &Happening::Var(name), depth + 1, chain);
                }
                chain.pop();
            }
            Err(why) => record_fired(session, &id, tick, happening, Err(why)),
        }
    }
}

/// The entity `reference` names in `world`, as `when.<field> "<reference>": <why>` when none.
fn resolve(world: &World, reference: &EntityRef, field: &str) -> Result<EntityId, String> {
    world
        .find(&reference.0)
        .map_err(|e| format!("when.{field} \"{}\": {e}", reference.0))
}

/// Whether the acting entity passes a `who` filter.
fn who_passes(world: &World, who: EntityId, filter: Option<&EntityRef>) -> Result<bool, String> {
    let Some(filter) = filter else {
        return Ok(true);
    };
    match filter.tag() {
        Some(tag) => Ok(world.entity(who).is_some_and(|e| e.has_tag(tag))),
        None => Ok(resolve(world, filter, "who")? == who),
    }
}

/// The entity that caused an event (`$who`) and the one it was about (`$target`).
type Parties = (Option<EntityId>, Option<EntityId>);

/// `Some((who, target))` when `when` matches `happening`; an error when a reference of the
/// trigger names no entity.
fn matches(world: &World, when: &When, happening: &Happening) -> Result<Option<Parties>, String> {
    let Happening::Sim(kind) = happening else {
        return Ok(Some((None, None)));
    };
    let (who, subject) = (kind.who(), kind.target());
    let (wanted, filter, field) = match when {
        When::Enter { zone, who } | When::Leave { zone, who } => (Some(zone), who.as_ref(), "zone"),
        When::Interact { target, who } => (target.as_ref(), who.as_ref(), "target"),
        _ => return Ok(Some((who, subject))),
    };
    if let (Some(wanted), Some(subject)) = (wanted, subject)
        && resolve(world, wanted, field)? != subject
    {
        return Ok(None);
    }
    if let Some(who) = who
        && !who_passes(world, who, filter)?
    {
        return Ok(None);
    }
    Ok(Some((who, subject)))
}

/// `params` with every top-level parameter that is exactly `$who` or `$target` replaced by
/// that entity's id (as a string, which every entity parameter takes).
fn substitute(
    params: &Value,
    who: Option<EntityId>,
    target: Option<EntityId>,
    event: &str,
) -> Result<Value, String> {
    let Value::Object(map) = params else {
        return Ok(params.clone());
    };
    let mut out = Map::new();
    for (key, value) in map {
        let replaced = match value.as_str() {
            Some(WHO) => json!(
                who.ok_or_else(|| format!("`{WHO}` has no value for a `{event}` event"))?
                    .0
                    .to_string()
            ),
            Some(TARGET) => json!(
                target
                    .ok_or_else(|| format!("`{TARGET}` has no value for a `{event}` event"))?
                    .0
                    .to_string()
            ),
            _ => value.clone(),
        };
        out.insert(key.clone(), replaced);
    }
    Ok(Value::Object(out))
}

/// Runs a trigger's commands as one transaction on the running world. On success the names
/// of the variables they changed; on failure the world is as before and the error names the
/// step.
fn run_actions(
    session: &mut Session,
    trigger: &Trigger,
    who: Option<EntityId>,
    target: Option<EntityId>,
) -> Result<Vec<String>, String> {
    let saved_live = session.live().cloned();
    let saved_messages = session.play().messages.clone();
    let saved_pending = std::mem::take(&mut session.play_mut().pending_vars);
    let outcome = run_steps(session, trigger, who, target);
    let changed = std::mem::replace(&mut session.play_mut().pending_vars, saved_pending);
    match outcome {
        Ok(()) => {
            let mut seen = BTreeSet::new();
            Ok(changed
                .into_iter()
                .filter(|name| seen.insert(name.clone()))
                .collect())
        }
        Err(why) => {
            session.restore_live(saved_live);
            session.play_mut().messages = saved_messages;
            Err(why)
        }
    }
}

fn run_steps(
    session: &mut Session,
    trigger: &Trigger,
    who: Option<EntityId>,
    target: Option<EntityId>,
) -> Result<(), String> {
    let event = trigger.when.event();
    for (i, step) in trigger.actions.iter().enumerate() {
        let n = i + 1;
        let at = format!("step {n} (`{}`)", crate::spec::excerpt(&step.command));
        let scope = session.spec(&step.command).map(|s| s.scope);
        match scope {
            None => {
                return Err(format!("{at}: unknown command"));
            }
            Some(Scope::World) => {}
            Some(_) if step.command == MESSAGE_COMMAND => {}
            Some(other) => {
                return Err(format!(
                    "{at}: a {} command; a trigger runs world commands and {MESSAGE_COMMAND}",
                    other.name()
                ));
            }
        }
        let params =
            substitute(&step.params, who, target, event).map_err(|why| format!("{at}: {why}"))?;
        session
            .run(&step.command, &params)
            .map_err(|e| format!("{at}: {e}"))?;
    }
    if trigger.once
        && let Some(sim) = session.live_mut()
    {
        let mut world = sim.world().clone();
        if let Some(t) = world.triggers.iter_mut().find(|t| t.id == trigger.id) {
            t.enabled = false;
            sim.replace_world(world)
                .map_err(|e| format!("disabling the trigger after it ran: {e}"))?;
        }
    }
    Ok(())
}
