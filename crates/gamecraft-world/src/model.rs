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
use crate::limits::{
    ADS_FOV_FACTOR_MIN, ADS_TIME_MAX_S, HELD_ATTACHMENTS_MAX, HELD_OFFSET_MAX_M, MUZZLE_MAX_M,
    RETICLE_RADIUS_MAX_MRAD, RETICLE_RADIUS_MIN_MRAD, SCALE_MAX, SCALE_MIN, ZERO_MAX_M, ZERO_MIN_M,
};
use crate::{Entity, WorldError};
use gamecraft_math::{Portable, deg_to_rad};
use serde::{Deserialize, Serialize};

/// Longest asset path, in bytes.
pub const MAX_ASSET_PATH_CHARS: usize = 200;

/// The file extensions of a model asset.
pub const MODEL_EXTENSIONS: [&str; 2] = ["glb", "gltf"];

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
}

impl Model {
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
        ctx.range("model.scale", self.scale, SCALE_MIN, SCALE_MAX, "")
    }
}

/// Where a [`HeldModel`] sits when its component leaves `offset_m` out: right, up and
/// forward of the camera, metres.
pub const DEFAULT_HELD_OFFSET_M: [f64; 3] = [0.12, -0.14, 0.32];

fn default_held_offset() -> [f64; 3] {
    DEFAULT_HELD_OFFSET_M
}

/// What a character holds as seen from its own eyes: a glTF model of a pack drawn at the
/// first-person camera (a weapon's view model). It follows the aim, not the body: the model's
/// own forward (-Z, as glTF faces) points where the character aims, `offset_m` to the right,
/// up and forward of the eye. It is drawn only while the character is seen in first person.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldModel {
    /// The asset's path inside the pack's `assets/` folder, ending in `.glb` or `.gltf`
    /// (`models/rifle.glb`).
    pub asset: String,
    /// Where the model's origin sits from the eye along the aim: `[right, up, forward]`,
    /// metres, each -2 to 2. Default `[0.12, -0.14, 0.32]`.
    #[serde(default = "default_held_offset")]
    pub offset_m: [f64; 3],
    /// Uniform scale of the asset, 0.001 to 1000 (the entity's scale does not apply).
    /// Default 1.
    #[serde(default = "default_model_scale")]
    pub scale: f64,
    /// Where the model's origin sits while the character aims down the sights, as `offset_m`
    /// (each -2 to 2): put the sight line on the camera's axis. `null` keeps it at `offset_m`.
    /// Default null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ads_offset_m: Option<[f64; 3]>,
    /// The share of the field of view left while aiming down the sights, 0.2 to 1. Default
    /// 0.85.
    #[serde(
        default = "default_ads_fov_factor",
        skip_serializing_if = "is_default_ads_fov_factor"
    )]
    pub ads_fov_factor: f64,
    /// Seconds the model takes to come up to the sights (and the view to narrow), and to go
    /// back, 0 to 5. Default 0.2.
    #[serde(
        default = "default_ads_time_s",
        skip_serializing_if = "is_default_ads_time_s"
    )]
    pub ads_time_s: f64,
    /// Extra models drawn rigidly with the held model (an optic on the rail, a light): each
    /// an asset placed in the held model's own axes. At most 8. Default none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<HeldAttachment>,
    /// The dot of a collimated sight (a red dot), drawn while the character is fully down the
    /// sights where the barrel points (with its sway and recoil), as a sight focused at
    /// infinity shows it. `null` draws none. Default null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reticle: Option<Reticle>,
    /// Where the barrel's mouth is, in the held model's own axes `[right, up, forward]`: metres
    /// before the model's scale, from the model's origin, each -5 to 5. Forward is glTF's -Z
    /// (the model's -Z axis points where the character aims, so a barrel pointing down the
    /// model's -Z has a positive forward). When set, a character's shot leaves from this point
    /// as the view model carries it (hip or sights, with the sway and the recoil), along the
    /// barrel, instead of from the eye; characters without it (and without a `held_model`) shoot
    /// from the eye. `null` shoots from the eye. Default null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muzzle_m: Option<[f64; 3]>,
    /// The distance, metres, 1 to 10000, at which the sights are zeroed: the barrel is tilted
    /// up (and to the side) relative to the sight line, once for the weapon, so that a shot
    /// fired with the model at the sights crosses the line of sight there, in still air at the
    /// standard atmosphere; it only matters with `muzzle_m`. `null` zeroes at 100. Default null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zero_m: Option<f64>,
}

/// The zeroing distance of a [`HeldModel`] that leaves `zero_m` out, metres.
pub const DEFAULT_ZERO_M: f64 = 100.0;

/// The axes of an aim in world coordinates: where the model's right (+X), up (+Y) and back
/// (+Z, opposite its forward) point for an aim of `yaw_deg` (counter-clockwise from north seen
/// from above) and `pitch_deg` (up from level). The model is turned by yaw about Y, then pitch
/// about X, as the first-person view model is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimAxes {
    /// The aim's right.
    pub right: [f64; 3],
    /// The aim's up.
    pub up: [f64; 3],
    /// Opposite the aim's forward.
    pub back: [f64; 3],
}

impl AimAxes {
    /// The axes of an aim, with portable sines and cosines.
    pub fn of(yaw_deg: f64, pitch_deg: f64) -> AimAxes {
        let (sy, cy) = deg_to_rad(yaw_deg).psin_cos();
        let (sp, cp) = deg_to_rad(pitch_deg).psin_cos();
        AimAxes {
            right: [cy, 0.0, -sy],
            up: [sy * sp, cp, cy * sp],
            back: [sy * cp, -sp, cy * cp],
        }
    }

    /// The world vector of `local` = (right, up, back) in these axes.
    pub fn vector(&self, local: [f64; 3]) -> [f64; 3] {
        let [r, u, b] = local;
        let [rx, ry, rz] = self.right;
        let [ux, uy, uz] = self.up;
        let [bx, by, bz] = self.back;
        [
            r * rx + u * ux + b * bx,
            r * ry + u * uy + b * by,
            r * rz + u * uz + b * bz,
        ]
    }
}

/// A model drawn rigidly with a [`HeldModel`]: an optic, a light, a grip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldAttachment {
    /// The asset's path inside the pack's `assets/` folder, ending in `.glb` or `.gltf`
    /// (`models/optic.glb`).
    pub asset: String,
    /// Where the attachment's origin sits in the held model's own axes (x right, y up, z
    /// back, as glTF faces), in the held model's units before its scale, each -2 to 2.
    /// Default `[0, 0, 0]`.
    #[serde(default)]
    pub offset_m: [f64; 3],
}

/// The dot of a collimated sight: its colour, size and brightness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Reticle {
    /// sRGB, each channel 0 to 1. Default red `[1, 0.08, 0.05]`.
    pub color_srgb: [f64; 3],
    /// The dot's radius as an angle, milliradians, 0.05 to 20 (a 2 MOA dot is about 0.3; it
    /// is drawn at least a few pixels wide). Default 0.6.
    pub radius_mrad: f64,
    /// How bright the dot is, 0 (off) to 1. Default 1.
    pub brightness: f64,
}

impl Default for Reticle {
    fn default() -> Reticle {
        Reticle {
            color_srgb: [1.0, 0.08, 0.05],
            radius_mrad: 0.6,
            brightness: 1.0,
        }
    }
}

/// The share of the field of view a [`HeldModel`] leaves while aiming down the sights when its
/// component leaves `ads_fov_factor` out.
pub const DEFAULT_ADS_FOV_FACTOR: f64 = 0.85;
/// Seconds a [`HeldModel`] takes to come up to the sights when its component leaves
/// `ads_time_s` out.
pub const DEFAULT_ADS_TIME_S: f64 = 0.2;

fn default_ads_fov_factor() -> f64 {
    DEFAULT_ADS_FOV_FACTOR
}

fn default_ads_time_s() -> f64 {
    DEFAULT_ADS_TIME_S
}

fn is_default_ads_fov_factor(v: &f64) -> bool {
    *v == DEFAULT_ADS_FOV_FACTOR
}

fn is_default_ads_time_s(v: &f64) -> bool {
    *v == DEFAULT_ADS_TIME_S
}

/// The defaults of the `held_model`'s sights fields as JSON (`ads_offset_m`,
/// `ads_fov_factor`, `ads_time_s`, `attachments`, `reticle`, `muzzle_m`, `zero_m`): a document leaves them out while they are at their
/// defaults, so a listing of the component's defaults adds them from here.
pub fn held_model_sights_defaults() -> serde_json::Value {
    serde_json::json!({
        "ads_offset_m": null,
        "ads_fov_factor": DEFAULT_ADS_FOV_FACTOR,
        "ads_time_s": DEFAULT_ADS_TIME_S,
        "attachments": [],
        "reticle": null,
        "muzzle_m": null,
        "zero_m": null,
    })
}

impl HeldModel {
    /// A held model of `asset` with every other field at its default.
    pub fn of(asset: impl Into<String>) -> HeldModel {
        HeldModel {
            asset: asset.into(),
            offset_m: DEFAULT_HELD_OFFSET_M,
            scale: 1.0,
            ads_offset_m: None,
            ads_fov_factor: DEFAULT_ADS_FOV_FACTOR,
            ads_time_s: DEFAULT_ADS_TIME_S,
            attachments: Vec::new(),
            reticle: None,
            muzzle_m: None,
            zero_m: None,
        }
    }

    /// The zeroing distance, metres: `zero_m`, else [`DEFAULT_ZERO_M`].
    pub fn zero_distance_m(&self) -> f64 {
        self.zero_m.unwrap_or(DEFAULT_ZERO_M)
    }

    /// The barrel's mouth `ads` of the way to the sights, from the eye, in the aim's axes
    /// `[right, up, back]` (metres): the model's offset ([`HeldModel::offset_at`]) plus the
    /// muzzle point times the scale. `None` without `muzzle_m` or when a number is not finite.
    pub fn muzzle_local(&self, ads: f64) -> Option<[f64; 3]> {
        let [mr, mu, mf] = self.muzzle_m?;
        let [r, u, f] = self.offset_at(ads);
        let s = self.scale;
        let local = [r + mr * s, u + mu * s, -f - mf * s];
        local.iter().all(|v| v.is_finite()).then_some(local)
    }

    /// The barrel's mouth in the world for an eye at `eye` and an aim of `yaw_deg` and
    /// `pitch_deg` (recoil and sway included), `ads` of the way to the sights: exactly where
    /// the view model carries it. `None` without `muzzle_m` or when a number is not finite.
    pub fn muzzle_point(
        &self,
        eye: [f64; 3],
        yaw_deg: f64,
        pitch_deg: f64,
        ads: f64,
    ) -> Option<[f64; 3]> {
        let local = self.muzzle_local(ads)?;
        let v = AimAxes::of(yaw_deg, pitch_deg).vector(local);
        let out = [eye[0] + v[0], eye[1] + v[1], eye[2] + v[2]];
        out.iter().all(|c| c.is_finite()).then_some(out)
    }

    /// The assets it draws: its own, then its attachments'.
    pub fn assets(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.asset).chain(self.attachments.iter().map(|a| &a.asset))
    }

    /// Where the origin sits `ads` of the way (0 hip to 1 at the sights) from `offset_m` to
    /// `ads_offset_m`.
    pub fn offset_at(&self, ads: f64) -> [f64; 3] {
        let t = if ads.is_finite() {
            ads.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let to = self.ads_offset_m.unwrap_or(self.offset_m);
        let mut out = self.offset_m;
        // Weighted so that 0 and 1 give the two offsets exactly.
        for (o, b) in out.iter_mut().zip(to) {
            *o = *o * (1.0 - t) + b * t;
        }
        out
    }

    /// The share of the field of view `ads` of the way to the sights.
    pub fn fov_factor_at(&self, ads: f64) -> f64 {
        let t = if ads.is_finite() {
            ads.clamp(0.0, 1.0)
        } else {
            0.0
        };
        (1.0 - t) + self.ads_fov_factor * t
    }
}

impl HeldModel {
    pub(crate) fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        if !is_model_path(&self.asset) {
            let shown: String = self.asset.chars().take(MAX_ASSET_PATH_CHARS + 8).collect();
            return Err(ctx.invalid(
                "held_model.asset",
                format!("{shown:?}"),
                format!(
                    "it must be a path inside the pack's assets/ folder ending in .glb or \
                     .gltf, 1 to {MAX_ASSET_PATH_CHARS} ASCII letters, digits, `-`, `_`, `.` \
                     and `/`, without `..`, such as \"models/rifle.glb\""
                ),
            ));
        }
        for (axis, v) in ["right", "up", "forward"].iter().zip(self.offset_m) {
            ctx.range(
                format!("held_model.offset_m ({axis})"),
                v,
                -HELD_OFFSET_MAX_M,
                HELD_OFFSET_MAX_M,
                " metres",
            )?;
        }
        ctx.range("held_model.scale", self.scale, SCALE_MIN, SCALE_MAX, "")?;
        if let Some(ads) = self.ads_offset_m {
            for (axis, v) in ["right", "up", "forward"].iter().zip(ads) {
                ctx.range(
                    format!("held_model.ads_offset_m ({axis})"),
                    v,
                    -HELD_OFFSET_MAX_M,
                    HELD_OFFSET_MAX_M,
                    " metres",
                )?;
            }
        }
        ctx.range(
            "held_model.ads_fov_factor",
            self.ads_fov_factor,
            ADS_FOV_FACTOR_MIN,
            1.0,
            "",
        )?;
        ctx.range(
            "held_model.ads_time_s",
            self.ads_time_s,
            0.0,
            ADS_TIME_MAX_S,
            " seconds",
        )?;
        if self.attachments.len() > HELD_ATTACHMENTS_MAX {
            return Err(ctx.invalid(
                "held_model.attachments",
                format!("{} attachments", self.attachments.len()),
                format!("at most {HELD_ATTACHMENTS_MAX}"),
            ));
        }
        for (i, a) in self.attachments.iter().enumerate() {
            if !is_model_path(&a.asset) {
                let shown: String = a.asset.chars().take(MAX_ASSET_PATH_CHARS + 8).collect();
                return Err(ctx.invalid(
                    format!("held_model.attachments[{i}].asset"),
                    format!("{shown:?}"),
                    "a path inside the pack's assets/ folder ending in .glb or .gltf, such as \
                     \"models/optic.glb\"",
                ));
            }
            for (axis, v) in ["x", "y", "z"].iter().zip(a.offset_m) {
                ctx.range(
                    format!("held_model.attachments[{i}].offset_m ({axis})"),
                    v,
                    -HELD_OFFSET_MAX_M,
                    HELD_OFFSET_MAX_M,
                    " metres",
                )?;
            }
        }
        if let Some(r) = &self.reticle {
            for (channel, v) in ["red", "green", "blue"].iter().zip(r.color_srgb) {
                ctx.range(
                    format!("held_model.reticle.color_srgb ({channel})"),
                    v,
                    0.0,
                    1.0,
                    "",
                )?;
            }
            ctx.range(
                "held_model.reticle.radius_mrad",
                r.radius_mrad,
                RETICLE_RADIUS_MIN_MRAD,
                RETICLE_RADIUS_MAX_MRAD,
                " mrad",
            )?;
            ctx.range("held_model.reticle.brightness", r.brightness, 0.0, 1.0, "")?;
        }
        if let Some(muzzle) = self.muzzle_m {
            for (axis, v) in ["right", "up", "forward"].iter().zip(muzzle) {
                let name = format!("held_model.muzzle_m ({axis})");
                ctx.range(name.clone(), v, -MUZZLE_MAX_M, MUZZLE_MAX_M, " metres")?;
                ctx.range(
                    format!("{name} times held_model.scale"),
                    v * self.scale,
                    -MUZZLE_MAX_M,
                    MUZZLE_MAX_M,
                    " metres",
                )?;
            }
        }
        if let Some(zero) = self.zero_m {
            ctx.range("held_model.zero_m", zero, ZERO_MIN_M, ZERO_MAX_M, " metres")?;
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
