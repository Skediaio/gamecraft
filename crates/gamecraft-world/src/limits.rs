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
