//! The GameCraft engine (layer L3): the [`Session`] and its command registry.
//!
//! Every action in GameCraft is a command: an id (`entity.add`) with JSON parameters, described
//! by a [`CommandSpec`] (parameters with types, units, defaults and allowed values, a result, a
//! runnable example and a [`Scope`]). The CLI, the JSON-lines server and the MCP server only
//! dispatch commands by id through [`Session::run`] and print the specs of
//! [`Session::specs`].
//!
//! # The session's two states
//!
//! - **Editing.** [`Session::document`] is the world being edited. A World command runs on a
//!   copy of it; when it succeeds the copy is validated and becomes the document, and the
//!   previous document is one undo step ([`Session::undo`], [`Session::redo`], at most
//!   [`UNDO_LIMIT`] steps). [`Session::transaction`] makes several commands one step, or none.
//! - **Playing.** `sim.start` runs a [`gamecraft_sim::Sim`] on a copy of the document
//!   ([`Session::live`]). World commands then act on the running world, with the same checks,
//!   and leave the document untouched (AGENTS.md rule 6: the game master's hand); there is no
//!   undo meanwhile. `sim.stop` throws the running world away, or with `keep` makes it the
//!   document as one undo step.
//!
//! [`Session::current_world`] is whichever of the two is shown, [`Session::view_revision`]
//! changes whenever it does, and [`Session::current_heights`] is its derived height field.
//!
//! Parameters are checked against the spec before a handler runs; every error is a
//! [`CommandError`] whose sentence starts with the command id. No input makes a command panic
//! (`tests/panic_hunt.rs` runs every command of the registry with hostile parameters).
//!
//! # Where a new command goes
//!
//! Commands live in one module per area, each with its own `SPECS` list of
//! `Registration { spec, handler }`: `world_cmds.rs`, `entity_cmds.rs`, `play_cmds.rs`,
//! `inspect_cmds.rs`, `scenario_cmds.rs`, `aim_cmds.rs`, .... A handler is a `fn` of its scope: a World handler edits a copy of the
//! world (`fn(&mut Edit, Params)`), a Query handler reads the session (`fn(&Session, Params)`),
//! a Play handler works on the play state (`fn(&mut Session, Params)`). Add the command to its
//! module's `SPECS`; a new module is one more line in `MODULES` in `session.rs`. Its spec's
//! `example` must run on [`example_world`] (`tests/examples.rs` checks every one), and the
//! panic hunt picks the new command up by itself.

mod aim_cmds;
mod catalog_cmds;
mod combat_cmds;
mod entity_cmds;
mod error;
mod example;
mod find_cmds;
mod geometry_cmds;
mod inspect_cmds;
mod params;
pub mod plants;
mod play_cmds;
mod scatter_cmds;
mod scenario_cmds;
mod session;
pub mod spec;
mod triggers;
mod world_cmds;

pub use error::CommandError;
pub use example::{EXAMPLE_SPECIES, example_world};
pub use inspect_cmds::MAX_INSPECT_LIMIT;
pub use session::{Session, UNDO_LIMIT};
pub use spec::{CommandSpec, ParamSpec, ParamType, Scope, param, spec};
