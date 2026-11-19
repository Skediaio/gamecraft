# Projects, packs and layers

How a game lives on disk once it is bigger than one test valley, and how mods change it.
Status: designed 2026-11-19 for M3 (`roadmap.md`); decisions in `decisions.md` (*Engine, packs,
games*; *One JSON file is M0's format*; *Text files are the source*).

## Method

### A pack is a directory

```text
skirmish/                      a pack (a game is a pack too)
  pack.json                    the manifest
  worlds/
    village/
      world.json               terrain recipe, environment, scenario (vars, objectives, triggers)
      layers/
        terrain-features.json  one layer: a list of entities
        village.json
        enemies.json
      terrain/                 sculpted and painted terrain, binary tiles (later in M3)
  prefabs/
    soldier.json               one prefab per file
    house.json
  assets/                      models, textures, sounds, each with a .license.json beside it
  scripts/                     (M5)
```

`pack.json`:

```json
{"gamecraft_pack": 1, "id": "skirmish", "name": "Skirmish", "version": "0.1.0",
 "depends": [{"id": "nature", "version": ">=0.1"}],
 "worlds": ["village"], "license": "MIT", "authors": ["..."]}
```

A **game** is a pack with `worlds`. A **mod** is a pack that depends on the game and changes it.

### Layering

Packs are loaded in dependency order; a later pack overrides an earlier one by **id**:

- a prefab file with the same id replaces the prefab (a mod rebalances the soldier);
- a layer file with the same path in the same world replaces that layer, and a new layer path
  adds one (a mod adds a layer `more-enemies.json`);
- an entity in a later layer with an `id` that already exists replaces that entity, and
  `{"id": "...", "removed": true}` removes it;
- `world.json` fields are merged field by field (a mod changes only the weather).

Entities in pack files carry **string ids** that are unique within their pack (`"village/house-3"`);
the engine maps them to its numeric runtime ids when the world loads, and keeps the mapping so
saving writes every entity back to the layer file it came from. New entities go to the layer the
command names (`entity.add {layer}`), or to the world's default layer.

### Loading and saving

- `project.open {path}` loads a pack and its dependencies from a search path (the project's own
  `packs/` directory, then the engine's built-in packs), resolves the layering into one `World`,
  and remembers where each entity, prefab and world field came from.
- `project.save` writes only what changed, each change to the file it belongs to, in the
  owning pack; changes to things that came from a dependency go into the project's own pack as
  overrides (so editing a game you depend on produces a mod, never edits the game).
- Files are pretty-printed with a stable order, one entity per line in layer files, so diffs show
  exactly what changed.
- `project.export {path}` writes the resolved world as one `.gamecraft` file (M0's format), which
  stays the format for tests, examples and exchange.

### The index

When a project opens, the engine builds a Turso database beside it (`.gamecraft-index/`, ignored
by git) with one row per entity, prefab and asset: id, name, tags, components, layer, pack,
position. `project.query {sql}` runs a read-only SQL query against it, so an agent asks "every
enemy within 100 m of the village" in one line. It is rebuilt when files change, and deleting it
loses nothing.

## As built

Built 2026-11-19 (M3): `gamecraft-project` (L2, pure: maps of path to text in, a `World` and its
provenance out, and back), `gamecraft-index` (L4, native only, Turso), the host commands
`project.new`, `project.open`, `project.save`, `project.export`, `project.info`,
`pack.add_dependency` and `project.query` in `gamecraft-automation`, `gamecraft-cli project
new|open|info|export|query` (and `run`, `play`, `info`, `render` take a project folder), and
`examples/skirmish/project` with the mod `examples/skirmish-night-mod`.

What the files are, where it differs from the method above or adds to it:

- **A layer file with the same path merges by id; `"replace": true` replaces the layer.** The
  method says the same path replaces the layer. Then a save could not write an override of a
  game entity into the mod's layer of the same name without dropping the rest of the game's
  layer, and the override would have to move to another layer. So a layer file of a later pack
  adds, replaces (by `id`) and removes (`{"id", "removed": true}`) entities of the layer of the
  same name, and `{"gamecraft_layer": 1, "replace": true, "entities": [...]}` first drops what
  earlier packs put in that layer. An entity's layer is the file it was last defined in.
- **Layer files** are `{"gamecraft_layer": 1, "entities": [...]}`, one compact entity per line:
  `{"id", "name", "position", "rotation_y_deg", "scale", "prefab"?, "parent"?, "components" |
  "overrides"}`, the `.gamecraft` entity with a string `id` (1 to 200 ASCII letters, digits,
  `-_./:`, no `..`) and the parent by string id. A prefab's children are lines of their own (a
  layer file does not spawn children). Ids are unique within a pack for each world (checked
  across its layers). A new entity gets an id from its name when saved (`Oak 3` -> `oak-3`,
  `-2`, `-3`, ... when taken).
- **`world.json`** is `{"gamecraft_world": 1, ...}` with the world's fields (`name`, `seed`,
  `terrain`, `environment`, `vars`, `objectives`, `triggers`; the name defaults to the folder)
  and two of the project: `default_layer` (default `main`) and `layers`, the order the layers
  load in (those not listed follow by name). Packs merge it as a JSON merge patch (RFC 7386);
  a save writes the root pack's `world.json` as the patch from what the dependencies give to the
  current world, so a mod's file holds only what it changes.
- **Prefab files** are named after the prefab's id (`prefabs/<id>.json`, the id ASCII letters,
  digits, `-_.`); `{"id", "removed": true}` removes the prefab of an earlier pack.
- **`pack.json`** takes `{"gamecraft_pack": 1, "id", "name", "version", "depends": [{"id",
  "version"?, "path"?}], "worlds", "license"?, "authors"?}`. The search path: a dependency's
  `path` relative to the pack that names it, else `packs/<id>` in the project. There are no
  built-in packs yet. Versions are not checked (*Open*).
- **Runtime ids** follow the load order: packs in dependency order, layers in `layers` order,
  entities in file order; an entity replaced by a later pack keeps its place. The Skirmish's
  `layers` keep the order of `world.gamecraft`, so its project resolves to the same world with
  the same ids (a test compares them).
- **Provenance** (`Project::provenance`): every entity's string id, pack and layer, every
  prefab's pack, and the pack that set each top-level `world.json` field last. `project.save`
  plans the files from it (`Project::plan_save`): an entity of the root pack is rewritten in its
  line, in place; an entity of a dependency that changed (or moved layer) becomes an override
  line in the root pack's layer of its name, one that was removed a removal line; a changed
  prefab is written whole; a removed one is deleted, or removed by a marker when a dependency
  brings it. What is compared is the file form against what the files hold, so a hand-written
  file in another layout is not rewritten by a save that changes nothing in it. Output is
  stable and pretty; the committed Skirmish files are byte for byte what a save writes (a
  test).
- **Hostile files.** Truncated JSON, an id used twice in a pack, a path with `..` (and a layer,
  world or prefab file whose name is not a plain name), a `default_layer` that is not one, a
  parent that does not exist, a missing pack and a cycle (both named with their chain, `mod ->
  game -> nature`) are errors that name the file; a file or folder of a pack that is a link
  out of the pack, and a dependency outside the read root, are refused by the host. Every
  truncation of every file of a project is an error, never a panic (a test).
- **The entity's `layer`** (`Entity::layer`, `EntityDraft::layer`, `Spawn::layer`): a plain name
  of 1 to 64 ASCII letters, digits, `-` and `_` (it becomes a file name), `None` for the world's
  default layer. `entity.add`, `entity.set` (with the entity's children) and `prefab.spawn` take
  it, `entity.get` and `world.inspect` show it, and the `.gamecraft` format keeps it. A loaded
  entity carries its layer's name.
- **The index** has the tables `entities(id, key, name, layer, pack, prefab, parent, x, y, z,
  rotation_y_deg, scale, tags, components)`, `entity_tags(entity_id, tag)`,
  `entity_components(entity_id, type, data)` and `prefabs(id, name, pack, components,
  instances)` (assets later). It is built when a project opens, after `project.save`, and
  before a `project.query` when the world shown changed since (the world's view revision),
  rather than after every command: a full rebuild of 10 000 entities takes about 600 ms on the
  build server (a tag query over them about 3 ms), too long for every `entity.set` of a batch,
  and the answer is the same. It lives in `.gamecraft-index/index.db` (with a `.gitignore`)
  when the project folder is inside the write root, in memory otherwise. `project.query` takes
  one `SELECT` (no `;` outside quotes and comments, no `load_extension`) and runs it on a
  connection set to `query_only`; at most 1000 rows.
- **Turso** 0.8.3 with `default-features = false`: no full-text search (tantivy), no mimalloc.
  The `aegis` cipher builds as pure Rust (`pure-rust-crypto`, on by default in Turso's SDK kit).
  One piece of C remains: `simsimd` (Apache-2.0, `c/lib.c`, vector distance kernels for
  Turso's vector functions), enabled by `turso_core`'s default feature `simd`, which the
  `turso` crate turns on and a user of it cannot turn off. Turso's API is async; a local
  database needs no runtime, but a step that waits for its file returns `Pending` after doing
  the I/O itself, and nothing in Turso's API promises that the waker is called then. So the
  index polls again with a no-op waker until the future is ready (`native::block_on`, capped)
  rather than use `pollster`, which parks the thread until a wake-up.

### Terrain

Built 2026-11-19 (M3): sculpting and painting as edits over the terrain's recipe
(`gamecraft_world::sculpt`), tiles and their binary cache (`gamecraft_world::tiles`), the
commands `terrain.sculpt`, `terrain.paint`, `terrain.layers`, `terrain.paint_rule`,
`terrain.remove_paint_rule`, `terrain.edits` and `terrain.remove_edit`
(`gamecraft-engine/src/terrain_cmds.rs`), the files of a project, and the painted layers in the
renderer.

- **The recipe.** `Terrain` keeps its source (`flat` or `noise`) and gains `layers` (at most 8
  `{name, color, roughness}`, the first under everything), `paint_rules` (`{layer,
  slope_min_deg?, slope_max_deg?, height_min_m?, height_max_m?}`, soft edges of 4 degrees and
  4 m, applied in order) and `edits`, an ordered list: `raise`, `lower` (`center`, `radius_m`,
  `strength_m`, `falloff`), `flatten` (`height_m`, `strength`), `smooth` (`strength`,
  `iterations` 1 to 8, a 3 x 3 mean per pass), `ramp` (`from`, `to`, `width_m`,
  `from_height_m`, `to_height_m`: a straight slope with round ends, for roads), `noise`
  (`strength_m`, `feature_m`, `seed`) and `paint` (`layer`, `strength`). A brush is full
  strength inside `radius_m * (1 - falloff)`, a smoothstep down to nothing at `radius_m`;
  nothing changes at or beyond it. The three lists are left out of a file when empty, so a world
  without them writes the same text and hashes the same.
- **Every edit is explicit.** A flatten without `height_m` and a ramp without its end heights
  take them from the ground when `terrain.sculpt` runs, and the edit keeps the numbers (the
  brief allowed "to the height at the centre" as a live reading). So an edit reads only the
  sample it changes (a smooth its neighbours within `iterations`), removing an earlier edit
  never moves a later flatten's level, and the order still matters where it should (a raise
  after a flatten lifts the level, before it is flattened away; raises and lowers commute).
- **Heights, then weights.** The height of a sample is the source plus the height edits in
  order, in `f64`, stored as `f32`; the weights are the rules over the final heights (the slope
  from central differences, as the renderer's normals) and then the paint edits in order, as
  shares that sum to 1 and become bytes summing to 255 exactly (largest remainder). Only
  `+ - * /`, `sqrt`, `floor`, the noise and `patan`: the same bits everywhere.
- **Tiles.** The field is cut in tiles of 128 cells (129 x 129 samples, neighbours sharing their
  border samples). Any rectangle evaluates exactly: going backwards through the edits it grows by
  `iterations` for every smooth that reaches it, the source is sampled over that, and the edits
  run forwards. `Terrain::rebuild(previous, field)` finds the edits between the lists' common
  start and common end, spreads their squares through later smooths, and recomputes only the
  tiles they touch (weights: one sample further, for the slope); a different size, cell or
  source builds everything. A sculpt, a removal, undo, redo, a live command and the simulation's
  takeover of a changed running world all rebuild that way; tests compare every rebuild with a
  full build bit for bit (40 random rounds of inserts and removals) and count the tiles (a small
  crater in a 4 x 4 grid rebuilds one).
- **In a project** the edits are text: `worlds/<world>/terrain/edits.json`,
  `{"gamecraft_terrain_edits": 1, "edits": [...]}` one edit per line, beside `world.json`, which
  keeps the rest of the terrain recipe (layers and rules included). A later pack's file
  replaces the edits of earlier packs as a whole; a save writes the root pack's file when the
  edits differ from what the files give. Edits for a world without terrain, a bad op, a number
  out of range or a layer that is not one are errors that name the file.
- **The tile cache** is binary and derived: `worlds/<world>/terrain/cache/tile_<x>_<z>.bin` in
  the root pack's folder with a `.gitignore` of `*`. A file is the 8 bytes `GCTILE1\n`, a key
  (u64), the tile's x and z, its columns and rows and the number of layers `n` (u32 each), then
  the heights (f32) and `n` weights per sample (u8), little-endian. The key is a hash of the
  grid, the source, the layers and rules and the edits that reach the tile (with their places),
  so a tile is stale exactly when its content would change. `project.open` reads the current
  tiles, builds the missing, stale or broken ones (a truncated file, another key, a NaN, weights
  that do not sum to 255) and writes them when the folder is in the write root; `project.save`
  writes the tiles whose key changed. Its answer says how many tiles came from the cache.
- **Size.** The sample limit stays 2049 per side, which is what a 4 km terrain at 2 m and an 8
  km terrain at 4 m need (the limit already allowed both; nothing was lifted). Measured on the
  build server (one thread): a 4096 m terrain at 2 m (2049 x 2049, 256 tiles) with noise, a
  200 m raise, a 3 km ramp, two layers and a slope rule builds in about 2 s (1.9 s optimised,
  2.3 s in the test profile), a crater rebuilds its one tile in 45 ms, and loading it from a
  full cache (25.6 MB in 256 files) takes 0.1 s; the field holds 17 MB of heights and 34 MB of
  weights, and the test's peak resident memory, with two fields and the cache files in memory,
  is 198 MB. Building the edits is capped at 2^28 sample updates (`TERRAIN_EDIT_WORK_MAX`),
  next to the hostile-size checks of the grid, the edit count (1024), the layer count (8) and
  the rule count (64).
- **Queries and the simulation** read the edited field without change: `query.ground_height`,
  `query.raycast`, the characters' ground and slope and scatter's slope test all use the one
  `HeightField`. `sim.start` takes the document's field instead of building it again.
- **The renderer** draws painted layers through VisionCraft's terrain layers: each layer's
  colour and roughness (texture maps when assets exist), the field's weights per vertex, the
  apron fading to the first layer, the ground plane in the first layer's colour. A terrain
  without layers is shaded by slope (grass, earth, rock) as before.

## Open

- Asset import commands and licence records.
- Version ranges and conflicts between two mods that change the same thing (last one wins, with a
  warning naming both).
