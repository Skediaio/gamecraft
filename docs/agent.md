# Driving GameCraft as an agent

GameCraft is a game engine that agents drive with commands. A world is terrain, an environment
(time of day, the sun, weather) and entities with components; you build it as a document with
undo, then play it with a deterministic simulation that the same commands steer while it runs.
Everything is a command with an id and JSON params, the same over MCP (`gamecraft-cli mcp`) and
JSON lines (`gamecraft-cli serve`). The protocol, every command and its parameters are in
[`control-protocol.md`](control-protocol.md).

## Conventions

- **Units.** Metres, seconds, degrees. World positions are numbers in metres; a world can be
  many kilometres wide.
- **Axes.** Y is up; the ground plane is x and z. North is -Z, east is +X (so south is +Z and
  west is -X). A house "north of the pond" has a smaller z.
- **Turning.** `rotation_y_deg` turns an entity about +Y, counter-clockwise seen from above. At 0
  it faces north (-Z); at 90 it faces west (-X); at -90 (or 270) east.
- **Points.** `[x, y, z]` is a point in space (`position`); `[x, z]` a point on the ground
  (`at`, `face`). `at` puts an entity's base on the ground there; it is an error where there is
  no ground.
- **The base of an entity.** An entity's `position` is its base: the point it stands on. A box
  of height 2 at `y = 3` spans from 3 to 5. `entity.snap_to_ground` puts bases on the ground.
- **Ids and names.** Every entity has a numeric id, handed out in order and never reused (not
  after removal, save and load, or undo). Commands take an entity as its id (a number) or its
  name (a string, any case). Names need not be unique; an ambiguous name is an error that lists
  the candidates, and the id always works.
- **Components.** At most one of each type per entity: `shape` (a `box`, `sphere` or
  `cylinder` with a `size` `[x, y, z]`, a `color` `[r, g, b]` and `solid`), `plant` (a `species`
  of the plant library, a `seed`, a `growth` 0 to 1) and `character` (a body that input moves:
  `height_m`, `radius_m`, `walk_speed_mps` 1.6, `run_speed_mps` 5, `jump_speed_mps`,
  `max_slope_deg`). Fields left out take their defaults.
- **Errors** are sentences that start with the command id and name the parameter, the value and
  what is allowed; an unknown key or command names the nearest one. Read them and correct the
  call; a failed command changes nothing.

## The working loop

1. **List.** `command_list {"brief": true}` (JSON lines: `engine.commands {"brief": true}`) gives
   every command's id, its params in one line and the first sentence of its summary;
   `{"id": "entity.add"}` gives one command in full: each parameter's type, unit, default,
   allowed values and doc, the result's shape, a tested example, its scope and whether it can
   run now.
2. **Plan in numbers**: the terrain's size and relief, every entity's name, place and
   components, before the first command.
3. **Change.** `command_batch` runs up to 256 steps in order and stops at the first error; with
   `"atomic": true` it is all or nothing, and a batch that succeeds is one undo step. One command
   at a time is `command_run`.
4. **Inspect.** `doc_inspect` (the command `world.inspect`) gives the whole world as JSON: the
   terrain with its height range, the environment and the sun, every entity with its
   components, and whether a simulation runs. Compare it with the plan.
5. **Look.** `doc_render_preview` draws the world (the whole of it, from a `camera`, or
   following an entity) and returns the picture. Look at it: what only a picture shows (a tree
   inside a rock, a character under the ground) is checked there.
6. **Undo, save.** `edit.undo` and `edit.redo` step through the history; `file.save {"path":
   "valley.gamecraft"}` writes the document when the server was given a write root.

## Two states: editing and playing

**Editing.** The document is the world you build. Each command that changes it is one undo step
(an atomic batch is one step as a whole).

**Playing.** `sim.start` starts a simulation on a copy of the document. From then on the same
world commands (`entity.add`, `entity.set`, `environment.set`, `terrain.set`, ...) change the
running world, not the document: that is the game master's hand. A wall added while a character
walks is in its way on the next tick. `world.inspect`, `doc_inspect` and `view.render` show the
running world. There is no undo while playing (`edit.undo` is refused), and saving writes the
document, never the running world. `sim.stop` throws the running world away; `sim.stop {"keep":
true}` makes it the document, as one undo step.

## How to play

- **Input is a record.** `sim.input {entity, forward?, right?, yaw_deg? | face?, run?, jump?,
  interact?}`
  stores what a character is asked to do: `forward` and `right` from -1 to 1 relative to its
  facing, the facing as `yaw_deg` or as a ground point to `face`, `run`, and `jump` (held: it
  jumps whenever it stands on the ground). The record holds until it is replaced; fields left out
  keep their last value. Any character can be driven; whoever writes the record does not matter.
- **Steps.** `sim.step {ticks}` or `{seconds}` advances whole ticks of 1/60 s, at most 36 000
  ticks (10 minutes) per call. Nothing moves between calls: the simulation is not tied to a
  clock, and runs as fast as it is stepped.
- **What a step answers.** `{tick, time_s, hash, characters: [{id, name, position,
  rotation_y_deg, velocity, grounded}]}`. `sim.state` answers the same, with the stored inputs,
  at any time (and `{playing: false}` when nothing runs).
- **The state hash.** `hash` (16 hex digits) covers the tick, the running world and every
  character's position, rotation, velocity, ground contact and input. The simulation is
  deterministic: a fixed step, fixed-order updates and maths that gives the same bits on every
  platform. So the same document with the same inputs at the same ticks gives the same hash, on
  every run and every machine. Two runs that end on different hashes did not get the same world
  or the same inputs at the same ticks; compare the step answers to find the first difference.
- **Movement in M0.** A character walks at `walk_speed_mps` (runs at `run_speed_mps`) at once,
  without inertia; gravity pulls it down, it lands on the terrain and on the tops of low solid
  boxes and cylinders, it steps up at most 0.35 m, and it does not climb slopes steeper than its
  `max_slope_deg`. Solid shapes push it out; plants and other characters do not block it.

## Scenarios

A scenario turns a world into a game that can be steered; it is data in the world, so it is
built, saved and changed with commands like everything else.

- **Components** make entities take part: `zone` (a circle or box volume that gives `enter` and
  `leave` events when characters come in and go out), `interactable` (a character whose input
  `interact` turns true, within `reach_m` of its eye and facing it within 45 degrees, gives an
  `interact` event for the nearest one), `light` (a point light a trigger can switch with
  `entity.set_component`) and `tags` (labels such as `player` or `beacon`).
- **Variables, objectives, messages.** `var.set {name, value | add}` keeps numbers, booleans and
  strings; `objective.add {id, title}` and `objective.set {id, status}` (`open`, `done`,
  `failed`) keep what the players are to achieve; `message.show {text, to?, seconds?}` puts a
  line in `sim.state` while playing.
- **Triggers.** `trigger.add {id, when, if?, do, once?, enabled?}`: when the event happens while
  playing (`enter`/`leave` a zone, `interact`, `time` at `at_s`, a `var` changed, `start`) and
  the condition over the variables holds, the commands of `do` run on the running world, exactly
  as you would send them, as one all-or-nothing transaction. In them `"$who"` and `"$target"`
  stand for the entity that caused the event and the zone or target, as the whole value of an
  entity parameter. A `who` filter takes an entity or `"tag:player"`. `once` (the default)
  disables a trigger after it ran.
- **Seeing what happened.** Each `sim.step` answers with the `events` of its ticks and the
  triggers that `fired` (`ok`, or the `error` that stopped one); `sim.state` lists the
  objectives, the messages showing and the last trigger errors; `scenario.inspect` shows every
  variable, objective and trigger. A trigger that fails changes nothing and the game goes on.
- **Directing a game.** There is no separate game-master API: read `sim.state`, the events and
  `objective.list`, decide, send world or scenario commands, step. A variable you change between
  steps fires its `var` triggers at the start of the next step.

## Known limits of M0

- Shapes are boxes, spheres and cylinders; there are no meshes, prefabs, scripts or packs yet.
- Characters do not collide with each other; spheres block but cannot be stood on; plants are no
  obstacles; there are no rigid bodies.
- A running world has no history and its game-master commands are not yet recorded in a journal,
  so a directed session is replayed by sending the same commands at the same ticks.
- `intent` is accepted and checked on commands and batches, but not recorded yet.
- There is no control channel to a running window yet (M1).
- Pictures need a GPU: without one, `view.render` and `doc_render_preview` say a renderer is
  needed, and everything else works.

## Worked example

A new world, a noise terrain, a crate, an oak and a character called Scout. Scout walks north;
two seconds later a wall is dropped in its way while the simulation runs; it walks on until the
wall stops it. Each line is one JSON-lines request; the tests run every line and check that each
succeeds and that Scout moved and stopped short of the wall.

```jsonl gamecraft-example
{"id": 1, "method": "engine.execute", "params": {"command": "file.new", "params": {"name": "Example valley", "seed": 7}, "intent": "start from an empty world"}}
{"id": 2, "method": "engine.execute", "params": {"command": "terrain.set", "params": {"kind": "noise", "size_m": 128, "cell_m": 2, "relief_m": 4, "feature_m": 80, "seed": 7}}}
{"id": 3, "method": "engine.execute", "params": {"command": "environment.set", "params": {"time_of_day_h": 17.5, "cloud_cover": 0.3}}}
{"id": 4, "method": "batch", "params": {"atomic": true, "intent": "a crate, an oak and a character", "steps": [{"command": "entity.add", "params": {"name": "Crate", "at": [-6, 4], "components": [{"type": "shape", "kind": "box", "size": [1, 1, 1], "color": [0.55, 0.4, 0.25]}]}}, {"command": "entity.add", "params": {"name": "Oak", "at": [8, -10], "components": [{"type": "plant", "species": "quercus-robur", "seed": 1, "growth": 1}]}}, {"command": "entity.add", "params": {"name": "Scout", "at": [0, 0], "components": [{"type": "character"}]}}]}}
{"id": 5, "method": "engine.execute", "params": {"command": "world.inspect", "params": {"limit": 10}}}
{"id": 6, "method": "engine.execute", "params": {"command": "sim.start"}}
{"id": 7, "method": "engine.execute", "params": {"command": "sim.input", "params": {"entity": "Scout", "forward": 1, "yaw_deg": 0}, "intent": "walk north"}}
{"id": 8, "method": "engine.execute", "params": {"command": "sim.step", "params": {"seconds": 2}}}
{"id": 9, "method": "engine.execute", "params": {"command": "entity.add", "params": {"name": "Wall", "at": [0, -8], "components": [{"type": "shape", "kind": "box", "size": [6, 2, 0.5], "color": [0.6, 0.6, 0.6], "solid": true}]}, "intent": "drop a wall in Scout's way"}}
{"id": 10, "method": "engine.execute", "params": {"command": "sim.step", "params": {"seconds": 4}}}
{"id": 11, "method": "engine.execute", "params": {"command": "sim.state"}}
{"id": 12, "method": "engine.execute", "params": {"command": "sim.stop", "params": {"keep": false}}}
{"id": 13, "method": "engine.execute", "params": {"command": "world.inspect", "params": {"entities": false}}}
```

What happens:

- Line 1 starts an empty world named "Example valley" with seed 7 and a new history.
- Line 2 makes a 128 m square of gentle hills (4 m from trough to peak) sampled every 2 m; the
  answer has the height range.
- Line 4 adds three entities in one atomic batch (one undo step), each with its base on the
  ground: entity 1 the crate, 2 the oak, 3 Scout.
- Line 6 starts playing. Line 7 asks Scout to walk forward facing north (`yaw_deg` 0); line 8
  steps 2 s (120 ticks): Scout walks at 1.6 m/s and is about 3.2 m north of where it started
  (z about -3.2), on the ground.
- Line 9 runs while the simulation runs: the wall (6 m wide, 2 m high, 0.5 m deep, from z -8.25
  to -7.75) is added to the running world, not to the document.
- Line 10 steps 4 s more. Scout walks on until the wall pushes it back: it stops with its
  0.3 m radius against the wall's south face, at z about -7.45, while its input still asks it to
  walk. Line 11 reads the tick (360), the state hash, Scout's position and its stored input.
  Run lines 1 to 11 again on a fresh server and the hash is the same.
- Line 12 throws the running world away (`keep: false`): line 13 shows the document as built,
  three entities and no wall. With `"keep": true` the wall and Scout's new place would have
  become the document, as one undo step.

To look at it, add `{"command": "view.render", "params": {"follow": {"entity": "Scout"}}}` (or the
MCP tool `doc_render_preview {"follow": {"entity": "Scout"}}`) while playing, on a server with a
renderer.
