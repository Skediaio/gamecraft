//! The renderer adapter of GameCraft (layer L4): a world in, a picture out.
//!
//! [`WorldRenderer`] turns a [`gamecraft_world::World`] into calls on VisionCraft's wgpu
//! renderer and reads offscreen pictures back:
//!
//! - **Terrain**: the height field as chunks of at most [`mesh::TERRAIN_CHUNK_CELLS`] cells per
//!   side, triangulated exactly as `HeightField::height_at` interpolates, with three plain colour
//!   layers (grass, earth, rock) blended by slope, and an endless ground plane beyond it.
//! - **Shapes**: one unit mesh per kind, one draw item per (kind, colour) with an instance per
//!   entity.
//! - **Plants**: VisionCraft's plant generator at its middle level of detail, one generation per
//!   distinct (species, seed, growth), textured, alpha-cut and moving in the wind.
//! - **Models**: glTF assets of a pack (`model` components), handed over by the host
//!   ([`WorldRenderer::provide_asset`]), read with their PBR materials and PNG or JPEG textures
//!   ([`model`]), uploaded once per asset and drawn as one item with an instance per entity. A
//!   model that cannot be drawn is a small magenta box with a problem line. A model with
//!   **levels of detail** (`_LOD<n>` names or `MSFT_lod` in its file, `gamecraft_project::lod`)
//!   draws its finest level near the camera and coarser ones farther away, and is not drawn
//!   beyond a draw distance; both come from its size unless its file or its component says
//!   ([`WorldRenderer::sync`]). What each model drew, per level, is counted again on the CPU by
//!   the renderer's own rule (`culling`) and comes with every picture
//!   ([`PictureStats::models`]).
//! - **Far levels of detail** ([`hlod`]): the static shapes and models of a cell of the
//!   ground (128 m) merged into one mesh per material, each model at its coarsest level,
//!   drawn where the cell is farther than its own items and nothing near; a cell's merged
//!   mesh is built again only when its content changes. What they drew is in every picture's
//!   stats ([`PictureStats::hlod`]).
//! - **Roads**: every `road` draped on the terrain's triangles a few centimetres above them
//!   ([`roads`]), one item per terrain chunk and colour, rough and lit like the ground under
//!   it; where roads overlap the higher tier lies on top, so no two surfaces are coplanar.
//! - **Characters**: a body (cylinder and head) with a small dark visor at eye height on the
//!   side the character faces.
//! - **Sky and light**: the physical sky, the sun from the environment, clouds, mist, rain, wind
//!   and an exposure that follows the light, so day, dusk and night are all readable.
//!
//! - **Combat**: tracers (overlay streaks), impact puffs (small emissive spheres), downed
//!   characters lying on their side and a first-person crosshair; the caller hands over plain
//!   data ([`Effects`]), never the simulation.
//!
//! No VisionCraft type appears in this crate's public API: everything that touches the renderer
//! lives in `renderer.rs`, everything that touches the plant generator in `plants.rs`. The
//! GPU-free half (meshes, cameras, PNG encoding, the light and wind mappings) is public and
//! tested without a GPU.
//!
//! Conventions: metres, Y up, north is -Z and east is +X; an entity's position is its base.
//! Positions reach the GPU as `f32` relative to the world origin.

// The renderer adapter is outside the deterministic core (layers L0 to L3): its camera, mesh
// and exposure maths is f32 and f64 at the GPU boundary and never reaches a simulation result,
// so the platform's sines and powers are fine here.
#![allow(clippy::disallowed_methods)]

mod blit;
mod camera;
mod culling;
pub mod effects;
pub mod hlod;
mod image;
mod light;
pub mod mesh;
pub mod model;
mod plants;
mod renderer;
pub mod roads;

#[cfg(test)]
mod tests;

use std::fmt;

pub use camera::{Camera, FOV_MAX_DEG, FOV_MIN_DEG};
pub use effects::{Effects, Impact, Tracer};
pub use image::encode_png;
pub use light::wind_towards;
pub use renderer::WorldRenderer;
#[cfg(not(target_arch = "wasm32"))]
pub use renderer::headless_gpu;

/// Smallest picture side, pixels.
pub const PICTURE_MIN_SIDE: u32 = 16;
/// Widest picture, pixels.
pub const PICTURE_MAX_WIDTH: u32 = 3840;
/// Tallest picture, pixels.
pub const PICTURE_MAX_HEIGHT: u32 = 2160;

/// The effects of live frames ([`WorldRenderer::set_quality`]): the renderer's presets of
/// shadows, ambient occlusion, anti-aliasing and render scale. Pictures are always drawn at the
/// highest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    /// For slow or integrated GPUs: fewer effects, a lower render scale.
    Low,
    /// The default: ambient occlusion, bloom, three quarters of the size upscaled.
    #[default]
    Medium,
    /// Full size, 4x MSAA, soft and contact shadows.
    High,
}

impl Quality {
    /// Every quality, lowest first.
    pub const ALL: [Quality; 3] = [Quality::Low, Quality::Medium, Quality::High];

    /// Its name: `low`, `medium` or `high`.
    pub fn name(self) -> &'static str {
        match self {
            Quality::Low => "low",
            Quality::Medium => "medium",
            Quality::High => "high",
        }
    }

    /// The quality of a name; the error names the value and the choices.
    pub fn parse(name: &str) -> Result<Quality, String> {
        Quality::ALL
            .into_iter()
            .find(|q| q.name() == name)
            .ok_or_else(|| format!("quality `{name}` is not known; use one of low, medium, high"))
    }
}

/// What to draw: the size, the camera and the animation clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PictureRequest {
    /// Width in pixels, 16 to 3840.
    pub width: u32,
    /// Height in pixels, 16 to 2160.
    pub height: u32,
    /// Where the picture is taken from.
    pub camera: Camera,
    /// Animation clock in seconds (the wind's sway); any finite value.
    pub time_s: f64,
}

impl PictureRequest {
    /// Checks the size, the clock and the camera; the error names the field, the value and
    /// what is allowed.
    pub fn validate(&self) -> Result<(), RenderError> {
        for (name, value, max) in [
            ("width", self.width, PICTURE_MAX_WIDTH),
            ("height", self.height, PICTURE_MAX_HEIGHT),
        ] {
            if !(PICTURE_MIN_SIDE..=max).contains(&value) {
                return Err(RenderError::BadRequest(format!(
                    "request.{name} {value} is outside {PICTURE_MIN_SIDE} to {max} pixels; \
                     use a value in that range"
                )));
            }
        }
        if !self.time_s.is_finite() {
            return Err(RenderError::BadRequest(format!(
                "request.time_s {} is not a finite number of seconds; use for example 0",
                self.time_s
            )));
        }
        self.camera.validate()
    }
}

/// A rendered picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// sRGB-encoded RGBA8, rows top to bottom, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// What the frame cost.
    pub stats: PictureStats,
    /// The problems of the last `sync` (an unknown plant species, ...), as sentences.
    pub problems: Vec<String>,
}

/// Counts of the frame a picture was drawn with, over all passes (shadows included).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PictureStats {
    /// Triangles submitted.
    pub triangles: u64,
    /// Draw calls.
    pub draws: u64,
    /// Instances drawn in the camera's view.
    pub instances: u64,
    /// Triangles submitted in the view pass alone (not the shadows, the reflection or the
    /// post-processing).
    pub forward_triangles: u64,
    /// What each model drew in the view: one row per model item (an asset as entities with
    /// the same distances and scale show it), in order of asset path, then of the first
    /// entity that shows it. Empty without models.
    pub models: Vec<ModelStats>,
    /// What the far levels of cells drew.
    pub hlod: HlodStats,
}

/// What the far levels of detail ([`hlod`]) drew in the view of a picture: the cells with a
/// merged mesh, and how many of them drew (their box farther than the cell's take-over
/// distance, within the view) with the triangles they submitted in the view pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HlodStats {
    /// Cells with a merged mesh in the scene.
    pub cells: usize,
    /// Cells whose merged mesh drew.
    pub drawn: usize,
    /// Triangles the merged meshes submitted in the view pass.
    pub triangles: u64,
}

/// What one model (an asset as entities with the same distances and scale show it) drew in
/// the view of a frame, counted by the renderer's settled rule on the CPU (`culling`) at the
/// quality's share of the distances: exact for a picture, which starts settled. A live frame
/// may differ where a cell lies within about a metre of a threshold, which the renderer keeps
/// at the level of the frame before.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelStats {
    /// The asset, by its path inside a pack's `assets/` folder.
    pub asset: String,
    /// Instances in the scene.
    pub instances: usize,
    /// Instances drawn in the view: after the levels and the culling.
    pub drawn: usize,
    /// Instances not drawn: outside the view or beyond the draw distance.
    pub culled: usize,
    /// Per level that draws, finest first: the instances drawn at it and the triangles they
    /// submit in the view pass.
    pub levels: Vec<LevelCount>,
    /// Where each level ends, metres from the camera; the last is the draw distance, infinite
    /// for a model that is never culled (a character the camera follows). Where a far level of
    /// the cell takes over ([`hlod`]), the last end is a cell's diagonal past that, so that no
    /// cluster leaves a hole: [`ModelStats::own_until_m`] has the model's own.
    pub until_m: Vec<f64>,
    /// Where each level ends by the component, the file and the rule alone, metres: the
    /// distances without the far level's margin (the same as `until_m` for a character, or
    /// where no cell has a merged mesh).
    pub own_until_m: Vec<f64>,
}

/// Counts of the last live frame ([`WorldRenderer::draw`]); a picture leaves them as they
/// were.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Instances drawn in the camera's view: after the levels and the culling.
    pub instances: u64,
    /// Triangles the view pass submitted (not the shadows, the reflection or the
    /// post-processing).
    pub forward_triangles: u64,
}

/// What one level of detail of a model drew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LevelCount {
    /// The level of the model's file (its number in `asset.list`'s `lods`); a level without
    /// geometry is left out, so the numbers may skip one.
    pub level: usize,
    /// Instances drawn at this level.
    pub instances: usize,
    /// Triangles they submit in the view pass.
    pub triangles: u64,
}

/// Why a picture could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    /// No GPU adapter or device could be had; the text says why.
    NoGpu(String),
    /// The request is out of range; the text names the field, the value and what is allowed.
    BadRequest(String),
    /// The GPU failed to draw or read back the picture; the text says how.
    Failed(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::NoGpu(why) => write!(
                f,
                "No GPU is available for pictures ({why}). Install a GPU driver or a software \
                 Vulkan adapter such as lavapipe (mesa-vulkan-drivers)."
            ),
            RenderError::BadRequest(why) => write!(f, "Bad picture request: {why}."),
            RenderError::Failed(why) => write!(f, "The picture could not be drawn: {why}."),
        }
    }
}

impl std::error::Error for RenderError {}
