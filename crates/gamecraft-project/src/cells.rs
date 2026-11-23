//! Spatial cells: a large world's layers split into a regular grid of files
//! (`worlds/<world>/layers/<layer>/<cx>_<cz>.json`), so no file holds the whole world, a change
//! touches the files of the places it changed, and a streamer can later load the cells near a
//! player (`docs/decisions.md`, *A large world is stored in cells*).
//!
//! Cell `(cx, cz)` covers `cx * size <= x < (cx + 1) * size` and the same for z, `size` being
//! the world's `cell_size_m`. An entity goes to the cell of its root: the entity at the top of
//! its chain of parents, so the children of a prefab instance go with their instance.

use crate::ProjectError;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The cell size when cells are turned on without one, metres: usual for maps of several km.
pub const DEFAULT_CELL_SIZE_M: f64 = 512.0;
/// Smallest cell size, metres.
pub const CELL_SIZE_MIN_M: f64 = 16.0;
/// Largest cell size, metres.
pub const CELL_SIZE_MAX_M: f64 = 16384.0;
/// Most cells one area query lists.
pub const MAX_AREA_CELLS: usize = 65_536;

/// A cell of the grid, by its indices along x and z.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Cell {
    /// Index along x: the cell covers `x * size` to `(x + 1) * size`.
    pub x: i32,
    /// Index along z.
    pub z: i32,
}

impl fmt::Display for Cell {
    /// The file stem: `3_-2`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", self.x, self.z)
    }
}

impl Cell {
    /// The cell a file stem names (`3_-2`), written exactly as [`Cell`]'s `Display` writes it.
    pub fn parse(stem: &str) -> Option<Cell> {
        let (x, z) = stem.split_once('_')?;
        let cell = Cell {
            x: x.parse().ok()?,
            z: z.parse().ok()?,
        };
        (cell.to_string() == stem).then_some(cell)
    }

    /// The cell of the point `(x, z)`; a number that is not finite counts as 0.
    pub fn of(x: f64, z: f64, size_m: f64) -> Cell {
        Cell {
            x: index(x, size_m),
            z: index(z, size_m),
        }
    }
}

/// The index of the cell `v` falls in, clamped to `i32`.
fn index(v: f64, size_m: f64) -> i32 {
    let i = (v / size_m).floor();
    if !i.is_finite() {
        return 0;
    }
    // Clamped first, so the cast cannot truncate.
    #[allow(clippy::cast_possible_truncation)]
    let out = i.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    out
}

/// `size_m` checked as a cell size (`what` names it in the error).
pub fn check_cell_size(size_m: f64, what: &str) -> Result<f64, ProjectError> {
    if size_m.is_finite() && (CELL_SIZE_MIN_M..=CELL_SIZE_MAX_M).contains(&size_m) {
        Ok(size_m)
    } else {
        Err(ProjectError::new(format!(
            "{what} is {size_m}; a cell is {CELL_SIZE_MIN_M} to {CELL_SIZE_MAX_M} metres on a \
             side ({DEFAULT_CELL_SIZE_M} is usual for a map of several km)"
        )))
    }
}

/// The cells of size `size_m` that intersect the area from `min` to `max` (`[x, z]`, metres,
/// corners in any order), in order (x first, then z). At most [`MAX_AREA_CELLS`].
pub fn cells_in_area(min: [f64; 2], max: [f64; 2], size_m: f64) -> Result<Vec<Cell>, ProjectError> {
    let size_m = check_cell_size(size_m, "the cell size")?;
    if min.iter().chain(&max).any(|v| !v.is_finite()) {
        return Err(ProjectError::new(
            "the area's corners must be finite numbers, [x, z] in metres",
        ));
    }
    let (x0, x1) = (min[0].min(max[0]), min[0].max(max[0]));
    let (z0, z1) = (min[1].min(max[1]), min[1].max(max[1]));
    let (a, b) = (Cell::of(x0, z0, size_m), Cell::of(x1, z1, size_m));
    let count =
        (i64::from(b.x) - i64::from(a.x) + 1).saturating_mul(i64::from(b.z) - i64::from(a.z) + 1);
    if count > MAX_AREA_CELLS as i64 {
        return Err(ProjectError::new(format!(
            "the area covers {count} cells of {size_m} m; ask for at most {MAX_AREA_CELLS} at a \
             time (a smaller area)"
        )));
    }
    let mut out = Vec::new();
    for x in a.x..=b.x {
        for z in a.z..=b.z {
            out.push(Cell { x, z });
        }
    }
    Ok(out)
}

/// The path of a cell of a layer, relative to its pack.
pub(crate) fn cell_path(world: &str, layer: &str, cell: Cell) -> String {
    format!("worlds/{world}/layers/{layer}/{cell}.json")
}
