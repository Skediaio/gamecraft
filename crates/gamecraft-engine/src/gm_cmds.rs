//! The game master's tools (`docs/guide/game-master.md`): `gm.overview`, one compact answer
//! to "what is going on"; `gm.sight`, who and what an entity sees, with the simulation's own
//! perception; and `gm.order`, which changes the mode and route of behaviours, one character
//! or a squad at a time, on a running world as on the document.

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Edit, Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, excerpt, param, spec};
use gamecraft_math::{Portable, yaw_forward};
use gamecraft_sim::aim::aim_point;
use gamecraft_sim::behaviour::team_of;
use gamecraft_sim::perception::{
    DEFAULT_FOV_DEG, DEFAULT_VIEW_RANGE_M, effective_range_m, light_factor, visibility_m,
};
use gamecraft_sim::{BrainState, Sim, ray};
use gamecraft_world::{Behaviour, BehaviourMode, Entity, EntityId, TAG_PREFIX, World};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: overview_spec,
        handler: Handler::Query(overview),
    },
    Registration {
        spec: sight_spec,
        handler: Handler::Query(sight),
    },
    Registration {
        spec: order_spec,
        handler: Handler::World(order),
    },
];

/// The radius of `gm.overview` around `near` when none is given, metres.
const DEFAULT_RADIUS_M: f64 = 100.0;
/// Most characters one `gm.overview` lists; the rest are counted.
const MAX_OVERVIEW_CHARACTERS: usize = 200;
/// Most entities other than characters one `gm.sight` lists, nearest first.
const MAX_SIGHT_ENTITIES: usize = 100;
/// Most entity references one `gm.order` takes.
const MAX_ORDER_ENTITIES: usize = 256;
/// Below this horizontal distance, metres, a thing counts as in view whatever the facing (as
/// the simulation's perception).
const MIN_FACING_DISTANCE_M: f64 = 1e-9;
/// The orders `gm.order` knows.
const ORDERS: [&str; 4] = ["patrol", "guard", "hold", "move"];

/// `v` with every number that is not a whole number rounded to centimetres (two decimals).
fn round_cm(v: Value) -> Value {
    match v {
        Value::Number(n) if n.is_f64() => {
            let x = n.as_f64().unwrap_or(0.0);
            let r = (x * 100.0).round() / 100.0;
            // -0.0 reads as 0.
            json!(if r == 0.0 { 0.0 } else { r })
        }
        Value::Array(items) => Value::Array(items.into_iter().map(round_cm).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, round_cm(v))).collect())
        }
        other => other,
    }
}

fn horizontal(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
    (dx * dx + dz * dz).sqrt()
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (dx, dy, dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

// ---------------------------------------------------------------------------------------------
// gm.overview

fn overview_spec() -> CommandSpec {
    spec(
        "gm.overview",
        "Game Master Overview",
        &format!(
            "One compact answer to what is going on, for a game master: the tick and time; \
             every character with its team, position, health, whether it is down, its \
             behaviour's mode and state (patrol, guard, alert, engage, search, return) and \
             target, the characters it sees (the simulation's own perception) and whether an \
             agent took it over; every team with its characters alive and down; the \
             objectives; the last 50 events (enter, leave, interact, hit, down, heard, time, \
             var, start) with their ticks; the messages showing; the weather with how far one \
             sees; the variables; the generators (forests, scatters) with how many instances \
             they placed (within `near` too, when given). Numbers are rounded to centimetres, entities named by id \
             (`names` gives the name of every id an event mentions). `near` and `team` narrow \
             the characters listed (at most {MAX_OVERVIEW_CHARACTERS}); the teams always \
             count everyone. While editing it describes the document, without states or \
             sight."
        ),
        vec![
            param(
                "near",
                T::Point2,
                "List only the characters within `radius_m` of this ground point [x, z].",
            )
            .unit("m"),
            param(
                "radius_m",
                T::Number,
                "With `near`: how far around it, above 0.",
            )
            .unit("m")
            .default(json!(DEFAULT_RADIUS_M)),
            param(
                "team",
                T::String,
                "List only the characters of this team (a behaviour's team or a `team:<name>` \
                 tag).",
            ),
        ],
        "{playing, tick (null while editing), time_s, hash, characters: [{id, name, team, \
         position, health (current, null without), down, mode, state, target?, sees: [ids] \
         (null while editing), taken_over}], characters_omitted?, teams: [{team, alive, down}], \
         objectives: [{id, title, status, hidden}], events: [{tick, event, who?, zone?, target?, \
         ...}], names: {id: name}, messages: [{text, to, recipients, shown_at_s, until_s}], \
         weather: {time_of_day_h, sun_elevation_deg, cloud_cover, mist, rain, \
         wind_direction_deg, wind_speed_mps, temperature_c, light, visibility_m (null in clear \
         air)}, vars: {name: value}, projectiles (in flight), generated: [{id, kind, placed, \
         near? (within `near` and `radius_m`)}]}",
        json!({"near": [0, 0], "radius_m": 50}),
    )
    .query()
}

/// The area filter of `gm.overview`: its centre and radius.
fn area(p: &Params<'_>) -> Result<Option<([f64; 2], f64)>, CommandError> {
    let radius = p.f64("radius_m")?;
    let Some(near) = p.point2("near")? else {
        if p.has("radius_m") {
            return Err(bad(
                "`radius_m` needs `near`, the ground point [x, z] it is measured from",
            ));
        }
        return Ok(None);
    };
    let radius = radius.unwrap_or(DEFAULT_RADIUS_M);
    if !(radius.is_finite() && radius > 0.0) {
        return Err(bad(format!(
            "`radius_m` is {radius}; it must be a number of metres above 0"
        )));
    }
    Ok(Some((near, radius)))
}

/// The teams of the characters of `world`: name to (alive, down); the characters without a
/// team as (alive, down).
fn teams(world: &World) -> (BTreeMap<String, (u64, u64)>, (u64, u64)) {
    let mut named: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut none = (0u64, 0u64);
    for e in world.entities().filter(|e| e.character().is_some()) {
        let slot = match team_of(e) {
            Some(t) => named.entry(t.to_string()).or_insert((0, 0)),
            None => &mut none,
        };
        if e.is_down() {
            slot.1 = slot.1.saturating_add(1);
        } else {
            slot.0 = slot.0.saturating_add(1);
        }
    }
    (named, none)
}

/// One character of the overview.
fn character_json(world: &World, sim: Option<&Sim>, e: &Entity) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(e.id.0));
    m.insert("name".into(), json!(e.name));
    m.insert("team".into(), json!(team_of(e)));
    m.insert("position".into(), json!(e.position));
    m.insert("health".into(), json!(e.health().map(|h| h.current)));
    m.insert("down".into(), json!(e.is_down()));
    m.insert("mode".into(), json!(e.behaviour().map(|b| b.mode.name())));
    let brain = sim.and_then(|s| s.brain(e.id));
    m.insert("state".into(), json!(brain.map(|b| b.state.name())));
    if let Some(b) = brain
        && let BrainState::Alert { target, .. } | BrainState::Engage { target } = b.state
    {
        m.insert("target".into(), json!(target.0));
    }
    let sees = sim.map(|s| {
        world
            .entities()
            .filter(|o| o.id != e.id && o.character().is_some() && s.sees(e.id, o.id))
            .map(|o| o.id.0)
            .collect::<Vec<_>>()
    });
    m.insert("sees".into(), json!(sees));
    m.insert(
        "taken_over".into(),
        json!(sim.is_some_and(|s| s.is_taken_over(e.id))),
    );
    Value::Object(m)
}

/// The ids an event mentions.
fn event_ids(event: &Value) -> impl Iterator<Item = u64> + '_ {
    ["who", "target", "zone", "shooter"]
        .into_iter()
        .filter_map(|k| event.get(k).and_then(Value::as_u64))
}

fn overview(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let world = session.current_world();
    let sim = session.live();
    let area = area(&p)?;
    let (named, none) = teams(world);
    let team = p.str("team")?;
    if let Some(t) = team
        && !named.contains_key(t)
    {
        let known: Vec<&str> = named.keys().map(String::as_str).collect();
        return Err(failed(format!(
            "`team`: no character is of the team \"{}\"; the teams are: {}",
            excerpt(t),
            if known.is_empty() {
                "none".to_string()
            } else {
                known.join(", ")
            }
        )));
    }
    let mut characters = Vec::new();
    let mut omitted = 0usize;
    for e in world.entities().filter(|e| e.character().is_some()) {
        if let Some(([x, z], r)) = area
            && horizontal([x, 0.0, z], e.position) > r
        {
            continue;
        }
        if let Some(t) = team
            && team_of(e) != Some(t)
        {
            continue;
        }
        if characters.len() >= MAX_OVERVIEW_CHARACTERS {
            omitted += 1;
            continue;
        }
        characters.push(character_json(world, sim, e));
    }
    let mut team_list: Vec<Value> = named
        .iter()
        .map(|(t, (alive, down))| json!({"team": t, "alive": alive, "down": down}))
        .collect();
    if none != (0, 0) {
        team_list.push(json!({"team": null, "alive": none.0, "down": none.1}));
    }
    let events: Vec<Value> = if sim.is_some() {
        session.play().recent.iter().cloned().collect()
    } else {
        Vec::new()
    };
    let ids: BTreeSet<u64> = events.iter().flat_map(event_ids).collect();
    let names: Map<String, Value> = ids
        .into_iter()
        .filter_map(|id| {
            world
                .entity(EntityId(id))
                .map(|e| (id.to_string(), json!(e.name)))
        })
        .collect();
    let env = &world.environment;
    let visibility = visibility_m(env);
    let weather = json!({
        "time_of_day_h": env.time_of_day_h,
        "sun_elevation_deg": env.sun().elevation_deg,
        "cloud_cover": env.cloud_cover,
        "mist": env.mist,
        "rain": env.rain,
        "wind_direction_deg": env.wind_direction_deg,
        "wind_speed_mps": env.wind_speed_mps,
        "temperature_c": env.temperature_c,
        "light": light_factor(env),
        "visibility_m": visibility.is_finite().then_some(visibility),
    });
    let mut out = Map::new();
    out.insert("playing".into(), json!(sim.is_some()));
    out.insert("tick".into(), json!(sim.map(Sim::tick)));
    out.insert("time_s".into(), json!(sim.map(Sim::time_s)));
    out.insert(
        "hash".into(),
        json!(sim.map(|s| format!("{:016x}", s.state_hash()))),
    );
    out.insert("characters".into(), Value::Array(characters));
    if omitted > 0 {
        out.insert("characters_omitted".into(), json!(omitted));
    }
    out.insert("teams".into(), Value::Array(team_list));
    out.insert(
        "objectives".into(),
        serde_json::to_value(&world.objectives).map_err(failed)?,
    );
    out.insert("events".into(), Value::Array(events));
    out.insert("names".into(), Value::Object(names));
    out.insert(
        "messages".into(),
        crate::scenario_cmds::messages_json(session, None),
    );
    out.insert("weather".into(), weather);
    out.insert(
        "vars".into(),
        serde_json::to_value(&world.vars).map_err(failed)?,
    );
    out.insert(
        "projectiles".into(),
        json!(sim.map_or(0, |s| s.projectiles().len())),
    );
    let generated: Vec<Value> = world
        .generated()
        .sets()
        .map(|set| {
            let mut m = Map::new();
            m.insert("id".into(), json!(set.generator().id));
            m.insert("kind".into(), json!(set.generator().kind.name()));
            m.insert("placed".into(), json!(set.instances().len()));
            if let Some(([x, z], r)) = area {
                let n = set
                    .near([x, z], r)
                    .into_iter()
                    .filter(|i| horizontal([x, 0.0, z], i.position) <= r)
                    .count();
                m.insert("near".into(), json!(n));
            }
            Value::Object(m)
        })
        .collect();
    out.insert("generated".into(), Value::Array(generated));
    Ok(round_cm(Value::Object(out)))
}

// ---------------------------------------------------------------------------------------------
// gm.sight

fn sight_spec() -> CommandSpec {
    spec(
        "gm.sight",
        "Game Master Sight",
        &format!(
            "Who and what a character sees now, with the simulation's own perception (the \
             function enemies decide by): its view range (a behaviour's `view_range_m`, else \
             {DEFAULT_VIEW_RANGE_M} m) shortened by night, mist and rain, its field of view \
             (a behaviour's `fov_deg`, else {DEFAULT_FOV_DEG} degrees) around its facing, and \
             a clear line of sight. `characters` are the characters it sees; `entities` the \
             shapes, zones, interactables and lights whose middle it sees by the same rules, \
             nearest first, at most {MAX_SIGHT_ENTITIES} (plants are left out). While editing \
             it looks at the document."
        ),
        vec![
            param(
                "from",
                T::Entity,
                "The character who looks: its id or name.",
            )
            .required(),
        ],
        "{from, name, eye: [x, y, z], facing_deg, range_m, fov_deg, light, visibility_m (null \
         in clear air), characters: [{id, name, team, enemy, down, distance_m}], entities: \
         [{id, name, kind, distance_m}], entities_omitted?}",
        json!({"from": "Scout"}),
    )
    .query()
}

/// The kind of a thing `gm.sight` lists, `None` for what it leaves out.
fn thing_kind(e: &Entity) -> Option<&'static str> {
    if e.character().is_some() {
        None
    } else if e.shape().is_some() {
        Some("shape")
    } else if e.zone().is_some() {
        Some("zone")
    } else if e.interactable().is_some() {
        Some("interactable")
    } else if e.light().is_some() {
        Some("light")
    } else {
        None
    }
}

fn sight(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let built;
    let sim = match session.live() {
        Some(sim) => sim,
        None => {
            built = Sim::with_heights(
                session.document().clone(),
                session.document_heights_shared(),
            )
            .map_err(failed)?;
            &built
        }
    };
    let world = sim.world();
    let id = p
        .entity_named("from", world)?
        .ok_or_else(|| bad("missing `from`, the character who looks (id or name)"))?;
    let me = world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity {id}")))?;
    let Some(body) = me.character() else {
        return Err(failed(format!(
            "`from` names entity {id} (\"{}\"), which is not a character; only characters \
             see (give one a `character` component)",
            me.name
        )));
    };
    let (view, fov) = me
        .behaviour()
        .map_or((DEFAULT_VIEW_RANGE_M, DEFAULT_FOV_DEG), |b| {
            (b.view_range_m, b.fov_deg)
        });
    let env = &world.environment;
    let range = effective_range_m(env, view);
    let [x, y, z] = me.position;
    let eye = [x, y + body.eye_height_m, z];
    let mine = team_of(me);
    let characters: Vec<Value> = world
        .entities()
        .filter(|o| o.id != id && o.character().is_some() && sim.sees(id, o.id))
        .map(|o| {
            let theirs = team_of(o);
            json!({
                "id": o.id.0,
                "name": o.name,
                "team": theirs,
                "enemy": matches!((mine, theirs), (Some(a), Some(b)) if a != b),
                "down": o.is_down(),
                "distance_m": distance(me.position, o.position),
            })
        })
        .collect();
    let forward = yaw_forward(me.rotation_y_deg);
    let half = (fov / 2.0).to_radians().pcos();
    let mut things: Vec<(f64, &Entity, &str)> = Vec::new();
    for o in world.entities() {
        let Some(kind) = thing_kind(o) else {
            continue;
        };
        let to = aim_point(o);
        let d = distance(eye, to);
        if !(d.is_finite() && d <= range) {
            continue;
        }
        let (dx, dz) = (to[0] - eye[0], to[2] - eye[2]);
        let flat = (dx * dx + dz * dz).sqrt();
        if flat > MIN_FACING_DISTANCE_M
            && fov < 360.0
            && (forward.x * dx + forward.z * dz) / flat < half
        {
            continue;
        }
        let clear = ray::line_of_sight(world, sim.heights(), eye, to, |e| e == id || e == o.id)
            .is_some_and(|s| s.blocked_by.is_none());
        if clear {
            things.push((d, o, kind));
        }
    }
    things.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
    let omitted = things.len().saturating_sub(MAX_SIGHT_ENTITIES);
    let entities: Vec<Value> = things
        .into_iter()
        .take(MAX_SIGHT_ENTITIES)
        .map(|(d, o, kind)| json!({"id": o.id.0, "name": o.name, "kind": kind, "distance_m": d}))
        .collect();
    let visibility = visibility_m(env);
    let mut out = Map::new();
    out.insert("from".into(), json!(id.0));
    out.insert("name".into(), json!(me.name));
    out.insert("eye".into(), json!(eye));
    out.insert("facing_deg".into(), json!(me.rotation_y_deg));
    out.insert("range_m".into(), json!(range));
    out.insert("fov_deg".into(), json!(fov));
    out.insert("light".into(), json!(light_factor(env)));
    out.insert(
        "visibility_m".into(),
        json!(visibility.is_finite().then_some(visibility)),
    );
    out.insert("characters".into(), Value::Array(characters));
    out.insert("entities".into(), Value::Array(entities));
    if omitted > 0 {
        out.insert("entities_omitted".into(), json!(omitted));
    }
    Ok(round_cm(Value::Object(out)))
}

// ---------------------------------------------------------------------------------------------
// gm.order

fn order_spec() -> CommandSpec {
    spec(
        "gm.order",
        "Game Master Order",
        "Changes what characters do when nothing catches their attention, without replacing \
         the rest of their behaviour (team, ranges, aim): `patrol` walks `route` (or the route \
         it has) point after point, starting at its first; `move` walks to `point` and stays \
         there (a patrol of one point); `hold` stands where it is (mode idle); `guard` goes \
         back to its post (where it stood when its behaviour started) and stands there. A \
         character without a behaviour gets one with the defaults (no team). One command \
         orders a squad: `entity` takes a list and `tag:<tag>`. A character that is fighting \
         or searching finishes that first and then follows the order; one an agent took over \
         follows it after sim.release. Works on the running world as on the document.",
        vec![
            param(
                "entity",
                T::Any,
                "Who: a character's id or name, `tag:<tag>` for every character with that tag, \
                 or a list of these (at most 256).",
            )
            .required(),
            param("order", T::String, "What to do.")
                .required()
                .allowed(&ORDERS),
            param(
                "route",
                T::Array,
                "patrol: the points [[x, z], ...] to walk, 1 to 256; left out, the route it has.",
            )
            .unit("m"),
            param("point", T::Point2, "move: where to go, [x, z].").unit("m"),
        ],
        "{ordered: [{id, name, mode, route}]}",
        json!({"entity": "Scout", "order": "move", "point": [10, -20]}),
    )
}

/// The characters one reference names: an id, a name, or `tag:<tag>`.
fn resolve_one(world: &World, v: &Value, out: &mut BTreeSet<EntityId>) -> Result<(), CommandError> {
    let reference = match v {
        Value::Number(n) => n.as_u64().map(|n| n.to_string()).ok_or_else(|| {
            bad(format!(
                "`entity` holds {}; an id is a whole number from 1",
                excerpt(&v.to_string())
            ))
        })?,
        Value::String(s) => s.clone(),
        other => {
            return Err(bad(format!(
                "`entity` holds {}; give a character's id or name, `tag:<tag>`, or a list of \
                 these",
                excerpt(&other.to_string())
            )));
        }
    };
    if let Some(tag) = reference.strip_prefix(TAG_PREFIX) {
        let found: Vec<EntityId> = world
            .entities()
            .filter(|e| e.character().is_some() && e.has_tag(tag))
            .map(|e| e.id)
            .collect();
        if found.is_empty() {
            return Err(failed(format!(
                "`entity`: no character has the tag \"{}\"",
                excerpt(tag)
            )));
        }
        out.extend(found);
        return Ok(());
    }
    let id = world
        .find(&reference)
        .map_err(|e| failed(format!("`entity`: {e}")))?;
    let e = world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity {id}")))?;
    if e.character().is_none() {
        return Err(failed(format!(
            "`entity` names entity {id} (\"{}\"), which is not a character; only characters \
             take orders",
            e.name
        )));
    }
    out.insert(id);
    Ok(())
}

/// The characters `entity` names, in id order.
fn order_targets(world: &World, p: &Params<'_>) -> Result<Vec<EntityId>, CommandError> {
    let v = p
        .value("entity")
        .ok_or_else(|| bad("missing `entity`: a character, `tag:<tag>` or a list"))?;
    let mut out = BTreeSet::new();
    match v {
        Value::Array(items) => {
            if items.is_empty() || items.len() > MAX_ORDER_ENTITIES {
                return Err(bad(format!(
                    "`entity` lists {} references; give 1 to {MAX_ORDER_ENTITIES}",
                    items.len()
                )));
            }
            for item in items {
                resolve_one(world, item, &mut out)?;
            }
        }
        other => resolve_one(world, other, &mut out)?,
    }
    Ok(out.into_iter().collect())
}

/// `route` as points [x, z].
fn route_param(p: &Params<'_>) -> Result<Option<Vec<[f64; 2]>>, CommandError> {
    let Some(v) = p.value("route") else {
        return Ok(None);
    };
    let wrong = || bad("`route` must be a list of points [[x, z], ...] in metres");
    let items = v.as_array().ok_or_else(wrong)?;
    if items.is_empty() {
        return Err(bad(
            "`route` is empty; a patrol needs at least one point [x, z]",
        ));
    }
    items
        .iter()
        .map(|item| match item.as_array().map(Vec::as_slice) {
            Some([x, z]) => match (x.as_f64(), z.as_f64()) {
                (Some(x), Some(z)) if x.is_finite() && z.is_finite() => Ok([x, z]),
                _ => Err(wrong()),
            },
            _ => Err(wrong()),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn order(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let what = p.str("order")?.ok_or_else(|| bad("missing `order`"))?;
    let route = route_param(&p)?;
    let point = p.point2("point")?;
    if route.is_some() && what != "patrol" {
        return Err(bad(format!(
            "`route` is for the order patrol, not {what}; leave it out (move takes `point`)"
        )));
    }
    if point.is_some() && what != "move" {
        return Err(bad(format!(
            "`point` is for the order move, not {what}; leave it out (patrol takes `route`)"
        )));
    }
    if what == "move" && point.is_none() {
        return Err(bad(
            "the order move needs `point`, the ground point [x, z] to go to",
        ));
    }
    let targets = order_targets(&edit.world, &p)?;
    let mut ordered = Vec::new();
    for id in targets {
        let entity = edit
            .world
            .entity_mut(id)
            .ok_or_else(|| failed(format!("no entity {id}")))?;
        let mut b: Behaviour = entity.behaviour().cloned().unwrap_or_default();
        match what {
            "patrol" => {
                if let Some(r) = &route {
                    b.route.clone_from(r);
                }
                if b.route.is_empty() {
                    return Err(bad(format!(
                        "entity {id} (\"{}\") has no route to patrol; give `route`",
                        entity.name
                    )));
                }
                b.mode = BehaviourMode::Patrol;
            }
            "move" => {
                b.route = point.into_iter().collect();
                b.mode = BehaviourMode::Patrol;
            }
            "hold" => b.mode = BehaviourMode::Idle,
            "guard" => b.mode = BehaviourMode::Guard,
            other => {
                return Err(bad(format!(
                    "`order` is \"{}\"; it must be one of {}",
                    excerpt(other),
                    ORDERS.join(", ")
                )));
            }
        }
        let summary =
            json!({"id": id.0, "name": entity.name, "mode": b.mode.name(), "route": b.route});
        entity.set_component(gamecraft_world::Component::Behaviour(b));
        entity
            .validate()
            .map_err(|e| bad(format!("entity {id} (\"{}\"): {e}", entity.name)))?;
        ordered.push(summary);
    }
    Ok(json!({"ordered": ordered}))
}
