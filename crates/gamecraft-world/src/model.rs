//! Models: an entity that shows a glTF asset of a pack, and the bounding boxes of the assets
//! that the host derives for the simulation.
//!
//! A `model` component names an asset by its path inside the pack's `assets/` folder
//! (`models/crate.glb`). The world document never reads the asset: what the simulation needs of
//! it (its local bounding box, for a solid model and for rays) is derived data that the host
//! computes from the asset's bytes and hands to the world with [`crate::World::set_model_bounds`].
//! It is not written to the file; a model whose asset has no bounds is not solid and is not hit
//! by rays.

use crate::error::Ctx;
use crate::limits::{SCALE_MAX, SCALE_MIN};
use crate::{Entity, WorldError};
use gamecraft_math::{Portable, deg_to_rad};
use serde::{Deserialize, Serialize};

/// Longest asset path, in bytes.
pub const MAX_ASSET_PATH_CHARS: usize = 200;

/// The file extensions of a model asset.
pub const MODEL_EXTENSIONS: [&str; 2] = ["glb", "gltf"];

/// Most levels of detail a model draws: the levels its asset has, or those its switch
/// distances name (one fewer than this).
pub const MAX_MODEL_LOD_LEVELS: usize = 8;

/// Whether `path` is a valid asset path: 1 to [`MAX_ASSET_PATH_CHARS`] ASCII letters, digits,
/// `-`, `_`, `.` and `/`, parts separated by single `/` (none empty, none `.` or `..`, no `/`
/// at either end). It names a file inside a pack's `assets/` folder.
pub fn is_asset_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_ASSET_PATH_CHARS
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Whether `path` is a valid asset path ending in `.glb` or `.gltf`.
pub fn is_model_path(path: &str) -> bool {
    is_asset_path(path)
        && path
            .rsplit_once('.')
            .is_some_and(|(_, ext)| MODEL_EXTENSIONS.contains(&ext))
}

/// What a model collides as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelCollider {
    /// Its asset's bounding box, turned by the entity's yaw: rays hit it, and with `solid`
    /// characters are blocked by it.
    #[default]
    Box,
    /// Nothing: rays pass and characters walk through, solid or not.
    None,
}

impl ModelCollider {
    /// The name as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            ModelCollider::Box => "box",
            ModelCollider::None => "none",
        }
    }
}

fn default_model_scale() -> f64 {
    1.0
}

/// A glTF model of a pack (`assets/<asset>`), drawn with its materials and textures, standing
/// on the entity's position (the asset's origin is the entity's base), turned by its yaw and
/// sized by `scale` times the entity's scale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    /// The asset's path inside the pack's `assets/` folder, ending in `.glb` or `.gltf`
    /// (`models/crate.glb`).
    pub asset: String,
    /// Uniform scale of the asset, on top of the entity's scale, 0.001 to 1000. Default 1.
    #[serde(default = "default_model_scale")]
    pub scale: f64,
    /// Whether characters are blocked by its collider. Default false.
    #[serde(default)]
    pub solid: bool,
    /// `box` (the asset's bounding box) or `none`. Default `box`.
    #[serde(default)]
    pub collider: ModelCollider,
    /// Where the asset's levels of detail give way to the next one, metres from the camera:
    /// the first ends level 0 (the finest), the second level 1, and so on, each farther than
    /// the one before. At most one fewer than the asset has levels; the ones left out follow
    /// the asset itself (its `extras.gamecraft`, else a rule from its size), which is also
    /// what applies when this is left out. Not written when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lod_distances_m: Option<Vec<f64>>,
    /// How far away the model is drawn, metres from the camera; beyond it nothing of it is
    /// drawn. Left out: the asset's own, else from its size. Not written when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_distance_m: Option<f64>,
}

impl Model {
    /// The component that shows `asset` (a path inside a pack's `assets/` folder) as it comes:
    /// scale 1, not solid, its bounding box as the collider, the asset's own distances.
    pub fn of(asset: impl Into<String>) -> Model {
        Model {
            asset: asset.into(),
            scale: 1.0,
            solid: false,
            collider: ModelCollider::Box,
            lod_distances_m: None,
            draw_distance_m: None,
        }
    }

    pub(crate) fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        if !is_model_path(&self.asset) {
            let shown: String = self.asset.chars().take(MAX_ASSET_PATH_CHARS + 8).collect();
            return Err(ctx.invalid(
                "model.asset",
                format!("{shown:?}"),
                format!(
                    "it must be a path inside the pack's assets/ folder ending in .glb or \
                     .gltf, 1 to {MAX_ASSET_PATH_CHARS} ASCII letters, digits, `-`, `_`, `.` \
                     and `/`, without `..`, such as \"models/crate.glb\""
                ),
            ));
        }
        ctx.range("model.scale", self.scale, SCALE_MIN, SCALE_MAX, "")?;
        self.validate_lod(ctx)
    }

    /// The level-of-detail distances: at most one fewer than a model may have levels, each
    /// finite and above 0, each farther than the one before, and the last nearer than the draw
    /// distance when the component gives one (a level past it would never be drawn).
    fn validate_lod(&self, ctx: Ctx) -> Result<(), WorldError> {
        let Some(distances) = &self.lod_distances_m else {
            if let Some(draw) = &self.draw_distance_m {
                ctx.positive("model.draw_distance_m", *draw, " m")?;
            }
            return Ok(());
        };
        if distances.len() >= MAX_MODEL_LOD_LEVELS {
            return Err(ctx.invalid(
                "model.lod_distances_m",
                format!("{} distances", distances.len()),
                format!(
                    "it must hold at most {} (one fewer than the {} levels a model may have)",
                    MAX_MODEL_LOD_LEVELS - 1,
                    MAX_MODEL_LOD_LEVELS
                ),
            ));
        }
        let mut previous: Option<f64> = None;
        for (k, &d) in distances.iter().enumerate() {
            ctx.positive(format!("model.lod_distances_m[{k}]"), d, " m")?;
            if let Some(before) = previous
                && d <= before
            {
                return Err(ctx.invalid(
                    format!("model.lod_distances_m[{k}]"),
                    d,
                    format!(
                        "it must be farther than the one before ({before} m): each coarser level \
                         takes over farther away"
                    ),
                ));
            }
            previous = Some(d);
        }
        if let (Some(&last), Some(&draw)) = (distances.last(), self.draw_distance_m.as_ref())
            && last >= draw
        {
            return Err(ctx.invalid(
                "model.lod_distances_m",
                last,
                format!(
                    "its last distance must be nearer than model.draw_distance_m ({draw} m), or \
                     the coarsest level is never drawn"
                ),
            ));
        }
        if let Some(draw) = &self.draw_distance_m {
            ctx.positive("model.draw_distance_m", *draw, " m")?;
        }
        Ok(())
    }
}

/// The local bounding box of a model asset, metres, in the asset's own axes (Y up; the origin
/// is the entity's base). Derived from the asset by the host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelBounds {
    /// The smallest x, y and z of the asset's triangles.
    pub min: [f64; 3],
    /// The largest x, y and z of the asset's triangles.
    pub max: [f64; 3],
}

impl ModelBounds {
    /// Whether every number is finite and `min <= max` on every axis.
    pub fn is_valid(&self) -> bool {
        self.min
            .iter()
            .zip(&self.max)
            .all(|(lo, hi)| lo.is_finite() && hi.is_finite() && lo <= hi)
    }
}

/// A model's collider in the world: a box turned about +Y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelBox {
    /// World centre of the box.
    pub center: [f64; 3],
    /// Half extents along the box's own x, y and z.
    pub half: [f64; 3],
    /// The turn about +Y, degrees (the entity's `rotation_y_deg`).
    pub yaw_deg: f64,
}

impl ModelBox {
    /// The collider box of `entity`'s model with the asset's `bounds`: the bounds times
    /// `model.scale * entity.scale`, turned by the entity's yaw about its position. `None`
    /// when the entity has no model, its collider is `none` or a number is not finite.
    pub fn of(entity: &Entity, bounds: &ModelBounds) -> Option<ModelBox> {
        let model = entity.model()?;
        if model.collider == ModelCollider::None || !bounds.is_valid() {
            return None;
        }
        let s = model.scale * entity.scale;
        let [x0, y0, z0] = bounds.min;
        let [x1, y1, z1] = bounds.max;
        let (lx, ly, lz) = (
            (x0 + x1) / 2.0 * s,
            (y0 + y1) / 2.0 * s,
            (z0 + z1) / 2.0 * s,
        );
        let half = [
            (x1 - x0) / 2.0 * s,
            (y1 - y0) / 2.0 * s,
            (z1 - z0) / 2.0 * s,
        ];
        let (sin, cos) = deg_to_rad(entity.rotation_y_deg).psin_cos();
        // Local to world: counter-clockwise seen from above, as the simulation's solids turn.
        let (wx, wz) = (lx * cos + lz * sin, -lx * sin + lz * cos);
        let [px, py, pz] = entity.position;
        let out = ModelBox {
            center: [px + wx, py + ly, pz + wz],
            half,
            yaw_deg: entity.rotation_y_deg,
        };
        let finite = out.center.iter().chain(&out.half).all(|v| v.is_finite());
        finite.then_some(out)
    }
}
