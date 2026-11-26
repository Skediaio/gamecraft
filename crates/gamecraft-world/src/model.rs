//! Models: an entity that shows a glTF asset of a pack, and the bounding boxes and collision
//! parts of the assets that the host derives for the simulation.
//!
//! A `model` component names an asset by its path inside the pack's `assets/` folder
//! (`models/crate.glb`). The world document never reads the asset: what the simulation needs of
//! it (its local bounding box, for a solid model and for rays, and with the `mesh` collider its
//! parts: a box per `*_collider` node or per mesh of the file) is derived data that the host
//! computes from the asset's bytes and hands to the world with [`crate::World::set_model_bounds`]
//! and [`crate::World::set_model_parts`]. It is not written to the file; a model whose asset has
//! no bounds is not solid and is not hit by rays; one with the `mesh` collider collides as its
//! bounding box until its parts are handed over.

use crate::attachments::Attachment;
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
/// Most collision parts an asset has (the host hands over no more; beyond, the asset's
/// bounding box is its collider).
pub const MAX_MODEL_PARTS: usize = 256;

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
    /// Its asset's own geometry, as the host measured it ([`crate::World::set_model_parts`]):
    /// one box per `*_collider` node of the file, else one per mesh, in the entity's frame. So
    /// a doorway in a wall is walked through and a balcony stood on. Rays hit each part and a
    /// solid model blocks with each; until the parts arrive (or when the asset has more than
    /// [`MAX_MODEL_PARTS`]) the model collides as its bounding box, as `box` does.
    Mesh,
    /// Nothing: rays pass and characters walk through, solid or not.
    None,
}

impl ModelCollider {
    /// The name as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            ModelCollider::Box => "box",
            ModelCollider::Mesh => "mesh",
            ModelCollider::None => "none",
        }
    }

    /// Whether the model collides with anything at all (rays or characters).
    pub fn collides(self) -> bool {
        self != ModelCollider::None
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
    /// `box` (the asset's bounding box), `mesh` (the asset's own geometry, a box per
    /// `*_collider` node or per mesh of the file) or `none`. Default `box`.
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
    /// Models mounted on this model's named sockets (glTF nodes `socket_<name>`), in the order
    /// given, each with what is mounted on its own sockets ([`crate::attachments`]): a mount
    /// tree at most [`MAX_ATTACHMENT_DEPTH`] deep. Default none; written only when there is one,
    /// so a world without attachments keeps its text and its hash.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

impl Model {
    /// The component that shows `asset` (a path inside a pack's `assets/` folder) as it comes:
    /// scale 1, not solid, its bounding box as the collider, the asset's own distances, no
    /// attachments.
    pub fn of(asset: impl Into<String>) -> Model {
        Model {
            asset: asset.into(),
            scale: 1.0,
            solid: false,
            collider: ModelCollider::Box,
            lod_distances_m: None,
            draw_distance_m: None,
            attachments: Vec::new(),
        }
    }

    /// The attachment at `path` below this model (each name a socket of the attachment it is
    /// on; the first one of this model's own attachments).
    pub fn attachment(&self, path: &[String]) -> Option<&Attachment> {
        crate::attachments::walk(self, path).and_then(|chain| chain.last().copied())
    }

    /// The attachment at `path` below this model, to change.
    pub fn attachment_mut(&mut self, path: &[String]) -> Option<&mut Attachment> {
        // Where each step lies in its list, found by a shared read; then one mutable walk.
        let mut steps = Vec::with_capacity(path.len());
        let mut list: &[Attachment] = &self.attachments;
        for socket in path {
            let i = list.iter().position(|a| a.socket == *socket)?;
            steps.push(i);
            list = &list.get(i)?.attachments;
        }
        let (&first, rest) = steps.split_first()?;
        let mut at = self.attachments.get_mut(first)?;
        for &i in rest {
            at = at.attachments.get_mut(i)?;
        }
        Some(at)
    }

    /// The list of attachments that holds the one at `path`: the list of the attachment at
    /// `path` itself (what is mounted on it), the model's own list for an empty `path`.
    pub fn attachments_at_mut(&mut self, path: &[String]) -> Option<&mut Vec<Attachment>> {
        if path.is_empty() {
            return Some(&mut self.attachments);
        }
        Some(&mut self.attachment_mut(path)?.attachments)
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
        self.validate_lod(ctx)?;
        self.validate_attachments(ctx)
    }

    /// The attachments: at most [`MAX_ATTACHMENTS`] on one entity, every socket of one list of
    /// siblings used once (a socket holds one attachment, so a path names one).
    fn validate_attachments(&self, ctx: Ctx) -> Result<(), WorldError> {
        let mut count = 0usize;
        for (i, a) in self.attachments.iter().enumerate() {
            a.validate(ctx, 1, &format!("model.attachments[{i}]"), &mut count)?;
        }
        check_siblings(ctx, "model.attachments", &self.attachments)?;
        Ok(())
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

/// No two attachments of one list share a socket: `attachment.add` to a mounted socket is
/// refused, and a file that does it is refused too, so a path names one attachment.
fn check_siblings(ctx: Ctx, field: &str, list: &[Attachment]) -> Result<(), WorldError> {
    for (i, a) in list.iter().enumerate() {
        if list.iter().take(i).any(|b| b.socket == a.socket) {
            return Err(ctx.invalid(
                format!("{field}[{i}].socket"),
                format!("{:?}", a.socket),
                format!(
                    "the socket `{}` already holds an attachment of this list; a socket holds one \
                     attachment (swap it with attachment.set)",
                    a.socket
                ),
            ));
        }
        check_siblings(ctx, &format!("{field}[{i}].attachments"), &a.attachments)?;
    }
    Ok(())
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
    /// `model.scale` and the entity's scale, axis by axis, turned by the entity's yaw about its
    /// position. `None` when the entity has no model, its collider is `none` or a number is not
    /// finite.
    pub fn of(entity: &Entity, bounds: &ModelBounds) -> Option<ModelBox> {
        let model = entity.model()?;
        if model.collider == ModelCollider::None || !bounds.is_valid() {
            return None;
        }
        let [x0, y0, z0] = bounds.min;
        let [x1, y1, z1] = bounds.max;
        let [sx, sy, sz] = entity.scale.times([model.scale; 3]);
        // The same products in the same order as before the parts existed: a model with the
        // `box` collider hashes as it did.
        let (cx, cy, cz) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (z0 + z1) / 2.0);
        let (hx, hy, hz) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0, (z1 - z0) / 2.0);
        Self::placed(entity, [cx, cy, cz], [hx, hy, hz], [sx, sy, sz])
    }

    /// A box of the asset's own axes, centred on `center` with half extents `half`, placed for
    /// `entity`'s model: times `model.scale` and the entity's scale, axis by axis, turned by
    /// the entity's yaw about its position. `None` when the entity has no model or a number is
    /// not finite.
    pub fn part(entity: &Entity, center: [f64; 3], half: [f64; 3]) -> Option<ModelBox> {
        let model = entity.model()?;
        let scale = entity.scale.times([model.scale; 3]);
        Self::placed(entity, center, half, scale)
    }

    /// `center` and `half` of the asset's axes, each times its scale, turned by the entity's
    /// yaw about its position; the box's own turn stays 0 (its axes are the entity's).
    fn placed(
        entity: &Entity,
        center: [f64; 3],
        half: [f64; 3],
        scale: [f64; 3],
    ) -> Option<ModelBox> {
        let [sx, sy, sz] = scale;
        let [cx, cy, cz] = center;
        let [hx, hy, hz] = half;
        let (lx, ly, lz) = (cx * sx, cy * sy, cz * sz);
        let half = [hx * sx, hy * sy, hz * sz];
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
