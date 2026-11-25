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

### A large world is stored in cells

A layer may be split into a regular grid of cells, one file per cell, so no file holds the whole
world, an edit touches the files of the places it changed, and a streamer can later load the
cells near a player (`gamecraft-project/src/cells.rs`):

```text
worlds/fallujah/
  world.json                 {"gamecraft_world": 1, "cell_size_m": 512.0, ...}
  layers/
    town/                    the layer `town`, split into cells
      -6_-3.json             cell (cx, cz) = (-6, -3): a layer file, one entity per line
      -6_-2.json
    markers.json             a layer that is still one file
```

- `cell_size_m` in `world.json` (16 to 16384 m; `DEFAULT_CELL_SIZE_M` is 512) turns cells on for
  the root pack's layers; without it each layer is one file. Cell `(cx, cz)` covers
  `cx * size <= x < (cx + 1) * size`, and the same for z. A cell file is an ordinary layer file;
  its name is written plainly (`3_-2.json`, never `+3_02.json`). A layer is one file or a folder
  of cells, never both.
- **An entity goes to the cell of its root**, the entity at the top of its chain of parents, so a
  prefab instance's children are in their instance's file. A removal marker goes where the
  removed entity stood.
- **Loading reads every cell** and joins them in cell order (x, then z), each cell's lines in
  their order: the authoritative simulation needs the whole world. `Project::load_cells`
  (`project.open` with `cells`) loads only some cells of layers split into cells, for the
  streamer to come; such a project refuses to save. `cells_in_area` (`project.cells`) lists the
  cells an area touches.
- **Saving** splits each layer by cell, writes the cells whose text changed and removes the files
  of cells that became empty; `project.save {cell_size_m}` switches between one file per layer
  and cells (0 for one file), removing the files of the old layout. The same world saves to the
  same bytes, and reopening a saved project gives the same world, ids included.
- The cap on a file of a pack applies to each cell file; at 512 m a cell of a dense city holds a
  few thousand entities, well under it.

Known limits: the runtime ids of a project come from the order of its lines, so an entity that
moves to another cell gets another id at the next open (as a new entity in an earlier layer does
today), and layer files still write every child of a prefab instance in full (the `.gamecraft`
file derives them; `docs/decisions.md`).

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

- **A bake** (2026-11-25): `gamecraft-cli project bake SCRIPT --into DIR` runs a script of
  commands (the steps of `run --script`, read one at a time) and saves the result as a project
  of cells in one step; a new project is written once every step has succeeded. In an existing
  project the bake replaces only what the script owns: the entities of the layers its steps name,
  of the default layer and of the layers the previous bake owned, and the terrain edits the
  previous bake added. What a bake owns is in `worlds/<world>/bake.json`
  (`gamecraft_project::BakeRecord`): `{gamecraft_bake: 1, layers: [names], terrain_edits:
  [before, after]}`, a file of the project like any other. The same script gives the same
  files: the id of an entity of the root pack that is gone is free again, so entities removed
  and added again by a bake take their old ids and their cells are not rewritten. The guideline
  is `guide/generated-worlds.md`; `decisions.md`, *A bake replaces what the script owns*.
- **`entity.remove_many {ids}`** removes a set with its children in one pass, checked once:
  per entity `entity.remove` walks the world once, which a layer of a hundred thousand cannot
  afford.

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

### Assets

Built 2026-11-19 (M3): the `model` component, licence records with the gate when a pack loads,
`asset.import`, `asset.list` and `asset.credits` (`gamecraft-automation/src/assets.rs`), models
drawn with their PBR materials and textures (`gamecraft-render/src/model.rs`), solid models in
the simulation, `examples/assets-demo` (a pack with a generated crate and fence) and three crates
as cover in the Skirmish's village. Added 2026-11-24: the **texture store**, the images the
models of a pack share stored once (`gamecraft-project/src/textures.rs`;
`decisions.md`, *Shared textures are files of the pack*).

- **The component.** `{"type": "model", "asset": "models/crate.glb", "scale"?: 1, "solid"?:
  false, "collider"?: "box" | "none", "lod_distances_m"?: [number, ...], "draw_distance_m"?:
  number}` (the last two are written only when set; *Levels of detail* below): `asset` is a path
  inside a pack's `assets/` folder (1 to
  200 ASCII letters, digits, `-_./`, no `.` or `..` parts) ending in `.glb` or `.gltf`; the
  asset's origin stands on the entity's position, turned by its yaw and sized by `scale` times
  the entity's scale. With the collider `box` rays hit the asset's bounding box (solid or not,
  as they hit any shape), and with `solid` characters are blocked by it like a solid shape box
  (steps included).
- **Bounds are derived data of the world.** The simulation needs each asset's local bounding
  box and must not read files or glTF. So `World` carries `model_bounds` (asset path to
  `ModelBounds {min, max}`), never written to a file and not part of the document's text:
  the host measures every model of the open project's packs when it opens
  (`gamecraft_project::inspect`, in `f64` over the node transforms with plain multiplications
  and additions, so the same bits everywhere), sets them on the world before it becomes the
  document, and after `asset.import` hands the new set to `Session::set_model_bounds`, which
  sets them on the document, every world in the undo history and the running world (derived
  data: no revision, no undo step). A world is cloned into undo steps, a running simulation and
  every command's copy, so the bounds travel with it. The static part of the state hash covers
  the bounds of every asset a `box` collider uses (only when there are any, so other worlds hash
  as before). The browser measures the models it fetches the same way. A `.gamecraft` file
  opened on its own has no assets: its models have no bounds (they do not collide) and are
  drawn as magenta boxes.
- **Licence records.** `assets/<path>.license.json`: `{"license": "CC0-1.0" | "CC-BY-4.0",
  "author", "source_url", "title"?, "changes"?}` (author and source not empty, texts at most
  2000 characters). Loading a pack refuses, naming the file: an asset without a record, a record
  with another licence (or not a record), a record whose asset does not exist, a file name that
  is not a plain asset path, a model that refers to a file that is missing, is not a plain path,
  or climbs out of `assets/`. A URI of a glTF document is against the document's folder
  (`models/`), with `..` resolved (`gamecraft_project::model::resolve_uri`): a model in
  `models/` names a texture of the store as `../textures/<name>`, and one at the top of
  `assets/` as `textures/<name>`. The files a model refers to inside its own folder (a `.gltf`'s
  buffers and images beside it) belong to it and need no record of their own; the textures of
  the store and any file elsewhere (`../sounds/x.ogg`) need one, how a model came to name it. A later pack's asset replaces an earlier one's of the same path (`Project::assets`).
  The host lists `assets/` recursively (16 folders deep at most, links out of the pack refused)
  and reads for the gate the records, the `.gltf` documents and the JSON chunks of the `.glb`
  files (only the chunk, `glb_json_len`) as text; the binary files are read when measured and
  when a renderer needs them.
- **The texture store.** `assets/textures/<content-hash>.<png | jpg>`, one file per distinct
  image. Every glTF model used to embed its own copy of every image: a pack of 29 models was
  20.6 MiB, 18 of them the same 12 JPEGs. `textures::import` now takes a model's images, however
  they come (a buffer view of a `.glb`, a file beside a `.gltf`, a buffer file, a base64 `data:`
  URI), stores each once keyed by the hash of its bytes, and rewrites the document so each image
  is a URI to the store. The views that held only images leave the model: a buffer that loses a
  view and whose bytes the import has (a `.glb`'s binary chunk, a buffer file) is rebuilt from the
  views it keeps and its accessors point at their new indices; a buffer that keeps no view at all
  goes (a `.gltf`'s buffer file that held only images is not written, a `.glb` that held only
  images in its chunk loses the chunk); a buffer that lost nothing is copied as it came.   A
  rewritten model is refused when its document names what the engine cannot read or renumber:
  `EXT_meshopt_compression` in `extensionsUsed` or `extensionsRequired` (compressed geometry),
  an extension object anywhere with a `buffer` key, or a key that names a buffer view outside
  the accessors and the images. The sentence says what and where, and to export the model
  uncompressed. A `.glb` stays a `.glb` (a JSON chunk only, when nothing is left for a binary chunk). The
  file that comes back reads exactly as the one that went in (checked by the import: same
  triangles, bounds and counts), so a model draws as before, and a model without images is stored
  as it came. The hash is FNV-1a, 64 bits, the workspace's own `gamecraft_math::StateHasher`:
  enough to tell a pack's few hundred textures apart, and an import compares an existing file of
  the store byte for byte with the image (a file replaced or renamed by hand, or another image
  whose hash collides, is an error, never a reuse). A texture is an asset with a record of its
  own: `asset.import` writes it from its `textures` parameter (the model's licence, author and
  source for one left out), the first import of an image sets it and a later one reuses it. A
  model's `bytes` are its own files (itself and the files beside it); the textures it refers to
  are counted once, as the textures' own. Models with embedded images, imported before the store
  existed or written by hand, load as before. Limits: the gate checks references pack by pack, so
  a mod's model that shares a base pack's texture gets its own copy in the mod (cross-pack
  sharing is not wanted now); a model at the top of `assets/` has that whole folder as its own,
  so its record still covers any non-model file there, as before the store.
- **Commands.** `asset.import {file, to, license, author, source_url, title?, changes?,
  textures?}` reads and measures the model before it writes anything (a broken one is refused),
  refuses a `to` that exists or lies under `textures/`, takes the images into the store (a new
  one with its record, one the pack has reused once its bytes are the image's and it has its
  record), copies the file and a `.gltf`'s buffers beside it (from inside the read root, under
  their path inside `assets/`, inside the model's own folder or the import refuses it with a
  sentence that says to move the file beside the model, and never a model's path, a record's
  name or a place in the store; a buffer the import rebuilt is written as rebuilt) and writes
  the record. The answer lists each texture with
  `stored` (new or reused). `changes` is an addition to the brief's parameters: CC BY asks for
  it. `asset.list` lists the store apart (`textures`, each once with the models that refer to
  it) and `asset.credits` names a texture once however many models use it, as in
  `control-protocol.md`. The three are native only.
- **Rendering.** The renderer is handed the bytes (`WorldRenderer::provide_asset(path, bytes)`,
  paths inside `assets/`: a model, a `.gltf`'s buffers beside it, the textures of the store;
  `..` in a path is resolved): `view.render` makes the host hand the files of the models shown
  to its `Pictures` first (each once per opened project; `Pictures::provide_asset`), the window's
  game does the same before every frame (`gamecraft PROJECT` opens a project folder), and the
  browser fetches them from `?assets=<url>` or the `assets/` folder beside `?world=<url>`
  (the files a `.glb` refers to too). A model's URIs are resolved against its folder and looked
  up among the handed files (`resolve_uri`). A model is read once per asset (`gltf`, opened through `gamecraft_project::model::open`, which
  splits the GLB itself and checks the indices and accessors the `gltf` crate's reader would
  otherwise panic on), its PNG and JPEG textures decoded with `image` (at most 8192 pixels a
  side; triangle lists, strips and fans are drawn, a strip or a fan as the triangles it stands
  for), one mesh and one material per glTF material, and drawn as one item per asset with an
  instance per entity; it is uploaded again only when a file it was read from changes. glTF
  PBR maps onto VisionCraft's material: base colour (sRGB), metallic-roughness as the packed
  occlusion-roughness-metallic map (red set to 1 unless the occlusion texture is the same
  image), a separate occlusion map, the normal map with its scale, emissive (a glTF emissive of
  1 is 1000 cd/m2; glTF has no unit), alpha modes (mask cut out, and lit as leaves when
  double-sided, as VisionCraft does; blend blended), double-sided; a material named like glass
  is the renderer's thin glass. Not read: emissive textures (the renderer's material has no
  emissive map), texture transforms, skins and animation, `KHR_materials_transmission` (the
  workspace builds `gltf` without it), data: URIs (an import turns data: images into files of
  the store). A missing or broken model, or one whose buffer or texture was not handed, is a
  small magenta box (0.5 m times the scales) at each entity and one problem line naming the
  asset, the file and the entities; it is read again when a file it waited for arrives.
- **Levels of detail** (2026-11-25; `decisions.md`, *Models change level per ground cell*). A
  glTF model may carry lower-detail versions, read by `gamecraft_project::lod` from the file
  itself: by names ending in `_LOD0`, `_LOD1`, ... (a node's or its mesh's, anywhere in the
  file: the nodes of one object name are its levels by their numbers), or by the `MSFT_lod`
  extension (the finest node carries `extensions.MSFT_lod.ids`, the indices of the coarser
  nodes, usually left out of the scene). Every level of a group is drawn where the group's
  finest node in the scene stands, keeping its own rotation and scale but not its
  translation; geometry of no group is drawn at every level; a level without geometry is left
  out (as the last one: nothing is drawn beyond where it would have started); a group's node
  may hold groups of its own. A number named twice cannot be told apart: only the finest
  level is drawn and the rest is left out, with a warning. Each level ends at a distance from
  the camera and the last end is the draw distance: what the component says
  (`lod_distances_m`, `draw_distance_m`), else the file (`asset.extras.gamecraft` with the
  same two fields, else `MSFT_screencoverage`, read as shares of the view's height), else the
  rule from the model's extent across, at a reference view 1080 pixels high at 60 degrees:
  level `k` ends where the extent spans a quarter of the height to the power `k + 1` (3.5, 14,
  55, ... times the extent) and the model is drawn while it spans 4 pixels (234 times: a
  crate 0.9 m on a side to 364 m, a house 15 m across to 3.5 km). The extent is the finest
  level's, over every vertex, strips and fans included; it counts the model's scale and the
  entity's, and the coverage follows it as the rule does. The distances given are metres as
  written, at the quality pictures are drawn at (the highest; a lower live quality switches
  sooner, and `count_models` counts a live view at that quality's share). Entities that
  differ only in scale share an item per bucket of scale (steps of 1.25) where a distance
  follows the scale, computed for the bucket's upper scale so nothing disappears early; with
  every distance given in metres, every scale shares one item. The renderer
  draws each cell of 8 m of instances at the level for its distance, culls cells against the
  view, and lets a cell that crosses the draw distance dissolve rather than pop. What each
  model drew is counted again by the same rule on the CPU and comes with every picture
  (`view.render`'s `stats.models`: per model item `asset`, `instances`, `drawn`, `culled`,
  `until_m` and `levels`, each with the number of its level in the file), from
  `WorldRenderer::count_models` for a live view and `frame_stats` for its last frame. A broken
  set (a coarser level with more triangles, a missing level, an id that points nowhere) is a
  problem line naming the asset; the model is drawn all the same. `asset.list` shows a
  model's `lods` (`level`, `triangles`, `until_m`), `draw_distance_m`, `lod_source` (`names`
  or `MSFT_lod`) and `lod_warnings`, one row per level that draws; its `triangles` and its
  bounding box are those of the finest level that draws, the box the simulation collides
  with. A character the camera follows has an item that is never culled by distance. Not
  read: material-level `MSFT_lod`; no mesh is simplified.

## Open

- Asset kinds beyond models (textures for terrain layers, sounds); assets in the project index.
- Version ranges and conflicts between two mods that change the same thing (last one wins, with a
  warning naming both).
