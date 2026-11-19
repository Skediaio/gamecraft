//! Sculpted and painted terrain: edits over the terrain's source, painted material layers and
//! automatic paint rules, and how the height field and the layer weights follow from them.
//!
//! **Recipes over results** (AGENTS.md rule 2). A terrain keeps its source (flat or noise)
//! and an ordered list of [`TerrainEdit`]s. The height of a sample is the source plus the
//! height edits applied in order; its layer weights are the paint rules over the final
//! heights, then the paint edits in order. Every edit is explicit: a flatten names its height
//! and a ramp the heights of its ends (the commands resolve them from the ground when they
//! are left out), so an edit reads nothing but the sample it changes, except `smooth`, which
//! reads the neighbours within `iterations` samples.
//!
//! **Evaluation by region.** [`eval_heights`] computes the final heights of any rectangle of
//! samples, bit for bit what the whole grid gives there: going backwards through the edits,
//! the rectangle grows by `iterations` samples for every `smooth` that reaches it, the source
//! is sampled over the grown rectangle, and the edits are applied forwards. That is what makes
//! a tile rebuild ([`crate::Terrain::rebuild`]) and a tile cache ([`crate::tiles`]) exact.
//!
//! Only `+ - * /`, `sqrt`, `floor` and `gamecraft_math` (noise, `patan`) are used: the same
//! terrain gives the same bits on every platform.

use crate::WorldError;
use crate::error::Ctx;
use crate::limits::{
    MAX_COORDINATE_M, MAX_PAINT_RULES, MAX_TERRAIN_EDITS, MAX_TERRAIN_LAYER_NAME_CHARS,
    MAX_TERRAIN_LAYERS, NOISE_FEATURE_MAX_M, NOISE_FEATURE_MIN_M, SCULPT_RADIUS_MIN_M,
    SCULPT_STRENGTH_MAX_M, SMOOTH_ITERATIONS_MAX, TERRAIN_EDIT_WORK_MAX, TERRAIN_SIZE_MAX_M,
};
use gamecraft_math::noise::fbm2;
use gamecraft_math::{Portable, rad_to_deg};
use serde::{Deserialize, Serialize};

/// Octaves of the detail a `noise` edit adds.
pub const NOISE_EDIT_OCTAVES: u32 = 4;
/// Width of the soft edge of a paint rule's slope bounds, degrees.
pub const RULE_SLOPE_BLEND_DEG: f64 = 4.0;
/// Width of the soft edge of a paint rule's height bounds, metres.
pub const RULE_HEIGHT_BLEND_M: f64 = 4.0;
/// Layer weights per sample: the renderer's eight layers.
pub const WEIGHT_SLOTS: usize = 8;

fn default_falloff() -> f64 {
    0.5
}

fn full() -> f64 {
    1.0
}

fn half() -> f64 {
    0.5
}

fn one_pass() -> u32 {
    1
}

fn default_layer_roughness() -> f64 {
    0.9
}

fn default_noise_feature_m() -> f64 {
    8.0
}

/// A painted material of a terrain: a name the paint edits and rules refer to, a colour and a
/// roughness. The first layer is the ground everywhere nothing else is painted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainLayer {
    /// 1 to 32 ASCII letters, digits, `-` and `_`; unique within the terrain.
    pub name: String,
    /// Linear RGB albedo, each 0 to 1.
    pub color: [f64; 3],
    /// Surface roughness, 0 (polished) to 1 (matte). Default 0.9.
    #[serde(default = "default_layer_roughness")]
    pub roughness: f64,
}

/// An automatic paint rule: where the ground's slope and height are within the bounds given,
/// the layer covers it. Rules apply in order over the final heights, before the paint edits;
/// each bound has a soft edge ([`RULE_SLOPE_BLEND_DEG`], [`RULE_HEIGHT_BLEND_M`]) centred on
/// it. A rule without bounds covers everything.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaintRule {
    /// The layer it paints, by name.
    pub layer: String,
    /// Steeper than this, degrees (0 to 90).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slope_min_deg: Option<f64>,
    /// Less steep than this, degrees (0 to 90).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slope_max_deg: Option<f64>,
    /// Higher than this, metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_min_m: Option<f64>,
    /// Lower than this, metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_max_m: Option<f64>,
}

/// One edit of a terrain, a recipe. In JSON the `op` field names the variant.
///
/// The brushes (`raise`, `lower`, `flatten`, `smooth`, `noise`, `paint`) act in a circle of
/// `radius_m` round `center` `[x, z]` with a weight `w`: 1 inside `radius_m * (1 - falloff)`,
/// then a smoothstep down to 0 at `radius_m`. Nothing changes at or beyond `radius_m`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
pub enum TerrainEdit {
    /// `h += strength_m * w`.
    Raise {
        /// Centre `[x, z]`, metres.
        center: [f64; 2],
        /// Radius, metres.
        radius_m: f64,
        /// Height added at the centre, metres, 0 to 1000.
        strength_m: f64,
        /// Share of the radius that fades out, 0 (a hard edge) to 1. Default 0.5.
        #[serde(default = "default_falloff")]
        falloff: f64,
    },
    /// `h -= strength_m * w`.
    Lower {
        /// Centre `[x, z]`, metres.
        center: [f64; 2],
        /// Radius, metres.
        radius_m: f64,
        /// Depth taken away at the centre, metres, 0 to 1000.
        strength_m: f64,
        /// Share of the radius that fades out, 0 to 1. Default 0.5.
        #[serde(default = "default_falloff")]
        falloff: f64,
    },
    /// `h += (height_m - h) * strength * w`: towards a level.
    Flatten {
        /// Centre `[x, z]`, metres.
        center: [f64; 2],
        /// Radius, metres.
        radius_m: f64,
        /// The level, metres.
        height_m: f64,
        /// How far towards the level, 0 to 1. Default 1.
        #[serde(default = "full")]
        strength: f64,
        /// Share of the radius that fades out, 0 to 1. Default 0.5.
        #[serde(default = "default_falloff")]
        falloff: f64,
    },
    /// `iterations` times `h += (mean - h) * strength * w`, where `mean` is the mean of the
    /// 3 x 3 samples round it (clamped at the terrain's edge) before the pass.
    Smooth {
        /// Centre `[x, z]`, metres.
        center: [f64; 2],
        /// Radius, metres.
        radius_m: f64,
        /// How far towards the mean per pass, 0 to 1. Default 0.5.
        #[serde(default = "half")]
        strength: f64,
        /// Passes, 1 to 8. Default 1.
        #[serde(default = "one_pass")]
        iterations: u32,
        /// Share of the radius that fades out, 0 to 1. Default 0.5.
        #[serde(default = "default_falloff")]
        falloff: f64,
    },
    /// A straight slope from `from` to `to`, `width_m` wide: `h += (target - h) * strength *
    /// w`, where `target` runs linearly from `from_height_m` to `to_height_m` along the line
    /// and `w` is the brush weight of the distance to the segment with radius `width_m / 2`.
    /// Its ends are round. For roads.
    Ramp {
        /// Start `[x, z]`, metres.
        from: [f64; 2],
        /// End `[x, z]`, metres.
        to: [f64; 2],
        /// Full width, metres.
        width_m: f64,
        /// The level at `from`, metres.
        from_height_m: f64,
        /// The level at `to`, metres.
        to_height_m: f64,
        /// How far towards the slope, 0 to 1. Default 1.
        #[serde(default = "full")]
        strength: f64,
        /// Share of the half width that fades out, 0 to 1. Default 0.5.
        #[serde(default = "default_falloff")]
        falloff: f64,
    },
    /// `h += strength_m * w * fbm2(seed, x / feature_m, z / feature_m)`: detail in a circle.
    Noise {
        /// Centre `[x, z]`, metres.
        center: [f64; 2],
        /// Radius, metres.
        radius_m: f64,
        /// Largest change, metres, 0 to 1000.
        strength_m: f64,
        /// Size of the largest bumps, metres, 1 to 100000. Default 8.
        #[serde(default = "default_noise_feature_m")]
        feature_m: f64,
        /// Seed of the noise. Default 0.
        #[serde(default)]
        seed: u64,
        /// Share of the radius that fades out, 0 to 1. Default 0.5.
        #[serde(default = "default_falloff")]
        falloff: f64,
    },
    /// Moves the layer weights towards `layer` by `strength * w` (the other layers give way in
    /// proportion); heights do not change.
    Paint {
        /// The layer, by name.
        layer: String,
        /// Centre `[x, z]`, metres.
        center: [f64; 2],
        /// Radius, metres.
        radius_m: f64,
        /// How far towards the layer, 0 to 1. Default 1.
        #[serde(default = "full")]
        strength: f64,
        /// Share of the radius that fades out, 0 to 1. Default 0.5.
        #[serde(default = "default_falloff")]
        falloff: f64,
    },
}

/// The names of the edit operations, in documentation order.
pub const EDIT_OPS: [&str; 7] = [
    "raise", "lower", "flatten", "smooth", "ramp", "noise", "paint",
];

impl TerrainEdit {
    /// The operation's name (`raise`, `ramp`, `paint`, ...).
    pub fn op(&self) -> &'static str {
        match self {
            TerrainEdit::Raise { .. } => "raise",
            TerrainEdit::Lower { .. } => "lower",
            TerrainEdit::Flatten { .. } => "flatten",
            TerrainEdit::Smooth { .. } => "smooth",
            TerrainEdit::Ramp { .. } => "ramp",
            TerrainEdit::Noise { .. } => "noise",
            TerrainEdit::Paint { .. } => "paint",
        }
    }

    /// Whether it changes layer weights (a `paint`) rather than heights.
    pub fn is_paint(&self) -> bool {
        matches!(self, TerrainEdit::Paint { .. })
    }

    /// The square `[x0, z0, x1, z1]` (metres) outside which it changes nothing.
    pub fn bounds(&self) -> [f64; 4] {
        match self {
            TerrainEdit::Raise {
                center, radius_m, ..
            }
            | TerrainEdit::Lower {
                center, radius_m, ..
            }
            | TerrainEdit::Flatten {
                center, radius_m, ..
            }
            | TerrainEdit::Smooth {
                center, radius_m, ..
            }
            | TerrainEdit::Noise {
                center, radius_m, ..
            }
            | TerrainEdit::Paint {
                center, radius_m, ..
            } => {
                let [x, z] = *center;
                [x - radius_m, z - radius_m, x + radius_m, z + radius_m]
            }
            TerrainEdit::Ramp {
                from, to, width_m, ..
            } => {
                let r = width_m / 2.0;
                let ([ax, az], [bx, bz]) = (*from, *to);
                [
                    ax.min(bx) - r,
                    az.min(bz) - r,
                    ax.max(bx) + r,
                    az.max(bz) + r,
                ]
            }
        }
    }

    /// How many samples away a sample's new height may read: `iterations` for a smooth, else 0.
    fn reach(&self) -> usize {
        match self {
            TerrainEdit::Smooth { iterations, .. } => *iterations as usize,
            _ => 0,
        }
    }

    /// Passes over its samples.
    fn passes(&self) -> u64 {
        match self {
            TerrainEdit::Smooth { iterations, .. } => u64::from(*iterations),
            _ => 1,
        }
    }

    /// Checks every number and the layer name; `index` is its place in `terrain.edits`.
    pub(crate) fn validate(&self, index: usize, layers: &[TerrainLayer]) -> Result<(), WorldError> {
        let ctx = Ctx::WORLD;
        let field = |name: &str| format!("terrain.edits[{index}].{name}");
        let point = |name: &str, p: [f64; 2]| -> Result<(), WorldError> {
            for v in p {
                ctx.range(
                    field(name),
                    v,
                    -MAX_COORDINATE_M,
                    MAX_COORDINATE_M,
                    " metres",
                )?;
            }
            Ok(())
        };
        let radius = |name: &str, v: f64| {
            ctx.range(
                field(name),
                v,
                SCULPT_RADIUS_MIN_M,
                TERRAIN_SIZE_MAX_M,
                " metres",
            )
        };
        let share = |name: &str, v: f64| ctx.range(field(name), v, 0.0, 1.0, "");
        let metres =
            |name: &str, v: f64| ctx.range(field(name), v, 0.0, SCULPT_STRENGTH_MAX_M, " metres");
        let level = |name: &str, v: f64| {
            ctx.range(
                field(name),
                v,
                -MAX_COORDINATE_M,
                MAX_COORDINATE_M,
                " metres",
            )
        };
        match self {
            TerrainEdit::Raise {
                center,
                radius_m,
                strength_m,
                falloff,
            }
            | TerrainEdit::Lower {
                center,
                radius_m,
                strength_m,
                falloff,
            } => {
                point("center", *center)?;
                radius("radius_m", *radius_m)?;
                metres("strength_m", *strength_m)?;
                share("falloff", *falloff)
            }
            TerrainEdit::Flatten {
                center,
                radius_m,
                height_m,
                strength,
                falloff,
            } => {
                point("center", *center)?;
                radius("radius_m", *radius_m)?;
                level("height_m", *height_m)?;
                share("strength", *strength)?;
                share("falloff", *falloff)
            }
            TerrainEdit::Smooth {
                center,
                radius_m,
                strength,
                iterations,
                falloff,
            } => {
                point("center", *center)?;
                radius("radius_m", *radius_m)?;
                share("strength", *strength)?;
                if !(1..=SMOOTH_ITERATIONS_MAX).contains(iterations) {
                    return Err(ctx.invalid(
                        field("iterations"),
                        iterations,
                        format!("it must be from 1 to {SMOOTH_ITERATIONS_MAX}"),
                    ));
                }
                share("falloff", *falloff)
            }
            TerrainEdit::Ramp {
                from,
                to,
                width_m,
                from_height_m,
                to_height_m,
                strength,
                falloff,
            } => {
                point("from", *from)?;
                point("to", *to)?;
                radius("width_m", *width_m)?;
                level("from_height_m", *from_height_m)?;
                level("to_height_m", *to_height_m)?;
                share("strength", *strength)?;
                share("falloff", *falloff)
            }
            TerrainEdit::Noise {
                center,
                radius_m,
                strength_m,
                feature_m,
                seed: _,
                falloff,
            } => {
                point("center", *center)?;
                radius("radius_m", *radius_m)?;
                metres("strength_m", *strength_m)?;
                ctx.range(
                    field("feature_m"),
                    *feature_m,
                    NOISE_FEATURE_MIN_M,
                    NOISE_FEATURE_MAX_M,
                    " metres",
                )?;
                share("falloff", *falloff)
            }
            TerrainEdit::Paint {
                layer,
                center,
                radius_m,
                strength,
                falloff,
            } => {
                known_layer(&field("layer"), layer, layers)?;
                point("center", *center)?;
                radius("radius_m", *radius_m)?;
                share("strength", *strength)?;
                share("falloff", *falloff)
            }
        }
    }
}

/// Whether `name` can name a terrain layer: 1 to 32 ASCII letters, digits, `-` and `_`.
pub fn is_terrain_layer_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_TERRAIN_LAYER_NAME_CHARS
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `layer` must be one of `layers`.
fn known_layer(field: &str, layer: &str, layers: &[TerrainLayer]) -> Result<(), WorldError> {
    if layers.iter().any(|l| l.name == layer) {
        return Ok(());
    }
    let names: Vec<&str> = layers.iter().map(|l| l.name.as_str()).collect();
    let expected = if names.is_empty() {
        "the terrain has no layers; define them first (terrain.layers)".to_string()
    } else {
        format!(
            "it must be one of the terrain's layers: {}",
            names.join(", ")
        )
    };
    Err(Ctx::WORLD.invalid(field, format!("{layer:?}"), expected))
}

/// Checks the layers, the paint rules and the edits of a terrain whose grid is `grid`.
pub(crate) fn validate(
    layers: &[TerrainLayer],
    rules: &[PaintRule],
    edits: &[TerrainEdit],
    grid: &Grid,
) -> Result<(), WorldError> {
    let ctx = Ctx::WORLD;
    if layers.len() > MAX_TERRAIN_LAYERS {
        return Err(ctx.invalid(
            "terrain.layers",
            format!("{} layers", layers.len()),
            format!("a terrain has at most {MAX_TERRAIN_LAYERS} layers"),
        ));
    }
    for (i, layer) in layers.iter().enumerate() {
        if !is_terrain_layer_name(&layer.name) {
            return Err(ctx.invalid(
                format!("terrain.layers[{i}].name"),
                format!("{:?}", crate::scenario::excerpt(&layer.name)),
                format!(
                    "it must be 1 to {MAX_TERRAIN_LAYER_NAME_CHARS} ASCII letters, digits, `-` \
                     and `_`"
                ),
            ));
        }
        if layers.iter().take(i).any(|l| l.name == layer.name) {
            return Err(ctx.invalid(
                format!("terrain.layers[{i}].name"),
                format!("{:?}", layer.name),
                "another layer has this name; layer names are unique",
            ));
        }
        for (k, c) in layer.color.iter().enumerate() {
            ctx.range(format!("terrain.layers[{i}].color[{k}]"), *c, 0.0, 1.0, "")?;
        }
        ctx.range(
            format!("terrain.layers[{i}].roughness"),
            layer.roughness,
            0.0,
            1.0,
            "",
        )?;
    }
    if rules.len() > MAX_PAINT_RULES {
        return Err(ctx.invalid(
            "terrain.paint_rules",
            format!("{} rules", rules.len()),
            format!("a terrain has at most {MAX_PAINT_RULES} paint rules"),
        ));
    }
    for (i, rule) in rules.iter().enumerate() {
        known_layer(
            &format!("terrain.paint_rules[{i}].layer"),
            &rule.layer,
            layers,
        )?;
        let bound = |name: &str, v: Option<f64>, lo: f64, hi: f64, unit: &str| {
            v.map_or(Ok(()), |v| {
                ctx.range(format!("terrain.paint_rules[{i}].{name}"), v, lo, hi, unit)
            })
        };
        bound("slope_min_deg", rule.slope_min_deg, 0.0, 90.0, " degrees")?;
        bound("slope_max_deg", rule.slope_max_deg, 0.0, 90.0, " degrees")?;
        let (lo, hi) = (-MAX_COORDINATE_M, MAX_COORDINATE_M);
        bound("height_min_m", rule.height_min_m, lo, hi, " metres")?;
        bound("height_max_m", rule.height_max_m, lo, hi, " metres")?;
        for (name, min, max) in [
            ("slope", rule.slope_min_deg, rule.slope_max_deg),
            ("height", rule.height_min_m, rule.height_max_m),
        ] {
            if let (Some(min), Some(max)) = (min, max)
                && min > max
            {
                return Err(ctx.invalid(
                    format!("terrain.paint_rules[{i}]"),
                    format!("{name} from {min} to {max}"),
                    format!("the {name} minimum must not be above the maximum"),
                ));
            }
        }
    }
    if edits.len() > MAX_TERRAIN_EDITS {
        return Err(ctx.invalid(
            "terrain.edits",
            format!("{} edits", edits.len()),
            format!("a terrain has at most {MAX_TERRAIN_EDITS} edits"),
        ));
    }
    let mut work: u64 = 0;
    for (i, edit) in edits.iter().enumerate() {
        edit.validate(i, layers)?;
        if let Some(r) = grid.rect_of(edit.bounds()) {
            work = work.saturating_add(r.area().saturating_mul(edit.passes()));
        }
    }
    if work > TERRAIN_EDIT_WORK_MAX {
        return Err(ctx.invalid(
            "terrain.edits",
            format!("{work} sample updates"),
            format!(
                "building the terrain may take at most {TERRAIN_EDIT_WORK_MAX} sample updates \
                 (the samples inside each edit's radius, times a smooth's iterations); use fewer \
                 or smaller edits, or a coarser cell_m"
            ),
        ));
    }
    Ok(())
}

/// The grid of a terrain: samples per side, spacing and the corner with the smallest
/// coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub cell: f64,
    pub min: [f64; 2],
}

/// A rectangle of samples, columns `c0..=c1` and rows `r0..=r1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Rect {
    pub c0: usize,
    pub r0: usize,
    pub c1: usize,
    pub r1: usize,
}

impl Rect {
    pub fn width(&self) -> usize {
        self.c1.saturating_sub(self.c0).saturating_add(1)
    }

    pub fn height(&self) -> usize {
        self.r1.saturating_sub(self.r0).saturating_add(1)
    }

    pub fn area(&self) -> u64 {
        (self.width() as u64).saturating_mul(self.height() as u64)
    }

    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let r = Rect {
            c0: self.c0.max(o.c0),
            r0: self.r0.max(o.r0),
            c1: self.c1.min(o.c1),
            r1: self.r1.min(o.r1),
        };
        (r.c0 <= r.c1 && r.r0 <= r.r1).then_some(r)
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.intersect(o).is_some()
    }

    /// Grown by `n` samples on every side, within `grid`.
    pub fn grow(&self, n: usize, grid: &Grid) -> Rect {
        Rect {
            c0: self.c0.saturating_sub(n),
            r0: self.r0.saturating_sub(n),
            c1: self.c1.saturating_add(n).min(grid.cols.saturating_sub(1)),
            r1: self.r1.saturating_add(n).min(grid.rows.saturating_sub(1)),
        }
    }

    /// Index of (`c`, `r`) in a row-major buffer over this rectangle.
    fn index(&self, c: usize, r: usize) -> usize {
        (r - self.r0) * self.width() + (c - self.c0)
    }

    /// Every sample, row by row.
    fn samples(&self) -> impl Iterator<Item = (usize, usize)> + use<> {
        let (c0, c1) = (self.c0, self.c1);
        (self.r0..=self.r1).flat_map(move |r| (c0..=c1).map(move |c| (c, r)))
    }
}

impl Grid {
    /// The whole grid.
    pub fn all(&self) -> Rect {
        Rect {
            c0: 0,
            r0: 0,
            c1: self.cols.saturating_sub(1),
            r1: self.rows.saturating_sub(1),
        }
    }

    /// World x of column `col` (the same formula everywhere, so the bits agree).
    pub fn x(&self, col: usize) -> f64 {
        self.min[0] + col as f64 * self.cell
    }

    /// World z of row `row`.
    pub fn z(&self, row: usize) -> f64 {
        self.min[1] + row as f64 * self.cell
    }

    /// The samples inside `[x0, z0, x1, z1]` and one more on every side; `None` when that
    /// misses the grid.
    pub fn rect_of(&self, b: [f64; 4]) -> Option<Rect> {
        let [x0, z0, x1, z1] = b;
        let last_c = self.cols.checked_sub(1)? as f64;
        let last_r = self.rows.checked_sub(1)? as f64;
        let lo = |v: f64, m: f64| ((v - m) / self.cell).floor() - 1.0;
        let hi = |v: f64, m: f64| ((v - m) / self.cell).ceil() + 1.0;
        let (c0, c1) = (lo(x0, self.min[0]), hi(x1, self.min[0]));
        let (r0, r1) = (lo(z0, self.min[1]), hi(z1, self.min[1]));
        if !(c0.is_finite() && c1.is_finite() && r0.is_finite() && r1.is_finite()) {
            return None;
        }
        if c1 < 0.0 || r1 < 0.0 || c0 > last_c || r0 > last_r || c0 > c1 || r0 > r1 {
            return None;
        }
        Some(Rect {
            c0: c0.max(0.0) as usize,
            r0: r0.max(0.0) as usize,
            c1: c1.min(last_c) as usize,
            r1: r1.min(last_r) as usize,
        })
    }
}

/// `t` eased: 0 at 0, 1 at 1, flat at both ends.
fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The brush weight at distance `d`: 1 inside `radius * (1 - falloff)`, a smoothstep down to
/// 0 at `radius`, 0 beyond.
pub fn brush_weight(d: f64, radius: f64, falloff: f64) -> f64 {
    let inner = radius * (1.0 - falloff);
    if d >= radius {
        0.0
    } else if d <= inner {
        1.0
    } else {
        1.0 - smoothstep((d - inner) / (radius - inner))
    }
}

fn distance(x: f64, z: f64, [cx, cz]: [f64; 2]) -> f64 {
    let (dx, dz) = (x - cx, z - cz);
    (dx * dx + dz * dz).sqrt()
}

/// A ramp at (`x`, `z`): the distance to the segment and the share of the way along it.
fn along(x: f64, z: f64, [ax, az]: [f64; 2], [bx, bz]: [f64; 2]) -> (f64, f64) {
    let (dx, dz) = (bx - ax, bz - az);
    let len2 = dx * dx + dz * dz;
    let u = if len2 > 0.0 {
        (((x - ax) * dx + (z - az) * dz) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (px, pz) = (x - ax - dx * u, z - az - dz * u);
    ((px * px + pz * pz).sqrt(), u)
}

/// Heights over a rectangle, row-major.
struct Buf {
    rect: Rect,
    h: Vec<f64>,
}

impl Buf {
    fn get(&self, c: usize, r: usize) -> f64 {
        self.h.get(self.rect.index(c, r)).copied().unwrap_or(0.0)
    }

    fn update(&mut self, c: usize, r: usize, f: impl FnOnce(f64) -> f64) {
        let i = self.rect.index(c, r);
        if let Some(v) = self.h.get_mut(i) {
            *v = f(*v);
        }
    }
}

/// Applies one height edit to the buffer, on the samples of `bounds` inside it.
fn apply(edit: &TerrainEdit, grid: &Grid, buf: &mut Buf) {
    let Some(b) = grid
        .rect_of(edit.bounds())
        .and_then(|r| r.intersect(&buf.rect))
    else {
        return;
    };
    let pos = |c: usize, r: usize| (grid.x(c), grid.z(r));
    match edit {
        TerrainEdit::Raise {
            center,
            radius_m,
            strength_m,
            falloff,
        } => {
            for (c, r) in b.samples() {
                let (x, z) = pos(c, r);
                let w = brush_weight(distance(x, z, *center), *radius_m, *falloff);
                buf.update(c, r, |h| h + strength_m * w);
            }
        }
        TerrainEdit::Lower {
            center,
            radius_m,
            strength_m,
            falloff,
        } => {
            for (c, r) in b.samples() {
                let (x, z) = pos(c, r);
                let w = brush_weight(distance(x, z, *center), *radius_m, *falloff);
                buf.update(c, r, |h| h - strength_m * w);
            }
        }
        TerrainEdit::Flatten {
            center,
            radius_m,
            height_m,
            strength,
            falloff,
        } => {
            for (c, r) in b.samples() {
                let (x, z) = pos(c, r);
                let w = brush_weight(distance(x, z, *center), *radius_m, *falloff);
                buf.update(c, r, |h| h + (height_m - h) * strength * w);
            }
        }
        TerrainEdit::Smooth {
            center,
            radius_m,
            strength,
            iterations,
            falloff,
        } => {
            let weights: Vec<f64> = b
                .samples()
                .map(|(c, r)| {
                    let (x, z) = pos(c, r);
                    brush_weight(distance(x, z, *center), *radius_m, *falloff)
                })
                .collect();
            let full = buf.rect;
            let window = b.grow(1, grid).intersect(&full).unwrap_or(b);
            for _ in 0..*iterations {
                let before = Buf {
                    rect: window,
                    h: window.samples().map(|(c, r)| buf.get(c, r)).collect(),
                };
                for ((c, r), w) in b.samples().zip(&weights) {
                    let mut sum = 0.0;
                    for rr in [r.saturating_sub(1).max(full.r0), r, (r + 1).min(full.r1)] {
                        for cc in [c.saturating_sub(1).max(full.c0), c, (c + 1).min(full.c1)] {
                            sum += before.get(cc, rr);
                        }
                    }
                    let mean = sum / 9.0;
                    buf.update(c, r, |h| h + (mean - h) * strength * w);
                }
            }
        }
        TerrainEdit::Ramp {
            from,
            to,
            width_m,
            from_height_m,
            to_height_m,
            strength,
            falloff,
        } => {
            for (c, r) in b.samples() {
                let (x, z) = pos(c, r);
                let (d, u) = along(x, z, *from, *to);
                let w = brush_weight(d, width_m / 2.0, *falloff);
                let target = from_height_m + (to_height_m - from_height_m) * u;
                buf.update(c, r, |h| h + (target - h) * strength * w);
            }
        }
        TerrainEdit::Noise {
            center,
            radius_m,
            strength_m,
            feature_m,
            seed,
            falloff,
        } => {
            for (c, r) in b.samples() {
                let (x, z) = pos(c, r);
                let w = brush_weight(distance(x, z, *center), *radius_m, *falloff);
                let n = fbm2(
                    *seed,
                    x / feature_m,
                    z / feature_m,
                    NOISE_EDIT_OCTAVES,
                    2.0,
                    0.5,
                );
                buf.update(c, r, |h| h + strength_m * w * n);
            }
        }
        TerrainEdit::Paint { .. } => {}
    }
}

/// The rectangle the source must be sampled over so that `rect` comes out exact after
/// `edits`: grown by `iterations` for every smooth that reaches it, going backwards. Also
/// returns the indices of the edits that reach it (in order).
pub(crate) fn source_region(edits: &[TerrainEdit], grid: &Grid, rect: Rect) -> (Rect, Vec<usize>) {
    let mut region = rect;
    let mut used = Vec::new();
    for (i, edit) in edits.iter().enumerate().rev() {
        if edit.is_paint() {
            continue;
        }
        let Some(b) = grid.rect_of(edit.bounds()) else {
            continue;
        };
        if b.intersects(&region) {
            used.push(i);
            region = region.grow(edit.reach(), grid);
        }
    }
    used.reverse();
    (region, used)
}

/// The final heights of the samples of `rect`, row-major, exactly what the whole grid gives
/// there. `source(x, z)` is the terrain's source.
pub(crate) fn eval_heights(
    edits: &[TerrainEdit],
    grid: &Grid,
    rect: Rect,
    source: impl Fn(f64, f64) -> f64,
) -> Vec<f64> {
    let (region, used) = source_region(edits, grid, rect);
    let mut buf = Buf {
        rect: region,
        h: region
            .samples()
            .map(|(c, r)| source(grid.x(c), grid.z(r)))
            .collect(),
    };
    for edit in used.iter().filter_map(|&i| edits.get(i)) {
        apply(edit, grid, &mut buf);
    }
    if region == rect {
        return buf.h;
    }
    rect.samples().map(|(c, r)| buf.get(c, r)).collect()
}

/// The slope in degrees at sample (`c`, `r`) of `heights` (the whole grid, row-major), from
/// central differences of its neighbours (one-sided at the edge).
fn slope_deg(heights: &[f32], grid: &Grid, c: usize, r: usize) -> f64 {
    let h = |c: usize, r: usize| {
        heights
            .get(r * grid.cols + c)
            .copied()
            .map_or(0.0, f64::from)
    };
    let (c0, c1) = (
        c.saturating_sub(1),
        (c + 1).min(grid.cols.saturating_sub(1)),
    );
    let (r0, r1) = (
        r.saturating_sub(1),
        (r + 1).min(grid.rows.saturating_sub(1)),
    );
    let dx = if c1 > c0 {
        (h(c1, r) - h(c0, r)) / ((c1 - c0) as f64 * grid.cell)
    } else {
        0.0
    };
    let dz = if r1 > r0 {
        (h(c, r1) - h(c, r0)) / ((r1 - r0) as f64 * grid.cell)
    } else {
        0.0
    };
    rad_to_deg((dx * dx + dz * dz).sqrt().patan())
}

/// How much a rule covers ground of `slope` degrees at `height` metres, 0 to 1.
fn rule_cover(rule: &PaintRule, slope: f64, height: f64) -> f64 {
    let above = |v: f64, min: f64, blend: f64| smoothstep((v - min) / blend + 0.5);
    let below = |v: f64, max: f64, blend: f64| smoothstep((max - v) / blend + 0.5);
    let mut f = 1.0;
    if let Some(min) = rule.slope_min_deg {
        f *= above(slope, min, RULE_SLOPE_BLEND_DEG);
    }
    if let Some(max) = rule.slope_max_deg {
        f *= below(slope, max, RULE_SLOPE_BLEND_DEG);
    }
    if let Some(min) = rule.height_min_m {
        f *= above(height, min, RULE_HEIGHT_BLEND_M);
    }
    if let Some(max) = rule.height_max_m {
        f *= below(height, max, RULE_HEIGHT_BLEND_M);
    }
    f
}

/// Moves `w` (shares summing to 1) towards `layer` by `amount` (0 to 1); the others give way
/// in proportion.
fn paint(w: &mut [f64; WEIGHT_SLOTS], layer: usize, amount: f64) {
    let amount = amount.clamp(0.0, 1.0);
    let Some(&own) = w.get(layer) else {
        return;
    };
    if amount <= 0.0 || own >= 1.0 {
        return;
    }
    let new = own + (1.0 - own) * amount;
    let rest = (1.0 - new) / (1.0 - own);
    for (i, v) in w.iter_mut().enumerate() {
        *v = if i == layer { new } else { *v * rest };
    }
}

/// Shares summing to 1 as bytes summing to 255 exactly: floors, then the remainder to the
/// largest fractions (the lower layer first on a tie). Non-finite shares give all to layer 0.
pub(crate) fn to_bytes(w: &[f64; WEIGHT_SLOTS]) -> [u8; WEIGHT_SLOTS] {
    let mut out = [0u8; WEIGHT_SLOTS];
    if w.iter().any(|v| !v.is_finite()) {
        out[0] = 255;
        return out;
    }
    let sum: f64 = w.iter().map(|v| v.max(0.0)).sum();
    if sum <= 0.0 {
        out[0] = 255;
        return out;
    }
    let scaled = w.map(|v| v.max(0.0) / sum * 255.0);
    let mut given: u32 = 0;
    for (o, s) in out.iter_mut().zip(scaled) {
        let f = s.floor().clamp(0.0, 255.0);
        *o = f as u8;
        given += u32::from(*o);
    }
    let mut order: Vec<usize> = (0..WEIGHT_SLOTS).collect();
    let frac = |i: usize| scaled.get(i).map_or(0.0, |s| s - s.floor());
    order.sort_by(|&a, &b| frac(b).total_cmp(&frac(a)).then(a.cmp(&b)));
    for i in order.into_iter().cycle().take(WEIGHT_SLOTS * 255) {
        if given >= 255 {
            break;
        }
        if let Some(o) = out.get_mut(i)
            && *o < 255
        {
            *o += 1;
            given += 1;
        }
    }
    out
}

/// The layer weights of the samples of `rect`, row-major, from the final `heights` of the whole
/// grid. Empty when there are no layers.
pub(crate) fn eval_weights(
    layers: &[TerrainLayer],
    rules: &[PaintRule],
    edits: &[TerrainEdit],
    grid: &Grid,
    heights: &[f32],
    rect: Rect,
) -> Vec<[u8; WEIGHT_SLOTS]> {
    if layers.is_empty() {
        return Vec::new();
    }
    let index = |name: &str| layers.iter().position(|l| l.name == name);
    let rule_layers: Vec<(usize, &PaintRule)> = rules
        .iter()
        .filter_map(|r| Some((index(&r.layer)?, r)))
        .collect();
    let mut shares: Vec<[f64; WEIGHT_SLOTS]> = rect
        .samples()
        .map(|(c, r)| {
            let mut w = [0.0; WEIGHT_SLOTS];
            w[0] = 1.0;
            if !rule_layers.is_empty() {
                let slope = slope_deg(heights, grid, c, r);
                let h = heights
                    .get(r * grid.cols + c)
                    .copied()
                    .map_or(0.0, f64::from);
                for (layer, rule) in &rule_layers {
                    paint(&mut w, *layer, rule_cover(rule, slope, h));
                }
            }
            w
        })
        .collect();
    for edit in edits {
        let TerrainEdit::Paint {
            layer,
            center,
            radius_m,
            strength,
            falloff,
        } = edit
        else {
            continue;
        };
        let Some(layer) = index(layer) else {
            continue;
        };
        let Some(b) = grid.rect_of(edit.bounds()).and_then(|b| b.intersect(&rect)) else {
            continue;
        };
        for (c, r) in b.samples() {
            let w = brush_weight(distance(grid.x(c), grid.z(r), *center), *radius_m, *falloff);
            if let Some(s) = shares.get_mut(rect.index(c, r)) {
                paint(s, layer, strength * w);
            }
        }
    }
    shares.iter().map(to_bytes).collect()
}
