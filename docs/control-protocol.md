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
`file.open`, `edit.undo` and `edit.redo` cannot be part of one (the batch is refused before
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
| `terrain.set` | `{kind? = "noise", size_m? = 256, cell_m? = 2, height_m? = 0, seed?, relief_m? = 40, feature_m? = 200, octaves? = 5, roughness? = 0.5}` | `{cols, rows, min_height_m, max_height_m}`; a square centred on the origin, `flat` (`height_m`) or seeded `noise` (the other keys); keys of the other kind are refused; entities are not moved |
| `terrain.clear` | `{}` | `null`; no ground |
| `environment.set` | `{time_of_day_h?, day_of_year?, latitude_deg?, wind_direction_deg?, wind_speed_mps?, cloud_cover?, mist?, rain?, temperature_c?, sea_level_m?}` | `{environment: {time_of_day_h, day_of_year, latitude_deg, wind_direction_deg, wind_speed_mps, cloud_cover, mist, rain, temperature_c, sea_level_m}, sun: {azimuth_deg, elevation_deg}}`; only the fields given change; `temperature_c` (at world y = 0, default 15) and `sea_level_m` (the height of y = 0 above sea level, default 0) give the air density projectiles fly through |

### Entities

| Command | Params | Result |
|---|---|---|
| `entity.add` | `{name? = "", position?, at?, rotation_y_deg? = 0, scale? = 1, components? = []}` | `{id, position}`; give `position` (`[x, y, z]`) or `at` (`[x, z]`, the base on the ground) |
| `entity.set` | `{id, name?, position?, at?, rotation_y_deg?, scale?}` | `{id, position}`; only the fields given change; the entities that belong to it (a prefab's children) move and turn with it |
| `entity.remove` | `{id}` | `null`; removes the entities that belong to it too (a prefab's children); the ids are never handed out again |
| `entity.set_component` | `{id, component}` | `null`; adds a component or replaces the one of its type |
| `entity.remove_component` | `{id, type}` | `null`; `type` is a component type (`component.list`) |
| `entity.snap_to_ground` | `{id?}` | `{moved, skipped}`; one entity, or all when `id` is left out |
| `prefab.add` | `{id, name? = "", components? = [], children? = []}` | `{id}`; an entity described once to place many times; `children` (at most 64) are `[{name? = "", offset? = [0, 0, 0], rotation_y_deg? = 0, components? = []}]`, placed relative to each instance (`offset` in metres in its frame: -Z its forward, +X its right); the id must be new |
| `prefab.set` | `{id, name?, components?, children?}` | `{id, instances: [ids]}`; only the fields given change, each as a whole; every instance keeps its overrides and takes the rest from the new prefab; children already placed stay |
| `prefab.remove` | `{id}` | `null`; refused while entities are placed from it, naming them |
| `prefab.spawn` | `{prefab, position?, at?, rotation_y_deg? = 0, name?, overrides? = []}` | `{id, children: [ids], position}`; an entity with `prefab` set, whose components are the prefab's with `overrides` replacing those of the same type (or adding a type the prefab lacks), and one entity per child with the instance as `parent`; `name` defaults to the prefab's; works on a running world too |
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
`{"type": "behaviour", "mode": "idle" | "patrol" | "guard", "route": [[x, z], ...], "team", "view_range_m", "fov_deg", "engage_range_m", "aim_error_mrad", "reaction_s", "search_s"}`.

### Playing

| Command | Params | Result |
|---|---|---|
| `sim.start` | `{}` | `{tick: 0, hash, characters: [ids], events, fired}`; plays a copy of the document; the `start` triggers run |
| `sim.input` | `{entity, forward?, right?, yaw_deg?, face?, face_entity?, run?, jump?, interact?, pitch_deg?, fire?}` | `{entity, forward, right, yaw_deg, pitch_deg, run, jump, interact, fire, face, face_entity, face_pitch}`: the stored input; it holds until replaced, fields left out keep their value; one of `yaw_deg`, `face` (`[x, z]`) and `face_entity`; `face` and `face_entity` are held: every tick, before anybody moves, the character turns towards the point or the entity (a character's chest), and with `face_entity` and a launcher also aims the pitch at it, allowing for the drop in a vacuum (`face_pitch`; a `pitch_deg` given takes the pitch back); `yaw_deg` replaces a held facing, `face: null` or `face_entity: null` clears it; `interact` held interacts once, when it turns true; `pitch_deg` (-89 to 89) is where the character aims; `fire` held fires its launcher every `rate_s`; a character with a behaviour is taken over until `sim.release`; a character that is down takes no input |
| `sim.fire` | `{entity, yaw_deg?, pitch_deg?}` | `{entity, projectile, magazine: {left, shots, ready_in_s, reloading}}`; fires the entity's launcher now (anything with a `launcher`, a character too); `yaw_deg` defaults to its rotation, `pitch_deg` (-90 to 90) to a character's input pitch, else 0; refused while the launcher is not ready (it says how long to wait) or the entity is down |
| `sim.release` | `{entity}` | `{entity, released, behaviour}`; hands a character an agent took over back to its behaviour |
| `sim.step` | `{ticks? = 1, seconds?}` | `{tick, time_s, hash, characters: [{id, name, position, rotation_y_deg, velocity, grounded, health, behaviour, magazine}], projectiles: [{id, shooter, position, velocity, speed_mps, age_s}], events: [{tick, event, ...}], fired: [{trigger, tick, event, ok, error?}]}`; whole ticks of 1/60 s, at most 36000 per call; the triggers run after each tick; at most 1000 events and 1000 fired listed (`events_omitted`, `fired_omitted` count the rest); `health` is `{max, current, armor, down}`, `behaviour` `{state, target?, wait_s?, point?, left_s?, waypoint, taken_over}` and `magazine` `{left, shots, ready_in_s, reloading}`, each null without the component |
| `sim.state` | `{}` | `{playing: false}` or `{playing: true, tick, time_s, hash, characters: [...], projectiles: [...], inputs: [{entity, forward, right, yaw_deg, pitch_deg, run, jump, interact, fire, face, face_entity, face_pitch}], objectives, messages: [{text, to, shown_at_s, until_s}], trigger_errors: [{trigger, tick, error}]}`; the last 20 trigger errors |
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
| `message.show` | `{text, to?, seconds? = 5}` | `{text, to, shown_at_s, until_s}`; while playing only; listed in `sim.state` until it expires; inside a trigger `{var:name}` in `text` becomes the variable's value and `{who}` and `{target}` the names of the event's entities, when the step runs; a placeholder without a value stays as written and is reported once in `trigger_errors` |
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
| `entity.get` | `{id}` | `{id, name, position, rotation_y_deg, scale, prefab, overrides, parent, children, components, ground_height_m, height_above_ground_m, body, input, health, behaviour, magazine}`; `prefab` (the prefab's id or null), `overrides` (its components that differ from its prefab's), `parent` (an id or null) and `children` (ids); `components` are the effective ones; `body` (`{velocity, grounded}`) and `input` while playing, for a character, else null; `health` (`{max, current, armor, down}`) with a `health` component; `behaviour` (its state) and `magazine` (its launcher's) while playing |
| `entity.find` | `{name?, component?, tag?, near?, radius_m?, limit? = 50}` | `{total, entities: [{id, name, position, components, distance_m?}]}`; part of the name in any case, a component type, a tag (exactly), a horizontal circle round `near` (`[x, z]`); by distance with `near`, else by id |
| `query.ground_height` | `{x, z}` | `{x, z, height_m, normal, slope_deg}`; the terrain only, null outside it |
| `query.raycast` | `{origin, direction, max_m? = 1000}` | `{hit: false}` or `{hit: true, kind: "terrain" \| "entity", entity?, name?, point, normal, distance_m}`; the nearest of the terrain and every shape, solid or not; plants and characters are not hit yet; a ray that starts inside a shape hits at distance 0 |
| `query.line_of_sight` | `{from, to}` | `{clear, distance_m, blocked_by?: {kind, entity?, name?, point, distance_m}}`; the same hits as `query.raycast`, between two points |
| `query.aim` | `{from, to: [x, y, z] \| entity, launcher?}` | `{yaw_deg, pitch_deg, time_of_flight_s, drop_m, drift_m, reachable, miss_m, from, target}`; the firing solution of the launcher of `from` (or of `launcher`, fired from `from`'s muzzle) at a point or an entity (a character's chest), in the current air and wind: trial shots flown with the simulation's own flight model until one passes within 2 cm, at most 40; the low solution only (pitch at most 45 degrees); `reachable: false` beyond the projectile's flight; obstacles, dispersion and the target's motion are not considered |
| `species.list` | `{filter?, category?, limit? = 100}` | `{total, species: [{id, common_name, botanical_name, category, mature_height_m, mature_width_m, evergreen}]}`; the plant library |
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
| `file.open` | `{path}` | `{path, name, entities, terrain}` (`terrain`: whether the world has one); a `.gamecraft` file inside the read root, at most 64 MB; a new history, any simulation stopped |
| `file.save` | `{path?, overwrite? = false}` | `{path, bytes}`; writes the document (never the running world) as `.gamecraft` inside the write root; without `path` the open file; an existing other file needs `overwrite` |
| `edit.undo`, `edit.redo` | `{}` | `{can_undo, can_redo}`; refused while a simulation runs and when there is nothing to undo or redo |
| `journal.save` | `{path, overwrite? = false}` | `{path, bytes, entries, final: {tick, hash}}`; writes the journal of the running (or last) simulation as a `.json` file inside the write root: `{gamecraft_journal: 1, world, entries, final}` (`docs/design/journal.md`); an existing file needs `overwrite`; `gamecraft-cli replay FILE --check` runs it again |
| `view.render` | `{path?, width? = 960, height? = 540, camera?, follow?, time_s?}` | `{mime: "image/png", base64, width, height, ...info}`, or with `path` `{path, bytes, width, height, ...info}` |

- **Files.** A relative path is inside the root, an absolute path must lie inside it. Refused,
  each with a sentence that says why: no root set (the sentence names the flag,
  `--automation-read-root DIR` or `--automation-write-root DIR`), `..` anywhere in the path, a
  path or a link that leads out of the root (the existing parent folder is resolved with its
  links), a folder that does not exist, an extension other than `.gamecraft` (`.png` for
  pictures, `.json` for journals). A save writes a temporary file beside the target and renames it, so a failed save
  never leaves half a file. A file that cannot be read, is too large, is not UTF-8 or is not a
  valid world is an error, and the session stays exactly as it was.
- **Pictures.** `view.render` draws `current_world` (the running world while playing) with its
  height field. `camera` is `{position: [x, y, z], target: [x, y, z], fov_y_deg? = 60}` (1 to
  179 degrees, target not equal to position); `follow` is `{entity, view: "first_person" |
  "third_person" = "third_person", distance_m? = 6}` (0.5 to 1000 m, third person only), the
  entity by id or name; at most one of the two, and neither frames the whole world. Width and
  height are 16 to 4096 pixels. `time_s` (at least 0) is the animation clock of wind and water:
  the simulation's time while playing, else 0. `info` is what the renderer says about the
  picture (the camera it used, counts, problems), merged into the answer. A picture with `path`
  is a `.png` inside the write root and may replace an earlier one. Without a renderer the
  answer is an error that says a renderer is needed.
- **Dirty.** The host knows whether the document changed since it was opened, saved or
  started (`Host::is_dirty`).

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

The prompt `build_world {description}` briefs an agent to build the described world with the
commands (plan in numbers, atomic batches), check it with `doc_inspect`, look at it with
`doc_render_preview`, try it by playing and save it.

## Limits

| Limit | Value |
|---|---|
| A request line | 1048576 bytes (1 MiB); a longer line is answered with an error and skipped |
| A reply line | 8388608 bytes (8 MiB); a larger reply is an error that says to ask for less |
| Steps in one batch | 256 |
| A file to open | 64 MB |
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
  `control_call`. Resources are `gamecraft://world` and `gamecraft://guide`; the prompt is
  `build_world`.
- **Paths** need a root: there is no unrestricted mode, and the existing parent folder is
  resolved with its links before the check.
- **No control channel yet** (M1).
