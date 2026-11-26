//! Water: `water.add` and `water.set`.
//!
//! Water is an entity with a `water` component (`gamecraft_world::water`): a flat surface at
//! one level over an outline of the ground plane, or everywhere (the sea). These commands take
//! the level as a world height and the outline in world `[x, z]`, and keep both in the
//! entity's frame, as `road.add` and `road.set` do for a road. A character whose feet are
//! under the surface wades (`gamecraft_sim::water`); rays, lines of sight and projectiles pass
//! through water. Water is removed with `entity.remove` (or its component with
//! `entity.remove_component`); `entity.add` and `entity.set_component` take a water component
//! as given, `level_m` above the entity's position and `outline` in its frame.

use crate::entity_cmds::{layer_of, layer_param};
use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Edit, Handler, Registration};
use crate::spec::{CommandSpec, ParamType as T, kind_of, param, spec};
use gamecraft_math::{Portable, deg_to_rad};
use gamecraft_world::limits::{MAX_COORDINATE_M, WATER_POINTS_MAX};
use gamecraft_world::{Component, Entity, EntityDraft, EntityId, Water, water_areas};
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

/// What the wading rule says, for the summaries.
const WADING: &str = "A character whose feet are under the surface wades: slower by the depth, \
     its full speed up to 0.5 m and 0.4 of it from 1.5 m; deeper water is refused (not stepped \
     into). Rays, lines of sight and projectiles pass through water.";

fn level_param(default: bool) -> crate::spec::ParamSpec {
    let p = param(
        "level_m",
        T::Number,
        "The height of the surface in the world, metres (within ±1000000).",
    )
    .unit("m");
    if default { p.default(json!(0)) } else { p }
}

fn outline_param(required: bool) -> crate::spec::ParamSpec {
    let p = param(
        "outline",
        T::Array,
        &format!(
            "[[x, z], ...] the outline of the area in world metres, 3 to {WATER_POINTS_MAX} \
             points (consecutive points at least 0.01 m apart); [] (the default): water \
             everywhere, the sea out to the horizon."
        ),
    )
    .unit("m");
    if required { p.required() } else { p }
}

fn add_spec() -> CommandSpec {
    spec(
        "water.add",
        "Add Water",
        &format!(
            "Adds water: an entity with a `water` component whose surface is `level_m` high \
             over `outline` (everywhere when the outline is empty: the sea). The entity stands \
             at the outline's centre, on the surface. {WADING} The renderer draws one water \
             surface: water everywhere, else the largest area; `view.render` names the areas it \
             does not draw. `query.ground` says where it is water and how deep."
        ),
        vec![
            param(
                "name",
                T::String,
                "Its name, at most 200 characters; need not be unique.",
            )
            .default(json!("")),
            level_param(true),
            outline_param(false),
            layer_param("The layer it belongs to."),
        ],
        "{id, level_m (the surface's height in the world), area_m2 (null for water everywhere), \
         position}",
        json!({"name": "Pond", "level_m": 0.4, "outline": [[8, -6], [18, -6], [18, 2], [8, 2]]}),
    )
}

fn set_spec() -> CommandSpec {
    spec(
        "water.set",
        "Edit Water",
        &format!(
            "Changes the water of an entity, or gives water to an entity without (then give \
             `outline`: [] for water everywhere at the entity's height): only the fields given \
             change. `level_m` moves the surface (and the entity's height); `outline` is kept in \
             the entity's frame, so the entity stays where it is. {WADING}"
        ),
        vec![
            param(
                "id",
                T::Entity,
                "The entity: its id as a number, or its name as a string (any case).",
            )
            .required(),
            level_param(false),
            outline_param(false),
        ],
        "{id, level_m, area_m2, position}",
        json!({"id": "Crate", "outline": [[-6, -12], [14, -12], [14, 6], [-6, 6]]}),
    )
}

/// `level_m` as a world height: finite and within the coordinate limit.
fn level(p: Params<'_>) -> Result<Option<f64>, CommandError> {
    match p.f64("level_m")? {
        None => Ok(None),
        Some(v) if v.is_finite() && v.abs() <= MAX_COORDINATE_M => Ok(Some(v)),
        Some(v) => Err(bad(format!(
            "`level_m` is {v}; it must be a height in metres within ±{MAX_COORDINATE_M}"
        ))),
    }
}

/// `outline` as world `[x, z]` points: empty, or 3 to [`WATER_POINTS_MAX`] finite points
/// within the coordinate limit. (The gaps between points the component checks.)
fn world_outline(p: Params<'_>) -> Result<Option<Vec<[f64; 2]>>, CommandError> {
    let Some(v) = p.value("outline") else {
        return Ok(None);
    };
    let items = v.as_array().ok_or_else(|| {
        bad(format!(
            "`outline` must be [[x, z], ...], got {}",
            kind_of(v)
        ))
    })?;
    if !items.is_empty() && !(3..=WATER_POINTS_MAX).contains(&items.len()) {
        return Err(bad(format!(
            "`outline` has {} points; give 3 to {WATER_POINTS_MAX}, or [] for water everywhere",
            items.len()
        )));
    }
    let mut out = Vec::with_capacity(items.len());
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
                "`outline[{i}]` must be [x, z] in metres within ±{MAX_COORDINATE_M}, got {}",
                crate::spec::excerpt(&item.to_string())
            )));
        };
        out.push([x, z]);
    }
    Ok(Some(out))
}

/// World `[x, z]` points in the frame of `e`: the inverse of its position, yaw and scale, as
/// `road.set` keeps a centre line.
fn into_frame(e: &Entity, points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let (s, c) = deg_to_rad(e.rotation_y_deg).psin_cos();
    let [px, _, pz] = e.position;
    points
        .iter()
        .map(|&[x, z]| {
            let (dx, dz) = (x - px, z - pz);
            [(dx * c - dz * s) / e.scale.x, (dx * s + dz * c) / e.scale.z]
        })
        .collect()
}

/// The answer for the water of entity `id`.
fn answer(edit: &Edit<'_>, id: EntityId) -> Result<Value, CommandError> {
    let entity = edit
        .world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    let water = entity
        .water()
        .ok_or_else(|| failed(format!("entity {id} has no water")))?;
    let level = water
        .surface_y(entity)
        .ok_or_else(|| failed(format!("the surface of entity {id} is not a usable height")))?;
    let area = water_areas(&edit.world)
        .into_iter()
        .find(|a| a.entity == id)
        .filter(|a| !a.outline.is_empty())
        .map(|a| a.signed_area_m2().abs());
    Ok(json!({
        "id": id.0,
        "level_m": level,
        "area_m2": area,
        "position": entity.position,
    }))
}

fn add(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let level = level(p)?.unwrap_or(0.0);
    let outline = world_outline(p)?.unwrap_or_default();
    // The entity stands at the outline's centre, on the surface.
    let (mut cx, mut cz) = (0.0, 0.0);
    if !outline.is_empty() {
        for &[x, z] in &outline {
            cx += x;
            cz += z;
        }
        cx /= outline.len() as f64;
        cz /= outline.len() as f64;
    }
    let offsets: Vec<[f64; 2]> = outline.iter().map(|&[x, z]| [x - cx, z - cz]).collect();
    let id = edit
        .world
        .add_entity(EntityDraft {
            name: p.str("name")?.unwrap_or("").to_string(),
            position: [cx, level, cz],
            components: vec![Component::Water(Water::new(0.0, offsets))],
            layer: layer_of(p)?.flatten(),
            ..EntityDraft::default()
        })
        .map_err(|e| bad(e.to_string()))?;
    answer(edit, id)
}

fn set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = p.required_entity("id", &edit.world)?;
    let level = level(p)?;
    let outline = world_outline(p)?;
    let entity = edit
        .world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    let local = outline.as_deref().map(|pts| into_frame(entity, pts));
    let mut water = match (entity.water(), &local) {
        (Some(w), _) => w.clone(),
        (None, Some(_)) => Water::default(),
        (None, None) => {
            return Err(bad(format!(
                "entity {id} has no water; give `outline` to make it some ([] for water \
                 everywhere)"
            )));
        }
    };
    if let Some(points) = local {
        water.outline = points;
    }
    let e = edit
        .world
        .entity_mut(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    if let Some(level) = level {
        e.position[1] = level;
    }
    e.set_component(Component::Water(water));
    answer(edit, id)
}
