//! Fighting while playing: `sim.fire` and `sim.release`, and the JSON of projectiles, health
//! and behaviours that `sim.state`, `sim.step` and `entity.get` show
//! (`docs/design/ballistics.md`).

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use gamecraft_sim::{MAX_FIRE_PITCH_DEG, Sim};
use gamecraft_world::{Entity, EntityId};
use serde_json::{Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: fire_spec,
        handler: Handler::Play(fire),
    },
    Registration {
        spec: release_spec,
        handler: Handler::Play(release),
    },
];

/// The notation of a projectile in flight.
pub(crate) const PROJECTILE_DOC: &str = "{id, shooter, position, velocity, speed_mps, age_s}";
/// The notation of a behaviour's state.
pub(crate) const BEHAVIOUR_DOC: &str = "{state: \"idle\" | \"patrol\" | \"guard\" | \"alert\" | \
    \"engage\" | \"search\" | \"return\", target?, wait_s?, point?, left_s?, waypoint, \
    taken_over}";

fn running(session: &mut Session) -> Result<&mut Sim, CommandError> {
    session
        .live_mut()
        .ok_or_else(|| failed("no simulation is running; start one with sim.start"))
}

/// Every projectile in flight, in id order.
pub(crate) fn projectiles_json(sim: &Sim) -> Vec<Value> {
    sim.projectiles()
        .iter()
        .map(|p| {
            let [vx, vy, vz] = p.velocity;
            json!({
                "id": p.id,
                "shooter": p.shooter.0,
                "position": p.position,
                "velocity": p.velocity,
                "speed_mps": (vx * vx + vy * vy + vz * vz).sqrt(),
                "age_s": p.age_s,
            })
        })
        .collect()
}

/// `{max, current, armor, down}` of an entity's health, null without one.
pub(crate) fn health_json(e: &Entity) -> Value {
    match e.health() {
        Some(h) => json!({
            "max": h.max,
            "current": h.current,
            "armor": h.armor,
            "down": h.is_down(),
        }),
        None => Value::Null,
    }
}

/// The state of a behaviour while playing, null without one.
pub(crate) fn behaviour_json(sim: &Sim, id: EntityId) -> Value {
    let Some(brain) = sim.brain(id) else {
        return Value::Null;
    };
    let mut v = serde_json::to_value(brain.state).unwrap_or_else(|_| json!({}));
    if let Value::Object(m) = &mut v {
        m.insert("waypoint".into(), json!(brain.waypoint));
        m.insert("taken_over".into(), json!(sim.is_taken_over(id)));
    }
    v
}

/// `{left, shots, ready_in_s, reloading}` of a launcher while playing, null without one.
pub(crate) fn magazine_json(sim: &Sim, id: EntityId) -> Value {
    match sim.magazine(id) {
        Some(m) => json!({
            "left": m.left,
            "shots": m.shots,
            "ready_in_s": m.ready_in_s,
            "reloading": m.reloading,
        }),
        None => Value::Null,
    }
}

fn fire_spec() -> CommandSpec {
    spec(
        "sim.fire",
        "Fire",
        "Fires the launcher of an entity now, between two ticks: for anything that is not a \
         character (a turret, a mortar), or a character outside its input. The projectile \
         flies from the next step; the shot is heard at once (`heard` events in the next \
         step's answer). Refused while the launcher is not ready (it says how long to wait) or \
         the entity is down.",
        vec![
            param(
                "entity",
                T::Entity,
                "The entity with the `launcher` component: its id or name.",
            )
            .required(),
            param(
                "yaw_deg",
                T::Number,
                "Where to fire, counter-clockwise from north seen from above; default the \
                 entity's rotation.",
            )
            .unit("deg"),
            param(
                "pitch_deg",
                T::Number,
                "Up from level, -90 to 90; default a character's input pitch, else 0.",
            )
            .unit("deg"),
        ],
        "{entity, projectile (its id), magazine: {left, shots, ready_in_s, reloading}}",
        json!({"entity": "Scout", "yaw_deg": 0, "pitch_deg": 2}),
    )
    .play()
}

fn fire(session: &mut Session, p: Params<'_>) -> Result<Value, CommandError> {
    let sim = running(session)?;
    let id = p.required_entity("entity", sim.world())?;
    let yaw = p.f64("yaw_deg")?;
    let pitch = p.f64("pitch_deg")?;
    if let Some(pitch) = pitch
        && !(-MAX_FIRE_PITCH_DEG..=MAX_FIRE_PITCH_DEG).contains(&pitch)
    {
        return Err(bad(format!(
            "`pitch_deg` is {pitch}; it must be from -{MAX_FIRE_PITCH_DEG} to \
             {MAX_FIRE_PITCH_DEG} degrees"
        )));
    }
    let projectile = sim.fire(id, yaw, pitch).map_err(failed)?;
    session.bump_view();
    let sim = running(session)?;
    Ok(json!({
        "entity": id.0,
        "projectile": projectile,
        "magazine": magazine_json(sim, id),
    }))
}

fn release_spec() -> CommandSpec {
    spec(
        "sim.release",
        "Release",
        "Hands a character whose behaviour an agent took over (by sending it sim.input) back to \
         its behaviour, which decides again from the next step. Releasing a character that is \
         not taken over changes nothing.",
        vec![param("entity", T::Entity, "The character: its id or name.").required()],
        "{entity, released (whether it was taken over), behaviour: {state, ...} | null}",
        json!({"entity": "Scout"}),
    )
    .play()
}

fn release(session: &mut Session, p: Params<'_>) -> Result<Value, CommandError> {
    let sim = running(session)?;
    let id = p.required_entity("entity", sim.world())?;
    let was = sim.is_taken_over(id);
    sim.release(id).map_err(failed)?;
    Ok(json!({
        "entity": id.0,
        "released": was,
        "behaviour": behaviour_json(sim, id),
    }))
}
