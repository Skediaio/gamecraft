//! World-wide settings: `world.set`, `terrain.set`, `terrain.clear`, `environment.set`,
//! `view.set`.

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Edit, Handler, Registration};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use gamecraft_world::{Environment, Terrain, TerrainSource, ViewMode, ViewRules, World};
use serde_json::{Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: world_set_spec,
        handler: Handler::World(world_set),
    },
    Registration {
        spec: terrain_set_spec,
        handler: Handler::World(terrain_set),
    },
    Registration {
        spec: terrain_clear_spec,
        handler: Handler::World(terrain_clear),
    },
    Registration {
        spec: environment_set_spec,
        handler: Handler::World(environment_set),
    },
    Registration {
        spec: view_set_spec,
        handler: Handler::World(view_set),
    },
];

fn world_set_spec() -> CommandSpec {
    spec(
        "world.set",
        "World Settings",
        "Renames the world or changes its seed; only the fields given change. Generators derive \
         their random streams from the seed.",
        vec![
            param(
                "name",
                T::String,
                "The world's name, at most 200 characters.",
            ),
            param("seed", T::Integer, "The world seed, 0 to 2^64 - 1."),
        ],
        "null",
        json!({"name": "Green valley", "seed": 42}),
    )
}

fn world_set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    if let Some(name) = p.str("name")? {
        edit.world.name = name.to_string();
    }
    if let Some(seed) = p.u64("seed")? {
        edit.world.seed = seed;
    }
    probe(|w| w.name = edit.world.name.clone())?;
    Ok(Value::Null)
}

/// Validates the part of a world that `set` writes, on an otherwise empty world (cheap: no
/// entities), and refuses a problem as bad parameters.
fn probe(set: impl FnOnce(&mut World)) -> Result<(), CommandError> {
    let mut w = World::new("", 0);
    set(&mut w);
    w.validate().map_err(|e| bad(e.to_string()))
}

const NOISE_ONLY: [&str; 5] = ["seed", "relief_m", "feature_m", "octaves", "roughness"];
const FLAT_ONLY: [&str; 1] = ["height_m"];

fn terrain_set_spec() -> CommandSpec {
    spec(
        "terrain.set",
        "Set Terrain",
        "Replaces the terrain with a square centred on the origin: flat, or rolling land from \
         seeded noise. Its painted layers, paint rules and sculpt edits (in world coordinates) \
         stay; terrain.clear drops them. Entities are not moved (entity.snap_to_ground puts \
         them on the new ground). Parameters of the other kind are refused.",
        vec![
            param("kind", T::String, "Where the heights come from.")
                .allowed(&["flat", "noise"])
                .default(json!("noise")),
            param(
                "size_m",
                T::Number,
                "Side of the square, 8 to 16384; a whole multiple of cell_m.",
            )
            .unit("m")
            .default(json!(256)),
            param(
                "cell_m",
                T::Number,
                "Distance between height samples, 0.25 to 64; at most 2049 samples per side.",
            )
            .unit("m")
            .default(json!(2)),
            param("height_m", T::Number, "flat: the height of the ground.")
                .unit("m")
                .default(json!(0)),
            param(
                "seed",
                T::Integer,
                "noise: the seed of the noise; the world's seed when left out.",
            ),
            param(
                "relief_m",
                T::Number,
                "noise: peak-to-trough height, 0 to 5000.",
            )
            .unit("m")
            .default(json!(40)),
            param(
                "feature_m",
                T::Number,
                "noise: size of the largest hills, 1 to 100000.",
            )
            .unit("m")
            .default(json!(200)),
            param("octaves", T::Integer, "noise: layers of detail, 1 to 10.").default(json!(5)),
            param(
                "roughness",
                T::Number,
                "noise: amplitude ratio between layers, 0 (smooth) to 1 (rough).",
            )
            .default(json!(0.5)),
        ],
        "{cols, rows, min_height_m, max_height_m}",
        json!({"kind": "noise", "size_m": 128, "cell_m": 2, "relief_m": 20, "seed": 3}),
    )
}

fn terrain_set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let kind = p.str("kind")?.unwrap_or("noise");
    let (other, foreign): (&str, &[&str]) = if kind == "flat" {
        ("noise", &NOISE_ONLY)
    } else {
        ("flat", &FLAT_ONLY)
    };
    if let Some(name) = foreign.iter().find(|n| p.has(n)) {
        return Err(bad(format!(
            "`{name}` belongs to kind \"{other}\", not to kind \"{kind}\"; leave it out or set \
             kind \"{other}\""
        )));
    }
    let source = if kind == "flat" {
        TerrainSource::Flat {
            height_m: p.f64("height_m")?.unwrap_or(0.0),
        }
    } else {
        TerrainSource::Noise {
            seed: p.u64("seed")?.unwrap_or(edit.world.seed),
            relief_m: p.f64("relief_m")?.unwrap_or(40.0),
            feature_m: p.f64("feature_m")?.unwrap_or(200.0),
            octaves: p.u32("octaves")?.unwrap_or(5),
            roughness: p.f64("roughness")?.unwrap_or(0.5),
        }
    };
    // The layers, rules and edits are in world coordinates: they stay.
    let kept = edit.world.terrain.take().unwrap_or_default();
    let terrain = Terrain {
        size_m: p.f64("size_m")?.unwrap_or(256.0),
        cell_m: p.f64("cell_m")?.unwrap_or(2.0),
        source,
        layers: kept.layers,
        paint_rules: kept.paint_rules,
        edits: kept.edits,
    };
    terrain.validate().map_err(|e| bad(e.to_string()))?;
    let field = terrain.build().map_err(failed)?;
    let (lo, hi) = field.height_range();
    let result = json!({
        "cols": field.cols(),
        "rows": field.rows(),
        "min_height_m": f64::from(lo),
        "max_height_m": f64::from(hi),
    });
    edit.world.terrain = Some(terrain);
    edit.built = Some(field);
    Ok(result)
}

fn terrain_clear_spec() -> CommandSpec {
    spec(
        "terrain.clear",
        "Clear Terrain",
        "Removes the terrain with its layers, paint rules and edits: the world has no ground. \
         Entities stay where they are.",
        vec![],
        "null",
        json!({}),
    )
}

fn terrain_clear(edit: &mut Edit<'_>, _p: Params<'_>) -> Result<Value, CommandError> {
    edit.world.terrain = None;
    Ok(Value::Null)
}

fn environment_set_spec() -> CommandSpec {
    spec(
        "environment.set",
        "Set Environment",
        "Sets the time, the place on Earth and the weather; only the fields given change. The \
         date, time and latitude give the sun.",
        vec![
            param(
                "time_of_day_h",
                T::Number,
                "Local solar time, at least 0 and below 24; 12 is the sun's highest point.",
            )
            .unit("h"),
            param("day_of_year", T::Integer, "Day of the year, 1 to 366."),
            param(
                "latitude_deg",
                T::Number,
                "Latitude, -90 (south pole) to 90 (north pole).",
            )
            .unit("deg"),
            param(
                "wind_direction_deg",
                T::Number,
                "Compass direction the wind comes from, clockwise from north (90: from the east).",
            )
            .unit("deg"),
            param("wind_speed_mps", T::Number, "Wind speed, 0 to 100.").unit("m/s"),
            param(
                "cloud_cover",
                T::Number,
                "Fraction of the sky covered by cloud, 0 to 1.",
            ),
            param("mist", T::Number, "Mist density, 0 to 1."),
            param("rain", T::Number, "Rain intensity, 0 to 1."),
            param(
                "temperature_c",
                T::Number,
                "Air temperature at world y = 0, -90 to 60; with sea_level_m it sets the air \
                 density projectiles fly through.",
            )
            .unit("degC"),
            param(
                "sea_level_m",
                T::Number,
                "Height of world y = 0 above sea level, -500 to 11000.",
            )
            .unit("m"),
        ],
        "{environment: {time_of_day_h, day_of_year, latitude_deg, wind_direction_deg, \
         wind_speed_mps, cloud_cover, mist, rain, temperature_c, sea_level_m}, sun: \
         {azimuth_deg, elevation_deg}}",
        json!({"time_of_day_h": 18.5, "cloud_cover": 0.6, "rain": 0.2}),
    )
}

/// The environment as JSON, every field shown (a file leaves out the air's defaults).
pub(crate) fn environment_json(env: &Environment) -> Result<Value, CommandError> {
    let mut v = serde_json::to_value(env).map_err(failed)?;
    if let Value::Object(m) = &mut v {
        m.insert("temperature_c".into(), json!(env.temperature_c));
        m.insert("sea_level_m".into(), json!(env.sea_level_m));
    }
    Ok(v)
}

fn environment_set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let mut env: Environment = edit.world.environment.clone();
    let fields: [(&str, &mut f64); 9] = [
        ("time_of_day_h", &mut env.time_of_day_h),
        ("latitude_deg", &mut env.latitude_deg),
        ("wind_direction_deg", &mut env.wind_direction_deg),
        ("wind_speed_mps", &mut env.wind_speed_mps),
        ("cloud_cover", &mut env.cloud_cover),
        ("mist", &mut env.mist),
        ("rain", &mut env.rain),
        ("temperature_c", &mut env.temperature_c),
        ("sea_level_m", &mut env.sea_level_m),
    ];
    for (name, slot) in fields {
        if let Some(v) = p.f64(name)? {
            *slot = v;
        }
    }
    if let Some(day) = p.u32("day_of_year")? {
        env.day_of_year = day;
    }
    probe(|w| w.environment = env.clone())?;
    let sun = env.sun();
    let environment = environment_json(&env)?;
    edit.world.environment = env;
    Ok(json!({
        "environment": environment,
        "sun": {"azimuth_deg": sun.azimuth_deg, "elevation_deg": sun.elevation_deg},
    }))
}

/// Most characters of a wrong value quoted in an error.
const MAX_SHOWN_CHARS: usize = 80;

fn view_set_spec() -> CommandSpec {
    spec(
        "view.set",
        "Set View Rules",
        "Sets which views a player may use and how wide the player's camera sees; only the \
         fields given change. A window starts in the first view listed and switches only \
         among those listed; [\"first\"] forbids the third-person view. A player may still \
         widen or narrow the field of view for themselves (the window's --fov).",
        vec![
            param(
                "modes",
                T::Array,
                "The views allowed, in order, at least one and none twice: \"first\" (from the \
                 eyes) and \"third\" (behind and above).",
            ),
            param(
                "fov_y_deg",
                T::Number,
                "Vertical field of view of the player's camera, 30 to 120.",
            )
            .unit("deg"),
        ],
        "{view: {modes, fov_y_deg}}",
        json!({"modes": ["first"], "fov_y_deg": 65}),
    )
}

fn view_set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let mut rules: ViewRules = edit.world.view.clone();
    if let Some(modes) = p.value("modes") {
        rules.modes = serde_json::from_value::<Vec<ViewMode>>(modes.clone()).map_err(|_| {
            let shown: String = modes.to_string().chars().take(MAX_SHOWN_CHARS).collect();
            bad(format!(
                "`modes` must be a list of views, \"first\" and \"third\", such as \
                 [\"first\"] or [\"third\", \"first\"]; got {shown}"
            ))
        })?;
    }
    if let Some(fov) = p.f64("fov_y_deg")? {
        rules.fov_y_deg = fov;
    }
    probe(|w| w.view = rules.clone())?;
    let view = serde_json::to_value(&rules).map_err(failed)?;
    edit.world.view = rules;
    Ok(json!({ "view": view }))
}
