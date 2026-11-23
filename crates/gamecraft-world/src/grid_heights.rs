//! The node heights of a control-grid terrain, shared between copies of the terrain.
//!
//! A grid holds up to 513 x 513 numbers (2 MB), and every World command works on a copy of
//! the world, compares it with the one before and checks it. Behind an `Arc` the copy is a
//! reference count and the comparison of two copies one pointer test, so a command that does
//! not touch the terrain (an `entity.add`) costs nothing for it. A change replaces the whole
//! list: terrain edits are layered on top of the grid, never written into it.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;

/// The heights of a grid's nodes, metres, row-major; reads as a `[f64]`. In JSON a plain
/// list of numbers.
#[derive(Clone, Default)]
pub struct GridHeights(Arc<[f64]>);

impl std::ops::Deref for GridHeights {
    type Target = [f64];

    fn deref(&self) -> &[f64] {
        &self.0
    }
}

impl From<Vec<f64>> for GridHeights {
    fn from(heights: Vec<f64>) -> GridHeights {
        GridHeights(heights.into())
    }
}

impl PartialEq for GridHeights {
    /// The same numbers; copies of one list are equal without looking at them.
    fn eq(&self, other: &GridHeights) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || *self.0 == *other.0
    }
}

impl std::fmt::Debug for GridHeights {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for GridHeights {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter())
    }
}

impl<'de> Deserialize<'de> for GridHeights {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<GridHeights, D::Error> {
        Vec::<f64>::deserialize(deserializer).map(GridHeights::from)
    }
}
