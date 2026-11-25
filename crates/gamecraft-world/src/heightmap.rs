//! The samples of a heightmap at full resolution, the terrain's source when real ground
//! comes in at 1 to 4 m: tiles of 128 cells (129 x 129 samples a side, neighbours sharing their
//! border samples, as the derived field's tiles), shared between copies of the terrain.
//!
//! The document names the heightmap (`cols`, `rows`, a hash of its content); the samples
//! themselves are too many for the text (a kilometre at 2 m is half a million) and are handed
//! to the world by the host ([`crate::World::set_heightmap`]), which reads them from the
//! project's `terrain/heightmap/` folder. They are plain numbers: no trigonometry, the same
//! bits on every platform.

use gamecraft_math::StateHasher;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The side of a tile in cells; a tile holds one more sample per side.
pub const TILE_CELLS: usize = 128;
/// Most samples of a heightmap per side (the field's limit).
pub const HEIGHTMAP_SIDE_MAX: u32 = 2049;
/// Fewest samples per side.
pub const HEIGHTMAP_SIDE_MIN: u32 = 2;
/// The bytes of one tile: `f32` little-endian, row-major from the north-west corner.
pub const TILE_SAMPLES: usize = (TILE_CELLS + 1) * (TILE_CELLS + 1);

/// The tiles of a heightmap, by `(tx, tz)`: tile `tx` covers the samples
/// `tx * TILE_CELLS ..= (tx + 1) * TILE_CELLS` in x, the same in z (the last sample shared with
/// the next tile). Sample `(c, r)` is at world `x = -size_m / 2 + c * spacing_m`,
/// `z = -size_m / 2 + r * spacing_m`, row-major from the north-west corner (the smallest x and
/// z), as the derived field's nodes.
#[derive(Clone, Debug, Default)]
pub struct Heightmap(Arc<Inner>);

#[derive(Clone, Debug, Default, PartialEq)]
struct Inner {
    cols: u32,
    rows: u32,
    tiles: BTreeMap<(i32, i32), Arc<[f32]>>,
}

impl PartialEq for Heightmap {
    /// The same shape and tiles; copies of one heightmap are equal without looking at them.
    fn eq(&self, other: &Heightmap) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

/// One sample as `f64`: exact, as the field stores heights.
fn sample_of(t: f32) -> f64 {
    f64::from(t)
}

impl Heightmap {
    /// An empty heightmap: the document's before the host hands the samples over.
    pub fn new() -> Heightmap {
        Heightmap::default()
    }

    /// How many tiles a heightmap of `cols` by `rows` samples has, per side.
    pub fn tiles_of(cols: u32, rows: u32) -> (usize, usize) {
        // Tile `t` covers the samples `t * TILE_CELLS ..= (t + 1) * TILE_CELLS`: 129 samples
        // are one tile, 130 are two.
        let per_side = |n: u32| {
            usize::try_from(n.max(1).saturating_sub(1))
                .unwrap_or(0)
                .div_ceil(TILE_CELLS)
                .max(1)
        };
        (per_side(cols), per_side(rows))
    }

    /// The heightmap of `cols` by `rows` samples given row-major by `rows_of` (`(c, r)` to
    /// metres): every number finite, `cols` and `rows` within the limits. An error, never a
    /// panic, for anything else.
    pub fn from_samples(
        cols: u32,
        rows: u32,
        sample: impl Fn(u32, u32) -> f64,
    ) -> Result<Heightmap, String> {
        if !(HEIGHTMAP_SIDE_MIN..=HEIGHTMAP_SIDE_MAX).contains(&cols) {
            return Err(format!(
                "cols {cols} is outside {HEIGHTMAP_SIDE_MIN} to {HEIGHTMAP_SIDE_MAX}"
            ));
        }
        if !(HEIGHTMAP_SIDE_MIN..=HEIGHTMAP_SIDE_MAX).contains(&rows) {
            return Err(format!(
                "rows {rows} is outside {HEIGHTMAP_SIDE_MIN} to {HEIGHTMAP_SIDE_MAX}"
            ));
        }
        let (tx, tz) = Self::tiles_of(cols, rows);
        let mut tiles = BTreeMap::new();
        for tz_i in 0..tz {
            for tx_i in 0..tx {
                let mut tile = Vec::with_capacity(TILE_SAMPLES);
                for r in 0..=TILE_CELLS {
                    for c in 0..=TILE_CELLS {
                        let (sc, sr) = (
                            (tx_i * TILE_CELLS + c) as u32,
                            (tz_i * TILE_CELLS + r) as u32,
                        );
                        // Past the last sample a tile repeats the edge: never read.
                        let h = sample(sc.min(cols - 1), sr.min(rows - 1));
                        if !h.is_finite() || h.abs() > 1_000_000.0 {
                            return Err(format!(
                                "the height at ({sc}, {sr}) is {h}; every height must be a finite \
                                 number of metres within plus or minus 1 000 000"
                            ));
                        }
                        tile.push(h as f32);
                    }
                }
                tiles.insert((tx_i as i32, tz_i as i32), Arc::from(tile));
            }
        }
        Ok(Heightmap(Arc::new(Inner { cols, rows, tiles })))
    }

    /// The heightmap of tiles read from files: every tile of `cols` by `rows` must be
    /// present and hold [`TILE_SAMPLES`] samples, every one finite.
    pub fn from_tiles(
        cols: u32,
        rows: u32,
        tiles: BTreeMap<(i32, i32), Arc<[f32]>>,
    ) -> Result<Heightmap, String> {
        if !(HEIGHTMAP_SIDE_MIN..=HEIGHTMAP_SIDE_MAX).contains(&cols)
            || !(HEIGHTMAP_SIDE_MIN..=HEIGHTMAP_SIDE_MAX).contains(&rows)
        {
            return Err(format!(
                "a heightmap of {cols} x {rows} samples is outside {HEIGHTMAP_SIDE_MIN} to \
                 {HEIGHTMAP_SIDE_MAX} a side"
            ));
        }
        let (tx, tz) = Self::tiles_of(cols, rows);
        if let Some(&(px, pz)) = tiles
            .keys()
            .find(|(px, pz)| *px < 0 || *pz < 0 || *px >= tx as i32 || *pz >= tz as i32)
        {
            return Err(format!(
                "tile ({px}, {pz}) lies outside a heightmap of {cols} x {rows} samples"
            ));
        }
        for tz_i in 0..tz {
            for tx_i in 0..tx {
                let tile = tiles
                    .get(&(tx_i as i32, tz_i as i32))
                    .ok_or_else(|| format!("tile ({tx_i}, {tz_i}) is missing"))?;
                if tile.len() != TILE_SAMPLES {
                    return Err(format!(
                        "tile ({tx_i}, {tz_i}) holds {} samples, {TILE_SAMPLES} expected",
                        tile.len()
                    ));
                }
                if tile.iter().any(|h| !h.is_finite()) {
                    return Err(format!(
                        "tile ({tx_i}, {tz_i}) holds a height that is not a finite number"
                    ));
                }
            }
        }
        Ok(Heightmap(Arc::new(Inner { cols, rows, tiles })))
    }

    /// Samples along x; 0 of no heightmap.
    pub fn cols(&self) -> u32 {
        self.0.cols
    }

    /// Samples along z; 0 of no heightmap.
    pub fn rows(&self) -> u32 {
        self.0.rows
    }

    /// Whether no sample was handed over.
    pub fn is_empty(&self) -> bool {
        self.0.tiles.is_empty()
    }

    /// The tiles, by their place.
    pub fn tiles(&self) -> &BTreeMap<(i32, i32), Arc<[f32]>> {
        &self.0.tiles
    }

    /// The sample `(c, r)` in metres; the nearest edge sample outside, 0 with no tiles.
    pub fn sample(&self, c: u32, r: u32) -> f64 {
        if self.0.tiles.is_empty() {
            return 0.0;
        }
        let (c, r) = (
            c.min(self.0.cols.saturating_sub(1)),
            r.min(self.0.rows.saturating_sub(1)),
        );
        let (lc, lr) = (c as usize, r as usize);
        self.at(lc / TILE_CELLS, lr / TILE_CELLS, lc, lr)
    }

    /// The sample at `(lc, lr)` of the tile grid, looking in the neighbour over the border
    /// when the map has no tile there: the last sample of a side is the first of the next
    /// tile, and a map that ends on it keeps it in its last tile.
    fn at(&self, tx: usize, tz: usize, lc: usize, lr: usize) -> f64 {
        let (lc, lr) = (lc % TILE_CELLS, lr % TILE_CELLS);
        let (over_x, over_z) = (lc == 0 && tx > 0, lr == 0 && tz > 0);
        let last = TILE_CELLS;
        for (dx, dz) in [(0usize, 0usize), (1, 0), (0, 1), (1, 1)] {
            if (dx == 1 && !over_x) || (dz == 1 && !over_z) {
                continue;
            }
            let (c, r) = (
                if dx == 1 { last } else { lc },
                if dz == 1 { last } else { lr },
            );
            let found = self
                .0
                .tiles
                .get(&((tx - dx) as i32, (tz - dz) as i32))
                .and_then(|tile| tile.get(r * (TILE_CELLS + 1) + c));
            if let Some(&h) = found {
                return sample_of(h);
            }
        }
        0.0
    }

    /// The lowest and the highest sample, in metres; `(0, 0)` with no tiles.
    pub fn height_range(&self) -> (f32, f32) {
        let mut range: Option<(f32, f32)> = None;
        for tile in self.0.tiles.values() {
            for &h in tile.iter() {
                range = Some(match range {
                    None => (h, h),
                    Some((lo, hi)) => (lo.min(h), hi.max(h)),
                });
            }
        }
        range.unwrap_or((0.0, 0.0))
    }

    /// The hash of the content: over every tile's place and samples, in order. The document
    /// keeps it, so that two heightmaps of one shape are not the same world.
    pub fn content_hash(&self) -> u64 {
        let mut hasher = StateHasher::new();
        hasher.write_bytes(b"gamecraft-heightmap");
        hasher.write_u64(self.0.tiles.len() as u64);
        for (&(tx, tz), tile) in self.0.tiles.iter() {
            hasher.write_i64(i64::from(tx));
            hasher.write_i64(i64::from(tz));
            hasher.write_u64(tile.len() as u64);
            for &h in tile.iter() {
                hasher.write_f32(h);
            }
        }
        hasher.finish()
    }

    /// The hash of a content as the document shows it: sixteen lowercase hexadecimal digits.
    pub fn content_text(hash: u64) -> String {
        format!("{hash:016x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(c: u32, r: u32) -> f64 {
        (c * 7 + r * 3) as f64 / 10.0 - 100.0
    }

    #[test]
    fn a_heightmap_keeps_its_samples_exactly_and_hash_its_content() {
        let map = Heightmap::from_samples(300, 129, sample).unwrap();
        assert_eq!(Heightmap::tiles_of(300, 129), (3, 1));
        assert_eq!(map.tiles().len(), 3);
        // Every sample read back exactly (the source stores f32; f64::from is exact).
        for r in [0, 1, 64, 127, 128, 200, 299] {
            for c in [0, 1, 127, 128, 129, 254, 255, 299] {
                // Outside the map the nearest edge sample.
                let (ec, er) = (c.min(299), r.min(128));
                assert_eq!(
                    map.sample(c, r),
                    f64::from(sample(ec, er) as f32),
                    "({c}, {r})"
                );
            }
        }
        let hash = map.content_hash();
        assert_eq!(
            Heightmap::from_samples(300, 129, sample)
                .unwrap()
                .content_hash(),
            hash,
            "the same content hashes the same"
        );
        let other = Heightmap::from_samples(300, 129, |c, r| sample(c, r) + 0.5).unwrap();
        assert_ne!(other.content_hash(), hash);
        assert_eq!(Heightmap::content_text(hash).len(), 16);
        assert!(
            Heightmap::content_text(hash)
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        );
        assert_eq!(map, Heightmap::from_samples(300, 129, sample).unwrap());
        assert!(Heightmap::new().is_empty() && Heightmap::new().sample(3, 3) == 0.0);
        assert_eq!((map.cols(), map.rows()), (300, 129));
        assert_eq!((Heightmap::new().cols(), Heightmap::new().rows()), (0, 0));
        // The last sample of a side lives in the last tile, not in a next one: a map of
        // exactly 129 or 257 samples a side reads it, and so does one sample short.
        for n in [129usize, 130, 257, 258] {
            let map = Heightmap::from_samples(n as u32, n as u32, sample).unwrap();
            for edge in [n - 2, n - 1] {
                let c = edge as u32;
                assert_eq!(map.sample(c, c), f64::from(sample(c, c) as f32), "{n}");
                assert_eq!(map.sample(c, 0), f64::from(sample(c, 0) as f32), "{n}");
                assert_eq!(map.sample(0, c), f64::from(sample(0, c) as f32), "{n}");
            }
            assert_eq!(
                map.sample(n as u32, 0),
                f64::from(sample(n as u32 - 1, 0) as f32)
            );
        }
    }

    #[test]
    fn a_heightmap_refuses_what_is_not_one() {
        assert!(Heightmap::from_samples(1, 10, sample).is_err());
        assert!(Heightmap::from_samples(10, 2050, sample).is_err());
        assert!(
            Heightmap::from_samples(10, 10, |_, r| if r == 3 { f64::NAN } else { 0.0 }).is_err()
        );
        assert!(Heightmap::from_samples(10, 10, |_, r| if r == 3 { 2e6 } else { 0.0 }).is_err());
        // Tiles read from files: a missing one, a short one, a NaN.
        let map = Heightmap::from_samples(129, 129, sample).unwrap();
        let tiles = map.tiles().clone();
        assert!(Heightmap::from_tiles(129, 129, tiles.clone()).is_ok());
        assert!(
            Heightmap::from_tiles(257, 129, tiles.clone()).is_err(),
            "tiles missing"
        );
        let mut short = tiles.clone();
        short.insert((0, 0), Arc::from(vec![0.0; 10]));
        assert!(Heightmap::from_tiles(129, 129, short).is_err());
        let mut nan = tiles.clone();
        nan.insert((0, 0), Arc::from(vec![f32::NAN; TILE_SAMPLES]));
        let e = Heightmap::from_tiles(129, 129, nan).unwrap_err();
        assert!(e.contains("not a finite number"), "{e}");
        let mut far = tiles.clone();
        far.insert((5, 0), Arc::from(vec![0.0; TILE_SAMPLES]));
        assert!(
            Heightmap::from_tiles(129, 129, far)
                .unwrap_err()
                .contains("outside"),
            "a tile outside"
        );
        assert!(Heightmap::from_tiles(0, 129, tiles.clone()).is_err());
        let from_files = Heightmap::from_tiles(129, 129, tiles).unwrap();
        assert_eq!((from_files.cols(), from_files.rows()), (129, 129));
        assert_eq!(from_files.content_hash(), map.content_hash());
    }
}
