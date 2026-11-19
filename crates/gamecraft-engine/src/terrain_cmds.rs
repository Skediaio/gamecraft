//! Sculpting and painting the terrain: `terrain.sculpt`, `terrain.paint`, `terrain.layers`,
//! `terrain.paint_rule`, `terrain.remove_paint_rule`, `terrain.edits` and
//! `terrain.remove_edit`.
//!
//! Every edit is a recipe appended to the terrain's `edits` (`gamecraft_world::sculpt`): the
//! height field is the source plus the edits in order, rebuilt in the tiles a change touches
//! only. A flatten without `height_m` and a ramp without its end heights take them from the
//! ground when the command runs, and the edit keeps them, so the recipe is explicit. All of
//! them work on a running world (a crater while the game is played).

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Edit, Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use gamecraft_world::sculpt::EDIT_OPS;
use gamecraft_world::{PaintRule, Terrain, TerrainEdit, TerrainLayer};
use serde_json::{Map, Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: sculpt_spec,
        handler: Handler::World(sculpt),
    },
    Registration {
        spec: paint_spec,
        handler: Handler::World(paint),
    },
    Registration {
        spec: layers_spec,
        handler: Handler::World(layers),
    },
    Registration {
        spec: paint_rule_spec,
        handler: Handler::World(paint_rule),
    },
    Registration {
        spec: remove_paint_rule_spec,
        handler: Handler::World(remove_paint_rule),
    },
    Registration {
        spec: edits_spec,
        handler: Handler::Query(list_edits),
    },
    Registration {
        spec: remove_edit_spec,
        handler: Handler::World(remove_edit),
    },
];

/// The sculpt operations (`paint` has a command of its own).
const SCULPT_OPS: [&str; 6] = ["raise", "lower", "flatten", "smooth", "ramp", "noise"];
/// Most tiles listed by name in a result; the count is always given.
const MAX_LISTED_TILES: usize = 64;

/// The parameters each sculpt operation takes besides `op`.
fn op_params(op: &str) -> &'static [&'static str] {
    match op {
        "raise" | "lower" => &["center", "radius_m", "strength_m", "falloff"],
        "flatten" => &["center", "radius_m", "height_m", "strength", "falloff"],
        "smooth" => &["center", "radius_m", "strength", "iterations", "falloff"],
        "ramp" => &[
            "from",
            "to",
            "width_m",
            "from_height_m",
            "to_height_m",
            "strength",
            "falloff",
        ],
        "noise" => &[
            "center",
            "radius_m",
            "strength_m",
            "feature_m",
            "seed",
            "falloff",
        ],
        _ => &[],
    }
}

fn sculpt_spec() -> CommandSpec {
    spec(
        "terrain.sculpt",
        "Sculpt Terrain",
        "Adds one sculpt edit to the end of the terrain's edits: `raise` or `lower` the ground \
         in a soft circle, `flatten` it towards a height (the ground's at the centre when left \
         out), `smooth` it, `ramp` a straight slope between two points (a road; the ends' \
         heights from the ground when left out) or add `noise` detail. The brush is full \
         strength inside radius_m * (1 - falloff) and fades to nothing at radius_m. Each op \
         takes only its own parameters. The ground is rebuilt in the tiles the edit touches; \
         one undo step; works on a running world (a crater during the game).",
        vec![
            param("op", T::String, "The operation.")
                .required()
                .allowed(&SCULPT_OPS),
            param(
                "center",
                T::Point2,
                "raise, lower, flatten, smooth, noise: the brush's centre.",
            )
            .unit("m"),
            param(
                "radius_m",
                T::Number,
                "raise, lower, flatten, smooth, noise: the brush's radius, 0.1 to 16384.",
            )
            .unit("m"),
            param(
                "strength_m",
                T::Number,
                "raise, lower: the height moved at the centre; noise: the largest change; 0 to \
                 1000.",
            )
            .unit("m"),
            param(
                "strength",
                T::Number,
                "flatten, ramp: how far towards the level, 0 to 1 (default 1); smooth: how far \
                 towards the mean per pass, 0 to 1 (default 0.5).",
            ),
            param(
                "falloff",
                T::Number,
                "Share of the radius (of the half width for a ramp) that fades out, 0 (a hard \
                 edge) to 1.",
            )
            .default(json!(0.5)),
            param(
                "height_m",
                T::Number,
                "flatten: the level; the ground's height at the centre when left out.",
            )
            .unit("m"),
            param("iterations", T::Integer, "smooth: passes, 1 to 8.").default(json!(1)),
            param("from", T::Point2, "ramp: where the slope starts.").unit("m"),
            param("to", T::Point2, "ramp: where it ends.").unit("m"),
            param("width_m", T::Number, "ramp: the full width, 0.1 to 16384.").unit("m"),
            param(
                "from_height_m",
                T::Number,
                "ramp: the level at `from`; the ground's height there when left out.",
            )
            .unit("m"),
            param(
                "to_height_m",
                T::Number,
                "ramp: the level at `to`; the ground's height there when left out.",
            )
            .unit("m"),
            param(
                "feature_m",
                T::Number,
                "noise: size of the largest bumps, 1 to 100000.",
            )
            .unit("m")
            .default(json!(8)),
            param(
                "seed",
                T::Integer,
                "noise: the seed of the noise; the world's seed when left out.",
            ),
        ],
        "{index, edit: {op, ...}, edits, tiles_rebuilt, tiles_total, tiles: [[x, z]], \
         min_height_m, max_height_m}",
        json!({"op": "raise", "center": [10, -20], "radius_m": 8, "strength_m": 2}),
    )
}

/// The edit a `terrain.sculpt` call describes, its missing heights taken from the ground.
fn sculpt_edit(edit: &Edit<'_>, p: Params<'_>) -> Result<TerrainEdit, CommandError> {
    let op = p.str("op")?.unwrap_or_default();
    let own = op_params(op);
    let all = [
        "center",
        "radius_m",
        "strength_m",
        "strength",
        "falloff",
        "height_m",
        "iterations",
        "from",
        "to",
        "width_m",
        "from_height_m",
        "to_height_m",
        "feature_m",
        "seed",
    ];
    if let Some(name) = all.iter().find(|n| p.has(n) && !own.contains(n)) {
        return Err(bad(format!(
            "`{name}` does not belong to op \"{op}\"; op \"{op}\" takes {}",
            own.join(", ")
        )));
    }
    let need =
        |name: &str, v: Option<f64>| v.ok_or_else(|| bad(format!("op \"{op}\" needs `{name}`")));
    let need_point = |name: &str, v: Option<[f64; 2]>| {
        v.ok_or_else(|| bad(format!("op \"{op}\" needs `{name}` [x, z]")))
    };
    let falloff = p.f64("falloff")?.unwrap_or(0.5);
    // The ground at a point, for a height left out.
    let ground_for = |name: &str, [x, z]: [f64; 2]| {
        edit.ground(x, z).map_err(|e| {
            failed(format!(
                "{}; give `{name}` to set the level yourself",
                e.message()
            ))
        })
    };
    let edit = match op {
        "raise" | "lower" => {
            let center = need_point("center", p.point2("center")?)?;
            let radius_m = need("radius_m", p.f64("radius_m")?)?;
            let strength_m = need("strength_m", p.f64("strength_m")?)?;
            if op == "raise" {
                TerrainEdit::Raise {
                    center,
                    radius_m,
                    strength_m,
                    falloff,
                }
            } else {
                TerrainEdit::Lower {
                    center,
                    radius_m,
                    strength_m,
                    falloff,
                }
            }
        }
        "flatten" => {
            let center = need_point("center", p.point2("center")?)?;
            let radius_m = need("radius_m", p.f64("radius_m")?)?;
            let height_m = match p.f64("height_m")? {
                Some(h) => h,
                None => ground_for("height_m", center)?,
            };
            TerrainEdit::Flatten {
                center,
                radius_m,
                height_m,
                strength: p.f64("strength")?.unwrap_or(1.0),
                falloff,
            }
        }
        "smooth" => TerrainEdit::Smooth {
            center: need_point("center", p.point2("center")?)?,
            radius_m: need("radius_m", p.f64("radius_m")?)?,
            strength: p.f64("strength")?.unwrap_or(0.5),
            iterations: p.u32("iterations")?.unwrap_or(1),
            falloff,
        },
        "ramp" => {
            let from = need_point("from", p.point2("from")?)?;
            let to = need_point("to", p.point2("to")?)?;
            let width_m = need("width_m", p.f64("width_m")?)?;
            let from_height_m = match p.f64("from_height_m")? {
                Some(h) => h,
                None => ground_for("from_height_m", from)?,
            };
            let to_height_m = match p.f64("to_height_m")? {
                Some(h) => h,
                None => ground_for("to_height_m", to)?,
            };
            TerrainEdit::Ramp {
                from,
                to,
                width_m,
                from_height_m,
                to_height_m,
                strength: p.f64("strength")?.unwrap_or(1.0),
                falloff,
            }
        }
        "noise" => TerrainEdit::Noise {
            center: need_point("center", p.point2("center")?)?,
            radius_m: need("radius_m", p.f64("radius_m")?)?,
            strength_m: need("strength_m", p.f64("strength_m")?)?,
            feature_m: p.f64("feature_m")?.unwrap_or(8.0),
            seed: p.u64("seed")?.unwrap_or(edit.world.seed),
            falloff,
        },
        other => {
            return Err(bad(format!(
                "`op` is {other:?}; it must be one of {}",
                SCULPT_OPS.join(", ")
            )));
        }
    };
    Ok(edit)
}

fn sculpt(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    terrain_of(edit)?;
    let new = sculpt_edit(edit, p)?;
    add_edit(edit, new)
}

/// The terrain of the edited world, or an error that says how to get one.
fn terrain_of<'a>(edit: &'a mut Edit<'_>) -> Result<&'a mut Terrain, CommandError> {
    edit.world
        .terrain
        .as_mut()
        .ok_or_else(|| failed("the world has no terrain to edit; add one with terrain.set first"))
}

/// The edit as JSON, with its index in front.
fn edit_json(index: usize, e: &TerrainEdit) -> Result<Value, CommandError> {
    let mut v = serde_json::to_value(e).map_err(failed)?;
    if let Value::Object(m) = &mut v {
        let mut out = Map::new();
        out.insert("index".into(), json!(index));
        out.extend(std::mem::take(m));
        return Ok(Value::Object(out));
    }
    Ok(v)
}

/// Appends `new` to the terrain's edits, checks the terrain and rebuilds the ground.
fn add_edit(edit: &mut Edit<'_>, new: TerrainEdit) -> Result<Value, CommandError> {
    let terrain = terrain_of(edit)?;
    let previous = terrain.clone();
    terrain.edits.push(new.clone());
    terrain.validate().map_err(|e| bad(e.to_string()))?;
    let index = terrain.edits.len() - 1;
    let count = terrain.edits.len();
    let mut result = rebuild(edit, &previous)?;
    result.insert("index".into(), json!(index));
    result.insert("edit".into(), serde_json::to_value(&new).map_err(failed)?);
    result.insert("edits".into(), json!(count));
    Ok(Value::Object(result))
}

/// Rebuilds the ground of the edited world from that of `previous` (the tiles the change
/// touches) for the session to take over, and says what was rebuilt.
fn rebuild(edit: &mut Edit<'_>, previous: &Terrain) -> Result<Map<String, Value>, CommandError> {
    let Some(terrain) = edit.world.terrain.as_ref() else {
        return Ok(Map::new());
    };
    let rebuilt = match edit.heights() {
        Some(field) => terrain.rebuild(previous, field).map_err(failed)?,
        None => {
            let field = terrain.build().map_err(failed)?;
            let [nx, nz] = field.tile_count();
            let tiles = (0..nz).flat_map(|z| (0..nx).map(move |x| [x, z])).collect();
            gamecraft_world::Rebuilt {
                field,
                tiles,
                tiles_total: nx * nz,
            }
        }
    };
    let (lo, hi) = rebuilt.field.height_range();
    let mut out = Map::new();
    out.insert("tiles_rebuilt".into(), json!(rebuilt.tiles.len()));
    out.insert("tiles_total".into(), json!(rebuilt.tiles_total));
    let listed: Vec<[usize; 2]> = rebuilt
        .tiles
        .iter()
        .take(MAX_LISTED_TILES)
        .copied()
        .collect();
    out.insert("tiles".into(), json!(listed));
    out.insert("min_height_m".into(), json!(f64::from(lo)));
    out.insert("max_height_m".into(), json!(f64::from(hi)));
    edit.built = Some(rebuilt.field);
    Ok(out)
}

fn paint_spec() -> CommandSpec {
    spec(
        "terrain.paint",
        "Paint Terrain",
        "Adds a paint edit to the end of the terrain's edits: moves the layer weights in a soft \
         circle towards the layer named (the other layers give way); heights do not change. The \
         layer must be one of terrain.layers. One undo step; works on a running world.",
        vec![
            param("layer", T::String, "The layer to paint, by name.").required(),
            param("center", T::Point2, "The brush's centre.")
                .required()
                .unit("m"),
            param("radius_m", T::Number, "The brush's radius, 0.1 to 16384.")
                .required()
                .unit("m"),
            param(
                "strength",
                T::Number,
                "How far towards the layer at the centre, 0 to 1.",
            )
            .default(json!(1)),
            param(
                "falloff",
                T::Number,
                "Share of the radius that fades out, 0 to 1.",
            )
            .default(json!(0.5)),
        ],
        "{index, edit: {op: \"paint\", ...}, edits, tiles_rebuilt, tiles_total, tiles, \
         min_height_m, max_height_m}",
        json!({"layer": "dirt", "center": [0, 20], "radius_m": 6}),
    )
}

fn paint(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    terrain_of(edit)?;
    let new = TerrainEdit::Paint {
        layer: p.str("layer")?.unwrap_or_default().to_string(),
        center: p
            .point2("center")?
            .ok_or_else(|| bad("missing `center` [x, z]"))?,
        radius_m: p
            .f64("radius_m")?
            .ok_or_else(|| bad("missing `radius_m`"))?,
        strength: p.f64("strength")?.unwrap_or(1.0),
        falloff: p.f64("falloff")?.unwrap_or(0.5),
    };
    add_edit(edit, new)
}

fn layers_spec() -> CommandSpec {
    spec(
        "terrain.layers",
        "Terrain Layers",
        "Sets the terrain's painted layers, at most 8, in order: each {name, color, roughness}. \
         The first covers the ground wherever nothing else is painted; the renderer draws them \
         in their colours. A layer that a paint edit or rule still names cannot go (remove \
         those first). An empty list removes the layers: the renderer shades by slope again.",
        vec![
            param(
                "layers",
                T::Array,
                "The layers: [{name: 1 to 32 ASCII letters, digits, `-`, `_`; color: [r, g, b] \
                 linear, 0 to 1; roughness: 0 to 1, default 0.9}].",
            )
            .required(),
        ],
        "{layers: [{name, color, roughness}], tiles_rebuilt, tiles_total, tiles, min_height_m, \
         max_height_m}",
        json!({"layers": [
            {"name": "grass", "color": [0.075, 0.13, 0.035]},
            {"name": "rock", "color": [0.28, 0.27, 0.25], "roughness": 0.8},
            {"name": "dirt", "color": [0.2, 0.15, 0.095]},
            {"name": "snow", "color": [0.8, 0.82, 0.85], "roughness": 0.6}
        ]}),
    )
}

fn layers(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let given = p.value("layers").cloned().unwrap_or(Value::Null);
    let Value::Array(items) = &given else {
        return Err(bad("`layers` must be an array of {name, color, roughness}"));
    };
    let mut list = Vec::with_capacity(items.len().min(16));
    for (i, item) in items
        .iter()
        .enumerate()
        .take(gamecraft_world::limits::MAX_TERRAIN_LAYERS + 1)
    {
        let layer: TerrainLayer = serde_json::from_value(item.clone()).map_err(|e| {
            bad(format!(
                "`layers[{i}]`: {e}; a layer is {{name, color, roughness}}"
            ))
        })?;
        list.push(layer);
    }
    let terrain = terrain_of(edit)?;
    let previous = terrain.clone();
    terrain.layers = list;
    terrain.validate().map_err(|e| bad(e.to_string()))?;
    let shown = serde_json::to_value(&terrain.layers).map_err(failed)?;
    let mut result = rebuild(edit, &previous)?;
    result.insert("layers".into(), shown);
    Ok(Value::Object(result))
}

fn paint_rule_spec() -> CommandSpec {
    spec(
        "terrain.paint_rule",
        "Paint Rule",
        "Adds an automatic paint rule to the end of the terrain's rules: where the ground's \
         slope and height are within the bounds given (each with a soft edge of 4 degrees or 4 \
         m), the layer covers it; rock above 35 degrees, snow above 300 m. Rules apply in \
         order over the sculpted heights, before the paint edits. One undo step.",
        vec![
            param("layer", T::String, "The layer it paints, by name.").required(),
            param("slope_min_deg", T::Number, "Steeper than this, 0 to 90.").unit("deg"),
            param("slope_max_deg", T::Number, "Less steep than this, 0 to 90.").unit("deg"),
            param("height_min_m", T::Number, "Higher than this.").unit("m"),
            param("height_max_m", T::Number, "Lower than this.").unit("m"),
        ],
        "{index, rule, paint_rules, tiles_rebuilt, tiles_total, tiles, min_height_m, \
         max_height_m}",
        json!({"layer": "rock", "slope_min_deg": 30}),
    )
}

fn paint_rule(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let rule = PaintRule {
        layer: p.str("layer")?.unwrap_or_default().to_string(),
        slope_min_deg: p.f64("slope_min_deg")?,
        slope_max_deg: p.f64("slope_max_deg")?,
        height_min_m: p.f64("height_min_m")?,
        height_max_m: p.f64("height_max_m")?,
    };
    let terrain = terrain_of(edit)?;
    let previous = terrain.clone();
    terrain.paint_rules.push(rule.clone());
    terrain.validate().map_err(|e| bad(e.to_string()))?;
    let index = terrain.paint_rules.len() - 1;
    let count = terrain.paint_rules.len();
    let mut result = rebuild(edit, &previous)?;
    result.insert("index".into(), json!(index));
    result.insert("rule".into(), serde_json::to_value(&rule).map_err(failed)?);
    result.insert("paint_rules".into(), json!(count));
    Ok(Value::Object(result))
}

fn remove_paint_rule_spec() -> CommandSpec {
    spec(
        "terrain.remove_paint_rule",
        "Remove Paint Rule",
        "Removes the terrain's paint rule at `index` (as terrain.edits lists them); the later \
         rules move up one. One undo step.",
        vec![param("index", T::Integer, "The rule's place, from 0.").required()],
        "{removed: {layer, ...}, paint_rules, tiles_rebuilt, tiles_total, tiles, min_height_m, \
         max_height_m}",
        json!({"index": 0}),
    )
}

fn remove_paint_rule(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let index = p.u64("index")?.unwrap_or(u64::MAX);
    let terrain = terrain_of(edit)?;
    let previous = terrain.clone();
    let count = terrain.paint_rules.len();
    let i = usize::try_from(index)
        .ok()
        .filter(|i| *i < count)
        .ok_or_else(|| bad(out_of_range("index", index, count, "paint rule")))?;
    let removed = terrain.paint_rules.remove(i);
    let left = terrain.paint_rules.len();
    let mut result = rebuild(edit, &previous)?;
    result.insert(
        "removed".into(),
        serde_json::to_value(&removed).map_err(failed)?,
    );
    result.insert("paint_rules".into(), json!(left));
    Ok(Value::Object(result))
}

/// The sentence for an index that is not one of `count` things.
fn out_of_range(name: &str, index: u64, count: usize, what: &str) -> String {
    match count {
        0 => format!("`{name}` is {index}, but the terrain has no {what}s"),
        1 => format!("`{name}` is {index}, but the terrain has 1 {what}: use 0"),
        n => format!(
            "`{name}` is {index}, but the terrain has {n} {what}s: use 0 to {}",
            n - 1
        ),
    }
}

fn edits_spec() -> CommandSpec {
    spec(
        "terrain.edits",
        "Terrain Edits",
        "Lists the current world's terrain recipe in full: the source, the painted layers, the \
         paint rules and every edit in order with its index (what terrain.remove_edit takes), \
         and the tiles of the height field. Null fields without terrain.",
        vec![],
        "{size_m, cell_m, source, layers: [{name, color, roughness}], paint_rules: [{layer, \
         slope_min_deg?, slope_max_deg?, height_min_m?, height_max_m?}], edits: [{index, op, \
         ...}], ops: [names], tiles: [x, z], samples: [cols, rows]} | {terrain: null}",
        json!({}),
    )
    .query()
}

fn list_edits(session: &Session, _p: Params<'_>) -> Result<Value, CommandError> {
    let Some(t) = &session.current_world().terrain else {
        return Ok(json!({"terrain": null}));
    };
    let edits = t
        .edits
        .iter()
        .enumerate()
        .map(|(i, e)| edit_json(i, e))
        .collect::<Result<Vec<_>, _>>()?;
    let field = session.current_heights();
    Ok(json!({
        "size_m": t.size_m,
        "cell_m": t.cell_m,
        "source": serde_json::to_value(&t.source).map_err(failed)?,
        "layers": serde_json::to_value(&t.layers).map_err(failed)?,
        "paint_rules": serde_json::to_value(&t.paint_rules).map_err(failed)?,
        "edits": edits,
        "ops": EDIT_OPS,
        "tiles": field.map(|f| f.tile_count()),
        "samples": field.map(|f| [f.cols(), f.rows()]),
    }))
}

fn remove_edit_spec() -> CommandSpec {
    spec(
        "terrain.remove_edit",
        "Remove Terrain Edit",
        "Removes the terrain edit at `index` (as terrain.edits lists them); the later edits \
         move up one and the ground is rebuilt where the edit and what came after it reach. \
         One undo step; works on a running world.",
        vec![param("index", T::Integer, "The edit's place, from 0.").required()],
        "{removed: {op, ...}, edits, tiles_rebuilt, tiles_total, tiles, min_height_m, \
         max_height_m}",
        json!({"index": 0}),
    )
}

fn remove_edit(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let index = p.u64("index")?.unwrap_or(u64::MAX);
    let terrain = terrain_of(edit)?;
    let previous = terrain.clone();
    let count = terrain.edits.len();
    let i = usize::try_from(index)
        .ok()
        .filter(|i| *i < count)
        .ok_or_else(|| bad(out_of_range("index", index, count, "edit")))?;
    let removed = terrain.edits.remove(i);
    let left = terrain.edits.len();
    let mut result = rebuild(edit, &previous)?;
    result.insert(
        "removed".into(),
        serde_json::to_value(&removed).map_err(failed)?,
    );
    result.insert("edits".into(), json!(left));
    Ok(Value::Object(result))
}
