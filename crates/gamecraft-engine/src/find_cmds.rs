//! Looking up entities: `entity.get` and `entity.find`.

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use gamecraft_world::{Component, Entity};
use serde_json::{Map, Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: get_spec,
        handler: Handler::Query(get),
    },
    Registration {
        spec: find_spec,
        handler: Handler::Query(find),
    },
];

/// Most entities one `entity.find` lists.
const MAX_FIND_LIMIT: u64 = 1000;
/// Entities `entity.find` lists when `limit` is left out.
const DEFAULT_FIND_LIMIT: u64 = 50;

fn get_spec() -> CommandSpec {
    spec(
        "entity.get",
        "Get Entity",
        "Everything about one entity of the current world (the running world while playing): \
         its placement and components, the ground beneath it, and while playing a character's \
         body and stored input.",
        vec![
            param(
                "id",
                T::Entity,
                "The entity: its id as a number, or its name as a string (any case).",
            )
            .required(),
        ],
        "{id, name, position, rotation_y_deg, scale, components: [...], ground_height_m: \
         number | null (null without ground beneath), height_above_ground_m: number | null, \
         body: null | {velocity, grounded} (a character while playing), input: null | \
         {forward, right, yaw_deg, run, jump} (the input stored for a character while \
         playing)}",
        json!({"id": "Scout"}),
    )
    .query()
}

fn get(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let world = session.current_world();
    let id = p.required_entity("id", world)?;
    let entity = world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    let [x, y, z] = entity.position;
    let ground = session.current_heights().and_then(|f| f.height_at(x, z));
    let body = session
        .live()
        .and_then(|sim| sim.body(id))
        .map(|b| json!({"velocity": b.velocity, "grounded": b.grounded}));
    let input = match session.live().and_then(|sim| sim.input(id)) {
        Some(i) => serde_json::to_value(i).map_err(failed)?,
        None => Value::Null,
    };
    Ok(json!({
        "id": id.0,
        "name": entity.name,
        "position": entity.position,
        "rotation_y_deg": entity.rotation_y_deg,
        "scale": entity.scale,
        "components": serde_json::to_value(&entity.components).map_err(failed)?,
        "ground_height_m": ground,
        "height_above_ground_m": ground.map(|g| y - g),
        "body": body,
        "input": input,
    }))
}

fn find_spec() -> CommandSpec {
    spec(
        "entity.find",
        "Find Entities",
        "Lists the entities of the current world that match every filter given: part of the \
         name, a component type, a tag, and nearness to a point on the ground plane. Sorted by \
         distance then id with `near`, by id otherwise; `total` counts all matches, also those \
         beyond `limit`.",
        vec![
            param(
                "name",
                T::String,
                "Part of the name, in any case (`oak` finds `Old Oak 3`).",
            ),
            param(
                "component",
                T::String,
                "Only entities with a component of this type.",
            )
            .allowed(&Component::KINDS),
            param(
                "tag",
                T::String,
                "Only entities whose `tags` component holds this tag, matched exactly.",
            ),
            param(
                "near",
                T::Point2,
                "Measure the horizontal distance from this point [x, z] and sort by it.",
            )
            .unit("m"),
            param(
                "radius_m",
                T::Number,
                "Only entities at most this far from `near` (needs `near`), 0 or more.",
            )
            .unit("m"),
            param("limit", T::Integer, "Most entities listed, 1 to 1000.").default(json!(50)),
        ],
        "{total, entities: [{id, name, position, components: [types], distance_m (with \
         near)}]}",
        json!({"component": "plant", "near": [0, 0], "radius_m": 30}),
    )
    .query()
}

fn find(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let name = p.str("name")?.map(str::to_lowercase);
    let component = p.str("component")?;
    let tag = p.str("tag")?;
    let near = p.point2("near")?;
    let radius = p.f64("radius_m")?;
    if radius.is_some() && near.is_none() {
        return Err(bad(
            "`radius_m` needs `near` [x, z], the point the radius is measured from",
        ));
    }
    if let Some(r) = radius
        && r < 0.0
    {
        return Err(bad(format!("`radius_m` is {r}; it must be 0 or more")));
    }
    let limit = p.u64("limit")?.unwrap_or(DEFAULT_FIND_LIMIT);
    if !(1..=MAX_FIND_LIMIT).contains(&limit) {
        return Err(bad(format!(
            "`limit` is {limit}; it must be from 1 to {MAX_FIND_LIMIT}"
        )));
    }
    let distance = |e: &Entity| {
        near.map(|[nx, nz]| {
            let [x, _, z] = e.position;
            let (dx, dz) = (x - nx, z - nz);
            (dx * dx + dz * dz).sqrt()
        })
    };
    let mut matches: Vec<(Option<f64>, &Entity)> = session
        .current_world()
        .entities()
        .filter(|e| {
            name.as_ref()
                .is_none_or(|n| e.name.to_lowercase().contains(n.as_str()))
        })
        .filter(|e| component.is_none_or(|k| e.components.iter().any(|c| c.kind() == k)))
        .filter(|e| tag.is_none_or(|t| e.has_tag(t)))
        .map(|e| (distance(e), e))
        .filter(|(d, _)| match (d, radius) {
            (Some(d), Some(r)) => *d <= r,
            _ => true,
        })
        .collect();
    if near.is_some() {
        matches.sort_by(|(da, a), (db, b)| {
            da.unwrap_or(f64::INFINITY)
                .total_cmp(&db.unwrap_or(f64::INFINITY))
                .then(a.id.cmp(&b.id))
        });
    }
    let total = matches.len();
    let take = usize::try_from(limit).unwrap_or(usize::MAX);
    let entities: Vec<Value> = matches
        .into_iter()
        .take(take)
        .map(|(d, e)| {
            let mut v = Map::new();
            v.insert("id".into(), json!(e.id.0));
            v.insert("name".into(), json!(e.name));
            v.insert("position".into(), json!(e.position));
            let kinds: Vec<&str> = e.components.iter().map(Component::kind).collect();
            v.insert("components".into(), json!(kinds));
            if let Some(d) = d {
                v.insert("distance_m".into(), json!(d));
            }
            Value::Object(v)
        })
        .collect();
    Ok(json!({"total": total, "entities": entities}))
}
