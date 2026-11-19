//! Looking up entities: `entity.get` and `entity.find`.

use crate::combat_cmds::{BEHAVIOUR_DOC, behaviour_json, health_json, magazine_json};
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
        "Everything about one entity of the current world (the running world while playing), \
         or about a generated instance (`<generator>#<n>`, marked `generated`): \
         its placement, its layer, its prefab and overrides, its parent and children, its effective \
         components, the ground beneath it, its health, and while playing a \
         character's body and stored input, its behaviour's state and its launcher's magazine.",
        vec![
            param(
                "id",
                T::Entity,
                "The entity: its id as a number, or its name as a string (any case); or a \
                 generated instance as `<generator>#<n>` (`pines#12`).",
            )
            .required(),
        ],
        &format!(
            "{{id, name, position, rotation_y_deg, scale, prefab: id | null, overrides: [...] \
             (its components that differ from its prefab's), parent: id | null, children: \
             [ids], layer: name | null (null: the world's default layer), components: [...] (the effective ones), ground_height_m: \
             number | null (null without ground beneath), height_above_ground_m: number | null, \
             body: null | {{velocity, grounded}} (a character while playing), input: null | \
             {{forward, right, yaw_deg, pitch_deg, run, jump, interact, fire}} (the input \
             stored for a character while playing), health: null | {{max, current, armor, \
             down}}, behaviour: null | {BEHAVIOUR_DOC} (while playing), magazine: null | \
             {{left, shots, ready_in_s, reloading}} (a launcher while playing)}}"
        ),
        json!({"id": "Scout"}),
    )
    .query()
}

/// `entity.get` of a generated instance `<generator>#<n>`.
fn get_generated(session: &Session, reference: &str) -> Option<Value> {
    let world = session.current_world();
    let (set, instance) = world.generated().find(reference)?;
    let [x, y, z] = instance.position;
    let ground = session.current_heights().and_then(|f| f.height_at(x, z));
    let components = serde_json::to_value(set.components(instance)).ok()?;
    let trunk = set
        .trunk(instance)
        .map(|t| json!({"radius_m": t.radius_m, "height_m": t.height_m}));
    Some(json!({
        "id": set.instance_id(instance),
        "generated": true,
        "generator": set.generator().id,
        "number": instance.number,
        "item": instance.item,
        "name": set.instance_id(instance),
        "position": instance.position,
        "rotation_y_deg": instance.rotation_y_deg,
        "scale": instance.scale,
        "components": components,
        "trunk": trunk,
        "ground_height_m": ground,
        "height_above_ground_m": ground.map(|g| y - g),
    }))
}

fn get(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let world = session.current_world();
    if let Some(reference) = p.value("id").and_then(Value::as_str)
        && reference.contains('#')
        && world.find(reference).is_err()
    {
        return get_generated(session, reference).ok_or_else(|| {
            failed(format!(
                "no generated instance is called {:?}; a generated instance is `<generator>#<n>` \
                 (generator.list gives the generators, entity.find with `name` \
                 \"<generator>#\" their instances)",
                crate::spec::excerpt(reference)
            ))
        });
    }
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
        "prefab": entity.prefab,
        "overrides": serde_json::to_value(world.overrides_of(id)).map_err(failed)?,
        "parent": entity.parent.map(|p| p.0),
        "layer": entity.layer,
        "children": world.children(id).iter().map(|c| c.0).collect::<Vec<_>>(),
        "components": serde_json::to_value(&entity.components).map_err(failed)?,
        "ground_height_m": ground,
        "height_above_ground_m": ground.map(|g| y - g),
        "body": body,
        "input": input,
        "health": health_json(entity),
        "behaviour": session.live().map_or(Value::Null, |sim| behaviour_json(sim, id)),
        "magazine": session.live().map_or(Value::Null, |sim| magazine_json(sim, id)),
    }))
}

fn find_spec() -> CommandSpec {
    spec(
        "entity.find",
        "Find Entities",
        "Lists the entities of the current world that match every filter given: part of the \
         name, a component type, a tag, and nearness to a point on the ground plane; with \
         them the generated instances (`<generator>#<n>`, marked `generated`; their name is \
         their id), unless `generated` is false. Sorted by distance then id with `near`, by id \
         otherwise (entities before generated instances, which go by generator then number); \
         `total` counts all matches, also those beyond `limit`.",
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
            param(
                "generated",
                T::Boolean,
                "Include the generators' instances; false lists entities only.",
            )
            .default(json!(true)),
        ],
        "{total, entities: [{id (a number, or `<generator>#<n>` for a generated instance), \
         name, position, components: [types], generated? (true for an instance), distance_m \
         (with near)}]}",
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
    let with_generated = p.bool("generated")?.unwrap_or(true);
    let distance_to = |[x, _, z]: [f64; 3]| {
        near.map(|[nx, nz]| {
            let (dx, dz) = (x - nx, z - nz);
            (dx * dx + dz * dz).sqrt()
        })
    };
    let distance = |e: &Entity| distance_to(e.position);
    let within = |d: Option<f64>| match (d, radius) {
        (Some(d), Some(r)) => d <= r,
        _ => true,
    };
    let world = session.current_world();
    let mut matches: Vec<(Option<f64>, Found<'_>)> = world
        .entities()
        .filter(|e| {
            name.as_ref()
                .is_none_or(|n| e.name.to_lowercase().contains(n.as_str()))
        })
        .filter(|e| component.is_none_or(|k| e.components.iter().any(|c| c.kind() == k)))
        .filter(|e| tag.is_none_or(|t| e.has_tag(t)))
        .map(|e| (distance(e), Found::Entity(e)))
        .filter(|(d, _)| within(*d))
        .collect();
    if with_generated {
        for (index, set) in world.generated().sets().enumerate() {
            let kinds_of = |item: u16| -> Vec<&'static str> {
                set.generator()
                    .items
                    .get(usize::from(item))
                    .map(|i| i.base_components(&world.prefabs))
                    .unwrap_or_default()
                    .iter()
                    .map(Component::kind)
                    .collect()
            };
            let items: Vec<(Vec<&'static str>, Vec<String>)> = (0..set.generator().items.len())
                .map(|i| {
                    let i = u16::try_from(i).unwrap_or(u16::MAX);
                    let tags = set
                        .generator()
                        .items
                        .get(usize::from(i))
                        .map(|it| it.base_components(&world.prefabs))
                        .unwrap_or_default()
                        .into_iter()
                        .find_map(|c| match c {
                            Component::Tags(t) => Some(t.tags),
                            _ => None,
                        })
                        .unwrap_or_default();
                    (kinds_of(i), tags)
                })
                .collect();
            let candidates = match (near, radius) {
                (Some(point), Some(r)) => set.near(point, r),
                _ => set.instances().iter().collect(),
            };
            for instance in candidates {
                let Some((kinds, tags)) = items.get(usize::from(instance.item)) else {
                    continue;
                };
                if component.is_some_and(|k| !kinds.contains(&k)) {
                    continue;
                }
                if tag.is_some_and(|t| !tags.iter().any(|x| x == t)) {
                    continue;
                }
                let id = set.instance_id(instance);
                if name
                    .as_ref()
                    .is_some_and(|n| !id.to_lowercase().contains(n.as_str()))
                {
                    continue;
                }
                let d = distance_to(instance.position);
                if !within(d) {
                    continue;
                }
                matches.push((
                    d,
                    Found::Generated {
                        set: index,
                        id,
                        position: instance.position,
                        kinds: kinds.clone(),
                        number: instance.number,
                    },
                ));
            }
        }
    }
    if near.is_some() {
        matches.sort_by(|(da, a), (db, b)| {
            da.unwrap_or(f64::INFINITY)
                .total_cmp(&db.unwrap_or(f64::INFINITY))
                .then(a.order().cmp(&b.order()))
        });
    }
    let total = matches.len();
    let take = usize::try_from(limit).unwrap_or(usize::MAX);
    let entities: Vec<Value> = matches
        .into_iter()
        .take(take)
        .map(|(d, found)| {
            let mut v = Map::new();
            match found {
                Found::Entity(e) => {
                    v.insert("id".into(), json!(e.id.0));
                    v.insert("name".into(), json!(e.name));
                    v.insert("position".into(), json!(e.position));
                    let kinds: Vec<&str> = e.components.iter().map(Component::kind).collect();
                    v.insert("components".into(), json!(kinds));
                }
                Found::Generated {
                    id,
                    position,
                    kinds,
                    ..
                } => {
                    v.insert("id".into(), json!(id));
                    v.insert("name".into(), json!(id));
                    v.insert("position".into(), json!(position));
                    v.insert("components".into(), json!(kinds));
                    v.insert("generated".into(), json!(true));
                }
            }
            if let Some(d) = d {
                v.insert("distance_m".into(), json!(d));
            }
            Value::Object(v)
        })
        .collect();
    Ok(json!({"total": total, "entities": entities}))
}

/// One match of `entity.find`: an entity or a generated instance.
enum Found<'w> {
    Entity(&'w Entity),
    Generated {
        set: usize,
        number: u32,
        id: String,
        position: [f64; 3],
        kinds: Vec<&'static str>,
    },
}

impl Found<'_> {
    /// The order among equal distances: entities by id, then instances by generator and number.
    fn order(&self) -> (u8, u64, u64) {
        match self {
            Found::Entity(e) => (0, e.id.0, 0),
            Found::Generated { set, number, .. } => (1, *set as u64, u64::from(*number)),
        }
    }
}
