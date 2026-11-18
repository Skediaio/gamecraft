//! Terrain recipes and the height field derived from them.

use crate::WorldError;
use crate::error::Ctx;
use crate::limits::{
    MAX_COORDINATE_M, NOISE_FEATURE_MAX_M, NOISE_FEATURE_MIN_M, NOISE_OCTAVES_MAX,
    NOISE_OCTAVES_MIN, NOISE_RELIEF_MAX_M, NOISE_RELIEF_MIN_M, NOISE_ROUGHNESS_MAX,
    NOISE_ROUGHNESS_MIN, TERRAIN_CELL_MAX_M, TERRAIN_CELL_MIN_M, TERRAIN_MAX_SAMPLES_PER_SIDE,
    TERRAIN_MULTIPLE_TOLERANCE, TERRAIN_SIZE_MAX_M, TERRAIN_SIZE_MIN_M,
};
use gamecraft_math::noise::fbm2;
use serde::{Deserialize, Serialize};

/// Frequency ratio between successive noise octaves.
const NOISE_LACUNARITY: f64 = 2.0;

/// The terrain recipe: a square of side `size_m` centred on the origin (x and z from
/// `-size_m / 2` to `+size_m / 2`), sampled every `cell_m`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terrain {
    /// Side of the square, metres, 8 to 16384; a whole multiple of `cell_m`.
    pub size_m: f64,
    /// Distance between height samples, metres, 0.25 to 64. At most 2049 samples per side.
    pub cell_m: f64,
    /// Where the heights come from.
    pub source: TerrainSource,
}

fn default_relief_m() -> f64 {
    40.0
}

fn default_feature_m() -> f64 {
    200.0
}

fn default_octaves() -> u32 {
    5
}

fn default_roughness() -> f64 {
    0.5
}

/// Where a terrain's heights come from. In JSON the `kind` field names the variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum TerrainSource {
    /// Level ground.
    Flat {
        /// Height of the ground, metres. Default 0.
        #[serde(default)]
        height_m: f64,
    },
    /// Rolling land from fractal value noise:
    /// `height = relief_m / 2 * fbm2(seed, x / feature_m, z / feature_m, octaves, 2, roughness)`
    /// at world x, z, so the height stays within `relief_m / 2` of 0.
    Noise {
        /// Seed of the noise. Default 0.
        #[serde(default)]
        seed: u64,
        /// Peak-to-trough height range, metres, 0 to 5000. Default 40.
        #[serde(default = "default_relief_m")]
        relief_m: f64,
        /// Size of the largest features, metres, 1 to 100000. Default 200.
        #[serde(default = "default_feature_m")]
        feature_m: f64,
        /// Layers of detail, 1 to 10. Default 5.
        #[serde(default = "default_octaves")]
        octaves: u32,
        /// Amplitude ratio between successive layers, 0 (smooth) to 1 (rough). Default 0.5.
        #[serde(default = "default_roughness")]
        roughness: f64,
    },
}

impl Terrain {
    /// Checks the terrain's limits (see [`crate::limits`]): every number finite and in range,
    /// `size_m` a whole multiple of `cell_m`, and at most 2049 samples per side. Never
    /// allocates.
    pub fn validate(&self) -> Result<(), WorldError> {
        self.cells().map(|_| ())
    }

    /// The number of cells per side, after checking every rule.
    fn cells(&self) -> Result<usize, WorldError> {
        let ctx = Ctx::WORLD;
        ctx.range(
            "terrain.size_m",
            self.size_m,
            TERRAIN_SIZE_MIN_M,
            TERRAIN_SIZE_MAX_M,
            " metres",
        )?;
        ctx.range(
            "terrain.cell_m",
            self.cell_m,
            TERRAIN_CELL_MIN_M,
            TERRAIN_CELL_MAX_M,
            " metres",
        )?;
        // Both are in range, so the ratio is at most 65536 and the casts are exact.
        let ratio = self.size_m / self.cell_m;
        let cells = ratio.round();
        let max_cells = TERRAIN_MAX_SAMPLES_PER_SIDE.saturating_sub(1);
        if cells > max_cells as f64 {
            return Err(WorldError::TerrainTooLarge {
                size_m: self.size_m,
                cell_m: self.cell_m,
                samples: cells as u64 + 1,
                max: TERRAIN_MAX_SAMPLES_PER_SIDE,
                fitting_cell_m: self.size_m / max_cells as f64,
            });
        }
        if cells < 1.0 || (ratio - cells).abs() > TERRAIN_MULTIPLE_TOLERANCE * ratio {
            let lower = (ratio.floor()).max(1.0);
            let upper = ratio.ceil().min(max_cells as f64);
            return Err(ctx.invalid(
                "terrain.cell_m",
                self.cell_m,
                format!(
                    "terrain.size_m {} must be a whole multiple of it; use cell_m {} or {} \
                     (or change size_m)",
                    self.size_m,
                    self.size_m / upper,
                    self.size_m / lower
                ),
            ));
        }
        match self.source {
            TerrainSource::Flat { height_m } => {
                ctx.range(
                    "terrain.source.height_m",
                    height_m,
                    -MAX_COORDINATE_M,
                    MAX_COORDINATE_M,
                    " metres",
                )?;
            }
            TerrainSource::Noise {
                seed: _,
                relief_m,
                feature_m,
                octaves,
                roughness,
            } => {
                ctx.range(
                    "terrain.source.relief_m",
                    relief_m,
                    NOISE_RELIEF_MIN_M,
                    NOISE_RELIEF_MAX_M,
                    " metres",
                )?;
                ctx.range(
                    "terrain.source.feature_m",
                    feature_m,
                    NOISE_FEATURE_MIN_M,
                    NOISE_FEATURE_MAX_M,
                    " metres",
                )?;
                if !(NOISE_OCTAVES_MIN..=NOISE_OCTAVES_MAX).contains(&octaves) {
                    return Err(ctx.invalid(
                        "terrain.source.octaves",
                        octaves,
                        format!("it must be from {NOISE_OCTAVES_MIN} to {NOISE_OCTAVES_MAX}"),
                    ));
                }
                ctx.range(
                    "terrain.source.roughness",
                    roughness,
                    NOISE_ROUGHNESS_MIN,
                    NOISE_ROUGHNESS_MAX,
                    "",
                )?;
            }
        }
        Ok(cells as usize)
    }

    /// The height at world `x`, `z` according to the recipe (no grid involved).
    fn height(&self, x: f64, z: f64) -> f64 {
        match self.source {
            TerrainSource::Flat { height_m } => height_m,
            TerrainSource::Noise {
                seed,
                relief_m,
                feature_m,
                octaves,
                roughness,
            } => {
                relief_m / 2.0
                    * fbm2(
                        seed,
                        x / feature_m,
                        z / feature_m,
                        octaves,
                        NOISE_LACUNARITY,
                        roughness,
                    )
            }
        }
    }

    /// Builds the height field: validates, then samples the recipe at every grid node.
    ///
    /// Node (col, row) lies at world `x = -size_m / 2 + col * cell_m`,
    /// `z = -size_m / 2 + row * cell_m`. The heights depend on the world coordinates only, so
    /// a larger terrain extends the same land.
    pub fn build(&self) -> Result<HeightField, WorldError> {
        let cells = self.cells()?;
        let samples = cells.saturating_add(1);
        let half = self.size_m / 2.0;
        let min = [-half, -half];
        let mut heights = Vec::with_capacity(samples.saturating_mul(samples));
        for row in 0..samples {
            let z = min[1] + row as f64 * self.cell_m;
            for col in 0..samples {
                let x = min[0] + col as f64 * self.cell_m;
                heights.push(self.height(x, z) as f32);
            }
        }
        Ok(HeightField {
            cols: samples,
            rows: samples,
            cell_m: self.cell_m,
            min,
            heights,
        })
    }
}

/// The ground as a grid of height samples, derived from a [`Terrain`] by [`Terrain::build`].
///
/// Samples are stored row-major: index `row * cols + col`, columns along +x and rows along
/// +z, node (col, row) at world `min + (col, row) * cell_m`.
///
/// **The surface is made of triangles.** Each cell is split along the diagonal from sample
/// (col, row) to sample (col + 1, row + 1): one triangle holds (col, row), (col + 1, row) and
/// (col + 1, row + 1), the other (col, row), (col, row + 1) and (col + 1, row + 1). The
/// renderer draws exactly these triangles, so the ground a character stands on is the ground
/// that is drawn. [`HeightField::height_at`], [`HeightField::normal_at`] and
/// [`HeightField::raycast`] all answer for this triangle surface.
#[derive(Clone, Debug, PartialEq)]
pub struct HeightField {
    cols: usize,
    rows: usize,
    cell_m: f64,
    min: [f64; 2],
    heights: Vec<f32>,
}

/// Where a point falls in the grid: its cell and its position inside it, 0 to 1 per axis.
#[derive(Clone, Copy, Debug)]
struct CellPoint {
    col: usize,
    row: usize,
    fu: f64,
    fv: f64,
}

/// The triangle under a point: its height there and its slopes, in metres per metre.
#[derive(Clone, Copy, Debug)]
struct Facet {
    height: f64,
    slope_x: f64,
    slope_z: f64,
}

impl HeightField {
    /// Samples along x (cells + 1).
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Samples along z (cells + 1).
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Distance between samples, metres.
    pub fn cell_m(&self) -> f64 {
        self.cell_m
    }

    /// World `[x, z]` of sample (0, 0), the corner with the smallest coordinates.
    pub fn min(&self) -> [f64; 2] {
        self.min
    }

    /// World `[x, z]` of the last sample, the corner with the largest coordinates.
    pub fn max(&self) -> [f64; 2] {
        let [x, z] = self.min;
        [
            x + self.cols.saturating_sub(1) as f64 * self.cell_m,
            z + self.rows.saturating_sub(1) as f64 * self.cell_m,
        ]
    }

    /// All heights, metres, row-major: index `row * cols + col`.
    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

    /// The height of sample (`col`, `row`), or `None` outside the grid.
    pub fn sample(&self, col: usize, row: usize) -> Option<f32> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        let index = row.checked_mul(self.cols)?.checked_add(col)?;
        self.heights.get(index).copied()
    }

    /// The lowest and highest sample, metres.
    pub fn height_range(&self) -> (f32, f32) {
        let mut it = self.heights.iter().copied();
        let first = it.next().unwrap_or(0.0);
        it.fold((first, first), |(lo, hi), h| (lo.min(h), hi.max(h)))
    }

    /// Locates world `x`, `z` in the grid; `None` outside it or for non-finite input.
    fn locate(&self, x: f64, z: f64) -> Option<CellPoint> {
        if !x.is_finite() || !z.is_finite() || self.cols < 2 || self.rows < 2 {
            return None;
        }
        let [x0, z0] = self.min;
        let u = (x - x0) / self.cell_m;
        let v = (z - z0) / self.cell_m;
        let max_u = (self.cols - 1) as f64;
        let max_v = (self.rows - 1) as f64;
        if !(0.0..=max_u).contains(&u) || !(0.0..=max_v).contains(&v) {
            return None;
        }
        // The far edge belongs to the last cell.
        let col = (u.floor() as usize).min(self.cols - 2);
        let row = (v.floor() as usize).min(self.rows - 2);
        Some(CellPoint {
            col,
            row,
            fu: u - col as f64,
            fv: v - row as f64,
        })
    }

    /// The triangle under a located point (see the split in the type's documentation).
    fn facet(&self, p: CellPoint) -> Option<Facet> {
        let h = |c: usize, r: usize| self.sample(c, r).map(f64::from);
        let h00 = h(p.col, p.row)?;
        let h10 = h(p.col + 1, p.row)?;
        let h01 = h(p.col, p.row + 1)?;
        let h11 = h(p.col + 1, p.row + 1)?;
        // Per-cell gradients (metres per cell) of the triangle the point is in.
        let (du, dv) = if p.fu >= p.fv {
            // (col, row), (col + 1, row), (col + 1, row + 1)
            (h10 - h00, h11 - h10)
        } else {
            // (col, row), (col, row + 1), (col + 1, row + 1)
            (h11 - h01, h01 - h00)
        };
        Some(Facet {
            height: h00 + p.fu * du + p.fv * dv,
            slope_x: du / self.cell_m,
            slope_z: dv / self.cell_m,
        })
    }

    /// The ground height at world `x`, `z`, metres, on the triangle surface: inside a cell
    /// it is the plane of the triangle the point lies in (the diagonal runs from sample
    /// (col, row) to (col + 1, row + 1)); at a sample it is that sample. `None` outside the
    /// terrain or for non-finite input.
    pub fn height_at(&self, x: f64, z: f64) -> Option<f64> {
        self.facet(self.locate(x, z)?).map(|f| f.height)
    }

    /// The unit upward normal of the triangle under world `x`, `z` (see
    /// [`HeightField::height_at`] for the split; on the diagonal itself the triangle holding
    /// (col + 1, row) is used). `None` outside the terrain or for non-finite input.
    pub fn normal_at(&self, x: f64, z: f64) -> Option<[f64; 3]> {
        let f = self.facet(self.locate(x, z)?)?;
        let (nx, nz) = (-f.slope_x, -f.slope_z);
        let len = (nx * nx + 1.0 + nz * nz).sqrt();
        if !len.is_finite() {
            return None;
        }
        Some([nx / len, 1.0 / len, nz / len])
    }

    /// The ground height under a point clamped into the terrain.
    fn clamped_height(&self, x: f64, z: f64) -> Option<f64> {
        let [x0, z0] = self.min;
        let [x1, z1] = self.max();
        self.height_at(x.clamp(x0, x1), z.clamp(z0, z1))
    }

    /// Distance in metres along the ray from `origin` in `direction` (any non-zero length) to
    /// the first point where it passes from above the ground to on or below it, within
    /// `max_m`.
    ///
    /// The test is exact for the triangle surface: the ray is cut where it crosses grid lines
    /// and cell diagonals, the height difference is linear between those cuts, and the first
    /// sign change is solved in closed form. A ray that starts below the ground hits only
    /// after it has come up above it. `None` for a miss, a ray that never passes over the
    /// terrain, or bad input (non-finite numbers, a zero direction, `max_m` not finite and
    /// positive).
    pub fn raycast(&self, origin: [f64; 3], direction: [f64; 3], max_m: f64) -> Option<f64> {
        let [ox, oy, oz] = origin;
        let [dx, dy, dz] = direction;
        if !(ox.is_finite() && oy.is_finite() && oz.is_finite()) {
            return None;
        }
        if !(max_m.is_finite() && max_m > 0.0) {
            return None;
        }
        let len = (dx * dx + dy * dy + dz * dz).sqrt();
        if !(len.is_finite() && len > 0.0) {
            return None;
        }
        let (dx, dy, dz) = (dx / len, dy / len, dz / len);

        // Clip to the terrain's square.
        let [x0, z0] = self.min;
        let [x1, z1] = self.max();
        let mut t0: f64 = 0.0;
        let mut t1: f64 = max_m;
        for (o, d, lo, hi) in [(ox, dx, x0, x1), (oz, dz, z0, z1)] {
            if d == 0.0 {
                if o < lo || o > hi {
                    return None;
                }
            } else {
                let a = (lo - o) / d;
                let b = (hi - o) / d;
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
        }
        if !t0.is_finite() || !t1.is_finite() || t0 > t1 {
            return None;
        }

        // Cut points: where the ray crosses x and z grid lines and cell diagonals.
        let c = self.cell_m;
        let (u0, du) = ((ox - x0) / c, dx / c);
        let (v0, dv) = ((oz - z0) / c, dz / c);
        let mut cuts = vec![t0, t1];
        push_crossings(&mut cuts, u0, du, t0, t1);
        push_crossings(&mut cuts, v0, dv, t0, t1);
        push_crossings(&mut cuts, u0 - v0, du - dv, t0, t1);
        cuts.sort_by(f64::total_cmp);

        let gap = |t: f64| -> Option<f64> {
            let ground = self.clamped_height(ox + t * dx, oz + t * dz)?;
            Some(oy + t * dy - ground)
        };
        let mut above = false;
        let mut prev: Option<(f64, f64)> = None;
        for &t in &cuts {
            let g = gap(t)?;
            if let Some((ta, ga)) = prev
                && above
                && g <= 0.0
            {
                let hit = if ga > 0.0 {
                    ta + (t - ta) * (ga / (ga - g))
                } else {
                    ta
                };
                return Some(hit.clamp(ta, t));
            }
            if g > 0.0 {
                above = true;
            }
            prev = Some((t, g));
        }
        None
    }
}

/// Appends the parameters `t` in `[t0, t1]` where `w0 + dw * t` is a whole number.
fn push_crossings(cuts: &mut Vec<f64>, w0: f64, dw: f64, t0: f64, t1: f64) {
    if dw == 0.0 || !dw.is_finite() {
        return;
    }
    let wa = w0 + dw * t0;
    let wb = w0 + dw * t1;
    let first = wa.min(wb).ceil();
    let last = wa.max(wb).floor();
    // The span is at most two grid sizes (diagonals), so this bound is never reached by a
    // valid height field; it guards against arithmetic surprises.
    let cap = TERRAIN_MAX_SAMPLES_PER_SIDE.saturating_mul(4);
    let mut k = first;
    let mut n = 0;
    while k <= last && n < cap {
        cuts.push(((k - w0) / dw).clamp(t0, t1));
        k += 1.0;
        n += 1;
    }
}
