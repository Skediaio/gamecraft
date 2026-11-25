# Architecture

The map of GameCraft: what must always hold, which crate does what, what the data is and how it
flows. It changes only when the structure changes. Status of each part: `roadmap.md`.

## Invariants

1. **One way in.** Every change to a world and every step of a game goes through a command of
   the `Session`. The window, the CLI, the control channel and MCP are four doors to the same
   room.
2. **Document, then derived.** The world document holds what was decided (a terrain's recipe, an
   entity's components, a generator's seed). Everything computed from it is a cache.
3. **Edit and play are two states of one session, and both take commands.** Editing changes
   the document, with undo. Playing runs a simulation on a copy of the document, and world
   commands sent meanwhile act on that running world: the game master's hand. Stopping throws
   the copy away, or keeps it as the new document when asked. Playing alone never changes the
   document.
4. **Bit-identical simulation.** Same document, same inputs, same ticks: the same state hash,
   on every platform.
5. **The core is headless.** Layers L0 to L3 have no GPU, no window, no file system, no clock
   and no network. They take data and return data, which is what makes them testable, portable
   to the browser and usable as a server.

## Crates and layers

Dependencies point downwards only (`cargo xtask layers`).

| Layer | Crate | Holds | Depends on outside the workspace |
|---|---|---|---|
| L0 | `gamecraft-math` | `Portable` maths, `Rng`, noise, `StateHasher`, angle helpers; re-exports glam | glam (with `libm`), libm |
| L1 | `gamecraft-world` | `World`, `Terrain`, `HeightField`, `Environment`, `Entity`, `Component`; JSON format; validation | serde, serde_json |
| L2 | `gamecraft-sim` | `Sim`: fixed step, `Input`, character movement, collisions, state hash | serde |
| L3 | `gamecraft-engine` | `Session`, `CommandSpec`, the commands, undo, edit and play, queries | serde, serde_json, visioncraft-flora (species list) |
| L4 | `gamecraft-render` | the adapter: world in, VisionCraft renderer calls and pictures out | visioncraft-render, visioncraft-flora, wgpu, glam, png |
| L4 | `gamecraft-automation` | `Host` (session, files, pictures), JSON lines, MCP | serde, serde_json |
| L5 | `gamecraft-cli` | the command line; wires the renderer into the host | pollster |
| L5 | `gamecraft` | the window, native and wasm: the game loop, input to input records, cameras | winit, wgpu |

## The world document

A world is one JSON document (`.gamecraft`, format version 1, `gamecraft-world`):

- **`name`, `seed`.** Generators derive their random streams from the world seed and a label of
  their own, so a world is reproducible from its text.
- **`terrain`.** A square centred on the origin, sampled on a grid, described by its source: a
  flat plane or seeded fractal noise. The height samples are derived (`Terrain::build` gives a
  `HeightField`). The height at a point is interpolated over the same two triangles per cell
  that the renderer draws, so the ground a character stands on is the ground in the picture.
- **`environment`.** Time of day, day of the year, latitude (they give the sun), wind, clouds,
  mist, rain.
- **`entities`.** Each has a numeric id that is never reused, a name, a position (its base: the
  point it stands on), a rotation about the vertical axis, a scale (one number when it is the
  same on every axis, else one along each of its own axes; `design/journal.md`), and components, at
  most one of each type. Component types today: `shape` (a box, sphere or cylinder to block out
  a world with, optionally solid), `plant` (a species of the plant library, a seed and a growth
  stage) and `character` (a body that an input can move).

Since then: scenario data (zones, objectives, triggers; `design/scenarios.md`), combat
(`design/ballistics.md`) and prefabs with overrides and children, which an entity refers to by
`prefab` and a child to its `parent` (`design/journal.md`). Models (`model` components referring to glTF assets of a pack, whose bounding boxes the host
measures and the world carries as derived data, so the simulation can collide without reading a
file), terrain edits (sculpting and painting as an ordered list of recipes over the source) and
projects of packs layered by id are in (`design/projects.md`). Planned: full orientation,
layers, generators (forests, roads, rivers) (M3); rigid bodies and colliders (M4); scripts (M5).

### Axes and units

Metres, seconds, degrees at the command surface. Y is up, the ground plane is x and z, north is
-Z and east is +X. `rotation_y_deg` turns counter-clockwise seen from above; at 0 an entity
faces north (-Z). World positions are `f64`; `f32` appears only in stored height samples,
colours and at the GPU boundary.

## Pipelines

**Edit.** A command arrives as `{command, params}`. The session finds its spec, checks the
parameters against it (types, required keys, allowed values; an unknown key is refused with the
nearest name), runs the handler on the world, validates the result and pushes the previous
world on the undo stack. A failed command leaves the world as it was.

**Play.** `sim.start` builds a `Sim` from a copy of the document: derived data first (the height
field), then one body per character. `sim.input` stores an input record for an entity; it stays
until replaced. `sim.step` advances whole ticks of 1/60 s: for each character in id order, the
input gives a wanted velocity, gravity and the ground give the vertical motion, solid shapes
push the body out. The answer carries the tick and the state hash. `sim.stop` drops the
simulation, or with `keep` makes the running world the document, as one undo step.

**Direct.** A world command that arrives while a simulation runs (`entity.add`,
`environment.set`, `terrain.set`, …) is the same command with the same checks, run on a copy of
the running world. When it succeeds the simulation takes the new world over between two ticks:
bodies are matched to the characters that exist now (a new character gets a body at rest, a
removed one loses it, the others keep their motion) and derived data is rebuilt if the terrain
changed. There is no undo on a running world. The session's journal records these commands, the inputs
and the steps with their ticks and hashes, so a directed session replays to the same hash
(`docs/design/journal.md`).

**Picture.** `view.render` is a host command: the automation asks its `Pictures` implementation
(the CLI's is `gamecraft-render` on a headless wgpu device) to draw the world of the running
simulation, or else the document. The adapter turns the height field into terrain chunks, each
plant into meshes and textures from VisionCraft's generator, each shape into a mesh, and the
environment into sun, sky, clouds, mist and wind, then reads the frame back and encodes a PNG.

## Determinism

The simulation's promise (rule 3) rests on five habits:

- **Fixed step.** `gamecraft_sim::TICK_HZ` ticks per second; a frame rate never reaches the
  simulation. A window interpolates between ticks for display only.
- **Portable maths.** Addition, multiplication, division and square root are exact IEEE-754 on
  every target. Sines, powers and logarithms are not, so they come from the `libm` crate through
  `gamecraft_math::Portable`, and glam is built with its `libm` feature. The simulation uses
  glam's `f64` types, whose code is scalar on every target; glam's `f32` types use SIMD on some
  targets and stay out of the simulation.
- **Ordered iteration.** Entities are stepped in id order; maps are `BTreeMap`.
- **Seeded randomness.** `gamecraft_math::Rng`, forked per use from the world seed.
- **A hash of everything that moves or decides what moves next.** `Sim::state_hash` covers the
  tick, the static content of the running world, and for every character its position,
  rotation, velocity, grounded flag, input record and held facing, and the state of
  zones, projectiles, health and behaviours when there is any. Tests pin golden hashes; a replay compares
  them.

Proven across targets: `cargo xtask wasm-hash` (and CI) runs the scenarios of
`gamecraft-wasmcheck` natively and compiled to `wasm32-unknown-unknown` in Node, and every hash
is bit-identical: the example world played for 600 ticks with every kind of input (hashed every
60 ticks), the Lantern and Skirmish prototypes played through to the end (every 60 ticks; their
final hashes are the golden hashes of the CLI tests), `query.aim` solutions in still air and in
wind, a 4 km noise terrain with layers, paint rules and sculpting (heights in bands of 128 rows,
layer weights, interpolated heights and normals) and two `entity.scatter` calls with the
document they leave. A difference fails with the scenario and its first differing tick or band.
Not covered yet: rendering (pictures may differ between GPUs; they are not part of the state),
threads (the simulation has none), and browsers other than Node's V8 (wasm floating point is
specified exactly, apart from NaN bit patterns, which the simulation never stores).

## The VisionCraft boundary

GameCraft uses two crates of VisionCraft as git dependencies pinned to one rev:
`visioncraft-render` (the wgpu renderer: meshes, materials, instanced items, terrain chunks,
physical sky, atmosphere, clouds, wind, water, shadows, post-processing) and `visioncraft-flora`
(procedural, deterministic plants). Rule 13 keeps them behind `gamecraft-render` and the
engine's `plants` module, so a VisionCraft API change touches one adapter.

What GameCraft needs and VisionCraft's renderer does not have is built in VisionCraft. Known
gaps today: skeletal animation, camera-relative rendering for very large worlds, streaming of
terrain chunks by distance, and statistics per level of detail (the renderer counts a frame's
instances and triangles only; the adapter counts what each model drew per level again by the
renderer's rule, `gamecraft-render/src/culling.rs`). If the shared part grows, the renderer and the plant generator move
to a repository of their own that both projects depend on (`decisions.md`).

## Structural debt

- Undo keeps whole copies of the world (100 of them); it must keep changes instead before worlds
  grow (M3).
- The renderer adapter rebuilds everything when the world changes. It needs to update only what
  changed before worlds grow (M3).
- Positions reach the renderer as `f32` relative to the world origin. Beyond a few kilometres
  they need to be relative to the camera (M3).
- A running world that a command changed is taken over whole by the simulation. With many
  entities that needs to become a change set (M2).
