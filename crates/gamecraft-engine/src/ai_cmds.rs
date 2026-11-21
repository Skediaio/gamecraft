//! Soldiers' skill, what they know and where they take cover: `ai.set` (a game master's or an
//! agent's hand on skill, precision, detection, courage, cover and alertness, one character or
//! a squad, on a running world as on the document), `ai.inspect` (what a behaviour knows and
//! feels) and `query.cover` (the simulation's own cover query). Presets (recruit, veteran)
//! are a pack's data: these commands take the numbers.

use crate::combat_cmds::behaviour_json;
use crate::error::{CommandError, bad, failed};
use crate::gm_cmds::order_targets;
use crate::params::Params;
use crate::session::{Edit, Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, excerpt, param, spec};
use gamecraft_sim::ai::{self, CoverPhase, REACT_KNOWLEDGE};
use gamecraft_sim::aim::aim_point;
use gamecraft_sim::behaviour::team_of;
use gamecraft_sim::cover::{
    CoverAsk, CoverPoint, DEFAULT_COVER_SEARCH_M, MAX_COVER_CANDIDATES, MAX_COVER_THREATS, Threat,
    evaluate_world,
};
use gamecraft_world::limits::{AIM_ERROR_MAX_MRAD, BEHAVIOUR_TIME_MAX_S, COVER_SEARCH_MAX_M};
use gamecraft_world::{Alertness, Behaviour, Character, Component, Entity, EntityId, World};
use serde_json::{Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: set_spec,
        handler: Handler::World(set),
    },
    Registration {
        spec: inspect_spec,
        handler: Handler::Query(inspect),
    },
    Registration {
        spec: cover_spec,
        handler: Handler::Query(cover),
    },
];

/// Candidates `query.cover` lists when `limit` is left out.
const DEFAULT_COVER_LIMIT: u64 = 20;

/// The numbers `ai.set` takes: name, smallest, largest, unit (for the sentences).
const NUMBERS: [(&str, f64, f64, &str); 10] = [
    ("skill", 0.0, 1.0, ""),
    ("precision", 0.0, 1.0, ""),
    ("courage", 0.0, 1.0, ""),
    ("spot_time_s", 0.0, BEHAVIOUR_TIME_MAX_S, " seconds"),
    ("aim_settle_s", 0.0, BEHAVIOUR_TIME_MAX_S, " seconds"),
    ("reaction_s", 0.0, BEHAVIOUR_TIME_MAX_S, " seconds"),
    ("calm_s", 0.0, BEHAVIOUR_TIME_MAX_S, " seconds"),
    ("aim_error_mrad", 0.0, AIM_ERROR_MAX_MRAD, " milliradians"),
    (
        "aim_error_floor_mrad",
        0.0,
        AIM_ERROR_MAX_MRAD,
        " milliradians",
    ),
    ("cover_search_m", 0.0, COVER_SEARCH_MAX_M, " metres"),
];

fn set_spec() -> CommandSpec {
    spec(
        "ai.set",
        "Set AI",
        "Sets how good soldiers are, as numbers (a pack keeps presets such as recruit or \
         veteran as data): `skill` scales reaction, spot time and how soon it looks for cover \
         again (1: as given, 0: twice as long); `precision` scales the aim error only (1: as \
         given, 0: four times as large); `spot_time_s` is how long an enemy in plain view at \
         half the view range takes to be fully known (0: at first sight; it reacts from \
         knowledge 0.25 and fires from 0.6); `aim_settle_s` and `aim_error_floor_mrad` let \
         the aim error shrink while the target stays in view; `courage` resists the stress of \
         near misses (1: none); `cover_search_m` above 0 makes it fight from cover in \
         combat; `state` puts it in an alertness and keeps it at least that alert. Leaves \
         the rest of the behaviour (team, mode, route, ranges) as it is; a character without \
         a behaviour gets one with the defaults. One command sets a squad: `entity` takes a \
         list and `tag:<tag>`. Works on the running world as on the document.",
        vec![
            param(
                "entity",
                T::Any,
                "Who: a character's id or name, `tag:<tag>` for every character with that tag, \
                 or a list of these (at most 256).",
            )
            .required(),
            param("skill", T::Number, "General skill, 0 to 1."),
            param("precision", T::Number, "Aim precision, 0 to 1."),
            param(
                "spot_time_s",
                T::Number,
                "Seconds to know an enemy at half the view range, 0 to 3600.",
            )
            .unit("s"),
            param(
                "aim_settle_s",
                T::Number,
                "Seconds over which the aim settles, 0 to 3600 (0: it does not).",
            )
            .unit("s"),
            param(
                "aim_error_mrad",
                T::Number,
                "The aim error at first sight, 0 to 1000.",
            )
            .unit("mrad"),
            param(
                "aim_error_floor_mrad",
                T::Number,
                "The aim error of a settled aim, 0 to 1000.",
            )
            .unit("mrad"),
            param("reaction_s", T::Number, "Reaction time, 0 to 3600.").unit("s"),
            param("courage", T::Number, "Courage against near misses, 0 to 1."),
            param(
                "cover_search_m",
                T::Number,
                "How far it looks for cover in combat, 0 to 100 (0: it does not).",
            )
            .unit("m"),
            param(
                "calm_s",
                T::Number,
                "Seconds without contact before it calms down a step, 0 to 3600.",
            )
            .unit("s"),
            param(
                "state",
                T::String,
                "The alertness to put it in and keep it at least at.",
            )
            .allowed(&Alertness::NAMES),
        ],
        "{set: [{id, name, behaviour: {skill, precision, spot_time_s, aim_settle_s, \
         aim_error_mrad, aim_error_floor_mrad, reaction_s, courage, cover_search_m, calm_s, \
         alertness}}]}",
        json!({"entity": "Scout", "skill": 0.8, "precision": 0.4, "spot_time_s": 2, "courage": 0.5}),
    )
}

/// The AI numbers of a behaviour.
fn numbers_json(b: &Behaviour) -> Value {
    json!({
        "skill": b.skill,
        "precision": b.precision,
        "spot_time_s": b.spot_time_s,
        "aim_settle_s": b.aim_settle_s,
        "aim_error_mrad": b.aim_error_mrad,
        "aim_error_floor_mrad": b.aim_error_floor_mrad,
        "reaction_s": b.reaction_s,
        "courage": b.courage,
        "cover_search_m": b.cover_search_m,
        "calm_s": b.calm_s,
        "alertness": b.alertness.name(),
    })
}

/// The number `name` within its range, or an error that names it.
fn number(
    p: &Params<'_>,
    name: &str,
    min: f64,
    max: f64,
    unit: &str,
) -> Result<Option<f64>, CommandError> {
    let Some(v) = p.f64(name)? else {
        return Ok(None);
    };
    if (min..=max).contains(&v) {
        Ok(Some(v))
    } else {
        Err(bad(format!(
            "`{name}` is {v}; it must be from {min} to {max}{unit} (nearest: {})",
            v.clamp(min, max)
        )))
    }
}

fn put(b: &mut Behaviour, name: &str, v: f64) {
    match name {
        "skill" => b.skill = v,
        "precision" => b.precision = v,
        "courage" => b.courage = v,
        "spot_time_s" => b.spot_time_s = v,
        "aim_settle_s" => b.aim_settle_s = v,
        "reaction_s" => b.reaction_s = v,
        "calm_s" => b.calm_s = v,
        "aim_error_mrad" => b.aim_error_mrad = v,
        "aim_error_floor_mrad" => b.aim_error_floor_mrad = v,
        "cover_search_m" => b.cover_search_m = v,
        _ => {}
    }
}

fn set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let mut given = Vec::new();
    for (name, min, max, unit) in NUMBERS {
        if let Some(v) = number(&p, name, min, max, unit)? {
            given.push((name, v));
        }
    }
    let state = match p.str("state")? {
        Some(s) => Some(Alertness::from_name(s).ok_or_else(|| {
            bad(format!(
                "`state` is \"{}\"; it must be one of {}",
                excerpt(s),
                Alertness::NAMES.join(", ")
            ))
        })?),
        None => None,
    };
    if given.is_empty() && state.is_none() {
        let names: Vec<&str> = NUMBERS.iter().map(|n| n.0).collect();
        return Err(bad(format!(
            "nothing to set; give at least one of {} or `state`",
            names.join(", ")
        )));
    }
    let targets = order_targets(&edit.world, &p)?;
    let mut set = Vec::new();
    for id in targets {
        let entity = edit
            .world
            .entity_mut(id)
            .ok_or_else(|| failed(format!("no entity {id}")))?;
        let mut b: Behaviour = entity.behaviour().cloned().unwrap_or_default();
        for (name, v) in &given {
            put(&mut b, name, *v);
        }
        if let Some(a) = state {
            b.alertness = a;
        }
        let summary = json!({"id": id.0, "name": entity.name, "behaviour": numbers_json(&b)});
        entity.set_component(Component::Behaviour(b));
        entity
            .validate()
            .map_err(|e| bad(format!("entity {id} (\"{}\"): {e}", entity.name)))?;
        set.push(summary);
    }
    Ok(json!({ "set": set }))
}

fn inspect_spec() -> CommandSpec {
    spec(
        "ai.inspect",
        "Inspect AI",
        "What a character's behaviour is made of and, while playing, what it knows and feels: \
         its numbers (skill, precision, spot time, aim, courage, cover, calm), the times and \
         aim error they make now, its state, its alertness, its stress from near misses, its \
         knowledge of every enemy it noticed (0 to 1, whether it sees it now, where it last \
         knew it to be) and the cover it chose (the point, the corners on the way, its peek \
         point and what it does there). While editing, the running parts are null; a \
         character without a behaviour has `behaviour` null.",
        vec![param("entity", T::Entity, "The character: its id or name.").required()],
        "{id, name, team, playing, behaviour: {skill, precision, ...} | null, effective: \
         {reaction_s, spot_time_s, aim_error_mrad} | null, state: {state, ...} | null, \
         alertness, calm_left_s, stress, knowledge: [{entity, name, knowledge, seen, \
         last_known}], cover: {point, via, peek, protection, score, phase, left_s} | null, \
         taken_over}",
        json!({"entity": "Scout"}),
    )
    .query()
}

/// A point list of the corners of a way.
fn via_json(c: &CoverPoint) -> Vec<[f64; 3]> {
    c.via.iter().flatten().copied().collect()
}

fn inspect(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let world = session.current_world();
    let id = p
        .entity_named("entity", world)?
        .ok_or_else(|| bad("missing `entity`, the character (id or name)"))?;
    let e = world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity {id}")))?;
    if e.character().is_none() {
        return Err(failed(format!(
            "`entity` names entity {id} (\"{}\"), which is not a character; only characters \
             have a behaviour",
            e.name
        )));
    }
    let sim = session.live();
    let brain = sim.and_then(|s| s.brain(id));
    let behaviour = e.behaviour();
    let effective = behaviour.map(|b| {
        let tf = ai::time_factor(b.skill);
        let (settled, stress) = brain.map_or((0.0, 0.0), |br| (br.aim_settled_s, br.stress));
        json!({
            "reaction_s": b.reaction_s * tf,
            "spot_time_s": b.spot_time_s * tf,
            "aim_error_mrad": ai::aim_error_mrad(b, settled, stress),
        })
    });
    let knowledge: Vec<Value> = brain
        .map(|br| {
            br.knowledge
                .iter()
                .map(|(other, k)| {
                    json!({
                        "entity": other.0,
                        "name": world.entity(*other).map(|o| o.name.as_str()),
                        "knowledge": k.knowledge,
                        "seen": k.seen,
                        "last_known": k.last_known,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let cover = brain.and_then(|br| br.cover).map(|c| {
        let (phase, left_s) = match c.phase {
            CoverPhase::Move { left_s } => ("move", left_s),
            CoverPhase::Hide { left_s } => ("hide", left_s),
            CoverPhase::Peek { left_s } => ("peek", left_s),
        };
        json!({
            "point": c.at.point,
            "via": via_json(&c.at),
            "peek": c.at.peek,
            "protection": c.at.protection,
            "score": c.at.score,
            "phase": phase,
            "left_s": left_s,
        })
    });
    Ok(json!({
        "id": id.0,
        "name": e.name,
        "team": team_of(e),
        "playing": sim.is_some(),
        "behaviour": behaviour.map(numbers_json),
        "effective": effective,
        "state": sim.map_or(Value::Null, |s| behaviour_json(s, id)),
        "alertness": brain.map(|br| br.alertness.name()),
        "calm_left_s": brain.map(|br| br.calm_left_s),
        "stress": brain.map(|br| br.stress),
        "knowledge": knowledge,
        "cover": cover,
        "taken_over": sim.is_some_and(|s| s.is_taken_over(id)),
    }))
}

fn cover_spec() -> CommandSpec {
    spec(
        "query.cover",
        "Cover",
        &format!(
            "Where a character (or a character standing at `position`) can take cover from \
             threats, with the simulation's own cover query, scored best first: candidates \
             where it stands, around every solid shape and model within `radius_m`, and on \
             two rings at half and all of it; `protection` is the share of the threats (at \
             most {MAX_COVER_THREATS}) that cannot see its chest there (terrain, shapes and \
             models block), `reachable` whether it can walk there (straight or round one or \
             two corners of the solid in the way, `via`; null when not checked, the 8 best \
             only), `peek` where it steps to fire at the first threat (the 8 best reachable \
             only), `crowding` how near friends are. `threat` is a point [x, y, z] or an \
             entity (a character's chest); left out, the threats the character's behaviour \
             knows of while playing (knowledge from {REACT_KNOWLEDGE})."
        ),
        vec![
            param(
                "entity",
                T::Entity,
                "The character looking for cover: its id or name. Or give `position`.",
            ),
            param(
                "position",
                T::Point3,
                "Where a default character stands (its feet), instead of `entity`.",
            )
            .unit("m"),
            param(
                "threat",
                T::Any,
                "Whom to hide from: a point [x, y, z] in metres or an entity (id or name).",
            ),
            param(
                "radius_m",
                T::Number,
                "How far around to look, above 0 and at most 100; default the behaviour's \
                 cover_search_m when above 0, else 15.",
            )
            .unit("m"),
            param("limit", T::Integer, "Most candidates listed, 1 to 64.")
                .default(json!(DEFAULT_COVER_LIMIT)),
        ],
        "{from: [x, y, z], radius_m, threats: [{point, entity}], count, candidates: [{point, \
         source: \"here\" | \"solid\" | \"ring\", protection, distance_m, via: [[x, y, z], \
         ...], reachable, peek, crowding, score}]}",
        json!({"entity": "Scout", "threat": [0, 1.5, -30]}),
    )
    .query()
}

/// The threat `threat` names, `None` when it is left out.
fn threat_param(
    p: &Params<'_>,
    world: &World,
    me: Option<EntityId>,
) -> Result<Option<Threat>, CommandError> {
    let Some(v) = p.value("threat") else {
        return Ok(None);
    };
    if v.is_array() {
        let point = p
            .point3("threat")?
            .ok_or_else(|| bad("`threat` must be a point [x, y, z] in metres, or an entity"))?;
        return Ok(Some(Threat {
            point,
            entity: None,
        }));
    }
    if !(v.is_string() || v.is_u64()) {
        return Err(bad(format!(
            "`threat` is {}; it must be a point [x, y, z] in metres, or an entity (its id or \
             name)",
            excerpt(&v.to_string())
        )));
    }
    let id = p
        .entity_named("threat", world)?
        .ok_or_else(|| bad("missing `threat`"))?;
    if Some(id) == me {
        return Err(bad(format!(
            "`threat` is the character itself (entity {id}); give another entity or a point"
        )));
    }
    let e = world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity {id}")))?;
    Ok(Some(Threat {
        point: aim_point(e),
        entity: Some(id),
    }))
}

/// Where the friends of `e` stand.
fn friends_of(world: &World, e: &Entity) -> Vec<[f64; 2]> {
    let Some(mine) = team_of(e) else {
        return Vec::new();
    };
    world
        .entities()
        .filter(|o| o.id != e.id && o.character().is_some() && !o.is_down())
        .filter(|o| team_of(o) == Some(mine))
        .map(|o| [o.position[0], o.position[2]])
        .collect()
}

fn cover(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let world = session.current_world();
    let entity = p.entity_named("entity", world)?;
    let position = p.point3("position")?;
    let me = match (entity, position) {
        (Some(_), Some(_)) => {
            return Err(bad(
                "give `entity` or `position`, not both: `position` is for a character that is \
                 not there",
            ));
        }
        (None, None) => {
            return Err(bad(
                "missing `entity` (the character looking for cover) or `position` (where one \
                 would stand)",
            ));
        }
        (Some(id), None) => {
            let e = world
                .entity(id)
                .ok_or_else(|| failed(format!("no entity {id}")))?;
            if e.character().is_none() {
                return Err(failed(format!(
                    "`entity` names entity {id} (\"{}\"), which is not a character; give a \
                     character or a `position`",
                    e.name
                )));
            }
            Some(e)
        }
        (None, Some(_)) => None,
    };
    let character = me
        .and_then(Entity::character)
        .cloned()
        .unwrap_or_else(Character::default);
    let feet = me.map_or(position.unwrap_or([0.0; 3]), |e| e.position);
    let radius = match p.f64("radius_m")? {
        Some(r) if r > 0.0 && r <= COVER_SEARCH_MAX_M => r,
        Some(r) => {
            return Err(bad(format!(
                "`radius_m` is {r}; it must be above 0 and at most {COVER_SEARCH_MAX_M} metres"
            )));
        }
        None => me
            .and_then(Entity::behaviour)
            .map(|b| b.cover_search_m)
            .filter(|r| *r > 0.0)
            .unwrap_or(DEFAULT_COVER_SEARCH_M),
    };
    let limit = p.u64("limit")?.unwrap_or(DEFAULT_COVER_LIMIT);
    if limit == 0 || limit > MAX_COVER_CANDIDATES as u64 {
        return Err(bad(format!(
            "`limit` is {limit}; it must be from 1 to {MAX_COVER_CANDIDATES}"
        )));
    }
    let me_id = me.map(|e| e.id);
    let threats: Vec<Threat> = match threat_param(&p, world, me_id)? {
        Some(t) => vec![t],
        None => {
            let known = me_id
                .and_then(|id| session.live().and_then(|s| s.brain(id)))
                .map(|br| {
                    let mut k: Vec<(EntityId, f64, [f64; 3])> = br
                        .knowledge
                        .iter()
                        .filter(|(_, k)| k.knowledge >= REACT_KNOWLEDGE)
                        .map(|(id, k)| (*id, k.knowledge, k.last_known))
                        .collect();
                    k.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                    k.into_iter()
                        .take(MAX_COVER_THREATS)
                        .map(|(id, _, point)| Threat {
                            point,
                            entity: Some(id),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if known.is_empty() {
                return Err(bad(
                    "missing `threat`: a point [x, y, z] or an entity to hide from (without it, \
                     the character's behaviour must know of a threat while playing)",
                ));
            }
            known
        }
    };
    let friends = me.map(|e| friends_of(world, e)).unwrap_or_default();
    let ask = CoverAsk {
        me: me_id,
        feet,
        height_m: character.height_m,
        radius_m: character.radius_m,
        eye_height_m: character.eye_height_m,
        search_m: radius,
        threats: &threats,
        friends: &friends,
    };
    let found = match session.live() {
        Some(sim) => sim.cover(&ask),
        None => evaluate_world(
            world,
            session.current_heights(),
            &session.current_trunks(),
            &ask,
        ),
    };
    let count = found.len();
    let candidates: Vec<Value> = found
        .iter()
        .take(usize::try_from(limit).unwrap_or(MAX_COVER_CANDIDATES))
        .map(|c| {
            json!({
                "point": c.point,
                "source": c.source,
                "protection": c.protection,
                "distance_m": c.distance_m,
                "via": via_json(c),
                "reachable": c.reachable,
                "peek": c.peek,
                "crowding": c.crowding,
                "score": c.score,
            })
        })
        .collect();
    Ok(json!({
        "from": feet,
        "radius_m": radius,
        "threats": threats
            .iter()
            .map(|t| json!({"point": t.point, "entity": t.entity.map(|id| id.0)}))
            .collect::<Vec<_>>(),
        "count": count,
        "candidates": candidates,
    }))
}
