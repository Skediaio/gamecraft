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
- **Scale.** `scale` is one number, or `[x, y, z]` along the entity's own axes. A prefab's
  children take the instance's scale in the instance's frame: a deck stretched along its length
  keeps its rails thin and its posts short. What has one size (a cylinder's width, a plant)
  takes the middle of the three.
- **Points.** `[x, y, z]` is a point in space (`position`); `[x, z]` a point on the ground
  (`at`, `face`). `at` puts an entity's base on the ground there (`offset_m`: that high above
  it); it is an error where there is no ground. `query.ground {points: [[x, z], ...]}` gives
  the ground (height, normal, slope) at thousands of points in one call, before anything is
  simulated: ask it for every pier and every end of a bridge, then place from its answers.
- **The base of an entity.** An entity's `position` is its base: the point it stands on. A box
  of height 2 at `y = 3` spans from 3 to 5. `entity.snap_to_ground` puts bases on the ground.
- **Spans.** `entity.add` and `prefab.spawn` take `span {from: [x, z], to: [x, z],
  length_m?}` instead of a position: a deck between two ground points, level with its base on
  the higher end's ground, facing `from` to `to`, scaled so that its length spans the ends. The
  answer says where each end stands and how far above its ground (`ends`): place an abutment or
  a pier where `gap_m` is not 0.
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

- **Input is a record.** `sim.input {entity, forward?, right?, yaw_deg? | face? | face_entity?,
  run?, jump?, interact?}`
  stores what a character is asked to do: `forward` and `right` from -1 to 1 relative to its
  facing, the facing as `yaw_deg`, as a ground point to `face` or as an entity to
  `face_entity`, `run`, and `jump` (held: it jumps whenever it stands on the ground). `face`
  and `face_entity` are held: the character keeps turning towards them every tick (so
  `forward: 1` walks to the point, and a moving entity is followed), and `face_entity` also
  aims a launcher's pitch at a character's chest; `yaw_deg` replaces them and `face: null`
  clears them. The record holds until it is replaced; fields left out
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
  line in `sim.state` while playing; `to` is a character or `tag:<tag>` (`tag:player`), and
  `sim.state {for: "Player"}` lists only what that player sees. In a trigger its `text` may hold `{var:name}`, `{who}` and
  `{target}`: `"{var:enemies_left} left"`.
- **Triggers.** `trigger.add {id, when, if?, do, once?, enabled?}`: when the event happens while
  playing (`enter`/`leave` a zone, `interact`, `time` at `at_s`, a `var` changed, `start`) and
  the condition holds (`{var, op, value}`, `{objective, status}`, `{all: [...]}`, `{any:
  [...]}`), the commands of `do` run on the running world, exactly
  as you would send them, as one all-or-nothing transaction. In them `"$who"` and `"$target"`
  stand for the entity that caused the event and the zone or target, as the whole value of an
  entity parameter. A `who` filter takes an entity or `"tag:player"`. `once` (the default)
  disables a trigger after it ran.
- **Seeing what happened.** Each `sim.step` answers with the `events` of its ticks and the
  triggers that `fired` (`ok`, or the `error` that stopped one); `sim.state` lists the
  objectives, the messages showing and the last trigger errors; `scenario.inspect` shows every
  variable, objective and trigger. A trigger that fails changes nothing and the game goes on.
- **Directing a game.** The game master is whoever sends world commands to a running world; see
  *The game master* below. A variable you change between steps fires its `var` triggers at the
  start of the next step.
- **The journal.** Everything you send to a running world that changes it (world commands,
  `sim.input`, `sim.fire`, `sim.release`, `message.show`, `sim.step`) is recorded with the tick
  it took effect at and the state hash after it; queries and the commands of triggers are not.
  `journal.get {from_tick?, to_tick?, limit?}` reads it, `journal.save {path}` writes it with the
  world the simulation started from, and `gamecraft-cli replay FILE --check` runs it again and
  names the first tick where the hash differs (`--to-tick N` stops there to bisect). The journal
  of the last simulation stays readable after `sim.stop`, until the next `sim.start`.

## Generated worlds

A world built by a program is kept in git as the project a bake writes, never as the
program's script; the guideline, with what a re-run replaces, is
[`guide/generated-worlds.md`](guide/generated-worlds.md).

## The game master

A person or an agent directs a running game with the same commands that build it (rule 6), and
three tools of its own; the full guide, with ten worked moves for the Skirmish, is
[`guide/game-master.md`](guide/game-master.md) (MCP: the resource
`gamecraft://guide/game-master` and the prompt `direct_scenario {goal}`).

- **`gm.overview {near?, radius_m?, team?}`**: what is going on, in one compact answer: the tick;
  every character with its team, position, health, `down`, its behaviour's `mode`, `state` and
  `target`, whom it `sees` and whether an agent `taken_over` it; the teams with their characters
  alive and down; the objectives; the last 50 events with their ticks (`names` gives the name
  of every id they mention); the messages showing; the weather with `visibility_m` and `light`;
  the variables. Numbers are rounded to centimetres.
- **`gm.sight {from}`**: who and what a character sees now, with distances, by the simulation's
  own perception (the rules enemies decide by).
- **`gm.order {entity, order, route? | point?}`**: `patrol` a route (or the one it has, from its
  first point), `move` to a point and stay, `hold` where it stands, `guard` its post. It changes
  only the mode and route of the behaviour and keeps the rest; `entity` is a character,
  `tag:<tag>` or a list, so one command moves a squad.

The loop: `gm.overview`, decide, act (`prefab.spawn`, `gm.order`, `environment.set`,
`message.show {to: "tag:player"}`, objectives, triggers), `sim.step` a few seconds, again. Give
each command an `intent`. Everything you send is in the journal and replays to the same hash.

## Prefabs

Describe an entity once and place it many times. `prefab.add {id, name?, components,
children?}` keeps a prefab in the world; `prefab.spawn {prefab, at | position | span,
rotation_y_deg?, scale?, name?, overrides?}` places one and answers `{id, children}`. A
`scale` is the instance's and its children's. The instance's
components are the prefab's, with each component of `overrides` replacing the prefab's of the
same type (or adding a type it lacks): `"overrides": [{"type": "health", "max": 250, "current":
250}]` makes a tougher soldier of the same prefab. `children` (`[{name?, offset: [x, y, z],
rotation_y_deg?, components}]`) are entities of their own placed relative to the instance
(`offset` in its frame: -Z is its forward, +X its right) with the instance as their `parent`:
a house with its door, a beacon with its ring. `entity.set` on the instance moves and turns its
children with it, `entity.remove` removes them too.

- `entity.get` shows an instance's `prefab`, its `overrides`, its `parent` and `children`, and
  its effective `components`; `world.inspect` and `prefab.list` list the prefabs with their
  instances.
- `prefab.set {id, components?, children?, name?}` changes a prefab and every instance with it:
  each keeps its overrides and takes the rest from the new prefab. An instance cannot lose a
  component type of its prefab (override it instead). `prefab.remove` is refused while the
  prefab is placed, and the error names the entities.
- `prefab.spawn` works on a running world: the game master's "drop a squad here". What is
  spawned takes part from the next tick.

## Roads

A road is a centre line with a width that lies on the terrain. `road.add {points, width_m?,
name?}` takes the centre line in world `[x, z]` (heights follow the ground) and answers `{id,
tier, length_m}`:

```json
{"command": "road.add", "params": {"name": "High Street", "points": [[-30, 20], [0, 18], [24, 36]], "width_m": 7}}
```

- The engine drapes the surface on the terrain's own triangles, `lift_m` (default 3 cm) above
  them, so it follows the ground exactly on any slope; bends get round joins and the ends are
  round (`ends: "flat"` cuts them square), so two roads that meet end to end at a point, as
  OpenStreetMap ways do, leave no gap. A road is one surface, not a row of boxes.
- Junctions: where roads overlap, the one with the higher `tier` (0 to 7) lies on top, 1 cm
  per tier higher, so two surfaces are never at the same height and nothing flickers.
  `road.add` picks the lowest tier no overlapping road uses; give `tier` to choose (a main road
  over a side street). For roads written as data (a pack's layer files, `entity.add` with a
  `road` component), `road.set {id, retier_all: true}` on any road gives every road a tier again.
- `query.raycast` straight down answers with the road's id where there is a road (`kind:
  "entity"`), else `kind: "terrain"`: the way to ask "is this point on asphalt?". Lines of sight
  and projectiles hit the surface too.
- `road.set {id, points?, width_m?, widths_m?, color?, lift_m?, tier?}` changes only what it is
  given; `widths_m` gives a width per point (a road that widens at a junction). Remove a road
  with `entity.remove`. Roads work on a running world like every world command.
- Characters walk on the terrain beneath the road (its surface is a few centimetres higher).
  The terrain is not flattened under a road yet: on a side slope the road tilts with the ground.
- A road needs a terrain: without one, or beyond its edge, nothing is drawn or hit. One road
  covers at most 16.7 million terrain triangles; split a longer one.

## Projects, packs and layers

A game on disk is a **project**: a folder of text files you can read, diff and merge
(`design/projects.md`). A pack has `pack.json` (`id`, `name`, `depends`, `worlds`), one
`prefabs/<id>.json` per prefab, and per world `worlds/<world>/world.json` (terrain, environment,
scenario; `default_layer`, `layers` order) and `worlds/<world>/layers/<layer>.json`, one entity
per line with a string `id` unique in its pack. A mod is a pack that depends on the game:
packs load in dependency order and a later one changes an earlier one by id (a prefab file
replaces the prefab, an entity with an existing `id` replaces it, `{"id": "...", "removed":
true}` removes it, `world.json` fields merge one by one, a layer file with `"replace": true`
drops the earlier layer of its name).

- **`project.open {path}`** loads a project and its dependencies into the document;
  **`project.info`** lists packs, worlds, layers with entity counts, prefabs with their pack,
  and the files `project.save` would write. **`project.new {path, id, name, world?}`** makes a
  project of the current world.
- **Layers.** `entity.add {layer}`, `entity.set {layer}` and `prefab.spawn {layer}` put an entity
  in a layer (a plain name; `""` or none is the world's default layer); `entity.get` and
  `world.inspect` show it. A layer is a file: put each area or kind in its own (`village`,
  `enemies`, `forest-north`) so an edit touches one small file.
- **`project.save`** writes each change to the file it came from: an edited entity is one line of
  its layer file. A change to something a dependency brought is written into the project's own
  pack as an override (or a removal), so editing the game under a mod makes the mod, never
  changes the game. Saving nothing changed writes nothing.
- **`terrain.import {path, scale_m?, offset_m?, size_m?}`** makes an elevation file (a PGM, rows
  of text, raw 16-bit or 32-bit samples, inside the read root) the terrain's heightmap at full
  resolution: `world.json` names it by its shape and a hash of its content, its samples are tiles
  in `worlds/<world>/terrain/heightmap/`, and sculpt edits and paint lie over it. Without its
  samples (a missing or bad tile is named by `project.open`) there is no ground, and a command
  that needs it says so.
- **`project.query {sql}`** asks the project's index with one `SELECT`: `entities` (`id, key,
  name, layer, pack, prefab, parent, x, y, z, rotation_y_deg, scale, tags, components`),
  `entity_tags (entity_id, tag)`, `entity_components (entity_id, type, data)`, `prefabs (id,
  name, pack, components, instances)`. Every enemy within 100 m of a point:
  `SELECT e.name FROM entities e JOIN entity_tags t ON t.entity_id = e.id WHERE t.tag = 'enemy'
  AND (e.x - 4) * (e.x - 4) + (e.z + 54) * (e.z + 54) <= 10000`. The index is rebuilt from the
  world when it changed; it cannot be written to, and change the world with commands instead.
- `project.export {path}` writes the resolved world as one `.gamecraft` file, the format for
  tests and exchange; its answer names the pack that has the world's models (`pack`), the files
  it needs of one (`needs`) and the models none has (`missing`). The file draws its models when it
  lies inside that pack or is opened with `--pack DIR`. `examples/skirmish/project` is the Skirmish as a project;
  `examples/skirmish-night-mod` turns it to night, gives the enemies lanterns and adds a guard.

## Projectiles and behaviour

Fighting is three components and two commands; they name no weapon, a pack fills them in.

- **`launcher`** on any entity fires projectiles: point masses flown with gravity, the G7 drag
  function, the air's density (from `environment.temperature_c` at y = 0 and
  `environment.sea_level_m`, by the standard atmosphere) and the wind (from
  `wind_direction_deg`, `wind_speed_mps` at 10 m above the ground and less below it). Give the
  usual G7 ballistic coefficient in lb/in2 times 895.2 as `ballistic_coefficient` (kg/m2). A
  character fires from its eyes while its input holds `fire` (`sim.input {fire: true,
  pitch_deg}`; `pitch_deg` aims up or down without turning the body), every `rate_s`, with a
  `magazine` and a `reload_s`; anything else fires with `sim.fire {entity, yaw_deg?,
  pitch_deg?}`. Spread (`dispersion_mrad`) is drawn from the world's seed, so the same world and
  inputs give the same shots. `query.aim {from, to, launcher?}` gives the yaw and pitch that
  hit a point or an entity in the current air and wind (`reachable`, `time_of_flight_s`,
  `drop_m`, `drift_m`); `sim.input {face_entity}` keeps a character facing a target with the
  pitch for the drop.
- **Hits.** A projectile hits the terrain, any shape (solid or not) and characters (a capsule
  from feet to head): a `hit` event `{projectile, shooter, target, point, normal, speed_mps,
  energy_j}`; `target` is null for the terrain. `sim.step` and `sim.state` list the
  `projectiles` in flight.
- **`health`** `{max, current, armor}`: a hit takes `energy_j * damage_per_joule - armor`. At 0
  a `down` event `{who: shooter, target}`; a character that is down stops and takes no input.
- **Hearing.** A shot gives a `heard` event `{who: listener, target: shooter, point}` to every
  character within the launcher's `sound_range_m`.
- **Seeing.** A character sees another within its view range, its field of view and a clear
  line of sight (the ray maths of `query.line_of_sight`). The range is
  `min(view_range_m * light, 3.912 / extinction)`: `light` is 1 by day and 0.15 at night (sun
  below -12 degrees, linear up to 6 degrees above the horizon), and the extinction is the
  renderer's own for mist (`3.912 * mist^2 / 120` per metre) and rain (`3.912 / (400 * (0.8 /
  rain)^1.5)`), so a fog of 1 hides everything beyond 120 m.
- **`behaviour`** `{mode: idle | patrol | guard, route, team, view_range_m, fov_deg,
  engage_range_m, aim_error_mrad, reaction_s, search_s}` makes a character an enemy: it walks
  its route or holds its post; on seeing a character of another team (a behaviour's `team` or a
  tag `team:<name>`; a character without a team is left alone) it turns, waits `reaction_s`, and
  fires with its aim error; when it hears a shot or loses its enemy it searches there for
  `search_s`, then returns. Its state is in `sim.step`, `sim.state` and `entity.get`
  (`behaviour: {state, target?, ...}`). It writes the same inputs a player does: send it
  `sim.input` and you play it, until `sim.release {entity}`.
- **Scenarios** react with triggers on `hit`, `down` and `heard`; `who` and `target` filter by
  entity or `"tag:<tag>"`. "When the last enemy is down": a trigger on `{"event": "down",
  "target": "tag:enemy"}` that adds 1 to a variable, and one on that variable reaching the count
  that sets the objective done.

## Known limits of M0

- Shapes are boxes, spheres and cylinders; there are no meshes or scripts yet, and packs hold
  prefabs and worlds but no assets yet. A prefab's
  children follow `entity.set`, not the simulation: a character that walks leaves its children
  where they were.
- Characters do not collide with each other; spheres block but cannot be stood on; plants are no
  obstacles; there are no rigid bodies.
- A running world has no history; its journal is how a directed session is kept and replayed.
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
