//! The named limits of a world document. [`crate::World::validate`] enforces all of them.

/// Smallest terrain side, metres.
pub const TERRAIN_SIZE_MIN_M: f64 = 8.0;
/// Largest terrain side, metres.
pub const TERRAIN_SIZE_MAX_M: f64 = 16384.0;
/// Smallest terrain cell, metres.
pub const TERRAIN_CELL_MIN_M: f64 = 0.25;
/// Largest terrain cell, metres.
pub const TERRAIN_CELL_MAX_M: f64 = 64.0;
/// Most height samples along one side of a terrain (2048 cells).
pub const TERRAIN_MAX_SAMPLES_PER_SIDE: usize = 2049;
/// Relative tolerance when checking that `size_m` is a whole multiple of `cell_m`.
pub const TERRAIN_MULTIPLE_TOLERANCE: f64 = 1e-9;
/// Cells along one side of a terrain tile (a tile holds 129 x 129 samples, sharing its border
/// samples with its neighbours).
pub const TERRAIN_TILE_CELLS: usize = 128;
/// Most edits of one terrain.
pub const MAX_TERRAIN_EDITS: usize = 1024;
/// Most painted layers of one terrain (the renderer's eight).
pub const MAX_TERRAIN_LAYERS: usize = 8;
/// Longest terrain layer name, in characters.
pub const MAX_TERRAIN_LAYER_NAME_CHARS: usize = 32;
/// Most paint rules of one terrain.
pub const MAX_PAINT_RULES: usize = 64;
/// Smallest radius (or ramp width) of a terrain edit, metres.
pub const SCULPT_RADIUS_MIN_M: f64 = 0.1;
/// Largest height a raise, lower or noise edit moves the ground, metres.
pub const SCULPT_STRENGTH_MAX_M: f64 = 1000.0;
/// Most passes of a smooth edit.
pub const SMOOTH_ITERATIONS_MAX: u32 = 8;
/// Most sample updates building a terrain's edits may take (the samples inside each edit's
/// square times a smooth's passes): about a second of work.
pub const TERRAIN_EDIT_WORK_MAX: u64 = 1 << 28;

/// Fewest noise octaves.
pub const NOISE_OCTAVES_MIN: u32 = 1;
/// Most noise octaves.
pub const NOISE_OCTAVES_MAX: u32 = 10;
/// Smallest noise roughness (the gain between octaves).
pub const NOISE_ROUGHNESS_MIN: f64 = 0.0;
/// Largest noise roughness.
pub const NOISE_ROUGHNESS_MAX: f64 = 1.0;
/// Smallest noise relief (peak to trough), metres.
pub const NOISE_RELIEF_MIN_M: f64 = 0.0;
/// Largest noise relief (peak to trough), metres.
pub const NOISE_RELIEF_MAX_M: f64 = 5000.0;
/// Smallest noise feature size, metres.
pub const NOISE_FEATURE_MIN_M: f64 = 1.0;
/// Largest noise feature size, metres.
pub const NOISE_FEATURE_MAX_M: f64 = 100_000.0;

/// Most prefabs in one world.
pub const MAX_PREFABS: usize = 1000;
/// Most children of one prefab.
pub const MAX_PREFAB_CHILDREN: usize = 64;
/// Deepest chain of parents above an entity (a loop in a bad file is longer than any).
pub const MAX_HIERARCHY_DEPTH: usize = 16;
/// Most entities in one world.
pub const MAX_ENTITIES: usize = 1_000_000;
/// Longest name (of a world, an entity or a plant species), in characters.
pub const MAX_NAME_CHARS: usize = 200;
/// Largest absolute value of any coordinate (positions, flat terrain height), metres.
pub const MAX_COORDINATE_M: f64 = 1e6;
/// Smallest entity scale.
pub const SCALE_MIN: f64 = 0.001;
/// Largest entity scale.
pub const SCALE_MAX: f64 = 1000.0;
/// Smallest extent of a shape along an axis, metres.
pub const SHAPE_SIZE_MIN_M: f64 = 0.001;
/// Largest extent of a shape along an axis, metres.
pub const SHAPE_SIZE_MAX_M: f64 = 10_000.0;

/// End of the day (exclusive), hours of local solar time.
pub const TIME_OF_DAY_MAX_H: f64 = 24.0;
/// First day of the year.
pub const DAY_OF_YEAR_MIN: u32 = 1;
/// Last day of the year (leap years included).
pub const DAY_OF_YEAR_MAX: u32 = 366;
/// Largest absolute latitude, degrees.
pub const LATITUDE_MAX_DEG: f64 = 90.0;
/// Strongest wind, metres per second.
pub const WIND_SPEED_MAX_MPS: f64 = 100.0;

/// Smallest walkable slope limit of a character, degrees.
pub const MAX_SLOPE_MIN_DEG: f64 = 1.0;
/// Largest walkable slope limit of a character, degrees.
pub const MAX_SLOPE_MAX_DEG: f64 = 89.0;

/// Longest free text (an objective's title, a variable's string, a prompt), in characters.
pub const MAX_TEXT_CHARS: usize = 2000;
/// Most tags on one entity.
pub const MAX_TAGS: usize = 64;
/// Largest reach of an `interactable`, metres.
pub const INTERACT_REACH_MAX_M: f64 = 100.0;
/// Strongest light, candela.
pub const LIGHT_INTENSITY_MAX_CD: f64 = 1e7;
/// Largest range of a light, metres.
pub const LIGHT_RANGE_MAX_M: f64 = 10_000.0;

/// Most variables in one world.
pub const MAX_VARS: usize = 10_000;
/// Most objectives in one world.
pub const MAX_OBJECTIVES: usize = 1000;
/// Most triggers in one world.
pub const MAX_TRIGGERS: usize = 1000;
/// Most commands in one trigger's `do`.
pub const MAX_TRIGGER_STEPS: usize = 64;
/// Deepest nesting of `all` and `any` in a trigger's condition (a comparison alone is depth 1).
pub const MAX_CONDITION_DEPTH: usize = 8;
/// Most conditions inside one `all` or `any`.
pub const MAX_CONDITION_ITEMS: usize = 64;
/// Latest `at_s` of a `time` event, seconds of simulated time.
pub const TRIGGER_TIME_MAX_S: f64 = 1e9;

/// Coldest air at the world's ground level (`environment.temperature_c`), degrees Celsius.
pub const TEMPERATURE_MIN_C: f64 = -90.0;
/// Hottest air at the world's ground level, degrees Celsius.
pub const TEMPERATURE_MAX_C: f64 = 60.0;
/// Lowest height of world y = 0 above sea level (`environment.sea_level_m`), metres.
pub const SEA_LEVEL_MIN_M: f64 = -500.0;
/// Highest height of world y = 0 above sea level, metres: the top of the troposphere, where the
/// barometric formula of the standard atmosphere used for the air ends.
pub const SEA_LEVEL_MAX_M: f64 = 11_000.0;

/// Fastest muzzle velocity of a `launcher`, metres per second.
pub const MUZZLE_VELOCITY_MAX_MPS: f64 = 3000.0;
/// Heaviest projectile of a `launcher`, kilograms.
pub const PROJECTILE_MASS_MAX_KG: f64 = 1000.0;
/// Largest ballistic coefficient of a `launcher`, kilograms per square metre.
pub const BALLISTIC_COEFFICIENT_MAX: f64 = 100_000.0;
/// Widest dispersion of a `launcher`, milliradians.
pub const DISPERSION_MAX_MRAD: f64 = 500.0;
/// Longest time between shots, or of a reload, seconds.
pub const LAUNCHER_TIME_MAX_S: f64 = 3600.0;
/// Most projectiles in one magazine of a `launcher`.
pub const MAGAZINE_MAX: u32 = 100_000;
/// Largest damage per joule of a `launcher`.
pub const DAMAGE_PER_JOULE_MAX: f64 = 1e6;
/// Longest flight of a projectile, seconds.
pub const MAX_FLIGHT_MAX_S: f64 = 600.0;
/// Largest range at which a shot is heard, metres.
pub const SOUND_RANGE_MAX_M: f64 = 100_000.0;
/// Most health (and armour) of a `health` component.
pub const HEALTH_MAX: f64 = 1e9;
/// Most points of a `behaviour`'s route.
pub const MAX_ROUTE_POINTS: usize = 256;
/// Largest view, engage or search range of a `behaviour`, metres.
pub const BEHAVIOUR_RANGE_MAX_M: f64 = 10_000.0;
/// Largest aim error of a `behaviour`, milliradians.
pub const AIM_ERROR_MAX_MRAD: f64 = 1000.0;
/// Longest reaction or search time of a `behaviour`, seconds.
pub const BEHAVIOUR_TIME_MAX_S: f64 = 3600.0;
/// Narrowest vertical field of view of a world's player camera, degrees.
pub const VIEW_FOV_MIN_DEG: f64 = 30.0;
/// Widest vertical field of view of a world's player camera, degrees.
pub const VIEW_FOV_MAX_DEG: f64 = 120.0;
/// Farthest a `held_model` sits from the camera along any axis, metres.
pub const HELD_OFFSET_MAX_M: f64 = 2.0;
/// Farthest a `behaviour` looks for cover, metres.
pub const COVER_SEARCH_MAX_M: f64 = 100.0;
