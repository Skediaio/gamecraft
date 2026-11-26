# Control protocol and MCP

How agents and scripts drive GameCraft: a JSON-lines protocol (`gamecraft-cli serve`) and an MCP
server (`gamecraft-cli mcp`), both over one host (`gamecraft_automation::Host`: a session, the
open file, the folders it may touch and, with a GPU, pictures). Both run the same commands: the
engine's and the host's own, in one namespace.

This is VisionCraft's control protocol (itself 3DCraft's), with the same envelope, method
names, tool names, limits and error conventions, on GameCraft's commands. Where GameCraft
differs it says so under *Differences from VisionCraft and 3DCraft* at the end. The guide for an
agent that uses GameCraft, with a worked example, is [`agent.md`](agent.md).

## Envelope (JSON lines)

Each request is one JSON line; each reply is one JSON line with the same `id`:

```json
{"id": 1, "method": "engine.execute", "params": {"command": "world.set", "params": {"name": "Green valley"}}}
{"id": 1, "ok": true, "result": null, "ms": 0.1}
{"id": 2, "method": "engine.execute", "params": {"command": "world.sett", "params": {}}}
{"id": 2, "ok": false, "error": "unknown command `world.sett`; did you mean `world.set`?", "ms": 0.0}
```

- A request has exactly `id`, `method` and `params`; any other key is refused. `params` of a
  method, and of a command, must be an object or absent (`null` counts as absent).
- `ms` (only in an `engine.execute` reply, ok or not) is the wall-clock milliseconds the command
  took, to a tenth; it is the only part of a reply that differs from run to run (`null` in a
  browser).
- Errors are sentences an agent can act on. A command's error starts with its id and names the
  parameter, the value and what is allowed:
  `entity.add: bad parameters: unknown parameter \`positon\`; did you mean \`position\`? The
  parameters are: name, position, at, rotation_y_deg, scale, components`. An unknown command
  or key names the nearest one.
- Invalid JSON gets `{"id": null, "ok": false, "error": "the request is not valid JSON: ..."}`,
  as does a line over the request limit or not UTF-8, and the next line is still read. Blank
  lines are skipped. No line makes the server stop or panic: a command that panics (a bug) is
  caught and answered with `<command>: internal error (a bug): ...`.

## Methods

| Method | Params | Result |
|---|---|---|
| `engine.execute` | `{command, params?, intent?}` | the command's result |
| `engine.commands` | `{filter?, id?, brief? = false}` | `{commands: [...]}` (below) |
| `batch` | `{steps: [{command, params?, intent?}], stopOnError? = true, terse? = false, atomic? = false, intent?}` | `{completed, failed, results: [{ok, result \| error, ms}], ms}`; terse: `{completed, failed, errors: [{step, error, ms}], last: {step, result, ms}, ms}`; with `atomic` also `rolledBack` |
| `methods` | `{}` | `{methods: [{name, params, summary}], limits: {requestBytes, replyBytes, batchSteps}}` |

**A batch runs as in 3DCraft and VisionCraft**: in order, checked as a whole (its shape, at most
256 steps, every step an object with known keys) before the first step runs; `failed` counts
the commands that returned an error and `stopOnError` (default true) stops at the first. **Not a
transaction**: steps that ran stay applied, each its own undo step.

**`atomic: true`** makes a batch all or nothing, inside one engine transaction
(`Session::transaction`): the first step that fails stops it and takes back every step; the
document, the history, the play state and the open file are as before (`rolledBack: true`). An
atomic batch that succeeds and changes the document is **one** undo step. `file.new`,
`file.open`, `project.new`, `project.open`, `pack.add_dependency`, `edit.undo` and `edit.redo`
cannot be part of one (the batch is refused before
anything runs). Files a step wrote (`file.save`, `view.render` with a path) stay written either
way.

**`intent`**: why the agent runs the command, in a short sentence; accepted on
`engine.execute`, `batch` and each batch step (MCP: `command_run`, `command_batch` and its
steps). It must be a string; it is not recorded yet.

### The command list

`engine.commands` (MCP: `command_list`) gives per command, from the engine's command specs
(`Session::specs()`) and the host's own:

```json
{"id": "sim.step", "label": "Step",
 "params": "{\"ticks\": integer = 1, \"seconds\": number (s) = not set}",
 "summary": "Advances the simulation by whole ticks of 1/60 s: ...",
 "parameters": [{"name": "ticks", "type": "integer", "unit": null, "required": false, "default": "1", "allowed": [], "doc": "Ticks to advance, 0 to 36000."}, "..."],
 "openParams": false, "result": "{tick, time_s, hash, characters: [...]}", "example": {"seconds": 0.5},
 "undoable": false, "scope": "play", "enabled": false,
 "disabledReason": "no simulation is running; start one with sim.start"}
```

- `params` is 3DCraft's notation (`"name": type (unit) = default`, `= not set` for an optional
  parameter without a constant default, `"a"|"b"` for allowed values, `...` for open params);
  `parameters` is the same as data. A parameter's `type` is one of `string`, `number`,
  `integer`, `boolean`, `point2` (`[x, z]`), `point3` (`[x, y, z]`), `color` (`[r, g, b]`),
  `entity` (an id or a name), `object`, `array`, `any`; `default` is JSON text.
- `example` runs: the engine tests every engine example on its example world, and this crate
  runs every example through the protocol.
- `scope` is `world` (changes the document, or the running world while playing), `query` (reads
  only), `play` (starts, steps, feeds or stops the simulation) or `host` (files, history,
  pictures).
- `enabled` and `disabledReason` say whether the command can run now: `edit.undo` with nothing to
  undo, `sim.step` while nothing plays, `view.render` without a renderer, `file.open` without a
  read root are disabled, with the reason.
- `brief: true` gives `{id, params, summary (its first sentence), enabled}`; `filter` is a
  case-insensitive part of the id or label; `id` picks one command (an unknown id is an error
  with the nearest).

Every command's parameters are checked against its spec before it runs: unknown keys are
refused with the nearest name (unless `openParams`), required keys must be there, values must
have their type and be among the allowed ones.

## Commands

The rows below follow the specs (a test compares the parameter names of every row with its
command's spec). `?` marks an optional parameter, `= v` its default. Positions are metres, angles
degrees, times seconds; an entity is its id or its name.

### World

| Command | Params | Result |
|---|---|---|
| `world.set` | `{name?, seed?}` | `null`; renames the world or changes its seed |
| `terrain.set` | `{kind? = "noise", size_m? = 256, cell_m? = 2, height_m? = 0, seed?, relief_m? = 40, feature_m? = 200, octaves? = 5, roughness? = 0.5, cols?, rows?, heights?, content?}` | `{cols, rows, min_height_m, max_height_m}`; a square centred on the origin, `flat` (`height_m`), seeded `noise` (the other keys), real ground as a control `grid` (`cols`, `rows` and `heights`, at most 513 per side, row-major from the north-west corner, interpolated bilinearly) or real ground at full resolution as a `heightmap` (`cols`, `rows` and `content`, the hash of its samples; `{cols, rows, samples}`): its samples are too many for a command, the host hands them over (`terrain.import`, which names and feeds it in one go) and until then there is no ground; keys of the other kinds are refused (`cols` and `rows` are shared by `grid` and `heightmap`); entities are not moved |
| `terrain.clear` | `{}` | `null`; no ground (the layers, paint rules and edits go with it) |
| `terrain.sculpt` | `{op, center?, radius_m?, strength_m?, strength?, falloff? = 0.5, height_m?, iterations? = 1, from?, to?, width_m?, from_height_m?, to_height_m?, feature_m? = 8, seed?}` | `{index, edit, edits, tiles_rebuilt, tiles_total, tiles, min_height_m, max_height_m}`; appends one edit: `raise`/`lower` (`center`, `radius_m`, `strength_m`), `flatten` (`height_m`, else the ground's at the centre), `smooth` (`strength`, `iterations`), `ramp` (`from`, `to`, `width_m`, end heights else the ground's), `noise` (`strength_m`, `feature_m`, `seed`); an op's foreign keys are refused; the ground is rebuilt in the tiles it touches; works on a running world |
| `terrain.paint` | `{layer, center, radius_m, strength? = 1, falloff? = 0.5}` | as `terrain.sculpt`; appends a paint edit that moves the layer weights towards `layer` (one of `terrain.layers`) |
| `terrain.layers` | `{layers}` | `{layers, tiles_rebuilt, ...}`; sets the painted layers `[{name, color, roughness?}]`, at most 8, the first under everything; a layer still painted or ruled cannot go; `[]` back to shading by slope |
| `terrain.paint_rule` | `{layer, slope_min_deg?, slope_max_deg?, height_min_m?, height_max_m?}` | `{index, rule, paint_rules, ...}`; appends an automatic rule (soft edges of 4 degrees, 4 m), applied before the paint edits |
| `terrain.remove_paint_rule` | `{index}` | `{removed, paint_rules, ...}` |
| `terrain.edits` | `{}` | `{size_m, cell_m, source, layers, paint_rules, edits: [{index, op, ...}], ops, tiles, samples}`; the terrain recipe in full |
| `terrain.remove_edit` | `{index}` | `{removed, edits, tiles_rebuilt, ...}`; later edits move up one; works on a running world |
| `environment.set` | `{time_of_day_h?, day_of_year?, latitude_deg?, wind_direction_deg?, wind_speed_mps?, cloud_cover?, mist?, rain?, temperature_c?, sea_level_m?}` | `{environment: {time_of_day_h, day_of_year, latitude_deg, wind_direction_deg, wind_speed_mps, cloud_cover, mist, rain, temperature_c, sea_level_m}, sun: {azimuth_deg, elevation_deg}}`; only the fields given change; `temperature_c` (at world y = 0, default 15) and `sea_level_m` (the height of y = 0 above sea level, default 0) give the air density projectiles fly through |

### Entities

| Command | Params | Result |
|---|---|---|
| `entity.add` | `{name? = "", position?, at?, offset_m?, rotation_y_deg? = 0, scale? = 1, span?, layer?, components? = []}` | `{id, position, span?}`; give `position` (`[x, y, z]`), or `at` (`[x, z]`, the base on the ground, `offset_m` above it), or `span` (`{from: [x, z], to: [x, z], offset_m? = 0, length_m?}`: a deck between two ground points, level with its base on the higher end's ground plus `offset_m`, facing from `from` to `to`, scaled so that its length along its forward axis spans the ends; the length is `length_m`, else the box's along z, a cylinder's or a sphere's diameter; the answer's `span` is `{from, to, length_m, scale, ends: [{point, ground_m, gap_m}, {…}]}`, where each end stands and how far above its ground), never two of them, and `span` sets the scale and the turn itself (it stretches the length only: `scale` `[1, 1, n]`); `scale` is one number or `[x, y, z]` along the entity's own axes, which a prefab's children take in the instance's frame; `layer` is a plain name (1 to 64 letters, digits, `-`, `_`; in a project the file `layers/<layer>.json`), `""` or left out the world's default layer |
| `entity.set` | `{id, name?, position?, at?, offset_m?, rotation_y_deg?, scale?, layer?}` | `{id, position}`; only the fields given change; the entities that belong to it (a prefab's children) move, turn and scale with it, and go to the same `layer` (`""`: the default layer); `at` with `offset_m` stands the base that high over the ground |
| `entity.remove` | `{id}` | `null`; removes the entities that belong to it too (a prefab's children); the ids are never handed out again |
| `entity.remove_many` | `{ids}` | `{removed}`; every entity of `ids` and the entities that belong to them go: an id or a name that no entity has is an error naming it (at most ten ids), and nothing goes; one undo step, checked once, for a layer, a selection or a generated set (1 to 1000000 of them) |
| `entity.set_component` | `{id, component}` | `null`; adds a component or replaces the one of its type |
| `entity.remove_component` | `{id, type}` | `null`; `type` is a component type (`component.list`) |
| `entity.snap_to_ground` | `{id?}` | `{moved, skipped}`; one entity, or all when `id` is left out |
| `road.add` | `{points, name? = "", width_m? = 6, widths_m? = [], color? = [0.05, 0.05, 0.055], lift_m? = 0.03, ends? = "round", tier? = "auto", layer?, retier_all? = false}` | `{id, tier, length_m, position}`; a road: an entity at the first point with a `road` component; `points` `[[x, z], ...]` in world metres (2 to 4096, consecutive ones at least 0.01 m apart), kept as offsets in the entity's frame; its surface is draped on the terrain's triangles `lift_m` above them, drawn, and hit by `query.raycast` and `query.line_of_sight` with the road's id; `tier` 0 to 7 or `"auto"` (the lowest tier no overlapping road uses; each tier 0.01 m higher, so overlapping roads are never coplanar); `retier_all` gives every road its tier again in id order afterwards; characters walk on the terrain beneath it |
| `water.add` | `{name? = "", level_m? = 0, outline?, layer?}` | `{id, level_m, area_m2, position}`; water: an entity with a `water` component standing at the outline's centre on the surface, `level_m` high in the world, over `outline` (`[[x, z], ...]` in world metres, 3 to 64 points; left out or `[]`: everywhere, the sea). A character whose feet are under the surface wades, slower by the depth (its full speed up to 0.5 m, 0.4 of it from 1.5 m), and deeper water is refused; rays and projectiles pass through it. The renderer draws one water surface (everywhere, else the largest area) and names the others in `problems` (`design/water.md`) |
| `water.set` | `{id, level_m?, outline?}` | `{id, level_m, area_m2, position}`; only the fields given change: `level_m` moves the surface (and the entity's height), `outline` (world metres) is kept in the entity's frame; an entity without water gets some (then `outline` is required: `[]` for everywhere). Remove water with `entity.remove` or `entity.remove_component {type: "water"}` |
| `road.set` | `{id, points?, width_m?, widths_m?, color?, lift_m?, ends?, tier?, retier_all? = false}` | `{id, tier, length_m, position, changed_tiers?}`; only the fields given change (`widths_m: []`: one width again); new `points` (world metres) are kept in the entity's frame, the entity stays; an entity without a road gets one (then `points` is required); the tier stays unless `tier` is given (`"auto"` picks it again). Remove a road with `entity.remove` or `entity.remove_component {type: "road"}` |
| `prefab.add` | `{id, name? = "", components? = [], children? = []}` | `{id}`; an entity described once to place many times; `children` (at most 64) are `[{name? = "", offset? = [0, 0, 0], rotation_y_deg? = 0, prefab?, components? = []}]`, placed relative to each instance (`prefab`: the child is an instance of that prefab, its `components` its overrides; 8 levels at most, never a loop); the file does not store children, it places them again from the prefab and writes only changed or removed ones on their instance (`offset` in metres in its frame: -Z its forward, +X its right); the id must be new |
| `prefab.set` | `{id, name?, components?, children?}` | `{id, instances: [ids]}`; only the fields given change, each as a whole; every instance keeps its overrides and takes the rest from the new prefab; its children follow the same rule, recognised by name (by index when names repeat): an edited child keeps its edits, a child the prefab lost is removed unless edited or with entities of its own below, a child it gained is placed in every instance, a removed child stays removed |
| `prefab.remove` | `{id}` | `null`; refused while entities are placed from it, naming them |
| `prefab.spawn` | `{prefab, position?, at?, offset_m?, rotation_y_deg? = 0, scale? = 1, span?, name?, layer?, overrides? = []}` | `{id, children: [ids], position, span?}`; the instance and its children go to `layer` (left out: the default layer); an entity with `prefab` set, whose components are the prefab's with `overrides` replacing those of the same type (or adding a type the prefab lacks), and one entity per child with the instance as `parent`; `name` defaults to the prefab's; works on a running world too; the instance and its children stand at the instance's `scale`; `span` places a deck between two ground points as for `entity.add`, its length measured from the prefab's children (shapes, and models whose asset has bounds, nested and turned ones included) when `length_m` is left out |
| `entity.scatter` | `{count, components, seed?, name? = "", center?, radius_m?, exclude? = [], min_distance_m? = 0, max_slope_deg? = 90, scale_range? = [1, 1], random_rotation? = true, vary_seed? = true}` | `{placed, requested, first_id, last_id}`; up to `count` (1 to 5000) copies of the template `components` on the terrain's ground, in a circle (`center` `[x, z]` and `radius_m`) or over the whole terrain, skipping points steeper than `max_slope_deg`, inside a circle of `exclude` (`[{center: [x, z], radius_m}]`, at most 256) or closer than `min_distance_m`; names get a number from 1 (`Oak 1`, `Oak 2`, ...); each plant its own seed with `vary_seed`; the same parameters give the same entities; one undo step |

Components, as in the file format (fields left out take their defaults):
`{"type": "shape", "kind": "box" | "sphere" | "cylinder", "size": [x, y, z], "color": [r, g, b], "solid": true}`,
`{"type": "plant", "species", "seed", "growth"}` (a species of the plant library, growth 0 to 1),
`{"type": "character", "height_m", "radius_m", "eye_height_m", "walk_speed_mps", "run_speed_mps", "jump_speed_mps", "max_slope_deg"}`,
`{"type": "zone", "shape": "circle" | "box", "radius_m", "size": [x, z], "height_m"}`,
`{"type": "interactable", "reach_m", "prompt"}`,
`{"type": "light", "color": [r, g, b], "intensity_cd", "range_m", "on"}`,
`{"type": "tags", "tags": ["..."]}`,
`{"type": "launcher", "muzzle_velocity_mps", "mass_kg", "ballistic_coefficient", "dispersion_mrad", "rate_s", "magazine", "reload_s", "damage_per_joule", "muzzle_height_m", "max_flight_s", "sound_range_m"}` (the ballistic coefficient is G7, in kg/m2: the usual lb/in2 figure times 895.2),
`{"type": "health", "max", "current", "armor"}`,
`{"type": "behaviour", "mode": "idle" | "patrol" | "guard", "route": [[x, z], ...], "team", "view_range_m", "fov_deg", "engage_range_m", "aim_error_mrad", "reaction_s", "search_s"}`,
`{"type": "road", "points": [[x, z], ...], "width_m", "widths_m": [...], "color": [r, g, b], "lift_m", "tier", "ends": "round" | "flat"}` (points are offsets from the entity's position, turned by its yaw and times its scale; `entity.add` and `entity.set_component` take the tier as given, `road.add` picks a free one),
`{"type": "water", "level_m"?, "outline"?: [[x, z], ...]}` (an area of water at one level, or everywhere without an outline; `level_m` is above the entity's position and `outline` in its frame; `water.add` takes a world height and world points; `design/water.md`).

### Playing

| Command | Params | Result |
|---|---|---|
| `sim.start` | `{}` | `{tick: 0, hash, characters: [ids], events, fired}`; plays a copy of the document; the `start` triggers run; refused while the terrain names a heightmap that has no samples yet (there is no ground to stand on) |
| `sim.input` | `{entity, forward?, right?, yaw_deg?, face?, face_entity?, run?, jump?, interact?, pitch_deg?, fire?}` | `{entity, forward, right, yaw_deg, pitch_deg, run, jump, interact, fire, face, face_entity, face_pitch}`: the stored input; it holds until replaced, fields left out keep their value; one of `yaw_deg`, `face` (`[x, z]`) and `face_entity`; `face` and `face_entity` are held: every tick, before anybody moves, the character turns towards the point or the entity (a character's chest), and with `face_entity` and a launcher also aims the pitch at it, allowing for the drop in a vacuum (`face_pitch`; a `pitch_deg` given takes the pitch back); `yaw_deg` replaces a held facing, `face: null` or `face_entity: null` clears it; `interact` held interacts once, when it turns true; `pitch_deg` (-89 to 89) is where the character aims; `fire` held fires its launcher every `rate_s`; a character with a behaviour is taken over until `sim.release`; a character that is down takes no input |
| `sim.fire` | `{entity, yaw_deg?, pitch_deg?}` | `{entity, projectile, magazine: {left, shots, ready_in_s, reloading}}`; fires the entity's launcher now (anything with a `launcher`, a character too); `yaw_deg` defaults to its rotation, `pitch_deg` (-90 to 90) to a character's input pitch, else 0; refused while the launcher is not ready (it says how long to wait) or the entity is down |
| `sim.release` | `{entity}` | `{entity, released, behaviour}`; hands a character an agent took over back to its behaviour |
| `sim.step` | `{ticks? = 1, seconds?}` | `{tick, time_s, hash, characters: [{id, name, position, rotation_y_deg, velocity, grounded, water_depth_m, health, behaviour, magazine}], projectiles: [{id, shooter, position, velocity, speed_mps, age_s}], events: [{tick, event, ...}], fired: [{trigger, tick, event, ok, error?}]}`; whole ticks of 1/60 s, at most 36000 per call; the triggers run after each tick; refused while the terrain names a heightmap that has no samples yet (there is no ground to stand on); at most 1000 events and 1000 fired listed (`events_omitted`, `fired_omitted` count the rest); `health` is `{max, current, armor, down}`, `behaviour` `{state, target?, wait_s?, point?, left_s?, waypoint, taken_over}` and `magazine` `{left, shots, ready_in_s, reloading}`, each null without the component |
| `sim.state` | `{for?}` | `{playing: false}` or `{playing: true, tick, time_s, hash, characters: [...], projectiles: [...], inputs: [{entity, forward, right, yaw_deg, pitch_deg, run, jump, interact, fire, face, face_entity, face_pitch}], objectives, messages: [{text, to, recipients, shown_at_s, until_s}], trigger_errors: [{trigger, tick, error}]}`; the last 20 trigger errors; with `for` (a character) only the messages that character sees, as its player would: those for everyone, for it, or for a tag it has |
| `sim.stop` | `{keep? = false}` | `{tick, kept}`; throws the running world away, or with `keep` makes it the document as one undo step; the journal stays readable until the next `sim.start` |
| `journal.get` | `{from_tick?, to_tick?, limit? = 1000}` | `{recording, world, entry_count, matched, entries: [{index, tick, command, params, end_tick?, hash}], final: {tick, hash} \| null, truncated}`; the journal of the running (or last) simulation: every world command, `sim.input`, `sim.fire`, `sim.release`, `message.show` and `sim.step` sent from outside, with the tick it took effect at and the state hash after it (`end_tick` for a step); queries and the commands of triggers are not recorded; entries whose tick is in `from_tick..=to_tick`, at most `limit` (1 to 10000); before the first `sim.start` `recording: false` and no entries |

While a simulation runs, the World, Entities and Scenarios commands change the running world
(with the same checks) and leave the document alone; `hash` is the state hash, 16 hex digits.

### Scenarios

| Command | Params | Result |
|---|---|---|
| `var.set` | `{name, value?, add?}` | `{name, value, previous}`; `value` a number, boolean or string, or `add` a number to a number variable (a missing one counts from 0); not both |
| `var.remove` | `{name}` | `{name, previous}` |
| `objective.add` | `{id, title, status? = "open", hidden? = false}` | `{id, title, status, hidden}`; `status` is `open`, `done` or `failed` |
| `objective.set` | `{id, title?, status?, hidden?}` | `{id, title, status, hidden}`; only the fields given change |
| `objective.remove` | `{id}` | `null` |
| `trigger.add` | `{id, when, if?, do, once? = true, enabled? = true}` | the trigger; every command of `do` is checked against its spec now (world commands and `message.show` only; `$who` and `$target` only as the whole value of an entity parameter of an `enter`, `leave`, `interact`, `hit`, `down` or `heard` trigger); `if` is `{var, op, value}`, `{objective, status}` (`open`, `done` or `failed`), `{all: [...]}` or `{any: [...]}` |
| `trigger.set` | `{id, when?, if?, do?, once?, enabled?}` | the trigger; only the fields given change, then the whole trigger is checked again |
| `trigger.remove` | `{id}` | `null` |
| `message.show` | `{text, to?, seconds? = 5}` | `{text, to, recipients, shown_at_s, until_s}`; while playing only; `to` is a character (id or name) or `tag:<tag>` for every character with that tag (some character must have it), everyone when left out; `recipients` are the characters it reaches now (null for everyone); listed in `sim.state` until it expires; inside a trigger `{var:name}` in `text` becomes the variable's value and `{who}` and `{target}` the names of the event's entities, when the step runs; a placeholder without a value stays as written and is reported once in `trigger_errors` |
| `gm.order` | `{entity, order, route?, point?}` | `{ordered: [{id, name, mode, route}]}`; changes the mode and route of behaviours and keeps the rest (team, ranges, aim): `order` is `patrol` (walk `route` `[[x, z], ...]`, or the route it has, from its first point), `move` (walk to `point` `[x, z]` and stay: a patrol of one point), `hold` (stand where it is: mode `idle`) or `guard` (back to its post); `entity` is a character, `tag:<tag>` or a list of these (at most 256), so one command moves a squad; a character without a behaviour gets one with the defaults; one that fights or searches finishes first; on the running world as on the document |
| `scenario.inspect` | `{}` | `{playing, tick, vars, objectives, triggers: [{id, when, if?, do, once, enabled, fired}], messages, trigger_errors}` |
| `objective.list` | `{}` | `{objectives: [{id, title, status, hidden}], open, done, failed}` |

A trigger: `when` is `{"event": "enter" | "leave", "zone", "who"?}`, `{"event": "interact",
"target"?, "who"?}`, `{"event": "time", "at_s"}`, `{"event": "var", "name"}`, `{"event":
"start"}`, `{"event": "hit" | "down", "who"? (the shooter), "target"? (the entity hit or
downed)}` or `{"event": "heard", "who"? (the listener), "target"? (the shooter)}` (`zone`,
`target` and `who` are entity ids or names, or `"tag:<tag>"`); `if`
is `{"var", "op": "==" | "!=" | "<" | "<=" | ">" | ">=", "value"}`, `{"all": [...]}` or
`{"any": [...]}`; `do` is `[{"command", "params"?}]`. Events in a step's answer: `{tick, event:
"enter" | "leave", who, zone}`, `{tick, event: "interact", who, target}`, `{tick, event: "time",
at_s}`, `{tick, event: "var", name}`, `{tick, event: "start"}`, `{tick, event: "hit", projectile,
shooter, target (null for the terrain), point, normal, speed_mps, energy_j}`, `{tick, event:
"down", who, target}`, `{tick, event: "heard", who, target, point}`. A trigger's commands run as one
all-or-nothing transaction on the running world; a failure is reported (`fired` with `ok:
false`, `trigger_errors`) and changes nothing. Triggers fired by triggers nest at most 32 deep.

### Queries

| Command | Params | Result |
|---|---|---|
| `world.inspect` | `{entities? = true, limit? = 200, offset? = 0}` | below |
| `prefab.list` | `{}` | `{prefabs: [{id, name, components, children, instances: [ids], instance_count}]}`; in the order given |
| `entity.get` | `{id}` | `{id, name, position, rotation_y_deg, scale, prefab, overrides, parent, prefab_child, children, layer, components, ground_height_m, height_above_ground_m, body, input, health, behaviour, magazine}`; `prefab` (the prefab's id or null), `overrides` (its components that differ from its prefab's), `parent` (an id or null), `prefab_child` (which child of its parent's prefab it is, or null) and `children` (ids), `layer` (its layer, null for the default one); `components` are the effective ones; `body` (`{velocity, grounded}`) and `input` while playing, for a character, else null; `health` (`{max, current, armor, down}`) with a `health` component; `behaviour` (its state) and `magazine` (its launcher's) while playing |
| `entity.find` | `{name?, component?, tag?, near?, radius_m?, limit? = 50}` | `{total, entities: [{id, name, position, components, distance_m?}]}`; part of the name in any case, a component type, a tag (exactly), a horizontal circle round `near` (`[x, z]`); by distance with `near`, else by id |
| `query.ground_height` | `{x, z}` | `{x, z, height_m, normal, slope_deg, surface, water_level_m, water_depth_m}`; the terrain only, nulls outside it; `surface` is `ground`, or `water` where a `water` component covers the point above the ground (then `water_level_m` is the surface's height and `water_depth_m` how deep the ground is under it; both null on dry land) |
| `query.ground` | `{points}` | `{points: [{x, z, height_m, normal, slope_deg, surface, water_level_m, water_depth_m}, ...]}`; `query.ground_height` for a list of `[[x, z], ...]` (1 to 65536), one answer per point in the given order, nulls outside the terrain; for the generator of a large map, before anything is simulated |
| `query.raycast` | `{origin, direction, max_m? = 1000}` | `{hit: false}` or `{hit: true, kind: "terrain" \| "entity", entity?, name?, point, normal, distance_m}`; the nearest of the terrain, every road surface (`entity` is the road's id: "is this point on asphalt?") and every shape, solid or not; plants and characters are not hit yet; a ray that starts inside a shape hits at distance 0; only the entities whose footprint the ray's ground projection crosses are looked at (the spatial index): on a world of 300 000 entities about 200 a kilometre of ray, 0.6 ms |
| `query.line_of_sight` | `{from, to}` | `{clear, distance_m, blocked_by?: {kind, entity?, name?, point, distance_m}}`; the same hits as `query.raycast`, between two points, through the spatial index |
| `query.aim` | `{from, to: [x, y, z] \| entity, launcher?}` | `{yaw_deg, pitch_deg, time_of_flight_s, drop_m, drift_m, reachable, miss_m, from, target}`; the firing solution of the launcher of `from` (or of `launcher`, fired from `from`'s muzzle) at a point or an entity (a character's chest), in the current air and wind: trial shots flown with the simulation's own flight model until one passes within 2 cm, at most 40; the low solution only (pitch at most 45 degrees); `reachable: false` beyond the projectile's flight; obstacles, dispersion and the target's motion are not considered |
| `species.list` | `{filter?, category?, limit? = 100}` | `{total, species: [{id, common_name, botanical_name, category, mature_height_m, mature_width_m, evergreen}]}`; the plant library |
| `gm.overview` | `{near?, radius_m? = 100, team?}` | `{playing, tick, time_s, hash, characters: [{id, name, team, position, health, down, mode, state, target?, sees, taken_over}], characters_omitted?, teams: [{team, alive, down}], objectives, events, names: {id: name}, messages, weather: {time_of_day_h, sun_elevation_deg, cloud_cover, mist, rain, wind_direction_deg, wind_speed_mps, temperature_c, light, visibility_m}, vars, projectiles}`; the game master's view: every character (at most 200; `near` `[x, z]` with `radius_m` and `team` narrow the list, the teams count everyone) with its behaviour's state and target, whom it sees (the simulation's perception) and whether an agent took it over; the last 50 events with their ticks; numbers rounded to centimetres; while editing the document, with `tick`, `state` and `sees` null |
| `gm.sight` | `{from}` | `{from, name, eye, facing_deg, range_m, fov_deg, light, visibility_m, characters: [{id, name, team, enemy, down, distance_m}], entities: [{id, name, kind, distance_m}], entities_omitted?}`; who and what a character sees now, by the simulation's perception (view range shortened by night, mist and rain, field of view, a clear line); `entities` are shapes, zones, interactables and lights whose middle it sees, nearest first, at most 100 |
| `component.list` | `{}` | `{components: [{type, summary, fields: [{name, type, unit, default, doc}], example}]}`; every component type with its fields and defaults |

`world.inspect` (MCP: `doc_inspect`, and the resource `gamecraft://world`) describes the current
world, the running world while playing: `{name, seed, revision, playing, tick (null when not
playing), terrain: null | {size_m, cell_m, source, cols, rows, min_height_m, max_height_m},
environment, sun: {azimuth_deg, elevation_deg, direction, above_horizon}, entity_count, vars,
objectives, trigger_count, prefabs: [{id, name, components, children, instances,
instance_count}], entities: [{id, name, position, rotation_y_deg, scale, prefab?, parent?,
components}], truncated, history: {can_undo, can_redo}}`. An entity placed from a prefab shows
`prefab` and its effective components (the prefab's with its overrides); a prefab's child shows
its `parent`. Entities are listed in id order, `limit` (1 to 1000) at a time from
`offset`; `truncated` says whether more follow; `entities: false` leaves the list (and
`truncated`) out. `source` is the terrain's recipe as in the file format; `components` are as
in the file format.

### The host's commands

| Command | Params | Result |
|---|---|---|
| `file.new` | `{name? = "Untitled", seed? = 0}` | `{name, seed}`: an empty world, a new history, any simulation stopped, the open file forgotten |
| `file.open` | `{path, pack?}` | `{path, name, entities, terrain, cache, pack}` (`terrain`: whether the world has one; `cache`: `{used, path, ignored, written, source_bytes, ms}`, the cooked cache `.gamecraft-cache/<file>.cooked` beside the file, used when made from the very same bytes, else ignored with the reason and written again where the host may write; never an error; `pack`: `null`, or `{path, id, packs, found, assets}` of the pack the world's models come from, `found` being `given`, `default` or `above`); a `.gamecraft` file inside the read root, at most 512 MiB by default (a safety net the host raises: `gamecraft-cli --max-world-mb N`, `GAMECRAFT_MAX_WORLD_MB`, `Access::max_world_bytes`; the error names the cap and how to raise it); a new history, any simulation stopped. **The pack of a world file:** the models of a world file are assets of a pack, found as most engines find the project a scene belongs to: the folder `pack` names, else the host's own (`gamecraft-cli --pack DIR`, `Host::with_pack`), else the nearest `pack.json` above the file, searched up to the read root; `pack: null` wants none. That pack and the packs it depends on are read for their assets only, each through the licence gate (a broken one is an error naming the file, and the session stays as it was), the models are measured and their bounds reach the session, so a solid model collides and a picture draws it |
| `file.save` | `{path?, overwrite? = false}` | `{path, bytes, cache_bytes}`; also writes the cooked cache of what it wrote (`cache_bytes`, or `{error}`, never fatal); writes the document (never the running world) as `.gamecraft` inside the write root; without `path` the open file; an existing other file needs `overwrite` |
| `edit.undo`, `edit.redo` | `{}` | `{can_undo, can_redo}`; refused while a simulation runs and when there is nothing to undo or redo |
| `journal.save` | `{path, overwrite? = false}` | `{path, bytes, entries, final: {tick, hash}}`; writes the journal of the running (or last) simulation as a `.json` file inside the write root: `{gamecraft_journal: 1, world, entries, final}` (`docs/design/journal.md`); an existing file needs `overwrite`; `gamecraft-cli replay FILE --check` runs it again |
| `view.render` | `{path?, width? = 960, height? = 540, camera?, follow?, time_s?}` | `{mime: "image/png", base64, width, height, renderer, camera, stats, problems}`, or with `path` `{path, bytes, width, height, renderer, camera, stats, problems}`; `stats` is `{triangles, draws, instances, forward_triangles, models}` (see *Pictures*); when models of the world have no pack that provides their files, `problems` carries one sentence naming them, why and what to do (see *Pictures*) |
| `project.new` | `{path, id, name, world? = "main", cell_size_m?}` | `{path, id, world, files, index}`; a new project folder inside the write root (new or empty) from the document: `pack.json`, `worlds/<world>/world.json`, one `worlds/<world>/layers/<layer>.json` per layer (entities without a layer in `main`), one `prefabs/<id>.json` per prefab; with `cell_size_m` (16 to 16384 m) each layer is a folder of cells `layers/<layer>/<cx>_<cz>.json` instead; the project is then open, with nothing to save |
| `project.open` | `{path, world?, cells?}` | `{path, root, world, worlds, packs, entities, cell_size_m, cells, warnings, index, terrain}`; with `cells` (`[[cx, cz], ...]`) only those cells of layers split into cells are loaded (layers of one file load whole) and the project refuses to save; the terrain is built from the tile cache (`worlds/<world>/terrain/cache`, git-ignored; `terrain: {tiles, from_cache, built, written, cache, problem, ms}`), and a heightmap's samples are read from `worlds/<world>/terrain/heightmap/` (`terrain.heightmap: {tiles, content, from, written, error}`; a missing or bad tile is named there and the project opens without ground, `terrain.error`); the project folder inside the read root and its dependencies (a dependency's `path` relative to the pack naming it, else `packs/<id>` in the project), layered into one world that becomes the document (a new history, any simulation stopped); `packs` in load order; a missing pack or a cycle is named with its chain (`mod -> game -> nature`) |
| `project.save` | `{cell_size_m?}` | `{path, written, removed, unchanged, terrain_tiles_written, heightmap_tiles_written}`; with `cell_size_m` the layers are from now on saved in cells of that size (each entity in the cell of the entity at the top of its parents), 0 for one file per layer, the old files removed; refused for a project opened with only some `cells`; writes what changed in the document since the project was opened or saved, each change to its file in the root pack (an edited entity is one line of its layer file, the terrain's edits `terrain/edits.json`), and the tiles of the cache whose recipe changed; a change to what a dependency brought becomes an override or a removal in the root pack, never an edit of the dependency; needs the project folder inside the write root |
| `project.export` | `{path, overwrite? = false}` | `{path, bytes, entities, pack, needs, missing}`; the open project's world (the document) as one `.gamecraft` file inside the write root, entities with their `layer`. The file refers to its models by their paths inside a pack's `assets/` folder: `pack` is `{path, id, packs}` of the pack that has them, `needs` the files the models need of a pack (`assets/models/crate.glb`, the textures of the store, each with its licence record) and `missing` the models no pack has. The exported file draws its models when it lies inside that pack or is opened with `--pack DIR` (or `file.open` `pack`) |
| `project.info` | `{}` | `{path, root, world, worlds, default_layer, packs, layers: [{name, entities, default}], prefabs: [{id, pack, instances}], entities, changes: {entities_added, entities_changed, entities_removed, prefabs_changed, world_fields_changed, terrain_edits_changed, files, removes}, warnings, index}`; `changes.files` are the files `project.save` would write |
| `pack.add_dependency` | `{id, path}` | `{id, path, packs, entities}`; the root pack depends on the pack `id` in the folder `path` (relative to the root pack); writes `pack.json` and opens the project again; refused with unsaved changes, and when the pack is missing, has another id or makes a cycle (`pack.json` stays as it was) |
| `project.cells` | `{min?, max?, center?, radius_m?, cell_size_m?}` | `{cell_size_m, count, cells: [[cx, cz]]}`; the cells that intersect the area from `min` to `max` (`[x, z]`) or the square around `center` reaching `radius_m`; the cell size is the open project's or `cell_size_m`; at most 65536 cells |
| `project.query` | `{sql, limit? = 1000}` | `{columns, rows, truncated, index}`; one read-only `SELECT` on the open project's index (rebuilt first when the world changed); anything else is refused; at most 1000 rows; native only (in the browser an error says so) |
| `terrain.import` | `{path, format?, cols?, rows?, scale_m? = 1, offset_m? = 0, size_m?, cell_m?}` | `{path, format, cols, rows, content, min_height_m, max_height_m, tiles, stored, tiles_written, tiles_removed}`; replaces the terrain with real ground read from an elevation file in the read root: a PGM (`.pgm`), rows of text (`.txt`), 16-bit integers (`.u16`) or 32-bit floats (`.f32`); `format` left out, the extension says; `cols` and `rows` for `.u16` and `.f32`; `scale_m` and `offset_m` for the raw formats (height = number * scale_m + offset_m); the result names the heightmap (`content`) and the engine gets its samples; with a project open inside the write root they are written to `worlds/<world>/terrain/heightmap/` (`stored: "project"`), else they live in the session only (`stored: "session"`: make or save a project to keep them); size, cell size, layers, paint rules and edits stay; a file that is not what it claims changes nothing |
| `asset.import` | `{file, to, license, author, source_url, title?, changes?, textures?}` | `{asset, pack, files, record, license, bytes, triangles, bounds: {min, max}, textures: [{path, bytes, stored, record}]}`; imports a `.glb` (or a `.gltf` with the buffers and images it refers to) from the read root into the open project's root pack as `assets/<to>` and writes `assets/<to>.license.json`; a `.gltf`'s buffers go beside it in its own folder (one anywhere else is refused: move the file beside the model), and a model that names `EXT_meshopt_compression` or an extension that refers to buffers is refused (export it uncompressed); the images go into the pack's texture store `assets/textures/<content-hash>.<ext>`: an image the pack has already is reused when the file's bytes are that image (nothing is stored again, its record stays; any other bytes under that name are an error), a new one is stored with `textures` as its record (`{license, author, source_url, title?, changes?}`; its licence, author and source left out are the model's), and the stored model refers to them, so it draws exactly as before; `license` is `CC0-1.0` or `CC-BY-4.0`; the model is read first (a broken one is refused, nothing written), an existing `to` is refused; solid models that use it collide from then on; needs the project folder inside the write root; native only |
| `asset.list` | `{}` | `{assets: [{path, pack, kind, license, author, source_url, title, changes, bytes, files, triangles, bounds: {min, max}, lods: [{level, triangles, until_m}], draw_distance_m, lod_source, lod_warnings, problem, used_by}], textures: [{path, pack, bytes, license, author, source_url, title, changes, used_by}], missing: [{asset, entities}]}`; the assets of the open project's packs, or of the pack of the open world file (a later pack's replaces an earlier one's of the same path; nothing, and every model of the world under `missing`, for a world file without a pack), models measured (metres, the asset's own axes) and how many entities of the world shown use each (a model's `bytes` are its own files; a shared texture is counted once, as its own); a model's levels of detail `lods`, finest first (`triangles` of each and `until_m`, where it ends as the file says, else by the rule from its size; the last end is `draw_distance_m`; a component's `lod_distances_m` and `draw_distance_m` are its entities' own), `lod_source` `names`, `MSFT_lod` or `MSFT_lod+names` (null for one level) and `lod_warnings` about a broken set, while `triangles` counts the finest level; the textures of the packs' store apart, each once, `used_by` the models that refer to it; and the model assets the world names that no pack has; `kind` is `model`, `texture` or `file`; native only |
| `asset.credits` | `{}` | `{text, assets: [{path, kind, title, author, license, source_url, changes}]}`; the attribution a game must show: one line per CC-BY asset (`"Title" by Author, CC BY 4.0 (url), from source.`) of the open project or of the open world file's pack, models and textures alike, each once; empty without one; native only |

- **Files.** A relative path is inside the root, an absolute path must lie inside it. Refused,
  each with a sentence that says why: no root set (the sentence names the flag,
  `--automation-read-root DIR` or `--automation-write-root DIR`), `..` anywhere in the path, a
  path or a link that leads out of the root (the existing parent folder is resolved with its
  links), a folder that does not exist, an extension other than `.gamecraft` (`.png` for
  pictures, `.json` for journals). A save writes a temporary file beside the target and renames it, so a failed save
  never leaves half a file. A file that cannot be read, is too large, is not UTF-8 or is not a
  valid world is an error, and the session stays exactly as it was.
- **The pack of a world file.** A world file refers to its models by their paths inside a pack's
  `assets/` folder. `file.open` finds that pack as most engines find the project a scene belongs
  to: the folder `pack` names, else the host's own pack (`gamecraft-cli --pack DIR`,
  `Host::with_pack`), else the nearest `pack.json` above the file, searched up to the read root;
  `pack: null` wants none. Its assets, and those of the packs it depends on, are read as an open
  project's are (the licence gate included) and belong to the host until the next `file.new`,
  `file.open` or `project.open`: `asset.list` and `asset.credits` list them, the renderer is
  handed their files, and the models' bounds reach the session.
- **Pictures.** `view.render` draws `current_world` (the running world while playing) with its
  height field. `camera` is `{position: [x, y, z], target: [x, y, z], fov_y_deg? = 60}` (1 to
  179 degrees, target not equal to position); `follow` is `{entity, view: "first_person" |
  "third_person" = "third_person", distance_m? = 6}` (0.5 to 1000 m, third person only), the
  entity by id or name; at most one of the two, and neither frames the whole world. Width and
  height are 16 to 4096 pixels. `time_s` (at least 0) is the animation clock of wind and water:
  the simulation's time while playing, else 0. `info` is what the renderer says about the
  picture (the camera it used, counts, problems), merged into the answer. A model changes
  level with its distance and is not drawn beyond its draw distance; `stats` says what was
  drawn: `triangles`, `draws` and `instances` over all passes, `forward_triangles` of the view
  alone, and `models`, one row per model item (an asset as entities with the same distances
  and scale show it, in order of asset path and then of the first entity that shows one):
  `{asset, instances, drawn, culled, until_m: [where each level ends, the last the draw
  distance], levels: [{level, instances, triangles}]}` (`level` is the level's number in the
  file; a level without geometry is left out), counted again by the renderer's rule on the
  CPU (exact for a picture). A character that `follow` names is never culled by distance.
  `stats.hlod` is `{cells, drawn, triangles}`: the cells of 128 m of the ground whose static
  shapes and models are merged into far levels of detail, how many of them drew (farther than
  their own items' draw distances) and the triangles their merged meshes submitted. A picture with `path`
  is a `.png` inside the write root and may replace an earlier one. A model whose files no pack
  provides is a small magenta box with a problem line of its own; `problems` then also carries
  one sentence of the host naming every such model, why (no `pack.json` above the file and none
  given, or the pack has no such file) and what to do (`--pack DIR`, `file.open` `pack`, or
  `asset.import`). Without a renderer the answer is an error that says a renderer is needed.
- **Dirty.** The host knows whether the document changed since it was opened, saved or
  started (`Host::is_dirty`).
- **Projects** (`docs/design/projects.md`). A project is a folder of text files: a pack
  (`pack.json`), its prefabs (`prefabs/<id>.json`) and worlds (`worlds/<world>/world.json` and
  `worlds/<world>/layers/<layer>.json`, one entity per line with a string `id` unique in its
  pack), layered over the packs it depends on. Later packs change earlier ones by id: a prefab
  file replaces the prefab, an entity replaces the entity with its id, `{"id", "removed": true}`
  removes one, a layer file with `"replace": true` drops the earlier layer of its name, and
  `world.json` fields merge field by field. A file of a pack that is a link leading out of its
  folder, a path with `..`, truncated JSON, an id used twice in one pack, a missing pack and a
  cycle are refused with a sentence that names the file or the chain. `file.new` and `file.open`
  close the project. The index lives in `.gamecraft-index/` inside the project folder when that
  is inside the write root (in memory otherwise); deleting it loses nothing. It is rebuilt
  when the project opens, after `project.save` and before a `project.query` when the world
  changed (`project.new`, `project.open` and `pack.add_dependency` cannot be part of an atomic
  batch).

## MCP

`gamecraft-cli mcp` speaks the Model Context Protocol over stdio: JSON-RPC 2.0, one message per
line, revisions `2025-11-25` (offered first), `2025-06-18`, `2025-03-26` and `2024-11-05`
(`initialize` echoes the client's when it is one of these). Server name `gamecraft`. Methods:
`initialize`, `ping`, `tools/list`, `tools/call`, `resources/list`,
`resources/templates/list` (none), `resources/read`, `prompts/list`, `prompts/get`.
Notifications (no `id`) and responses get no reply. JSON-RPC batches (arrays) are refused.

Two kinds of error, as in 3DCraft: a request the server cannot act on (an unknown method,
`-32601`; an unknown tool, an unknown or ill-typed argument, `-32602`; invalid JSON, `-32700`;
not a JSON-RPC 2.0 request, `-32600`) is a JSON-RPC error, raised before anything runs; a
command that ran and failed is a result with `isError: true` and the message as text, so the
model reads it and corrects itself.

### Tools

| Tool | Arguments | Does |
|---|---|---|
| `command_list` | `{filter?, id?, brief? = false}` | `engine.commands`, as text |
| `command_run` | `{id \| command, params?, intent?}` | runs one command; the result as text (a picture of `view.render` without a path as an image block plus the rest as text); `_meta.ms` the wall-clock milliseconds |
| `command_batch` | `{steps: [{id \| command, params?, intent?}], stop_on_error? = true, terse? = false, atomic? = false, intent?}` | `batch`; a batch with a failed step is `isError`, with the report as its text |
| `doc_inspect` | `{entities? = true, limit? = 200, offset? = 0}` | `world.inspect`, as text |
| `doc_render_preview` | `{width? = 640, height? = 360, camera?, follow?}` | `view.render` without a path: an image block (`image/png`) and the renderer's info as text; at most 5 MB of PNG |

### Resources and prompt

| URI | Type | Holds |
|---|---|---|
| `gamecraft://world` | `application/json` | `world.inspect` of the current world |
| `gamecraft://guide` | `text/markdown` | the agent guide, [`agent.md`](agent.md) |
| `gamecraft://guide/game-master` | `text/markdown` | the game master's guide, [`guide/game-master.md`](guide/game-master.md) |

The prompt `build_world {description}` briefs an agent to build the described world with the
commands (plan in numbers, atomic batches), check it with `doc_inspect`, look at it with
`doc_render_preview`, try it by playing and save it. The prompt `direct_scenario {goal}`
briefs an agent to direct the running scenario towards `goal` as its game master: read
`gm.overview` every few seconds of game time, decide, act with world commands, `prefab.spawn`,
`gm.order`, `environment.set` and `message.show`, step, keep it fair and fun, and give each
command an `intent`.

## Limits

| Limit | Value |
|---|---|
| A request line | 1048576 bytes (1 MiB); a longer line is answered with an error and skipped |
| A reply line | 8388608 bytes (8 MiB); a larger reply is an error that says to ask for less |
| Steps in one batch | 256 |
| A world file to open, a file of a project's pack | 512 MiB by default, set by the host (`--max-world-mb`); a project at most 512 MiB together, or the host's cap when it is higher |
| A model or texture asset | 64 MiB |
| A picture | 16 to 4096 pixels a side; a preview's PNG at most 5 MB |
| Entities in one `world.inspect` | 1000 (`limit`), paged with `offset` |
| Ticks in one `sim.step` | 36000 (600 s) |

`methods` lists the first three as `limits: {requestBytes, replyBytes, batchSteps}`.

## Control channel

A loopback control channel to a running window, with the bridge that lets an MCP client reach
it, is planned for M1. Today both servers run in the process that owns the session
(`gamecraft-cli serve`, `gamecraft-cli mcp`).

## Differences from VisionCraft and 3DCraft

- **The commands are GameCraft's**: a world of terrain, environment and entities with
  components, and play commands (`sim.*`) whose world commands steer a running simulation.
  Every listed command has a `scope` (`world`, `query`, `play`, `host`), and a parameter's
  `type` is a one-word name (`point3`, `entity`).
- **Host commands are specs too**: `file.*`, `edit.*` and `view.render` are checked and listed
  exactly as the engine's commands, with the same error sentences.
- **No agent log.** `intent` is accepted and checked, not recorded; there is no `agent.log`
  command.
- **No `stopOnBreak`** (a GameCraft world has no features that break); a batch takes
  `stopOnError`, `terse`, `atomic` and `intent`.
- **`engine.commands` with an unknown `id`** is an error naming the nearest id, not an empty
  list; `command_list` has no `enabled_only`.
- **`command_run` and MCP batch steps** take the command as `id` (as VisionCraft) or as
  `command` (as the JSON-lines protocol); not both.
- **Tools.** `doc_inspect` runs `world.inspect`; `doc_render_preview` takes a picture size, a
  `camera` or a `follow` (no viewpoints, quality or photo settings); there is no window tool
  `control_call`. Resources are `gamecraft://world`, `gamecraft://guide` and
  `gamecraft://guide/game-master`; the prompts are `build_world` and `direct_scenario`.
- **Paths** need a root: there is no unrestricted mode, and the existing parent folder is
  resolved with its links before the check.
- **No control channel yet** (M1).
