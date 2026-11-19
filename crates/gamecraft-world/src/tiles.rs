//! Terrain tiles: the height field in squares of [`TERRAIN_TILE_CELLS`] cells (129 x 129
//! samples, neighbours sharing their border samples), which part of it an edit touches, and
//! the binary cache of resolved tiles a project keeps beside its text files.
//!
//! **The cache is derived data** (AGENTS.md rule 2): a tile file holds the final heights and
//! layer weights of one tile with a key, a hash of everything they follow from (the grid, the
//! source, the layers and rules, and the edits that reach the tile, in order). A tile whose
//! key does not match is stale and is built again; deleting the cache loses nothing.
//!
//! A tile file (`tile_<x>_<z>.bin`) is little-endian: the 8 bytes `GCTILE1\n`, the key (u64),
//! the tile's x and z (u32 each), its columns and rows of samples (u32 each), the number of
//! layers `n` (u32), then `cols * rows` heights (f32, row-major) and `cols * rows * n` weights
//! (u8, `n` per sample, summing to 255).

use crate::limits::TERRAIN_TILE_CELLS;
use crate::sculpt::{Grid, Rect, WEIGHT_SLOTS, eval_heights, eval_weights, source_region};
use crate::terrain::rect_samples;
use crate::{HeightField, Terrain, WorldError};
use gamecraft_math::StateHasher;
use std::collections::{BTreeMap, BTreeSet};

/// The first bytes of a tile file.
pub const TILE_MAGIC: [u8; 8] = *b"GCTILE1\n";
/// Bytes of a tile file before its heights.
pub const TILE_HEADER_BYTES: usize = 36;
/// The version of what a tile key covers; a new version makes every cached tile stale.
const KEY_VERSION: u64 = 1;
/// Most dirty rectangles tracked through the edits before a rebuild takes every tile.
const MAX_DIRTY_RECTS: usize = 4096;

/// The file name of tile `[x, z]`: `tile_<x>_<z>.bin`.
pub fn tile_file_name(tile: [usize; 2]) -> String {
    let [x, z] = tile;
    format!("tile_{x}_{z}.bin")
}

/// Tiles per side `[x, z]` of a grid (at least one).
pub(crate) fn tile_count(grid: &Grid) -> [usize; 2] {
    let per = |samples: usize| {
        samples
            .saturating_sub(1)
            .div_ceil(TERRAIN_TILE_CELLS)
            .max(1)
    };
    [per(grid.cols), per(grid.rows)]
}

/// Every tile, row by row.
pub(crate) fn all_tiles(grid: &Grid) -> Vec<[usize; 2]> {
    let [nx, nz] = tile_count(grid);
    (0..nz).flat_map(|z| (0..nx).map(move |x| [x, z])).collect()
}

/// The samples of tile `[x, z]`.
pub(crate) fn tile_rect(grid: &Grid, [x, z]: [usize; 2]) -> Rect {
    let last_c = grid.cols.saturating_sub(1);
    let last_r = grid.rows.saturating_sub(1);
    let c0 = x.saturating_mul(TERRAIN_TILE_CELLS).min(last_c);
    let r0 = z.saturating_mul(TERRAIN_TILE_CELLS).min(last_r);
    Rect {
        c0,
        r0,
        c1: c0.saturating_add(TERRAIN_TILE_CELLS).min(last_c),
        r1: r0.saturating_add(TERRAIN_TILE_CELLS).min(last_r),
    }
}

/// The tiles that hold the samples of `rect` (a sample on a shared border counts for the tile
/// it starts).
fn tiles_of(grid: &Grid, rect: &Rect, out: &mut BTreeSet<[usize; 2]>) {
    let [nx, nz] = tile_count(grid);
    let (x0, x1) = (
        (rect.c0 / TERRAIN_TILE_CELLS).min(nx - 1),
        (rect.c1 / TERRAIN_TILE_CELLS).min(nx - 1),
    );
    let (z0, z1) = (
        (rect.r0 / TERRAIN_TILE_CELLS).min(nz - 1),
        (rect.r1 / TERRAIN_TILE_CELLS).min(nz - 1),
    );
    for z in z0..=z1 {
        for x in x0..=x1 {
            out.insert([x, z]);
        }
    }
}

/// The tiles whose heights, and whose weights, differ between two terrains of the same grid
/// and source.
pub(crate) struct Dirty {
    pub heights: Vec<[usize; 2]>,
    pub weights: Vec<[usize; 2]>,
}

/// Which tiles change from `old` to `new` (same grid and source): the edits between their
/// common start and their common end changed, and a later smooth spreads a change by its
/// iterations.
pub(crate) fn dirty(old: &Terrain, new: &Terrain, grid: &Grid) -> Dirty {
    let (a, b) = (&old.edits, &new.edits);
    let prefix = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    let changed = a
        .get(prefix..a.len() - suffix)
        .unwrap_or_default()
        .iter()
        .chain(b.get(prefix..b.len() - suffix).unwrap_or_default());
    let mut height_rects: Vec<Rect> = Vec::new();
    let mut paint_rects: Vec<Rect> = Vec::new();
    for edit in changed {
        if let Some(r) = grid.rect_of(edit.bounds()) {
            if edit.is_paint() {
                paint_rects.push(r);
            } else {
                height_rects.push(r);
            }
        }
    }
    let mut overflow = false;
    for edit in b.get(b.len() - suffix..).unwrap_or_default() {
        let reach = match edit {
            crate::TerrainEdit::Smooth { iterations, .. } => *iterations as usize,
            _ => continue,
        };
        let Some(bounds) = grid.rect_of(edit.bounds()) else {
            continue;
        };
        let spread: Vec<Rect> = height_rects
            .iter()
            .filter(|d| d.intersects(&bounds))
            .map(|d| d.grow(reach, grid))
            .collect();
        height_rects.extend(spread);
        if height_rects.len() > MAX_DIRTY_RECTS {
            overflow = true;
            break;
        }
    }
    if overflow {
        let all = all_tiles(grid);
        return Dirty {
            heights: all.clone(),
            weights: all,
        };
    }
    let mut heights = BTreeSet::new();
    for r in &height_rects {
        tiles_of(grid, r, &mut heights);
    }
    let weights = if old.layers != new.layers || old.paint_rules != new.paint_rules {
        all_tiles(grid)
    } else if new.layers.is_empty() {
        Vec::new()
    } else {
        let mut w = BTreeSet::new();
        for r in height_rects
            .iter()
            .map(|r| r.grow(1, grid))
            .chain(paint_rects)
        {
            tiles_of(grid, &r, &mut w);
        }
        w.into_iter().collect()
    };
    Dirty {
        heights: heights.into_iter().collect(),
        weights,
    }
}

/// A height field built with a tile cache ([`Terrain::build_cached`]).
#[derive(Clone, Debug, PartialEq)]
pub struct CachedBuild {
    /// The height field, bit for bit what [`Terrain::build`] gives.
    pub field: HeightField,
    /// The key of every tile.
    pub keys: BTreeMap<[usize; 2], u64>,
    /// The tiles that were missing or stale in the cache and were built, in order.
    pub rebuilt: Vec<[usize; 2]>,
}

impl Terrain {
    /// The key of every tile `[x, z]`: a hash of what its heights and weights follow from.
    pub fn tile_keys(&self) -> Result<BTreeMap<[usize; 2], u64>, WorldError> {
        let grid = self.grid()?;
        let mut base = StateHasher::new();
        base.write_u64(KEY_VERSION);
        base.write_f64(self.size_m);
        base.write_f64(self.cell_m);
        base.write_str(&json(&self.source)?);
        base.write_str(&json(&self.layers)?);
        base.write_str(&json(&self.paint_rules)?);
        let edits: Vec<String> = self.edits.iter().map(json).collect::<Result<_, _>>()?;
        let mut keys = BTreeMap::new();
        for tile in all_tiles(&grid) {
            let rect = tile_rect(&grid, tile);
            // Weights read the heights one sample beyond the tile (the slope).
            let (_, used) = source_region(&self.edits, &grid, rect.grow(1, &grid));
            let mut h = base.clone();
            h.write_u64(tile[0] as u64);
            h.write_u64(tile[1] as u64);
            for i in used {
                h.write_u64(i as u64);
                h.write_str(edits.get(i).map_or("", String::as_str));
            }
            for (i, edit) in self.edits.iter().enumerate() {
                if edit.is_paint()
                    && grid
                        .rect_of(edit.bounds())
                        .is_some_and(|b| b.intersects(&rect))
                {
                    h.write_u64(u64::MAX - i as u64);
                    h.write_str(edits.get(i).map_or("", String::as_str));
                }
            }
            keys.insert(tile, h.finish());
        }
        Ok(keys)
    }

    /// Builds the height field from cached tile files (by tile `[x, z]`, as
    /// [`HeightField::tile_bytes`] wrote them), building only the tiles that are missing, stale
    /// (another key) or not valid. The result is bit for bit what [`Terrain::build`] gives.
    pub fn build_cached(
        &self,
        cached: &BTreeMap<[usize; 2], Vec<u8>>,
    ) -> Result<CachedBuild, WorldError> {
        let grid = self.grid()?;
        let keys = self.tile_keys()?;
        let layers = self.layers.len();
        let mut field = HeightField::blank(&grid, layers);
        let mut stale = Vec::new();
        for (&tile, &key) in &keys {
            let rect = tile_rect(&grid, tile);
            let loaded = cached
                .get(&tile)
                .and_then(|bytes| decode(bytes, key, tile, &rect, layers));
            match loaded {
                Some((heights, weights)) => {
                    let as_f64: Vec<f64> = heights.iter().map(|h| f64::from(*h)).collect();
                    field.put_heights(rect, &as_f64);
                    field.put_weights(rect, &weights);
                }
                None => stale.push(tile),
            }
        }
        if stale.len() == keys.len() {
            return Ok(CachedBuild {
                field: self.build()?,
                keys,
                rebuilt: stale,
            });
        }
        for &tile in &stale {
            let rect = tile_rect(&grid, tile);
            let h = eval_heights(&self.edits, &grid, rect, |x, z| self.source_height(x, z));
            field.put_heights(rect, &h);
        }
        if layers > 0 {
            for &tile in &stale {
                let rect = tile_rect(&grid, tile);
                let w = eval_weights(
                    &self.layers,
                    &self.paint_rules,
                    &self.edits,
                    &grid,
                    field.heights(),
                    rect,
                );
                field.put_weights(rect, &w);
            }
        }
        Ok(CachedBuild {
            field,
            keys,
            rebuilt: stale,
        })
    }
}

/// A recipe part as JSON text, for a key.
fn json<T: serde::Serialize>(v: &T) -> Result<String, WorldError> {
    serde_json::to_string(v).map_err(|e| WorldError::Serialize {
        message: e.to_string(),
    })
}

impl HeightField {
    /// Tiles per side `[x, z]`.
    pub fn tile_count(&self) -> [usize; 2] {
        tile_count(&self.grid())
    }

    /// The tile file of tile `[x, z]` with `key` (see the module documentation); `None` for a
    /// tile the field does not have.
    pub fn tile_bytes(&self, tile: [usize; 2], key: u64) -> Option<Vec<u8>> {
        let grid = self.grid();
        let [nx, nz] = tile_count(&grid);
        if tile[0] >= nx || tile[1] >= nz {
            return None;
        }
        let rect = tile_rect(&grid, tile);
        let layers = self.layer_count();
        let n = rect.width() * rect.height();
        let mut out = Vec::with_capacity(TILE_HEADER_BYTES + n * (4 + layers));
        out.extend_from_slice(&TILE_MAGIC);
        out.extend_from_slice(&key.to_le_bytes());
        for v in [tile[0], tile[1], rect.width(), rect.height(), layers] {
            out.extend_from_slice(&u32::try_from(v).ok()?.to_le_bytes());
        }
        for (c, r) in rect_samples(rect) {
            out.extend_from_slice(&self.sample(c, r)?.to_le_bytes());
        }
        if layers > 0 {
            for (c, r) in rect_samples(rect) {
                let w = self.weight(c, r)?;
                out.extend_from_slice(w.get(..layers)?);
            }
        }
        Some(out)
    }

    /// The grid of this field.
    fn grid(&self) -> Grid {
        Grid {
            cols: self.cols(),
            rows: self.rows(),
            cell: self.cell_m(),
            min: self.min(),
        }
    }
}

/// A tile file's heights and weights when it is valid for `key`, `tile`, `rect` and `layers`.
fn decode(
    bytes: &[u8],
    key: u64,
    tile: [usize; 2],
    rect: &Rect,
    layers: usize,
) -> Option<(Vec<f32>, Vec<[u8; WEIGHT_SLOTS]>)> {
    let (header, body) = bytes.split_at_checked(TILE_HEADER_BYTES)?;
    let (magic, rest) = header.split_at_checked(8)?;
    if magic != TILE_MAGIC {
        return None;
    }
    let (k, rest) = rest.split_at_checked(8)?;
    if u64::from_le_bytes(k.try_into().ok()?) != key {
        return None;
    }
    let mut fields = rest
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c));
    for want in [tile[0], tile[1], rect.width(), rect.height(), layers] {
        if fields.next()? as usize != want {
            return None;
        }
    }
    let n = rect.width().checked_mul(rect.height())?;
    let height_bytes = n.checked_mul(4)?;
    let weight_bytes = n.checked_mul(layers)?;
    if body.len() != height_bytes.checked_add(weight_bytes)? {
        return None;
    }
    let (h, w) = body.split_at_checked(height_bytes)?;
    let heights: Vec<f32> = h
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();
    if heights.len() != n || heights.iter().any(|h| !h.is_finite()) {
        return None;
    }
    let mut weights = Vec::new();
    if layers > 0 {
        for chunk in w.chunks_exact(layers) {
            if chunk.iter().map(|b| u32::from(*b)).sum::<u32>() != 255 {
                return None;
            }
            let mut slot = [0u8; WEIGHT_SLOTS];
            for (s, b) in slot.iter_mut().zip(chunk) {
                *s = *b;
            }
            weights.push(slot);
        }
        if weights.len() != n {
            return None;
        }
    }
    Some((heights, weights))
}
