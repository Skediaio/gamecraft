//! The GameCraft window (layer L5): a person walks a character through a world, natively and
//! in the browser on WebGPU, with plain winit on wgpu (no UI toolkit; the interface is M10).
//!
//! - [`game::Game`] is the game without a window: the engine's session (every change is a
//!   command: `sim.start`, `sim.input`, `sim.step`), a fixed simulation step with at most a
//!   few ticks per frame, the picture interpolated between the last two ticks, the cameras and
//!   what the scenario tells (messages, objectives).
//! - [`view::GameView`] draws a game into a colour target with `gamecraft-render`: the
//!   window's surface, or a texture in the offscreen test.
//! - `app` is the winit window around them, natively and in the browser; natively it can read
//!   the JSON-lines control protocol on stdin (`--stdin`) so an agent drives the window a
//!   person is watching.
//!
//! The pure parts ([`input`], [`clock`], [`pose`], [`rig`], [`notices`], [`args`]) are tested
//! without a GPU.

// The window is outside the deterministic core (layers L0 to L3): its camera and interpolation
// maths is for display only and never reaches a simulation result, so the platform's sines are
// fine here.
#![allow(clippy::disallowed_methods)]

pub mod app;
pub mod args;
pub mod clock;
pub mod game;
pub mod input;
pub mod notices;
pub mod pose;
pub mod rig;
pub mod view;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(test)]
mod tests;
