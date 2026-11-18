//! `query.aim`: the firing solution of a launcher against the air and wind of the current
//! world, found by flying the simulation's own flight model ([`gamecraft_sim::aim`]).

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use gamecraft_sim::aim::{AIM_TOLERANCE_M, aim_point, solve};
use gamecraft_sim::combat::muzzle;
use gamecraft_world::{EntityId, World};
use serde_json::{Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[Registration {
    spec: aim_spec,
    handler: Handler::Query(aim),
}];

fn aim_spec() -> CommandSpec {
    spec(
        "query.aim",
        "Aim",
        &format!(
            "The firing solution of the launcher of `from` (or of `launcher`, fired from \
             `from`'s muzzle: a character's eyes) at a point or an entity (a character's chest, \
             a shape's middle) in the current world, in its air and wind: the yaw and pitch to \
             give sim.fire or sim.input. It flies trial shots with the simulation's own flight \
             model and corrects the aim until one passes within {AIM_TOLERANCE_M} m of the \
             target, at most 40 trials; the low, direct solution only (pitch at most 45 \
             degrees). `reachable` is false when the target is beyond the projectile's flight. \
             Obstacles, the dispersion and the target's motion are not considered: lead a \
             moving target yourself, and ask query.line_of_sight whether the way is clear."
        ),
        vec![
            param("from", T::Entity, "The shooter: its id or name.").required(),
            param(
                "to",
                T::Any,
                "The target: a point [x, y, z] in metres, or an entity (its id or name).",
            )
            .required(),
            param(
                "launcher",
                T::Entity,
                "The entity whose launcher is fired; `from` when left out.",
            ),
        ],
        "{yaw_deg, pitch_deg, time_of_flight_s, drop_m, drift_m, reachable, miss_m (null when \
         every trial fell short), from: [x, y, z] (the muzzle), target: [x, y, z]}",
        json!({"from": "Scout", "to": [0, 1, -60]}),
    )
    .query()
}

/// The target point of `to`: a point [x, y, z] or an entity's aim point.
fn target(p: &Params<'_>, world: &World, from: EntityId) -> Result<[f64; 3], CommandError> {
    let Some(to) = p.value("to") else {
        return Err(bad("missing `to`: a point [x, y, z] or an entity"));
    };
    if to.is_array() {
        return p
            .point3("to")?
            .ok_or_else(|| bad("`to` must be a point [x, y, z] in metres, or an entity"));
    }
    if !(to.is_string() || to.is_u64()) {
        return Err(bad(format!(
            "`to` is {}; it must be a point [x, y, z] in metres, or an entity (its id or name)",
            crate::spec::excerpt(&to.to_string())
        )));
    }
    let id = p
        .entity_named("to", world)?
        .ok_or_else(|| bad("missing `to`: a point [x, y, z] or an entity"))?;
    if id == from {
        return Err(bad(format!(
            "`to` is the shooter itself (entity {id}); aim at another entity or a point"
        )));
    }
    let entity = world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity {id}")))?;
    Ok(aim_point(entity))
}

fn aim(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let world = session.current_world();
    let from_id = p
        .entity_named("from", world)?
        .ok_or_else(|| bad("missing `from`, the shooter (id or name)"))?;
    let from = world
        .entity(from_id)
        .ok_or_else(|| failed(format!("no entity {from_id}")))?;
    let launcher = match p.entity_named("launcher", world)? {
        Some(id) => {
            let holder = world
                .entity(id)
                .ok_or_else(|| failed(format!("no entity {id}")))?;
            holder.launcher().ok_or_else(|| {
                bad(format!(
                    "`launcher` names entity {id} (\"{}\"), which has no `launcher` component",
                    holder.name
                ))
            })?
        }
        None => from.launcher().ok_or_else(|| {
            failed(format!(
                "entity {from_id} (\"{}\") has no `launcher` component; give `launcher`, an \
                 entity that has one, or add one with entity.set_component",
                from.name
            ))
        })?,
    };
    let to = target(&p, world, from_id)?;
    let muzzle = muzzle(from, launcher);
    let s = solve(world, session.current_heights(), muzzle, launcher, to);
    Ok(json!({
        "yaw_deg": s.yaw_deg,
        "pitch_deg": s.pitch_deg,
        "time_of_flight_s": s.time_of_flight_s,
        "drop_m": s.drop_m,
        "drift_m": s.drift_m,
        "reachable": s.reachable,
        "miss_m": s.miss_m,
        "from": muzzle,
        "target": to,
    }))
}
