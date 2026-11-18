//! Playing: `sim.start`, `sim.stop`, `sim.step`, `sim.input`, `sim.state`.

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use crate::triggers::{self, Happening};
use gamecraft_math::{Portable, rad_to_deg, wrap_deg};
use gamecraft_sim::{Input, MAX_STEP_TICKS, Sim, TICK_HZ};
use serde_json::{Map, Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: start_spec,
        handler: Handler::Play(start),
    },
    Registration {
        spec: stop_spec,
        handler: Handler::Play(stop),
    },
    Registration {
        spec: step_spec,
        handler: Handler::Play(step),
    },
    Registration {
        spec: input_spec,
        handler: Handler::Play(input),
    },
    Registration {
        spec: state_spec,
        handler: Handler::Play(state),
    },
];

/// Below this horizontal distance, metres, a `face` point is the character's own position.
const MIN_FACE_DISTANCE_M: f64 = 1e-9;

const CHARACTERS_DOC: &str = "[{id, name, position, rotation_y_deg, velocity, grounded}]";

/// The state hash as 16 lowercase hex digits (a JSON number cannot hold a `u64` exactly).
fn hash(sim: &Sim) -> String {
    format!("{:016x}", sim.state_hash())
}

/// Every character of the running world, in id order.
fn characters(sim: &Sim) -> Vec<Value> {
    sim.world()
        .entities()
        .filter(|e| e.character().is_some())
        .map(|e| {
            let body = sim.body(e.id).copied().unwrap_or_default();
            json!({
                "id": e.id.0,
                "name": e.name,
                "position": e.position,
                "rotation_y_deg": e.rotation_y_deg,
                "velocity": body.velocity,
                "grounded": body.grounded,
            })
        })
        .collect()
}

fn not_playing() -> CommandError {
    failed("no simulation is running; start one with sim.start")
}

fn running(session: &mut Session) -> Result<&mut Sim, CommandError> {
    session.live_mut().ok_or_else(not_playing)
}

fn start_spec() -> CommandSpec {
    spec(
        "sim.start",
        "Play",
        "Starts a simulation on a copy of the document. World commands then change the running \
         world, not the document, until sim.stop. The triggers waiting for `start` run.",
        vec![],
        "{tick: 0, hash, characters: [ids], events: [{tick, event: \"start\"}], fired: \
         [{trigger, tick, event, ok, error?}]}",
        json!({}),
    )
    .play()
}

fn start(session: &mut Session, _p: Params<'_>) -> Result<Value, CommandError> {
    if session.is_playing() {
        return Err(failed(
            "a simulation is already running; stop it first with sim.stop",
        ));
    }
    let sim = Sim::new(session.document().clone()).map_err(failed)?;
    let ids: Vec<u64> = sim.bodies().map(|(id, _)| id.0).collect();
    session.start_live(sim);
    triggers::begin_report(session);
    triggers::happen(session, 0, Happening::Start);
    let report = triggers::take_report(session);
    let sim = running(session)?;
    let mut result = Map::new();
    result.insert("tick".into(), json!(sim.tick()));
    result.insert("hash".into(), json!(hash(sim)));
    result.insert("characters".into(), json!(ids));
    report.into_answer(&mut result);
    Ok(Value::Object(result))
}

fn stop_spec() -> CommandSpec {
    spec(
        "sim.stop",
        "Stop",
        "Stops the simulation and throws the running world away, or with `keep` makes it the \
         document, as one undo step.",
        vec![
            param(
                "keep",
                T::Boolean,
                "Make the running world (moved characters, game-master changes) the document.",
            )
            .default(json!(false)),
        ],
        "{tick, kept}",
        json!({"keep": false}),
    )
    .play()
    .undoable()
}

fn stop(session: &mut Session, p: Params<'_>) -> Result<Value, CommandError> {
    let keep = p.bool("keep")?.unwrap_or(false);
    let sim = session.take_live().ok_or_else(not_playing)?;
    let tick = sim.tick();
    if keep {
        let world = sim.into_world();
        if world != *session.document() {
            session.set_document(world, None);
        }
    }
    Ok(json!({"tick": tick, "kept": keep}))
}

fn step_spec() -> CommandSpec {
    spec(
        "sim.step",
        "Step",
        "Advances the simulation by whole ticks of 1/60 s: `ticks`, or `seconds` rounded to \
         whole ticks (not both); at most 36000 ticks (600 s) per call. After each tick the \
         scenario's triggers run for the events of that tick; variables changed by commands \
         since the last step fire their triggers first. The answer lists the events and the \
         triggers that fired (at most 1000 of each; `events_omitted` and `fired_omitted` count \
         the rest).",
        vec![
            param("ticks", T::Integer, "Ticks to advance, 0 to 36000.").default(json!(1)),
            param("seconds", T::Number, "Simulated time to advance, 0 to 600.").unit("s"),
        ],
        &format!(
            "{{tick, time_s, hash, characters: {CHARACTERS_DOC}, events: [{{tick, event, who?, \
             zone?, target?, at_s?, name?}}], fired: [{{trigger, tick, event, ok, error?}}]}}"
        ),
        json!({"seconds": 0.5}),
    )
    .play()
}

fn step(session: &mut Session, p: Params<'_>) -> Result<Value, CommandError> {
    let max = u64::from(MAX_STEP_TICKS);
    let max_s = max / u64::from(TICK_HZ);
    let ticks = match (p.u64("ticks")?, p.f64("seconds")?) {
        (Some(_), Some(_)) => {
            return Err(bad("give `ticks` or `seconds`, not both"));
        }
        (Some(t), None) => {
            if t > max {
                return Err(bad(format!(
                    "`ticks` is {t}; at most {max} ticks ({max_s} s) per call; step again for \
                     more"
                )));
            }
            t
        }
        (None, Some(s)) => {
            let t = (s * f64::from(TICK_HZ)).round();
            if !(0.0..=max as f64).contains(&t) {
                return Err(bad(format!(
                    "`seconds` is {s}; it must be from 0 to {max_s} ({max} ticks) per call; \
                     step again for more"
                )));
            }
            t as u64
        }
        (None, None) => 1,
    };
    let ticks = u32::try_from(ticks).unwrap_or(MAX_STEP_TICKS);
    let start = running(session)?.tick();
    triggers::begin_report(session);
    triggers::run_pending(session, start);
    for _ in 0..ticks {
        let sim = running(session)?;
        sim.step(1);
        let tick = sim.tick();
        for event in sim.take_events() {
            triggers::happen(session, event.tick, Happening::Sim(event.kind));
        }
        triggers::run_pending(session, tick);
        triggers::expire_messages(session, tick);
    }
    let report = triggers::take_report(session);
    let sim = running(session)?;
    let mut result = Map::new();
    result.insert("tick".into(), json!(sim.tick()));
    result.insert("time_s".into(), json!(sim.time_s()));
    result.insert("hash".into(), json!(hash(sim)));
    result.insert("characters".into(), json!(characters(sim)));
    report.into_answer(&mut result);
    if ticks > 0 {
        session.bump_view();
    }
    Ok(Value::Object(result))
}

fn input_spec() -> CommandSpec {
    spec(
        "sim.input",
        "Input",
        "Stores what a character is asked to do; it holds until replaced. Fields left out keep \
         their last value; a character's first input starts from standing still, facing its \
         current rotation.",
        vec![
            param("entity", T::Entity, "The character: its id or name.").required(),
            param(
                "forward",
                T::Number,
                "-1 (back) to 1 (forward), along the facing.",
            ),
            param(
                "right",
                T::Number,
                "-1 (left) to 1 (right), across the facing.",
            ),
            param(
                "yaw_deg",
                T::Number,
                "The facing, counter-clockwise from north seen from above; give this or `face`.",
            )
            .unit("deg"),
            param(
                "face",
                T::Point2,
                "Turns the character towards the ground point [x, z]; give this or `yaw_deg`.",
            )
            .unit("m"),
            param("run", T::Boolean, "Run instead of walk."),
            param(
                "jump",
                T::Boolean,
                "Held: a jump starts whenever the character stands on the ground.",
            ),
            param(
                "interact",
                T::Boolean,
                "Held: interacts once, when it turns true, with the nearest interactable \
                 within reach that the character faces.",
            ),
        ],
        "{entity, forward, right, yaw_deg, run, jump, interact}: the stored input",
        json!({"entity": "Scout", "forward": 1, "face": [0, -60], "run": true}),
    )
    .play()
}

fn input_json(id: u64, i: &Input) -> Value {
    json!({
        "entity": id,
        "forward": i.forward,
        "right": i.right,
        "yaw_deg": i.yaw_deg,
        "run": i.run,
        "jump": i.jump,
        "interact": i.interact,
    })
}

fn input(session: &mut Session, p: Params<'_>) -> Result<Value, CommandError> {
    let sim = running(session)?;
    let id = p.required_entity("entity", sim.world())?;
    let entity = sim
        .world()
        .entity(id)
        .ok_or_else(|| failed(format!("no entity {id} in the running world")))?;
    if entity.character().is_none() {
        return Err(failed(format!(
            "entity {id} (\"{}\") has no `character` component, so it takes no input; add one \
             with entity.set_component",
            entity.name
        )));
    }
    let [ex, _, ez] = entity.position;
    let mut record = sim.input(id).copied().unwrap_or(Input {
        yaw_deg: entity.rotation_y_deg,
        ..Input::default()
    });
    if p.has("yaw_deg") && p.has("face") {
        return Err(bad("give `yaw_deg` or `face`, not both"));
    }
    if let Some(v) = p.f64("forward")? {
        record.forward = v;
    }
    if let Some(v) = p.f64("right")? {
        record.right = v;
    }
    if let Some(v) = p.f64("yaw_deg")? {
        record.yaw_deg = v;
    }
    if let Some([x, z]) = p.point2("face")? {
        let (dx, dz) = (x - ex, z - ez);
        let distance = (dx * dx + dz * dz).sqrt();
        if !(distance.is_finite() && distance > MIN_FACE_DISTANCE_M) {
            return Err(bad(format!(
                "`face` [{x}, {z}] is where the character stands; give a point away from it"
            )));
        }
        // yaw_forward(yaw) = (-sin yaw, 0, -cos yaw), so yaw = atan2(-dx, -dz).
        record.yaw_deg = wrap_deg(rad_to_deg((-dx).patan2(-dz)));
    }
    if let Some(v) = p.bool("run")? {
        record.run = v;
    }
    if let Some(v) = p.bool("jump")? {
        record.jump = v;
    }
    if let Some(v) = p.bool("interact")? {
        record.interact = v;
    }
    sim.set_input(id, record).map_err(failed)?;
    let stored = sim.input(id).copied().unwrap_or(record);
    Ok(input_json(id.0, &stored))
}

fn state_spec() -> CommandSpec {
    spec(
        "sim.state",
        "Play State",
        "Whether a simulation runs and, if so, its tick, hash, characters, stored inputs, \
         objectives, the messages showing and the last 20 trigger errors.",
        vec![],
        &format!(
            "{{playing: false}} or {{playing: true, tick, time_s, hash, characters: \
             {CHARACTERS_DOC}, inputs: [{{entity, forward, right, yaw_deg, run, jump, \
             interact}}], objectives: [{{id, title, status, hidden}}], messages: [{{text, to, \
             shown_at_s, until_s}}], trigger_errors: [{{trigger, tick, error}}]}}"
        ),
        json!({}),
    )
    .play()
}

fn state(session: &mut Session, _p: Params<'_>) -> Result<Value, CommandError> {
    let Some(sim) = session.live() else {
        return Ok(json!({"playing": false}));
    };
    let inputs: Vec<Value> = sim
        .bodies()
        .filter_map(|(id, _)| sim.input(id).map(|i| input_json(id.0, i)))
        .collect();
    Ok(json!({
        "playing": true,
        "tick": sim.tick(),
        "time_s": sim.time_s(),
        "hash": hash(sim),
        "characters": characters(sim),
        "inputs": inputs,
        "objectives": serde_json::to_value(&sim.world().objectives).map_err(failed)?,
        "messages": crate::scenario_cmds::messages_json(session),
        "trigger_errors": session.play().errors.iter().cloned().collect::<Vec<_>>(),
    }))
}
