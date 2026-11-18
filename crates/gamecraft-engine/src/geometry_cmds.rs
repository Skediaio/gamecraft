//! Spatial queries: `query.ground_height`, `query.raycast` and `query.line_of_sight`.
//!
//! A ray hits the terrain (the triangle surface of [`gamecraft_world::HeightField`]) and the
//! `shape` component of every entity, solid or not: a box turned by the entity's
//! `rotation_y_deg`, a sphere or a capped cylinder, sized by `size * scale` and standing on the
//! entity's position. Plants and characters are not hit yet. A ray that starts inside a shape
//! hits it at distance 0. The ray maths is [`gamecraft_sim::ray`], the very code the simulation
//! uses for projectiles and perception.

use crate::error::{CommandError, bad};
use crate::params::Params;
use crate::session::{Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use gamecraft_math::{Portable, rad_to_deg};
use gamecraft_sim::ray::{self, Found, along, normalize};
use serde_json::{Map, Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: ground_height_spec,
        handler: Handler::Query(ground_height),
    },
    Registration {
        spec: raycast_spec,
        handler: Handler::Query(raycast),
    },
    Registration {
        spec: line_of_sight_spec,
        handler: Handler::Query(line_of_sight),
    },
];

/// Shortest `max_m` of a ray, metres.
const RAY_MIN_M: f64 = 0.01;
/// Longest `max_m` of a ray, metres.
const RAY_MAX_M: f64 = 100_000.0;
/// `max_m` when left out, metres.
const RAY_DEFAULT_M: f64 = 1000.0;

const NOT_HIT: &str = "Plants and characters are not hit yet.";

fn ground_height_spec() -> CommandSpec {
    spec(
        "query.ground_height",
        "Ground Height",
        "The terrain's height, surface normal and slope at a point of the ground plane of the \
         current world. Terrain only (shapes are not ground); null outside the terrain or \
         without one.",
        vec![
            param("x", T::Number, "East-west coordinate.")
                .required()
                .unit("m"),
            param("z", T::Number, "North-south coordinate (north is -z).")
                .required()
                .unit("m"),
        ],
        "{x, z, height_m: number | null, normal: [x, y, z] | null, slope_deg: number | null}",
        json!({"x": 8, "z": -10}),
    )
    .query()
}

fn ground_height(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let x = p.f64("x")?.ok_or_else(|| bad("missing `x`"))?;
    let z = p.f64("z")?.ok_or_else(|| bad("missing `z`"))?;
    let field = session.current_heights();
    let height = field.and_then(|f| f.height_at(x, z));
    let normal = field.and_then(|f| f.normal_at(x, z));
    let slope = normal.map(|[nx, ny, nz]| rad_to_deg((nx * nx + nz * nz).sqrt().patan2(ny)));
    Ok(json!({"x": x, "z": z, "height_m": height, "normal": normal, "slope_deg": slope}))
}

/// The nearest hit within `max_m` of a ray with a unit `dir` in the current world.
fn cast(session: &Session, origin: [f64; 3], dir: [f64; 3], max_m: f64) -> Option<Found> {
    ray::cast(
        session.current_world(),
        session.current_heights(),
        origin,
        dir,
        max_m,
        |_| false,
    )
}

/// `{kind, entity?, name?, point, normal, distance_m}` of a hit.
fn describe(
    session: &Session,
    found: &Found,
    origin: [f64; 3],
    dir: [f64; 3],
) -> Map<String, Value> {
    let mut v = Map::new();
    match found.entity {
        Some(id) => {
            let name = session
                .current_world()
                .entity(id)
                .map_or("", |e| e.name.as_str());
            v.insert("kind".into(), json!("entity"));
            v.insert("entity".into(), json!(id.0));
            v.insert("name".into(), json!(name));
        }
        None => {
            v.insert("kind".into(), json!("terrain"));
        }
    }
    v.insert(
        "point".into(),
        json!(along(origin, dir, found.hit.distance_m)),
    );
    v.insert("normal".into(), json!(found.hit.normal));
    v.insert("distance_m".into(), json!(found.hit.distance_m));
    v
}

fn raycast_spec() -> CommandSpec {
    spec(
        "query.raycast",
        "Raycast",
        &format!(
            "Casts a ray in the current world and reports the nearest hit: the terrain or the \
             shape of an entity (solid or not; a box turned by the entity's rotation, a sphere \
             or a capped cylinder, sized by size times scale). A ray that starts inside a \
             shape hits it at distance 0. {NOT_HIT}"
        ),
        vec![
            param("origin", T::Point3, "Where the ray starts.")
                .required()
                .unit("m"),
            param(
                "direction",
                T::Point3,
                "Which way it goes; any length but zero.",
            )
            .required(),
            param("max_m", T::Number, "How far it reaches, 0.01 to 100000.")
                .unit("m")
                .default(json!(1000)),
        ],
        "{hit: false} or {hit: true, kind: \"terrain\" | \"entity\", entity (id, for an \
         entity), name (for an entity), point: [x, y, z], normal: [x, y, z], distance_m}",
        json!({"origin": [0, 30, 0], "direction": [0, -1, 0.2]}),
    )
    .query()
}

/// The unit direction from a parameter; an error naming it for a zero or unusable one.
fn direction(p: Params<'_>, name: &str) -> Result<[f64; 3], CommandError> {
    let d = p
        .point3(name)?
        .ok_or_else(|| bad(format!("missing `{name}`")))?;
    normalize(d).ok_or_else(|| {
        bad(format!(
            "`{name}` is {d:?}, which has no direction; give a vector of non-zero length, \
             such as [0, -1, 0] for straight down"
        ))
    })
}

fn raycast(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let origin = p.point3("origin")?.ok_or_else(|| bad("missing `origin`"))?;
    let dir = direction(p, "direction")?;
    let max_m = p.f64("max_m")?.unwrap_or(RAY_DEFAULT_M);
    if !(RAY_MIN_M..=RAY_MAX_M).contains(&max_m) {
        return Err(bad(format!(
            "`max_m` is {max_m}; it must be from {RAY_MIN_M} to {RAY_MAX_M} metres"
        )));
    }
    let Some(found) = cast(session, origin, dir, max_m) else {
        return Ok(json!({"hit": false}));
    };
    let mut v = Map::new();
    v.insert("hit".into(), json!(true));
    v.extend(describe(session, &found, origin, dir));
    Ok(Value::Object(v))
}

fn line_of_sight_spec() -> CommandSpec {
    spec(
        "query.line_of_sight",
        "Line of Sight",
        &format!(
            "Whether the straight line between two points is clear: a ray from `from` to `to` \
             with the hit rules of query.raycast (the terrain and every shape, solid or not). \
             A hit within 1 mm of `to` does not block, so a point on the ground or on a shape \
             can be seen. A `from` inside a shape is blocked at distance 0. {NOT_HIT}"
        ),
        vec![
            param("from", T::Point3, "Where the line starts (an eye, say).")
                .required()
                .unit("m"),
            param("to", T::Point3, "Where it ends (the target).")
                .required()
                .unit("m"),
        ],
        "{clear: bool, distance_m (from `from` to `to`), blocked_by (when not clear): {kind, \
         entity, name (for an entity), point, distance_m}}",
        json!({"from": [0, 3, 0], "to": [8, 3, -10]}),
    )
    .query()
}

fn line_of_sight(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let from = p.point3("from")?.ok_or_else(|| bad("missing `from`"))?;
    let to = p.point3("to")?.ok_or_else(|| bad("missing `to`"))?;
    let sight = ray::line_of_sight(
        session.current_world(),
        session.current_heights(),
        from,
        to,
        |_| false,
    )
    .ok_or_else(|| {
        bad(format!(
            "`from` {from:?} and `to` {to:?} are too far apart to measure"
        ))
    })?;
    let Some(found) = sight.blocked_by else {
        return Ok(json!({"clear": true, "distance_m": sight.distance_m}));
    };
    let mut b = describe(session, &found, from, sight.direction);
    b.remove("normal");
    Ok(json!({"clear": false, "distance_m": sight.distance_m, "blocked_by": Value::Object(b)}))
}
