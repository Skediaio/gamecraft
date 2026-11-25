//! Terrain recipes and the height field derived from them.

use crate::error::Ctx;
use crate::heightmap::{HEIGHTMAP_SIDE_MAX, HEIGHTMAP_SIDE_MIN};
use crate::limits::{
    GRID_SAMPLES_MAX, GRID_SIDE_MAX, GRID_SIDE_MIN, MAX_COORDINATE_M, NOISE_FEATURE_MAX_M,
    NOISE_FEATURE_MIN_M, NOISE_OCTAVES_MAX, NOISE_OCTAVES_MIN, NOISE_RELIEF_MAX_M,
    NOISE_RELIEF_MIN_M, NOISE_ROUGHNESS_MAX, NOISE_ROUGHNESS_MIN, TERRAIN_CELL_MAX_M,
    TERRAIN_CELL_MIN_M, TERRAIN_MAX_SAMPLES_PER_SIDE, TERRAIN_MULTIPLE_TOLERANCE,
    TERRAIN_SIZE_MAX_M, TERRAIN_SIZE_MIN_M,
};
use crate::sculpt::{
    self, Grid, PaintRule, Rect, TerrainEdit, TerrainLayer, WEIGHT_SLOTS, eval_heights,
    eval_weights,
};
use crate::{GridHeights, Heightmap, WorldError};
use gamecraft_math::noise::fbm2;
use serde::{Deserialize, Serialize};

/// Frequency ratio between successive noise octaves.
const NOISE_LACUNARITY: f64 = 2.0;

/// The terrain recipe: a square of side `size_m` centred on the origin (x and z from
/// `-size_m / 2` to `+size_m / 2`), sampled every `cell_m`, its heights from `source` changed
/// by `edits` in order, and its painted `layers` (see [`crate::sculpt`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terrain {
    /// Side of the square, metres, 8 to 16384; a whole multiple of `cell_m`.
    pub size_m: f64,
    /// Distance between height samples, metres, 0.25 to 64. At most 2049 samples per side.
    pub cell_m: f64,
    /// Where the heights come from.
    pub source: TerrainSource,
    /// The painted materials, at most 8; the first covers what nothing else paints. None: the
    /// renderer shades by slope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<TerrainLayer>,
    /// Automatic paint rules, applied in order before the paint edits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paint_rules: Vec<PaintRule>,
    /// Sculpt and paint edits, applied in order over the source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<TerrainEdit>,
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
    /// Real ground as a control grid: a square of `rows` by `cols` height nodes spanning the
    /// whole terrain, reduced from elevation data. Node (col, row) lies at world
    /// `x = -size_m / 2 + col * size_m / (cols - 1)`,
    /// `z = -size_m / 2 + row * size_m / (rows - 1)`; `cell_m` only sets the sampling of the
    /// built field. Heights are interpolated bilinearly; outside the square the edge nodes
    /// hold.
    /// Real ground at full resolution: a heightmap of `cols` by `rows` samples, handed to
    /// the world by the host ([`Heightmap`]; in JSON only `cols`, `rows` and the `content`
    /// hash appear: the samples are too many for the text). Sample `(c, r)` lies as a grid
    /// node, `size_m / (cols - 1)` metres apart; heights are interpolated bilinearly and the
    /// edge samples hold outside. Sculpt edits and paint lie over it as over any source, so
    /// baked features belong in the heightmap.
    Heightmap {
        /// Samples along x, 2 to 2049.
        cols: u32,
        /// Samples along z, 2 to 2049.
        rows: u32,
        /// The hash of the samples, sixteen lowercase hexadecimal digits
        /// ([`Heightmap::content_text`]).
        content: String,
        /// The samples, handed over by the host; never written to the document.
        #[serde(skip)]
        tiles: Heightmap,
    },
    Grid {
        /// Nodes along x, 2 to 513.
        cols: u32,
        /// Nodes along z, 2 to 513.
        rows: u32,
        /// The height of every node, metres, row-major: index `row * cols + col`. Exactly
        /// `cols * rows` numbers, each finite and within ±1000000. Shared between copies of
        /// the terrain.
        heights: GridHeights,
    },
}

impl Default for Terrain {
    /// Level ground at 0 m, 256 m across at 2 m cells, without layers, rules or edits.
    fn default() -> Terrain {
        Terrain::new(256.0, 2.0, TerrainSource::Flat { height_m: 0.0 })
    }
}

impl TerrainSource {
    /// `flat`, `noise`, `grid` or `heightmap`: the name of the kind.
    pub fn kind(&self) -> &'static str {
        match self {
            TerrainSource::Flat { .. } => "flat",
            TerrainSource::Noise { .. } => "noise",
            TerrainSource::Grid { .. } => "grid",
            TerrainSource::Heightmap { .. } => "heightmap",
        }
    }
}

impl Terrain {
    /// A terrain of `source` without layers, rules or edits.
    pub fn new(size_m: f64, cell_m: f64, source: TerrainSource) -> Terrain {
        Terrain {
            size_m,
            cell_m,
            source,
            layers: Vec::new(),
            paint_rules: Vec::new(),
            edits: Vec::new(),
        }
    }

    /// Checks the terrain's limits (see [`crate::limits`]): every number finite and in range,
    /// `size_m` a whole multiple of `cell_m`, at most 2049 samples per side, and the layers,
    /// paint rules and edits (every layer they name exists, and building them stays within
    /// [`crate::limits::TERRAIN_EDIT_WORK_MAX`]). Never allocates more than the edit list.
    pub fn validate(&self) -> Result<(), WorldError> {
        self.grid().map(|_| ())
    }

    /// The grid, after checking every rule.
    pub(crate) fn grid(&self) -> Result<Grid, WorldError> {
        let cells = self.cells()?;
        let samples = cells.saturating_add(1);
        let half = self.size_m / 2.0;
        let grid = Grid {
            cols: samples,
            rows: samples,
            cell: self.cell_m,
            min: [-half, -half],
        };
        sculpt::validate(&self.layers, &self.paint_rules, &self.edits, &grid)?;
        Ok(grid)
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
            TerrainSource::Heightmap {
                ref cols,
                ref rows,
                ref content,
                ..
            } => {
                for (field, n) in [
                    ("terrain.source.cols", *cols),
                    ("terrain.source.rows", *rows),
                ] {
                    if !(HEIGHTMAP_SIDE_MIN..=HEIGHTMAP_SIDE_MAX).contains(&n) {
                        return Err(ctx.invalid(
                            field,
                            n,
                            format!("it must be from {HEIGHTMAP_SIDE_MIN} to {HEIGHTMAP_SIDE_MAX}"),
                        ));
                    }
                }
                let hex = content.len() == 16
                    && content
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                if !hex {
                    return Err(ctx.invalid(
                        "terrain.source.content",
                        content,
                        "it must be the hash of the heightmap's samples, sixteen lowercase \
                         hexadecimal digits as terrain.import writes it",
                    ));
                }
            }
            TerrainSource::Grid {
                ref cols,
                ref rows,
                ref heights,
            } => {
                if !(GRID_SIDE_MIN..=GRID_SIDE_MAX).contains(cols) {
                    return Err(ctx.invalid(
                        "terrain.source.cols",
                        cols,
                        format!("it must be from {GRID_SIDE_MIN} to {GRID_SIDE_MAX}"),
                    ));
                }
                if !(GRID_SIDE_MIN..=GRID_SIDE_MAX).contains(rows) {
                    return Err(ctx.invalid(
                        "terrain.source.rows",
                        rows,
                        format!("it must be from {GRID_SIDE_MIN} to {GRID_SIDE_MAX}"),
                    ));
                }
                // Both sides are in range, so the product cannot overflow and stays within
                // GRID_SAMPLES_MAX; check the length before looking at any height.
                let count = *cols as usize * *rows as usize;
                if heights.len() != count {
                    return Err(ctx.invalid(
                        "terrain.source.heights",
                        heights.len(),
                        format!("it must hold exactly {count} heights ({cols} x {rows})"),
                    ));
                }
                debug_assert!(count <= GRID_SAMPLES_MAX);
                for (i, h) in heights.iter().enumerate() {
                    if !h.is_finite() || h.abs() > MAX_COORDINATE_M {
                        return Err(ctx.invalid(
                            "terrain.source.heights",
                            h,
                            format!(
                                "heights[{i}] must be a finite number from {} to {} metres",
                                -MAX_COORDINATE_M, MAX_COORDINATE_M
                            ),
                        ));
                    }
                }
            }
        }
        Ok(cells as usize)
    }

    /// The height at world `x`, `z` according to the source (no grid, no edits).
    pub(crate) fn source_height(&self, x: f64, z: f64) -> f64 {
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
            TerrainSource::Grid {
                cols,
                rows,
                ref heights,
            } => grid_height(self.size_m, cols, rows, heights, x, z),
            TerrainSource::Heightmap {
                cols,
                rows,
                ref tiles,
                ..
            } => bilinear(self.size_m, cols as usize, rows as usize, x, z, |c, r| {
                tiles.sample(c as u32, r as u32)
            }),
        }
    }

    /// The heightmap's samples when the source is one; `None` for any other source.
    pub fn heightmap(&self) -> Option<&Heightmap> {
        match &self.source {
            TerrainSource::Heightmap { tiles, .. } => Some(tiles),
            _ => None,
        }
    }

    /// Whether the source is a heightmap whose samples the host has not handed over yet.
    pub fn needs_samples(&self) -> bool {
        matches!(self.source, TerrainSource::Heightmap { ref tiles, .. } if tiles.is_empty())
    }

    /// Hands over the heightmap's samples (the host reads them from the project). They must
    /// be what the document names: the same `cols` and `rows` and the same hash of their
    /// content. The document's text does not change. An error for another source, or for
    /// samples of another heightmap.
    pub fn set_heightmap(&mut self, tiles: Heightmap) -> Result<(), WorldError> {
        let TerrainSource::Heightmap {
            cols,
            rows,
            content,
            tiles: own,
        } = &mut self.source
        else {
            return Err(WorldError::Invalid {
                entity: None,
                field: "terrain.source".into(),
                value: self.source.kind().into(),
                expected: "a heightmap, to hand its samples over; terrain.set with kind \
                          \"heightmap\" makes one"
                    .into(),
            });
        };
        let said = Heightmap::content_text(tiles.content_hash());
        if tiles.cols() != *cols || tiles.rows() != *rows || said != *content {
            return Err(WorldError::Invalid {
                entity: None,
                field: "terrain.source.content".into(),
                value: format!("{said} ({} x {})", tiles.cols(), tiles.rows()),
                expected: format!(
                    "the content the document names, {content} ({cols} x {rows}); import the \
                     heightmap again, or name the one these samples belong to"
                ),
            });
        }
        *own = tiles;
        Ok(())
    }

    /// Builds the height field: validates, then samples the source at every grid node, applies
    /// the edits in order and derives the layer weights.
    ///
    /// Node (col, row) lies at world `x = -size_m / 2 + col * cell_m`,
    /// `z = -size_m / 2 + row * cell_m`. The heights depend on the world coordinates only, so
    /// a larger terrain extends the same land.
    pub fn build(&self) -> Result<HeightField, WorldError> {
        let grid = self.grid()?;
        if self.needs_samples() {
            return Err(WorldError::Invalid {
                entity: None,
                field: "terrain.source".into(),
                value: "heightmap".into(),
                expected: "a heightmap needs its samples, which the host hands to the world \
                          from the project's terrain/heightmap folder; terrain.import makes them"
                    .into(),
            });
        }
        let all = grid.all();
        let heights: Vec<f32> =
            eval_heights(&self.edits, &grid, all, |x, z| self.source_height(x, z))
                .into_iter()
                .map(|h| h as f32)
                .collect();
        let mut field = HeightField {
            cols: grid.cols,
            rows: grid.rows,
            cell_m: grid.cell,
            min: grid.min,
            heights,
            layers: 0,
            weights: Vec::new(),
        };
        if !self.layers.is_empty() {
            field.layers = self.layers.len();
            field.weights = vec![[0u8; WEIGHT_SLOTS]; field.heights.len()];
            // Tile by tile: the weights are worked out in f64 shares, eight per sample.
            for tile in crate::tiles::all_tiles(&grid) {
                let rect = crate::tiles::tile_rect(&grid, tile);
                let w = eval_weights(
                    &self.layers,
                    &self.paint_rules,
                    &self.edits,
                    &grid,
                    &field.heights,
                    rect,
                );
                field.put_weights(rect, &w);
            }
        }
        Ok(field)
    }

    /// The height field of this terrain, rebuilt from `field`, the height field of `previous`,
    /// in the tiles the difference touches only (see [`crate::tiles`]). Bit for bit what
    /// [`Terrain::build`] gives. A different size, cell or source, or a `field` of another
    /// grid, builds everything.
    pub fn rebuild(&self, previous: &Terrain, field: &HeightField) -> Result<Rebuilt, WorldError> {
        let grid = self.grid()?;
        let tiles = crate::tiles::tile_count(&grid);
        let total = tiles[0].saturating_mul(tiles[1]);
        let same_grid = self.size_m == previous.size_m
            && self.cell_m == previous.cell_m
            && self.source == previous.source
            && field.cols == grid.cols
            && field.rows == grid.rows
            && field.cell_m == grid.cell
            && field.min == grid.min
            && field.layers == previous.layers.len();
        if !same_grid {
            return Ok(Rebuilt {
                field: self.build()?,
                tiles: crate::tiles::all_tiles(&grid),
                tiles_total: total,
            });
        }
        let dirty = crate::tiles::dirty(previous, self, &grid);
        let mut out = field.clone();
        for &tile in &dirty.heights {
            let rect = crate::tiles::tile_rect(&grid, tile);
            let h = eval_heights(&self.edits, &grid, rect, |x, z| self.source_height(x, z));
            out.put_heights(rect, &h);
        }
        if self.layers.is_empty() {
            out.layers = 0;
            out.weights = Vec::new();
        } else {
            if out.weights.len() != out.heights.len() {
                out.weights = vec![[0u8; WEIGHT_SLOTS]; out.heights.len()];
            }
            out.layers = self.layers.len();
            for &tile in &dirty.weights {
                let rect = crate::tiles::tile_rect(&grid, tile);
                let w = eval_weights(
                    &self.layers,
                    &self.paint_rules,
                    &self.edits,
                    &grid,
                    &out.heights,
                    rect,
                );
                out.put_weights(rect, &w);
            }
        }
        let mut tiles: Vec<[usize; 2]> = dirty.heights;
        tiles.extend(dirty.weights);
        tiles.sort_unstable();
        tiles.dedup();
        Ok(Rebuilt {
            field: out,
            tiles,
            tiles_total: total,
        })
    }

    /// The index of the layer named `name`.
    pub fn layer_index(&self, name: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.name == name)
    }
}

/// A height field rebuilt from an earlier one ([`Terrain::rebuild`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Rebuilt {
    /// The new height field.
    pub field: HeightField,
    /// The tiles `[x, z]` whose heights or weights were computed again, in order.
    pub tiles: Vec<[usize; 2]>,
    /// How many tiles the terrain has.
    pub tiles_total: usize,
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
    /// How many layers the weights hold (0: no weights).
    layers: usize,
    /// Per sample the weights of up to eight layers, summing to 255; empty without layers.
    weights: Vec<[u8; WEIGHT_SLOTS]>,
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

    /// The unit upward vertex normal at sample (`col`, `row`), from central differences of the
    /// neighbouring samples (one-sided at the border); +Y where it cannot be computed. The
    /// renderer shades the terrain with these, and a road lying on it with the same.
    pub fn sample_normal(&self, col: usize, row: usize) -> [f64; 3] {
        let h = |c: usize, r: usize| self.sample(c, r).map_or(0.0, f64::from);
        let cell = self.cell_m;
        let (c0, c1) = (
            col.saturating_sub(1),
            (col + 1).min(self.cols.saturating_sub(1)),
        );
        let (r0, r1) = (
            row.saturating_sub(1),
            (row + 1).min(self.rows.saturating_sub(1)),
        );
        let dx = if c1 > c0 {
            (h(c1, row) - h(c0, row)) / ((c1 - c0) as f64 * cell)
        } else {
            0.0
        };
        let dz = if r1 > r0 {
            (h(col, r1) - h(col, r0)) / ((r1 - r0) as f64 * cell)
        } else {
            0.0
        };
        let v = [-dx, 1.0, -dz];
        let len = (dx * dx + 1.0 + dz * dz).sqrt();
        if len.is_finite() && len > 0.0 {
            v.map(|c| c / len)
        } else {
            [0.0, 1.0, 0.0]
        }
    }

    /// How many painted layers the terrain has (0: none, the weights are empty).
    pub fn layer_count(&self) -> usize {
        self.layers
    }

    /// Per sample, row-major like [`HeightField::heights`], the weights of the terrain's
    /// layers (unused slots 0), summing to 255. Empty when the terrain has no layers.
    pub fn weights(&self) -> &[[u8; WEIGHT_SLOTS]] {
        &self.weights
    }

    /// The layer weights of sample (`col`, `row`); `None` outside the grid or without layers.
    pub fn weight(&self, col: usize, row: usize) -> Option<[u8; WEIGHT_SLOTS]> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        let index = row.checked_mul(self.cols)?.checked_add(col)?;
        self.weights.get(index).copied()
    }

    /// The layer weights at world `x`, `z`: those of the nearest sample. `None` outside the
    /// terrain, for non-finite input or without layers.
    pub fn weight_at(&self, x: f64, z: f64) -> Option<[u8; WEIGHT_SLOTS]> {
        let p = self.locate(x, z)?;
        let col = p.col + usize::from(p.fu >= 0.5);
        let row = p.row + usize::from(p.fv >= 0.5);
        self.weight(col, row)
    }

    /// Writes `values` (row-major over `rect`) into the heights.
    pub(crate) fn put_heights(&mut self, rect: Rect, values: &[f64]) {
        let cols = self.cols;
        for ((c, r), v) in rect_samples(rect).zip(values) {
            if let Some(slot) = self.heights.get_mut(r * cols + c) {
                *slot = *v as f32;
            }
        }
    }

    /// Writes `values` (row-major over `rect`) into the weights.
    pub(crate) fn put_weights(&mut self, rect: Rect, values: &[[u8; WEIGHT_SLOTS]]) {
        let cols = self.cols;
        for ((c, r), v) in rect_samples(rect).zip(values) {
            if let Some(slot) = self.weights.get_mut(r * cols + c) {
                *slot = *v;
            }
        }
    }

    /// An empty field of `grid` with `layers` layers, to fill tile by tile.
    pub(crate) fn blank(grid: &Grid, layers: usize) -> HeightField {
        let n = grid.cols.saturating_mul(grid.rows);
        HeightField {
            cols: grid.cols,
            rows: grid.rows,
            cell_m: grid.cell,
            min: grid.min,
            heights: vec![0.0; n],
            layers,
            weights: if layers == 0 {
                Vec::new()
            } else {
                vec![[0u8; WEIGHT_SLOTS]; n]
            },
        }
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

/// The samples of `rect`, row by row.
pub(crate) fn rect_samples(rect: Rect) -> impl Iterator<Item = (usize, usize)> {
    (rect.r0..=rect.r1).flat_map(move |r| (rect.c0..=rect.c1).map(move |c| (c, r)))
}

/// The bilinear height at world `x`, `z` of a control grid of `cols` x `rows` `heights`
/// (row-major) spanning the square of side `size_m` centred on the origin. Outside the square
/// the nearest edge node's bilinear value holds; non-finite coordinates give the first height
/// of a guard path rather than NaN (a validated document never meets this).
fn grid_height(size_m: f64, cols: u32, rows: u32, heights: &[f64], x: f64, z: f64) -> f64 {
    debug_assert_eq!(heights.len(), cols as usize * rows as usize);
    bilinear(size_m, cols as usize, rows as usize, x, z, |c, r| {
        heights.get(r * cols as usize + c).copied().unwrap_or(0.0)
    })
}

/// The bilinear height at world `x`, `z` of the samples `at(c, r)` of `cols` by `rows`
/// spanning the square of side `size_m` centred on the origin; the control grid and the
/// heightmap share it.
fn bilinear(
    size_m: f64,
    cols: usize,
    rows: usize,
    x: f64,
    z: f64,
    at: impl Fn(usize, usize) -> f64,
) -> f64 {
    debug_assert!(size_m.is_finite() && size_m > 0.0);
    debug_assert!(cols >= 2 && rows >= 2);
    let half = size_m / 2.0;
    // Fractional node coordinates, clamped to the square.
    let u = if x.is_finite() {
        ((x + half) / size_m * (cols - 1) as f64).clamp(0.0, (cols - 1) as f64)
    } else {
        0.0
    };
    let v = if z.is_finite() {
        ((z + half) / size_m * (rows - 1) as f64).clamp(0.0, (rows - 1) as f64)
    } else {
        0.0
    };
    let c0 = (u.floor() as usize).min(cols.saturating_sub(2));
    let r0 = (v.floor() as usize).min(rows.saturating_sub(2));
    let fu = u - c0 as f64;
    let fv = v - r0 as f64;
    let h00 = at(c0, r0);
    let h10 = at(c0 + 1, r0);
    let h01 = at(c0, r0 + 1);
    let h11 = at(c0 + 1, r0 + 1);
    h00 * (1.0 - fu) * (1.0 - fv) + h10 * fu * (1.0 - fv) + h01 * (1.0 - fu) * fv + h11 * fu * fv
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
