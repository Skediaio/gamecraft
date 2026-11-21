//! `world.inspect`: everything about the current world as JSON.

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use serde_json::{Map, Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[Registration {
    spec: inspect_spec,
    handler: Handler::Query(inspect),
}];

/// Most entities one `world.inspect` lists.
pub const MAX_INSPECT_LIMIT: u64 = 1000;
/// Entities `world.inspect` lists when `limit` is left out.
const DEFAULT_INSPECT_LIMIT: u64 = 200;

fn inspect_spec() -> CommandSpec {
    spec(
        "world.inspect",
        "Inspect World",
        "Describes the current world (the running world while playing): name, seed, terrain, \
         environment and sun, the view rules, the prefabs, the scenario's variables, \
         objectives and trigger count (scenario.inspect has the triggers), and its entities in \
         id order, a page at a time.",
        vec![
            param(
                "entities",
                T::Boolean,
                "List the entities; false leaves the list out.",
            )
            .default(json!(true)),
            param("limit", T::Integer, "Most entities listed, 1 to 1000.").default(json!(200)),
            param("offset", T::Integer, "Entities skipped first, in id order.").default(json!(0)),
        ],
        "{name, seed, revision, playing, tick (null when not playing), terrain: null | \
         {size_m, cell_m, source, layers: [names], paint_rules, edits (counts; terrain.edits \
         lists them), cols, rows, min_height_m, max_height_m}, environment, sun: \
         {azimuth_deg, elevation_deg, direction, above_horizon}, view: {modes, fov_y_deg}, \
         entity_count, vars: {name: \
         value}, objectives: [{id, title, status, hidden}], trigger_count, prefabs: [{id, \
         name, components, children, instances, instance_count}], entities: [{id, name, \
         position, rotation_y_deg, scale, prefab?, parent?, layer? (left out: the default layer), \
         components (effective)}], \
         truncated, history: {can_undo, \
         can_redo}}",
        json!({"limit": 10}),
    )
    .query()
}

fn inspect(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let list = p.bool("entities")?.unwrap_or(true);
    let limit = p.u64("limit")?.unwrap_or(DEFAULT_INSPECT_LIMIT);
    if !(1..=MAX_INSPECT_LIMIT).contains(&limit) {
        return Err(bad(format!(
            "`limit` is {limit}; it must be from 1 to {MAX_INSPECT_LIMIT}; page with `offset`"
        )));
    }
    let offset = p.u64("offset")?.unwrap_or(0);
    let world = session.current_world();
    let terrain = match &world.terrain {
        None => Value::Null,
        Some(t) => {
            let source = serde_json::to_value(&t.source).map_err(failed)?;
            let mut v = Map::new();
            v.insert("size_m".into(), json!(t.size_m));
            v.insert("cell_m".into(), json!(t.cell_m));
            v.insert("source".into(), source);
            let layers: Vec<&str> = t.layers.iter().map(|l| l.name.as_str()).collect();
            v.insert("layers".into(), json!(layers));
            v.insert("paint_rules".into(), json!(t.paint_rules.len()));
            v.insert("edits".into(), json!(t.edits.len()));
            if let Some(field) = session.current_heights() {
                let (lo, hi) = field.height_range();
                v.insert("cols".into(), json!(field.cols()));
                v.insert("rows".into(), json!(field.rows()));
                v.insert("min_height_m".into(), json!(f64::from(lo)));
                v.insert("max_height_m".into(), json!(f64::from(hi)));
            }
            Value::Object(v)
        }
    };
    let sun = world.environment.sun();
    let summary = json!({
        "name": world.name,
        "seed": world.seed,
        "revision": session.revision(),
        "playing": session.is_playing(),
        "tick": session.live().map(|s| s.tick()),
        "terrain": terrain,
        "environment": crate::world_cmds::environment_json(&world.environment)?,
        "sun": {
            "azimuth_deg": sun.azimuth_deg,
            "elevation_deg": sun.elevation_deg,
            "direction": sun.direction,
            "above_horizon": sun.elevation_deg > 0.0,
        },
        "view": serde_json::to_value(&world.view).map_err(failed)?,
        "entity_count": world.entity_count(),
        "vars": serde_json::to_value(&world.vars).map_err(failed)?,
        "objectives": serde_json::to_value(&world.objectives).map_err(failed)?,
        "trigger_count": world.triggers.len(),
        "history": {"can_undo": session.can_undo(), "can_redo": session.can_redo()},
    });
    let Value::Object(mut out) = summary else {
        return Err(failed("the world summary is not an object"));
    };
    let prefabs = world
        .prefabs
        .iter()
        .map(|prefab| crate::prefab_cmds::prefab_json(world, prefab))
        .collect::<Result<Vec<_>, _>>()?;
    out.insert("prefabs".into(), Value::Array(prefabs));
    if list {
        let skip = usize::try_from(offset).unwrap_or(usize::MAX);
        let take = usize::try_from(limit).unwrap_or(0);
        let mut entities = Vec::new();
        for e in world.entities().skip(skip).take(take) {
            entities.push(serde_json::to_value(e).map_err(failed)?);
        }
        out.insert("entities".into(), Value::Array(entities));
        out.insert(
            "truncated".into(),
            json!(skip.saturating_add(take) < world.entity_count()),
        );
    }
    Ok(Value::Object(out))
}
