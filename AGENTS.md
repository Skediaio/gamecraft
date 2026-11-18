# AGENTS.md: guide for AI agents and contributors

GameCraft is an open-source game engine built with and for AI agents. It makes three promises:

- **Anyone can build a game in it.** Every action is a command with JSON parameters, so the same
  engine is driven from a window, the CLI, a JSON control channel or an MCP server. A person says
  what they want; an agent builds the world, plays it, looks at it and tests it. No part of making
  a game needs a specialist's tool or a person at the mouse.
- **Everything can be changed.** A game is data and scripts layered over the engine, and a mod is
  one more layer over the game.
- **A running game can be steered.** Scenarios are part of the world, and a game master, a
  person or an agent, changes the world while it is being played: places units, sets objectives,
  turns the weather, starts the mayhem. The commands that build a world are the commands that
  direct it.

Its aim is the class of engine behind large outdoor sandbox games: big terrains with weather and
vegetation, a world made of entities, components and prefabs kept as data, an authoritative
simulation that many players share, and a workbench to build all of it. In GameCraft that
workbench is not a set of windows but a set of commands.

It is written in **Rust only**, with its own world model, simulation and command layer: no Bevy,
Godot, Unity or other engine, in any form. It draws with the wgpu renderer of
[VisionCraft](https://github.com/Skediaio/visioncraft) and grows its plants with VisionCraft's
generator. It follows the principles of [3DCraft](https://github.com/johancoppens/3dcraft) and
VisionCraft; where this file is silent, their `AGENTS.md` shows the intent.

The product name is written **GameCraft** in prose. Machine names are lowercase `gamecraft`:
crates (`gamecraft-*`), binaries (`gamecraft`, `gamecraft-cli`), environment variables
(`GAMECRAFT_*`). The native file extension is `.gamecraft`.

**Status (2026-11-18): M0, the foundation, is done; nothing is released.** One path works end
to end: a command edits the world document, the simulation
steps it deterministically and answers with a state hash, and a picture of it is drawn with
VisionCraft's renderer, all from `gamecraft-cli` and over MCP. What is done and what is open:
`docs/roadmap.md`, starting at **Current focus**. Everything else in this file and in `docs/` is
the target design. Commands and paths marked *(planned)* do not exist until the milestone that
creates them; when you build one, remove its marker here and in the docs. Licence: MIT.

Read this file first, then `docs/`.

## Who does what

- **The main session orchestrates.** It plans, writes the shared scaffolding (the root
  `Cargo.toml`, this file, the docs' maps, the interfaces between crates), briefs agents, checks
  their results itself and reports. It is responsible for the whole.
- **Agents do the bulk work,** each in the directories it owns: the strongest model for work
  that needs design or reasoning (a crate's logic, a protocol, rendering, physics), a faster one
  for work that needs little of it (a mechanical port, CI files, a survey).
- Only the orchestrator edits shared files. An agent that needs a change in one says so in its
  report.
- An agent's report is a claim, not evidence: the orchestrator runs the checks and looks at the
  pictures itself.

## 1. Orientation

| Read | Why |
|---|---|
| `docs/architecture.md` | The map: invariants, crates and layers, the world document, the pipelines, how determinism is kept |
| `docs/roadmap.md` | Milestones, **current focus**, definition of done per milestone |
| `docs/decisions.md` | Decisions taken, and the open ones only the developer can settle |
| `docs/development.md` | Build, test, run; the VisionCraft dependency and how to move it |
| `docs/control-protocol.md` | The JSON-lines protocol and MCP: how agents drive the engine |
| `docs/agent.md` | The guide for an agent that *uses* GameCraft to build and play a world, with a worked example |

## 2. Workspace map

```text
crates/
  gamecraft-math         L0  deterministic maths: portable sines and powers, seeded randomness, noise, the state hash
  gamecraft-world        L1  the world document: terrain, environment, entities, components (pure data, serde)
  gamecraft-sim          L2  the simulation: fixed step, input as data, character movement, collisions, state hash
  gamecraft-engine       L3  Session and command registry: every action is a command; undo; edit and play
  gamecraft-render       L4  draws a world with VisionCraft's renderer and plants; offscreen pictures. No UI
  gamecraft-automation   L4  JSON-lines protocol and MCP server over one host (session, files, pictures)
apps/
  gamecraft-cli          L5  headless CLI: commands, run, batch, serve, mcp, render
  gamecraft              L5  the window, native and in the browser (planned, M1)
xtask/                       cargo xtask layers | wasm | ci
```

**Layering is enforced** by `cargo xtask layers`. A crate may depend only on lower layers;
`gamecraft-render` and `gamecraft-automation` share a layer and do not depend on each other (a
host hands pictures to the automation through a trait). Nothing below L5 opens a window, and
nothing below L4 touches a GPU, a file or a clock. A new crate is registered in
`xtask/src/layers.rs`.

## 3. Golden rules

### Never crash (outranks feature work)

A game engine runs other people's worlds, mods and commands, for hours, with players in it. A
malformed file, a bad command or MCP parameter, a NaN in a position, an entity removed while
something refers to it or a full disk must produce an error the user or agent can act on, never
a panic. Don't ship a feature by adding a panic path; fix a crash before building on top of it.

- **Non-test code never panics.** No `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`
  or `unimplemented!`. Return the crate's error type and propagate with `?`; use `ok_or(..)?`,
  `let .. else { return Err(..) }`, `if let`, or `unwrap_or*` where a fallback is truly correct.
  Unfinished features return an "unsupported" error. The only exception is a provably infallible
  literal: `#[allow(clippy::expect_used)]` plus `.expect("why it can't fail")`.
- **No `unsafe`.** The workspace sets `unsafe_code = "forbid"`.
- **Input-derived numbers are hostile.** Use `get()` rather than `[i]` for indices from files,
  params or arithmetic on them; `checked_*`/`saturating_*` for sizes and counts; cap allocations
  sized by input (a terrain of a billion cells is an error, not an out-of-memory abort).
- **Floating point is hostile too.** Positions, directions and time steps meet NaN, infinity
  and zero-length vectors. Normalising, dividing, `sqrt`, `acos` and casts to integers are
  guarded. The simulation never writes a non-finite number into the world.
- **Bound recursion and iteration.** Depth limits for hierarchies (a prefab that inherits from
  itself in a bad file), iteration caps on every solver and search.
- **Last-resort guard.** The automation servers and the app shell catch an escaped panic around
  command dispatch and file reading, report it as an error and keep the world. It is a safety
  net, not a licence to panic.
- **Prove it.** Every crash fix comes with a small regression test that panicked before the fix.
- **Enforced by clippy.** The workspace denies `unwrap_used`, `expect_used`, `panic`,
  `unimplemented`, `todo`, `unreachable` and `indexing_slicing`; `clippy.toml` allows them in
  tests only. Every crate opts in with `[lints] workspace = true`; `cargo xtask layers` fails for
  a crate that does not.

### The rules

1. **Everything is a command.** New behaviour a person or agent can cause is a command in the
   engine: an id (`entity.add`), a label, parameters with types, units, defaults and allowed
   values, a summary, a result and a runnable example, plus tests. The window, the CLI, the
   control channel and MCP all dispatch commands by id. That holds for building a world, for
   playing it (input is a command) and for looking at it (a picture is a command). A feature
   that can only be used with a mouse, a keyboard or a gamepad is not finished.
2. **The world document is the source of truth, and it holds recipes.** A world is terrain,
   environment, entities with components, and generators with their seeds. What follows from
   them (height fields, meshes, placed forests, navigation data) is *derived*, rebuilt
   deterministically and thrown away at will. Never store a result that cannot be recomputed
   from the document, and never let a command change derived data behind the document's back.
   Documents are text, with a stable order, so they diff and merge.
3. **The simulation is deterministic.** The same world and the same inputs give bit-identical
   state on every run, thread count and platform, natively and in the browser: a fixed time
   step, ordered iteration (no `HashMap` order in results), `f64` maths with transcendental
   functions through `gamecraft_math::Portable` (clippy refuses `f64::sin` and its kin), seeded
   randomness only, parallel work merged in a fixed order. The state hash is the oracle: a
   replay that ends on another hash is a bug.
4. **Nothing is hidden.** Whatever the engine knows, an agent can ask for as JSON:
   `world.inspect`, `entity.get`, `sim.state`, the queries (`query.ground_height`,
   `query.raycast`, …). State that can only be seen on a screen is a bug. Errors are sentences
   an agent can act on: they name the command, the parameter and the nearest valid value.
5. **An agent can play.** Input is data, not devices: a character moves because of an input
   record, whoever wrote it (a keyboard, a script, an agent, the network). Any character can be
   driven that way, the simulation runs headless and faster than real time, and what a player
   would see is available as state and as a picture.
6. **A running world takes the same commands.** Whatever a command can do to the document, it
   can do to a world that is being played: that is what a game master is. While a simulation
   runs, world commands act on the running world, and the document is untouched until the
   session is told to keep the result. A command that works only while nobody plays needs a
   reason, written in its summary.
7. **Stable references.** Commands refer to entities by id or by name, never by an index into
   a list. An id survives save, load, undo and replication.
8. **Units are not assumed.** Metres, Y up, the ground plane is x and z, angles in degrees at
   the command surface, seconds; world positions are `f64` and `f32` exists only at the GPU
   boundary, so a world can be tens of kilometres wide. Sizes, tolerances and step counts are
   data of the document or constants with a name, never magic numbers in a loop.
9. **Layers, dependencies only downwards** (§2). The core is headless: no GPU, no window, no
   files, no clock, no threads it does not join.
10. **Never break wasm.** L0 to L4 must `cargo check --target wasm32-unknown-unknown` (run
   `cargo xtask wasm`). Browser first (WebGPU), desktop second: a milestone with something to
   see is done when it runs in Chromium on WebGPU.
11. **Tests are the gate, and agents close the loop.** Every change comes with tests. Prefer
    oracles a machine can check: closed-form motion (a fall, a walk at a known speed), invariants
    (never below the ground, never inside a wall), state-hash equality between runs, round trips
    through the file format, every command's example. Never weaken or delete a failing test to
    get green; fix the code, or record the limitation and return an "unsupported" error. When
    your code and your test disagree, do not assume the test is the one that is wrong.
12. **Never panic on input.** A command's handler and anything it calls returns `Err`, never
    panics, for *any* parameters and world state. The `panic_hunt` test of `gamecraft-engine`
    runs every command of the registry with adversarial parameters and must stay green; a new
    command is picked up automatically from its spec.
13. **VisionCraft stays behind an adapter.** Only `gamecraft-render` depends on
    `visioncraft-render`, and only `gamecraft-render` and `gamecraft-engine` (its `plants`
    module) depend on `visioncraft-flora`. No VisionCraft type appears in a public signature of
    any other crate. VisionCraft is under active development: a new rev may change its API, and
    then one adapter changes, not the engine. Same stack and versions where we overlap (wgpu,
    glam, WGSL). A renderer feature the game needs is built in VisionCraft, not copied here.
14. **Clean room.** Work only from public specifications, papers, textbooks and openly licensed
    code. Nothing from closed engines or games: no code, shaders, assets, file formats, UI
    designs or names. Ported open code keeps its licence and gets a row in `ATTRIBUTION.md` in
    the same change.
15. **Assets are CC0 or CC-BY only,** each with licence, author and source recorded. Never
    commit a font or an asset without them.
16. **Mechanisms in the engine, things in packs.** GameCraft is the engine and must become
    complete; a game built on it is a list of packs (content and rules for a family of games)
    plus its own data and scripts. Ask of anything new: does it name a *thing in a game world*
    (a weapon, a soldier, a block) or a *capability* (to collide, to hear, to replicate, to
    trigger)? A capability belongs here, in its general form; a thing belongs in a pack. A
    mechanism moves from a pack into the engine only when a second, unrelated pack needs it. The
    examples in this repository exist to test the platform (`docs/decisions.md`).
17. **Let the developer watch.** Anything visual is verified by looking at a picture: render
    offscreen with `view.render`, then actually read the PNG. Leave the evidence where the
    developer can see it and say what you are about to do before a long run.
18. **Performance is a feature.** Count work, not wall-clock time, where a test can: ticks per
    second of a headless world, entities stepped, triangles and draws of a picture. Record
    before and after when you change a hot path.

## 4. Picking work

1. Read `docs/roadmap.md` → **Current focus**. Pick from the top unless the developer asked for
   something else.
2. Check `docs/decisions.md` → *Open*. If your work depends on one, do not choose silently: an
   agent asks the orchestrator; the orchestrator decides how something is built and writes the
   decision down with its reason, and takes a fork in the architecture or the product to the
   developer.

When you finish a roadmap item, update its status in `docs/roadmap.md` with the date. The map
(`docs/architecture.md`) changes only when the structure changes.

## 5. Before you finish a task

```sh
cargo fmt --all
cargo test -p <crates you touched>
cargo clippy -p <crates you touched> --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm            # if you touched L0 to L4
cargo test -p gamecraft-engine --test panic_hunt   # if you added or changed commands
```

`cargo xtask ci` runs fmt, clippy, the tests, layers and wasm in one go; it is the gate, and CI
runs the same. While iterating, run the tests of the crates you touched and keep the full gate
for the end of a piece of work. Tests that need a GPU skip themselves, saying so, when no adapter
exists; with `GAMECRAFT_REQUIRE_GPU=1` a missing adapter is a failure.

A new command comes with: its spec with a runnable example, a test of what it does, a test that
bad parameters give an error that names the parameter, and a line in `docs/control-protocol.md`.

## 6. Parallel agents

- Edit only the directories you own. Shared files (the root `Cargo.toml`, `AGENTS.md`, the
  docs' maps, a crate's command registration list) get small, surgical edits by their owner.
- Put new commands in a **new module** (`gamecraft-engine/src/<area>_cmds.rs` with its own
  `SPECS`) rather than growing a shared file.
- Keep every `Cargo.toml` valid at all times: the `crates/*` glob means one broken manifest
  breaks everyone's build. A dependency that is not in `[workspace.dependencies]` is asked for,
  not added.
- Use a build directory of your own when others build at the same time, so nobody waits on
  Cargo's lock; delete it when you are done.
- If someone else's in-progress edit breaks the build, wait and retry; don't fix their files.
- Agents do not commit, push or start other agents. The orchestrator commits finished pieces;
  nothing is pushed without the developer's word.

## 7. Where things are tracked

- `docs/roadmap.md`: milestones, status and the current focus.
- `docs/decisions.md`: decisions taken and open.
- `ATTRIBUTION.md`: where ported code and methods come from.
