//! Deterministic maths for GameCraft (layer L0).
//!
//! Everything here gives bit-identical results on every platform, natively and in wasm:
//!
//! - [`Portable`]: transcendental functions (`psin`, `pcos`, `ppowf`, ...) computed by the
//!   pure-Rust `libm` crate instead of the platform's maths library.
//! - [`Rng`]: a small seeded generator (SplitMix64) with forkable, labelled streams.
//! - [`StateHasher`]: the 64-bit FNV-1a hash used as the simulation's state-hash oracle.
//! - [`noise`]: smooth 2D value noise and fractal Brownian motion.
//! - Angle helpers: [`deg_to_rad`], [`rad_to_deg`], [`wrap_deg`], [`wrap_deg_signed`],
//!   [`yaw_forward`].
//!
//! `glam` is re-exported, built with its `libm` feature so its own sines and cosines are
//! portable too. Conventions: metres, Y up, north is -Z, east is +X; a yaw turns
//! counter-clockwise about +Y seen from above and yaw 0 faces -Z.

mod angle;
mod hash;
pub mod noise;
mod portable;
mod rng;

#[cfg(test)]
mod tests;

pub use angle::{deg_to_rad, rad_to_deg, wrap_deg, wrap_deg_signed, yaw_forward};
pub use glam;
pub use glam::{DMat4, DQuat, DVec2, DVec3};
pub use hash::StateHasher;
pub use portable::Portable;
pub use rng::Rng;
