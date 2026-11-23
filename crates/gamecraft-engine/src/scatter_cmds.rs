//! `entity.scatter`: many copies of a template entity on the ground, placed deterministically.

use crate::error::{CommandError, bad, failed};
use crate::params::{Params, component};
use crate::session::{Edit, Handler, Registration};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use gamecraft_math::{Portable, Rng, rad_to_deg};
use gamecraft_world::limits::{MAX_NAME_CHARS, SCALE_MAX, SCALE_MIN};
use gamecraft_world::{Component, Entity, EntityDraft, EntityId, Terrain};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::f64::consts::TAU;

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[Registration {
    spec: scatter_spec,
    handler: Handler::World(scatter),
}];

/// Most copies one `entity.scatter` places.
const MAX_SCATTER_COUNT: u64 = 5000;
/// Points drawn per requested copy before the command gives up on the rest.
const TRIES_PER_COPY: u64 = 30;
/// Most circles one `entity.scatter` keeps clear.
const MAX_EXCLUDE: usize = 256;

fn scatter_spec() -> CommandSpec {
    spec(
        "entity.scatter",
        "Scatter Entities",
        "Places copies of a template entity on the ground of the terrain, at points drawn from \
         a seeded generator: inside a circle, or anywhere on the terrain. Points steeper than \
         `max_slope_deg`, inside a circle of `exclude`, or closer than `min_distance_m` to a \
         copy placed by the same call are skipped, with a bounded number of tries (30 per \
         copy), so `placed` can be lower than \
         `requested`. The same parameters on the same world give the same entities in the same \
         order. One undo step; works on a running world. Needs a terrain.",
        vec![
            param("count", T::Integer, "Copies wanted, 1 to 5000.").required(),
            param(
                "components",
                T::Array,
                "The template's components as in entity.add: {\"type\": \"shape\"|\"plant\"|\
                 \"character\", ...}, at most one of each type.",
            )
            .required(),
            param(
                "seed",
                T::Integer,
                "Seed of the placement; the world's seed when left out.",
            ),
            param(
                "name",
                T::String,
                "Base name: copies are called `<name> 1`, `<name> 2`, ...; empty names them \
                 nothing.",
            )
            .default(json!("")),
            param(
                "center",
                T::Point2,
                "Centre [x, z] of the circle to scatter in, with `radius_m`; the whole terrain \
                 when left out.",
            )
            .unit("m"),
            param(
                "radius_m",
                T::Number,
                "Radius of the circle around `center`, above 0.",
            )
            .unit("m"),
            param(
                "exclude",
                T::Array,
                "Circles kept clear, [{\"center\": [x, z], \"radius_m\": r}, ...], at most 256: \
                 no copy stands inside one (a path, a clearing, a camp).",
            )
            .default(json!([])),
            param(
                "min_distance_m",
                T::Number,
                "Least horizontal distance between two copies of this call, 0 or more.",
            )
            .unit("m")
            .default(json!(0)),
            param(
                "max_slope_deg",
                T::Number,
                "Steepest ground a copy is put on, 0 to 90.",
            )
            .unit("deg")
            .default(json!(90)),
            param(
                "scale_range",
                T::Array,
                "[min, max] of each copy's uniform scale, drawn evenly, 0.001 to 1000, min <= \
                 max.",
            )
            .default(json!([1, 1])),
            param(
                "random_rotation",
                T::Boolean,
                "Turn each copy about +Y by a random angle; false faces them all north.",
            )
            .default(json!(true)),
            param(
                "vary_seed",
                T::Boolean,
                "Give the `plant` component of each copy a seed of its own; false keeps the \
                 template's.",
            )
            .default(json!(true)),
        ],
        "{placed, requested, first_id (null when none placed), last_id}",
        json!({
            "count": 12,
            "components": [{"type": "plant", "species": "quercus-robur"}],
            "name": "Young oak",
            "center": [0, 0],
            "radius_m": 40,
            "min_distance_m": 5,
            "max_slope_deg": 30,
            "scale_range": [0.6, 1.0]
        }),
    )
}

/// `[min, max]` of the scale.
fn scale_range(p: Params<'_>) -> Result<[f64; 2], CommandError> {
    let Some(v) = p.value("scale_range") else {
        return Ok([1.0, 1.0]);
    };
    let wrong = || {
        bad(format!(
            "`scale_range` must be [min, max] with {SCALE_MIN} <= min <= max <= {SCALE_MAX}, \
             got {}",
            crate::spec::excerpt(&v.to_string())
        ))
    };
    let Some([lo, hi]) = v.as_array().and_then(|a| match a.as_slice() {
        [lo, hi] => Some([lo.as_f64()?, hi.as_f64()?]),
        _ => None,
    }) else {
        return Err(wrong());
    };
    if !(SCALE_MIN <= lo && lo <= hi && hi <= SCALE_MAX) {
        return Err(wrong());
    }
    Ok([lo, hi])
}

/// Where points are drawn.
enum Area {
    Circle { center: [f64; 2], radius: f64 },
    Square { half: f64 },
}

impl Area {
    /// The next point, two draws of `rng`.
    fn draw(&self, rng: &mut Rng) -> [f64; 2] {
        match self {
            Area::Circle {
                center: [cx, cz],
                radius,
            } => {
                let r = radius * rng.unit().sqrt();
                let (s, c) = (TAU * rng.unit()).psin_cos();
                [cx + r * c, cz + r * s]
            }
            Area::Square { half } => [rng.range(-half, *half), rng.range(-half, *half)],
        }
    }
}

/// A circle no copy stands in.
struct Clearing {
    center: [f64; 2],
    radius: f64,
}

impl Clearing {
    fn contains(&self, [x, z]: [f64; 2]) -> bool {
        let (dx, dz) = (x - self.center[0], z - self.center[1]);
        dx * dx + dz * dz < self.radius * self.radius
    }
}

/// The circles of `exclude`.
fn clearings(p: Params<'_>) -> Result<Vec<Clearing>, CommandError> {
    let Some(v) = p.value("exclude") else {
        return Ok(Vec::new());
    };
    let form = "[{\"center\": [x, z], \"radius_m\": r}, ...]";
    let Some(list) = v.as_array() else {
        return Err(bad(format!("`exclude` must be a list of circles, {form}")));
    };
    if list.len() > MAX_EXCLUDE {
        return Err(bad(format!(
            "`exclude` holds {} circles; at most {MAX_EXCLUDE}",
            list.len()
        )));
    }
    let mut out = Vec::with_capacity(list.len());
    for (i, item) in list.iter().enumerate() {
        let Some(map) = item.as_object() else {
            return Err(bad(format!(
                "`exclude[{i}]` must be a circle {{\"center\": [x, z], \"radius_m\": r}}"
            )));
        };
        if let Some(extra) = map.keys().find(|k| *k != "center" && *k != "radius_m") {
            return Err(bad(format!(
                "`exclude[{i}]` has an unknown field `{}`; a circle has `center` and `radius_m`",
                crate::spec::excerpt(extra)
            )));
        }
        let center = map
            .get("center")
            .and_then(Value::as_array)
            .and_then(|a| match a.as_slice() {
                [x, z] => Some([x.as_f64()?, z.as_f64()?]),
                _ => None,
            });
        let Some(center) = center.filter(|c| c.iter().all(|v| v.is_finite())) else {
            return Err(bad(format!(
                "`exclude[{i}].center` must be a point [x, z] in metres"
            )));
        };
        let radius = map.get("radius_m").and_then(Value::as_f64);
        let Some(radius) = radius.filter(|r| r.is_finite() && *r > 0.0) else {
            return Err(bad(format!(
                "`exclude[{i}].radius_m` must be a number of metres above 0"
            )));
        };
        out.push(Clearing { center, radius });
    }
    Ok(out)
}

fn area(p: Params<'_>, terrain: &Terrain) -> Result<Area, CommandError> {
    match (p.point2("center")?, p.f64("radius_m")?) {
        (Some(center), Some(radius)) => {
            if radius <= 0.0 {
                return Err(bad(format!(
                    "`radius_m` is {radius}; it must be above 0 metres"
                )));
            }
            Ok(Area::Circle { center, radius })
        }
        (Some(_), None) => Err(bad(
            "`center` needs `radius_m`, the radius of the circle to scatter in; leave both out \
             to scatter over the whole terrain",
        )),
        (None, Some(_)) => Err(bad(
            "`radius_m` needs `center` [x, z]; leave both out to scatter over the whole terrain",
        )),
        (None, None) => Ok(Area::Square {
            half: terrain.size_m / 2.0,
        }),
    }
}

/// The slope in degrees of the terrain triangle under `x`, `z` (the same triangles as
/// [`gamecraft_world::HeightField`]), from the heights of its corners; `None` without ground.
fn slope_deg(edit: &Edit<'_>, terrain: &Terrain, x: f64, z: f64) -> Option<f64> {
    let cell = terrain.cell_m;
    let half = terrain.size_m / 2.0;
    let last = (terrain.size_m / cell).round() - 1.0;
    if !(cell > 0.0 && last >= 0.0) {
        return None;
    }
    let (u, v) = ((x + half) / cell, (z + half) / cell);
    let (col, row) = (u.floor().clamp(0.0, last), v.floor().clamp(0.0, last));
    let (fu, fv) = (u - col, v - row);
    let (x0, z0) = (-half + col * cell, -half + row * cell);
    let h = |dc: f64, dr: f64| edit.ground_or_none(x0 + dc * cell, z0 + dr * cell);
    let (h00, h10, h01, h11) = (h(0.0, 0.0)?, h(1.0, 0.0)?, h(0.0, 1.0)?, h(1.0, 1.0)?);
    let (du, dv) = if fu >= fv {
        (h10 - h00, h11 - h10)
    } else {
        (h11 - h01, h01 - h00)
    };
    let gradient = (du * du + dv * dv).sqrt() / cell;
    Some(rad_to_deg(gradient.patan()))
}

/// The points placed so far, bucketed by cells of `min_distance_m` for the spacing check.
struct Spacing {
    cell: f64,
    min2: f64,
    buckets: BTreeMap<(i64, i64), Vec<[f64; 2]>>,
}

impl Spacing {
    fn new(min_distance: f64) -> Spacing {
        Spacing {
            cell: min_distance,
            min2: min_distance * min_distance,
            buckets: BTreeMap::new(),
        }
    }

    fn key(&self, [x, z]: [f64; 2]) -> (i64, i64) {
        // Saturating float-to-int casts: far or odd values share an edge bucket.
        (
            (x / self.cell).floor() as i64,
            (z / self.cell).floor() as i64,
        )
    }

    /// Whether `point` keeps the distance to every point placed so far.
    fn fits(&self, point: [f64; 2]) -> bool {
        if self.cell <= 0.0 {
            return true;
        }
        let (kx, kz) = self.key(point);
        let [x, z] = point;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let key = (kx.saturating_add(dx), kz.saturating_add(dz));
                let Some(points) = self.buckets.get(&key) else {
                    continue;
                };
                for [px, pz] in points {
                    let (ex, ez) = (x - px, z - pz);
                    if ex * ex + ez * ez < self.min2 {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn add(&mut self, point: [f64; 2]) {
        if self.cell > 0.0 {
            self.buckets.entry(self.key(point)).or_default().push(point);
        }
    }
}

fn scatter(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let count = p.u64("count")?.ok_or_else(|| bad("missing `count`"))?;
    if !(1..=MAX_SCATTER_COUNT).contains(&count) {
        return Err(bad(format!(
            "`count` is {count}; it must be from 1 to {MAX_SCATTER_COUNT}"
        )));
    }
    let list = p
        .value("components")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("missing `components`, the template's components"))?;
    let mut template = Vec::with_capacity(list.len());
    for (i, v) in list.iter().enumerate() {
        template.push(component(v, &format!("components[{i}]"))?);
    }
    let name = p.str("name")?.unwrap_or("");
    let suffix_chars = count.to_string().chars().count() + 1;
    let name_chars = name.chars().count();
    if !name.is_empty() && name_chars + suffix_chars > MAX_NAME_CHARS {
        return Err(bad(format!(
            "`name` has {name_chars} characters; with the number appended (\" {count}\") a \
             name may have at most {MAX_NAME_CHARS}, so give at most {} characters",
            MAX_NAME_CHARS.saturating_sub(suffix_chars)
        )));
    }
    let min_distance = p.f64("min_distance_m")?.unwrap_or(0.0);
    if min_distance < 0.0 {
        return Err(bad(format!(
            "`min_distance_m` is {min_distance}; it must be 0 or more metres"
        )));
    }
    let max_slope = p.f64("max_slope_deg")?.unwrap_or(90.0);
    if !(0.0..=90.0).contains(&max_slope) {
        return Err(bad(format!(
            "`max_slope_deg` is {max_slope}; it must be from 0 to 90 degrees"
        )));
    }
    let [scale_lo, scale_hi] = scale_range(p)?;
    let random_rotation = p.bool("random_rotation")?.unwrap_or(true);
    let vary_seed = p.bool("vary_seed")?.unwrap_or(true);
    let seed = p.u64("seed")?.unwrap_or(edit.world.seed);
    let Some(terrain) = edit.world.terrain.clone() else {
        return Err(failed(
            "there is no ground to scatter on: the world has no terrain; add one with \
             terrain.set",
        ));
    };
    let area = area(p, &terrain)?;
    let clear = clearings(p)?;

    // The template must be a valid entity on its own, even when nothing is placed.
    let probe = Entity {
        id: EntityId(0),
        name: String::new(),
        position: [0.0; 3],
        rotation_y_deg: 0.0,
        scale: 1.0,
        prefab: None,
        parent: None,
        layer: None,
        prefab_child: None,
        components: template.clone(),
    };
    probe
        .validate()
        .map_err(|e| bad(format!("the template in `components` is not valid: {e}")))?;

    let mut rng = Rng::new(seed);
    let mut spacing = Spacing::new(min_distance);
    let tries = count.saturating_mul(TRIES_PER_COPY);
    let (mut placed, mut first, mut last) = (0u64, None, None);
    for _ in 0..tries {
        if placed >= count {
            break;
        }
        let point = area.draw(&mut rng);
        let [x, z] = point;
        if clear.iter().any(|c| c.contains(point)) {
            continue;
        }
        let Some(y) = edit.ground_or_none(x, z) else {
            continue;
        };
        if max_slope < 90.0 && slope_deg(edit, &terrain, x, z).is_none_or(|s| s > max_slope) {
            continue;
        }
        if !spacing.fits(point) {
            continue;
        }
        spacing.add(point);
        placed += 1;
        let scale = rng.range(scale_lo, scale_hi).clamp(scale_lo, scale_hi);
        let rotation_y_deg = if random_rotation {
            rng.range(0.0, 360.0) % 360.0
        } else {
            0.0
        };
        let mut components = template.clone();
        if vary_seed {
            for c in &mut components {
                if let Component::Plant(plant) = c {
                    plant.seed = u64::from(rng.next_u32());
                }
            }
        }
        let draft = EntityDraft {
            name: if name.is_empty() {
                String::new()
            } else {
                format!("{name} {placed}")
            },
            position: [x, y, z],
            rotation_y_deg,
            scale,
            components,
            layer: None,
        };
        let id = edit
            .world
            .add_entity(draft)
            .map_err(|e| failed(e.to_string()))?;
        first.get_or_insert(id.0);
        last = Some(id.0);
    }
    Ok(json!({"placed": placed, "requested": count, "first_id": first, "last_id": last}))
}
