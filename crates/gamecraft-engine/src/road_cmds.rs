//! Roads: `road.add` and `road.set`.
//!
//! A road is an entity with a `road` component: a centre line with a width that lies on the
//! terrain (`gamecraft_world::road`). These commands take the centre line in world `[x, z]`
//! and keep it in the entity's frame, and they choose the road's tier where it overlaps other
//! roads, so junctions never show two coplanar surfaces. A road is removed with
//! `entity.remove` (or its component with `entity.remove_component`); `entity.add` and
//! `entity.set_component` take a road component as given, tier included.

use crate::entity_cmds::{layer_of, layer_param};
use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Edit, Handler, Registration};
use crate::spec::{CommandSpec, ParamType as T, kind_of, param, spec};
use gamecraft_math::{Portable, deg_to_rad};
use gamecraft_world::limits::{
    MAX_COORDINATE_M, ROAD_POINT_GAP_MIN_M, ROAD_POINTS_MAX, ROAD_POINTS_MIN, ROAD_TIER_MAX,
};
use gamecraft_world::{
    Component, Entity, EntityDraft, EntityId, Road, RoadEnds, free_road_tier, retier_roads,
};
use serde_json::{Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: add_spec,
        handler: Handler::World(add),
    },
    Registration {
        spec: set_spec,
        handler: Handler::World(set),
    },
];

const ENDS: [&str; 2] = ["round", "flat"];

fn points_param(doc: &str) -> crate::spec::ParamSpec {
    param("points", T::Array, doc).unit("m")
}

/// The parameters both commands share, after `points`.
fn shape_params(defaults: bool) -> Vec<crate::spec::ParamSpec> {
    let with = |p: crate::spec::ParamSpec, v: Value| if defaults { p.default(v) } else { p };
    vec![
        with(
            param("width_m", T::Number, "Width, 0.1 to 200.").unit("m"),
            json!(6),
        ),
        with(
            param(
                "widths_m",
                T::Array,
                "A width per point (each 0.1 to 200) replacing `width_m`, changing linearly \
                 along each segment; [] uses `width_m` everywhere.",
            )
            .unit("m"),
            json!([]),
        ),
        with(
            param(
                "color",
                T::Color,
                "Surface colour, linear RGB 0 to 1; asphalt by default.",
            ),
            json!([0.05, 0.05, 0.055]),
        ),
        with(
            param(
                "lift_m",
                T::Number,
                "How high the surface lies above the ground, 0.005 to 1.",
            )
            .unit("m"),
            json!(0.03),
        ),
        with(
            param(
                "ends",
                T::String,
                "How the road ends at its first and last point.",
            )
            .allowed(&ENDS),
            json!("round"),
        ),
    ]
}

fn tier_param(add: bool) -> crate::spec::ParamSpec {
    let doc = if add {
        "Which road is on top where roads overlap, 0 to 7 (each tier 0.01 m higher), or \
         \"auto\": the lowest tier no road it overlaps uses."
    } else {
        "0 to 7, or \"auto\": the lowest tier no road it overlaps uses. Left out, the tier \
         stays."
    };
    let p = param("tier", T::Any, doc);
    if add { p.default(json!("auto")) } else { p }
}

fn retier_param() -> crate::spec::ParamSpec {
    param(
        "retier_all",
        T::Boolean,
        "After the change, gives every road of the world its tier again: in id order, the \
         lowest tier no earlier road it overlaps uses (for a world whose roads were written as \
         data). Explicit tiers are replaced.",
    )
    .default(json!(false))
}

fn add_spec() -> CommandSpec {
    let mut params = vec![
        param(
            "name",
            T::String,
            "Its name, at most 200 characters; need not be unique.",
        )
        .default(json!("")),
        points_param(&format!(
            "The centre line, [[x, z], ...] in world metres, {ROAD_POINTS_MIN} to \
             {ROAD_POINTS_MAX} points, consecutive points at least {ROAD_POINT_GAP_MIN_M} m \
             apart. Heights follow the terrain."
        ))
        .required(),
    ];
    params.extend(shape_params(true));
    params.push(tier_param(true));
    params.push(layer_param("The layer it belongs to."));
    params.push(retier_param());
    spec(
        "road.add",
        "Add Road",
        "Adds a road: an entity at the first point of its centre line with a `road` component. \
         The engine drapes its surface on the terrain (clipped to the terrain's triangles, \
         `lift_m` above them, so it follows the ground exactly), draws it, and rays and lines \
         of sight hit it with the road's id. Where it overlaps other roads it takes a tier none \
         of them uses, so a junction never shows two surfaces at the same height. Characters \
         walk on the terrain beneath it.",
        params,
        "{id, tier, length_m, position}",
        json!({"name": "High Street", "points": [[-30, 20], [0, 18], [24, 36]], "width_m": 7}),
    )
}

fn set_spec() -> CommandSpec {
    let mut params = vec![
        param(
            "id",
            T::Entity,
            "The entity: its id as a number, or its name as a string (any case). One without a \
             road gets one (then `points` is required).",
        )
        .required(),
        points_param(&format!(
            "A new centre line, [[x, z], ...] in world metres, {ROAD_POINTS_MIN} to \
             {ROAD_POINTS_MAX} points; kept in the entity's frame, so the entity stays where it \
             is."
        )),
    ];
    params.extend(shape_params(false));
    params.push(tier_param(false));
    params.push(retier_param());
    spec(
        "road.set",
        "Edit Road",
        "Changes the road of an entity, or gives one to an entity without: only the fields \
         given change (`widths_m: []` goes back to one width). A road that gets new points or \
         widths keeps its tier unless `tier` says otherwise.",
        params,
        "{id, tier, length_m, changed_tiers (with retier_all)}",
        json!({"id": "Crate", "points": [[-6, 4], [12, 4]], "width_m": 4, "tier": "auto"}),
    )
}

/// The centre line of `points` in world `[x, z]`: the count, every number and the gaps
/// checked, the error naming the point.
fn world_points(p: Params<'_>) -> Result<Option<Vec<[f64; 2]>>, CommandError> {
    let Some(v) = p.value("points") else {
        return Ok(None);
    };
    let items = v.as_array().ok_or_else(|| {
        bad(format!(
            "`points` must be [[x, z], ...], got {}",
            kind_of(v)
        ))
    })?;
    if !(ROAD_POINTS_MIN..=ROAD_POINTS_MAX).contains(&items.len()) {
        return Err(bad(format!(
            "`points` has {} points; a road has {ROAD_POINTS_MIN} to {ROAD_POINTS_MAX} (split a \
             longer one into several roads)",
            items.len()
        )));
    }
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let xz = item.as_array().and_then(|a| match a.as_slice() {
            [x, z] => Some([x.as_f64()?, z.as_f64()?]),
            _ => None,
        });
        let Some([x, z]) = xz.filter(|q| {
            q.iter()
                .all(|c| c.is_finite() && c.abs() <= MAX_COORDINATE_M)
        }) else {
            return Err(bad(format!(
                "`points[{i}]` must be [x, z] in metres within ±{MAX_COORDINATE_M}, got {}",
                crate::spec::excerpt(&item.to_string())
            )));
        };
        if let Some(&[px, pz]) = out.last() {
            let gap = ((x - px) * (x - px) + (z - pz) * (z - pz)).sqrt();
            if gap.is_nan() || gap < ROAD_POINT_GAP_MIN_M {
                return Err(bad(format!(
                    "`points[{i}]` [{x}, {z}] is {gap} m from `points[{}]`; consecutive points \
                     must be at least {ROAD_POINT_GAP_MIN_M} m apart (leave out the repeated \
                     point)",
                    i - 1
                )));
            }
        }
        out.push([x, z]);
    }
    Ok(Some(out))
}

/// `widths_m` as numbers (ranges are checked with the road).
fn widths(p: Params<'_>) -> Result<Option<Vec<f64>>, CommandError> {
    let Some(v) = p.value("widths_m") else {
        return Ok(None);
    };
    let items = v.as_array().ok_or_else(|| {
        bad(format!(
            "`widths_m` must be an array of numbers, got {}",
            kind_of(v)
        ))
    })?;
    if items.len() > ROAD_POINTS_MAX {
        return Err(bad(format!(
            "`widths_m` has {} widths; give one per point, at most {ROAD_POINTS_MAX}",
            items.len()
        )));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, w)| {
            w.as_f64().filter(|x| x.is_finite()).ok_or_else(|| {
                bad(format!(
                    "`widths_m[{i}]` must be a number of metres, got {}",
                    kind_of(w)
                ))
            })
        })
        .collect::<Result<Vec<f64>, _>>()
        .map(Some)
}

/// `color` as linear RGB.
fn color(p: Params<'_>) -> Result<Option<[f32; 3]>, CommandError> {
    let Some(v) = p.value("color") else {
        return Ok(None);
    };
    let rgb = v.as_array().and_then(|a| match a.as_slice() {
        [r, g, b] => Some([r.as_f64()?, g.as_f64()?, b.as_f64()?]),
        _ => None,
    });
    match rgb.filter(|c| c.iter().all(|x| (0.0..=1.0).contains(x))) {
        Some(c) => Ok(Some(c.map(|x| x as f32))),
        None => Err(bad(format!(
            "`color` must be [r, g, b], each 0 to 1 (linear), got {}",
            crate::spec::excerpt(&v.to_string())
        ))),
    }
}

/// What `tier` asks for: a tier, `Some(None)` for "auto", `None` when left out.
fn tier(p: Params<'_>) -> Result<Option<Option<u32>>, CommandError> {
    let Some(v) = p.value("tier") else {
        return Ok(None);
    };
    if v.as_str() == Some("auto") {
        return Ok(Some(None));
    }
    match v.as_u64().and_then(|t| u32::try_from(t).ok()) {
        Some(t) if t <= ROAD_TIER_MAX => Ok(Some(Some(t))),
        _ => Err(bad(format!(
            "`tier` must be a whole number from 0 to {ROAD_TIER_MAX} or \"auto\", got {}",
            crate::spec::excerpt(&v.to_string())
        ))),
    }
}

fn ends(p: Params<'_>) -> Result<Option<RoadEnds>, CommandError> {
    let Some(name) = p.str("ends")? else {
        return Ok(None);
    };
    RoadEnds::ALL
        .into_iter()
        .find(|e| e.name() == name)
        .map(Some)
        .ok_or_else(|| bad(format!("`ends` is {name:?}; use \"round\" or \"flat\"")))
}

/// Changes `road` by the shape parameters given; `points` are already in the entity's frame.
fn apply(
    road: &mut Road,
    p: Params<'_>,
    points: Option<Vec<[f64; 2]>>,
) -> Result<(), CommandError> {
    if let Some(points) = points {
        road.points = points;
    }
    if let Some(w) = p.f64("width_m")? {
        road.width_m = w;
    }
    if let Some(w) = widths(p)? {
        road.widths_m = w;
    }
    if let Some(c) = color(p)? {
        road.color = c;
    }
    if let Some(l) = p.f64("lift_m")? {
        road.lift_m = l;
    }
    if let Some(e) = ends(p)? {
        road.ends = e;
    }
    Ok(())
}

/// World `[x, z]` points in the frame of `e`: the inverse of its position, yaw and scale.
fn into_frame(e: &Entity, points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let (s, c) = deg_to_rad(e.rotation_y_deg).psin_cos();
    let [px, _, pz] = e.position;
    points
        .iter()
        .map(|&[x, z]| {
            let (dx, dz) = (x - px, z - pz);
            [(dx * c - dz * s) / e.scale, (dx * s + dz * c) / e.scale]
        })
        .collect()
}

/// Sets the tier of the road on `id` (`None`: the free one), checks the entity, and with
/// `retier_all` re-tiers every road. Returns the result object.
fn finish(
    edit: &mut Edit<'_>,
    id: EntityId,
    wanted: Option<Option<u32>>,
    retier_all: bool,
) -> Result<Value, CommandError> {
    let entity = edit
        .world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    entity.validate().map_err(|e| bad(e.to_string()))?;
    if let Some(wanted) = wanted {
        let road = entity
            .road()
            .ok_or_else(|| failed(format!("entity {id} has no road")))?;
        let tier = match wanted {
            Some(t) => t,
            None => free_road_tier(&edit.world, entity, road),
        };
        if let Some(r) = edit.world.entity_mut(id).and_then(Entity::road_mut) {
            r.tier = tier;
        }
    }
    let changed = retier_all.then(|| retier_roads(&mut edit.world).changed);
    let entity = edit
        .world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    let road = entity
        .road()
        .ok_or_else(|| failed(format!("entity {id} has no road")))?;
    let mut out = json!({
        "id": id.0,
        "tier": road.tier,
        "length_m": road.length_m(entity),
        "position": entity.position,
    });
    if let (Some(n), Some(map)) = (changed, out.as_object_mut()) {
        map.insert("changed_tiers".into(), json!(n));
    }
    Ok(out)
}

fn add(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let points = world_points(p)?.ok_or_else(|| bad("missing `points`"))?;
    let Some(&[x0, z0]) = points.first() else {
        return Err(bad("missing `points`"));
    };
    let offsets: Vec<[f64; 2]> = points.iter().map(|&[x, z]| [x - x0, z - z0]).collect();
    let mut road = Road::new(Vec::new());
    apply(&mut road, p, Some(offsets))?;
    let wanted = tier(p)?.unwrap_or(None);
    road.tier = wanted.unwrap_or(0);
    let position = [x0, edit.ground_or_none(x0, z0).unwrap_or(0.0), z0];
    let id = edit
        .world
        .add_entity(EntityDraft {
            name: p.str("name")?.unwrap_or("").to_string(),
            position,
            components: vec![Component::Road(road)],
            layer: layer_of(p)?.flatten(),
            ..EntityDraft::default()
        })
        .map_err(|e| bad(e.to_string()))?;
    finish(
        edit,
        id,
        Some(wanted),
        p.bool("retier_all")?.unwrap_or(false),
    )
}

fn set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = p.required_entity("id", &edit.world)?;
    let points = world_points(p)?;
    let entity = edit
        .world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    let local = points.as_deref().map(|pts| into_frame(entity, pts));
    let mut road = match (entity.road(), &local) {
        (Some(r), _) => r.clone(),
        (None, Some(_)) => Road::new(Vec::new()),
        (None, None) => {
            return Err(bad(format!(
                "entity {id} has no road; give `points` to make it one"
            )));
        }
    };
    apply(&mut road, p, local)?;
    let wanted = tier(p)?;
    if let Some(Some(t)) = wanted {
        road.tier = t;
    }
    let e = edit
        .world
        .entity_mut(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    e.set_component(Component::Road(road));
    finish(edit, id, wanted, p.bool("retier_all")?.unwrap_or(false))
}
