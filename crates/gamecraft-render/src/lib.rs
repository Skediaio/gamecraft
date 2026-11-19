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
pub mod effects;
mod image;
mod light;
pub mod mesh;
mod plants;
mod renderer;

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PictureStats {
    /// Triangles submitted.
    pub triangles: u64,
    /// Draw calls.
    pub draws: u64,
    /// Instances drawn in the camera's view.
    pub instances: u64,
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
