# Decisions

Decisions taken, newest first, each with its reason; then the open ones. The developer steers the
product and the architecture and leaves how it is built to the orchestrator (2026-11-18): an
implementation decision is taken by the orchestrator, written down here with its reason, and
stands until the developer overrules it. A fork in the architecture or the product goes to the
developer as a recommendation.

## Taken

### 2026-11-26: Attachments are a tree on the model component, placed through sockets the host measures

Roadmap item 14 asked for attachments as separate models on nested sockets, with the full rotation
of a socket applied on the render side only. Decisions:

- **The recipe is data of the `model` component**: `attachments: [{socket, asset, scale?,
  attachments: [...]}]`, a tree readably nested, written only when there is one, so a world without
  attachments keeps its text and its hash. Chosen over entities of their own (an attachment is not
  a thing an agent places or finds: it is part of its root entity, moves with it and is removed with
  it; ids, layers and undo for every screw would be noise) and over a flat list with parent
  references (a tree reads as it mounts, and the file is the format people edit).
- **A socket holds one attachment**, so a path (the chain of socket names from the entity's model)
  names one attachment, and a command can act at any depth. `attachment.add` to a mounted socket is
  refused, with the command that swaps; a file with two on one socket is refused.
- **Sockets are derived data** like the bounds: the glTF nodes `socket_<name>` of a model (position
  and full rotation in the asset's axes) are measured by `gamecraft_project::inspect` and handed to
  the session (`Session::set_model_sockets`). The world document never reads the asset. An asset
  whose sockets were not measured (a world opened alone, without its pack) gives no place, and
  `attachment.add` says so instead of guessing.
- **Where an attachment lands** is composed in `f64` through `glam`'s `libm` maths, operation for
  operation, on every platform: the entity's frame (position, yaw, scale and the model's scale), then
  for each attachment of the chain the socket's frame and the attachment's scale. An attachment's
  origin meets its socket and its axes are the socket's: +Y is the outward normal, +Z runs along the
  parent's length.
- **Removing one that holds attachments refuses** unless `cascade` is given: the safe default is the
  one that loses nothing by surprise; an agent that means to take a branch off says so.
- **The simulation treats an attachment as part of its root entity**: no collider, no mass, nothing
  it changes yet; the state hash covers the document, so a world with attachments hashes as it
  reads. The renderer draws each as its own item at the socket's full rotation, following its
  entity between ticks as the entity's own model does; on a downed character it lies with the
  character's model. A hidden character (a first-person view's own body) keeps its attachments: they
  are what the player holds.

### 2026-11-26: Collision from a model's own geometry by `*_collider` nodes or a box per mesh, and hidden shapes

A model collided as its bounding box, so a doorway in a building model was cosmetic. Most
engines let the asset say what it collides with (Unreal's `UCX_`/`UBX_` meshes, Godot's `-col`
suffix, Unity's colliders per mesh) and otherwise take each mesh as a box or a convex hull.

- **The collider is the component's choice**: `collider: "box"` (as before), `"mesh"` (new) or
  `"none"`. Chosen over finer collision for every model automatically: a world that does not use
  the new collider keeps its file text and its hash, to the bit, and a tree of leaf cards or a
  vehicle's mirrors keep their one box. A file may later carry a default for its asset
  (`asset.extras.gamecraft.collider`), as the levels of detail do.
- **`*_collider` nodes when the file has them** (`is_collider_name`: the suffix in any case, on
  the node or on its mesh; the node and everything below it is never drawn, counted or measured
  in the bounds): one box per node. **Both spellings of the suffix are read, `_collider` and
  `-collider`** (the orchestrator's decision: packs already use both, and refusing one would
  make a pack's collision helpers drawn and its colliders the whole box without a word).
- **Else a box per mesh of the finest level that draws**, axis-aligned in the entity's frame: the
  simplest deterministic thing (the same `f64` products as the bounds, on every platform), cheap
  for the spatial index (boxes are what it holds) and enough for a building made of wall, floor
  and lintel meshes. Chosen over convex hulls (a hull needs a new dependency or a solver, and a
  hull of an L-shaped wall fills the corner again) and over trimesh collision (character against
  triangles is M4). A mesh turned in its file gets the box around it there, so collider nodes
  are best drawn as axis-aligned boxes. More than 256 parts fall back to the bounding box with a
  sentence, and until the host hands the parts over the model collides as its bounding box:
  never less collision than before.
- **The parts are derived data** like the bounds: measured by `gamecraft_project::inspect`,
  handed to the session (`Session::set_model_parts`: the document, the history and the running
  world), placed by the entity's yaw and scale, one solid per part, in the index with the
  entity's footprint, hit by rays as the nearest part. The static hash writes them (a mark of
  their own) only for models with the `mesh` collider, so no hash made before moves.
- **A shape with `visible: false`** is a collision helper: written only when false (so no file
  or hash moves), drawn nowhere (items, far levels), and kept by everything that collides or
  casts. It takes `entity.add`, `entity.set`, `entity.set_component` and prefab children like
  any field of a shape. Chosen over a separate "collider" component (a shape already is the
  collider: only its drawing is unwanted).

### 2026-11-26: Water is a component, an outline and a level; wading slows by depth and deep water is refused

The roadmap promised "water: sea level, lakes, rivers, through VisionCraft's water". What
existed: `environment.sea_level_m`, which is a datum (how high world y = 0 is above sea level,
for the density of the air projectiles fly through), not a surface; VisionCraft's renderer draws
one water surface per scene, a level with an optional outline of 64 points; its own world model
has one sea per project, with a knee-deep limit where walking stops. So the smallest addition
that uses all of it:

- **A `water` component**: an entity is an area of water, a flat surface at `level_m` over an
  `outline` (3 to 64 points in the entity's frame) or everywhere. Chosen over one water surface
  per world (VisionCraft's model): an entity gets ids, names, layers, undo, prefabs, replication
  and pack overrides for free, the simulation and the queries can know any number of areas (a
  town's river and its lakes), and the game master adds a pond with `entity.add`. The renderer
  draws one surface, as it can: the sea when there is one, else the largest area, and names the
  others in every picture's problems. When VisionCraft draws several, the component is ready.
- **`water.add` and `water.set`** take a world height and world points, as `road.add` and
  `road.set` do, and keep them in the entity's frame (the entity stands at the outline's centre
  on the surface). `entity.add` takes the component as given.
- **Wading, by depth**: full speed up to 0.5 m (knee-deep), falling linearly to 0.4 of it at
  1.5 m (chest-deep on a 1.8 m character). **Deeper water is refused**: a character does not
  step in (the step is not taken, as a wall blocks it), and one that is in it moves at the
  slowest share until it is out. Chosen over VisionCraft's flat stop at 0.6 m (a soldier fords
  a river chest-deep; slowing by depth reads well and keeps a map traversable) and over
  swimming (buoyancy, a third state of a body: M4 and M6). A speed share that reached zero
  would trap a character dropped in deep water, so the slowest share stands and the refusal is
  a one-way door.
- **Nothing else changes**: rays, lines of sight and projectiles pass through water; the depth
  follows from positions, so the state hash needs nothing new; a world without water takes the
  same code path as before, to the bit (the wading code returns early without water).
- **The waves follow the wind** (Pierson-Moskowitz's fully developed sea, capped at sea state 6);
  an outlined area is inland water with little fetch (waves under 1 m), as in VisionCraft's own
  model. Constants with names in `gamecraft-render/src/water.rs`; a first rule, to be replaced
  by per-water data when a pack needs it.

### 2026-11-26: A world file finds the pack above it, and an export says what it needs

`view.render` of a world file inside a pack drew magenta boxes for every model: nothing told the
host where the pack's `assets/` folder was. A world file refers to its models by their paths
inside a pack (`models/crate.glb`), so it needs the pack, and the question was where that comes
from.

- **The nearest `pack.json` above the file**, as most engines find the project a scene belongs
  to (Godot finds `project.godot` by walking up from a scene given on its command line; Unity
  and Unreal find the project by the folder the scene is in). Chosen over a world that records
  its pack (a path inside a document goes stale when either file moves, and a file sent to
  someone else points at a folder they do not have) and over always naming the pack (a world
  file inside its pack should just work). The climb stops at the host's read root, so automation
  reads nothing it was not given; the command line and the window, which were given the file
  itself, widen their reach to the folder that holds the pack, where its dependencies lie
  (`../game`).
- **The pack given wins**: `file.open {pack: DIR}` and `gamecraft-cli --pack DIR` (the host's
  pack, for every world file opened later without `pack`) come first, and `pack: null` wants
  none even above a pack. The command line refuses `--pack` on a project folder, which brings
  its own assets.
- **Only the assets are read**, not the worlds: a pack of 60 000 entities in cells opens a
  world file as fast as a small one. Each pack of the chain passes the licence gate as for a
  project; a broken one is an error naming the file, and the sentence says the world opens
  without it with `pack: null`. The models are measured and their bounds reach the session with
  the world, so a solid model collides from the first tick; the renderer is handed their files;
  `asset.list` and `asset.credits` list the pack's assets as a project's.
- **A world without a pack is said in one sentence**: the renderer names each model it cannot
  draw; the host adds one sentence naming them all, why (no `pack.json` above the file and none
  given, or the pack has no such file) and what to do. An agent reads `problems`; one sentence
  with the remedy is what it can act on.
- **An export says what it needs, and carries nothing.** `project.export` writes one
  `.gamecraft` file; its answer names the pack that has the world's models (`pack`), the files
  it needs of one, licence records included (`needs`), and the models no pack has (`missing`).
  The file draws its models when it lies inside that pack or is opened with `--pack DIR`. A
  bundle that carries its assets is distribution (M11, "packaging a game and its mods"): copying
  a pack's files beside a loose world file would make a second, half-pack on disk that the
  licence gate does not know. Chosen over embedding models in the world file (the document is
  copied by every command and every undo step) and over a side folder of copies.

### 2026-11-26: A uniform grid over the ground plane for what a ray or a character can meet

On a world of 300 000 entities (boxes and prefab instances with their children over 8 km) a
ray walked every entity: 44 ms a kilometre of ray and 43 ms a line of sight of 300 m in
release, and a tick with eight walkers tested millions of solids (52 ms against 16.7). The
spec asked for well under a millisecond and a tick well under its step.

- **A uniform grid of 32 m cells** (`gamecraft_sim::index`, `SpatialIndex`): every entity with
  a shape, a model's collider box or a road is a member of each cell its footprint touches
  (the smallest box around what the simulation collides with and the renderer draws). A ray
  walks the cells its ground projection crosses (Amanatides and Woo) and tests only their
  members; a character takes the members of the cells around it. Chosen over a BVH (a grid is
  rebuilt and updated in place far more simply, and the world is flat: two dimensions are
  enough) and over trees of bounding boxes per entity (the same). 32 m because a kilometre of
  ray then crosses some 40 cells and holds about 215 entities on that world, and a character's
  square touches one to four.
- **What the index cannot place is found everywhere**: a footprint over more than 256 cells (a
  long road), a character, a footprint without a box. All are few. A ray that would cross
  more cells than there are members to look at looks at every member instead (a small world).
- **It changes no result.** The candidates are a superset of what a walk hits, each is tested by
  the same code, and the two places where order decided something are settled as the walk in
  id order settled them: a ray keeps the first of equal distances, which was the lowest id;
  the solids around a character are sorted by id before they push (a push depends on the
  order). A character carried by a push out of the square its solids were gathered in (placed
  deep inside something large) is pushed again from the same start with the square grown as
  far as it was carried, which is complete again, and after four tries over every solid, which
  is the old computation. Tested to the bit: 800 random rays and 400 lines of sight over a
  mixed world, with skips; every existing suite of positions, hashes and combat.
- **Built once, updated for what changed.** The simulation builds it with its solids when the
  world is taken over and updates it from the world's own list of changed entities
  (`World::changed_entities_since`: the shared-subtree diff of the entity map); when that
  list cannot be made (something was removed, a prefab changed) it is built again. The
  session keeps one for the document the first time a query needs it and updates it the same
  way, including through undo and redo; while playing the queries use the simulation's.
  Derived data: it is never written, hashed or undone.
- **Characters are a list, not cells**: they move every tick. The code that looked for them by
  walking every entity (a projectile's hits and `heard`, a behaviour's enemy, the game master's
  sight) walks the list.
- **The work is counted**, not only timed: `ray::cast_counted` says how many entities were
  looked at, `Sim::solid_tests` how many solids the steps tested; `at_scale.rs` asserts the
  counts and prints the times.

### 2026-11-25: A scale per axis, one number in the file when the three are equal

One deck prefab for every span length (roadmap item 7) needs an instance stretched along its
length and nothing else: with a uniform scale a 10 m deck made 40 m long is four times as wide
and thick, and its posts four times as tall.

- **`Scale`** (`gamecraft_world::scale`): `x`, `y` and `z` along the entity's own axes, each
  0.001 to 1000. In a file, and in every answer, one number when the three are equal to the bit,
  else `[x, y, z]`; three equal numbers read as one scale and are written back as one. So every
  world made before has the text it had, and the state hash it had, since the hash covers the
  text: the pinned examples did not move. The cooked form keeps an enum (format 3: older caches
  are ignored and built again).
- **Shapes, models and children take it along their own axes.** A box stays a box: the renderer's
  placement takes a size per axis, and so do the collider box, the rays and the solids. A
  prefab's children take the instance's scale **in the instance's frame**, as most engines do:
  each offset is scaled axis by axis before it is turned, and a child's own scale is the
  instance's turned into the child's frame. A child turned by a right angle (or any multiple of
  it) has the instance's x and z swapped; turned by any other angle it keeps the scale along its
  own axes as it is, since a box scaled across a skew turn is not a box. Full orientation is M3
  work and replaces this rule. `entity.set {scale}` rescales what is below the entity the same
  way: a derived child to the bit, an entity with a scale of its own by the ratio in its own
  frame. A span stretches the length only now (`[1, 1, scale]` along its own z).
- **What has one size takes the middle of the three** (`Scale::middle`): the width of a cylinder
  or a sphere (a cylinder's height is the y scale), a plant, a character's model body, a road's
  width (its points follow x and z), the height of a light's spot over a box and of a framed
  camera. Stretching something along one axis leaves those as they were; scaling it on two or
  three takes them along. The simulation and the renderer use the one rule, so what is drawn is
  what is collided with; a sphere is never drawn as an ellipsoid.
- **Level of detail and the index take the largest of the three**: a model's extent across grew
  most, and `WHERE scale > 1` in the project index should find a stretched deck.
- Chosen over scaling the children only at spawn (`entity.set {scale}` would leave them), a full
  matrix per entity (there is no rotation but yaw yet), refusing a per-axis scale on round shapes
  (a deck's posts are cylinders, and the middle rule is what a modeller expects), and always
  writing `[x, y, z]` (every file and every hash would change).

### 2026-11-25: The ground is asked for while building, and a span is placed from two ground points

A generator of a large map had to copy the engine's sculpt maths to put bridge ends and pier
feet at the right height, which breaks with every sculpt op the engine adds. So the engine
answers the question itself, and places what depends on it.

- **`query.ground {points: [[x, z], ...]}`** is `query.ground_height` for up to 65 536 points in
  one call, from the document's own height field (a recipe or a heightmap, with the sculpt
  edits, the paint and the rules), before any simulation runs, in a script and over MCP. One
  answer per point, in the order given, nulls where there is no ground, exactly what the
  single-point query says for that point (one function answers both). Chosen over a raster
  answer (a generator has a list of places, not a window) and over batching in the protocol
  (a script is a fixed list of steps and cannot join many answers into one).
- **`at` takes `offset_m`** on `entity.add`, `entity.set` and `prefab.spawn`: the base stands
  that high over the ground. Refused without `at`, never ignored.
- **A span is `{from, to, offset_m?, length_m?}`**, on `entity.add` and `prefab.spawn`. What it
  places stands level at the midpoint, its base on the **higher** end's ground plus the offset
  (a deck never sinks into the ground it spans; over sloped ground the lower end is left above
  its ground, and the answer's `ends` say how far, so the generator can place an abutment or a
  pier), it faces from `from` to `to`, and it is scaled so that its length along its forward
  axis spans the two ends. The length is `length_m`, or measured: an entity's box along z (a
  cylinder's or a sphere's diameter), or a prefab's smallest box around its children's shapes
  and models with bounds, nested and turned children included. A thing whose geometry is not
  centred on its origin is still centred on the span: its origin goes past the midpoint.
  Chosen over a tilted span (entities turn about the vertical axis only until M3), over the
  mean of the two heights (it buries one end), and over guessing a length for a model without
  bounds (the span asks for `length_m` instead).
- **Uniform for now.** A span scales the whole thing, its width and height with it, since an
  entity's scale is one number; per-axis scale (roadmap item 7) makes a span stretch its
  length only.
- **A prefab instance's scale reaches its children.** It did not: a scaled instance's children
  stayed where the prefab put them, at scale 1. Each child now stands at its offset times the
  instance's scale, at that scale; `entity.set {scale}` rescales everything below the entity;
  an unedited child stays derived data, to the bit. No example of this repository has a scaled
  instance, so no pinned hash moved; a world made before, with scaled instances, shows its
  children scaled from now on.

### 2026-11-25: A heightmap at full resolution is a source, named by its content

The control grid (2026-11-21) carries the landforms at 30-60 m; a game built an 8 km map at 4 m and
needed the ground itself at that resolution. `TerrainSource` gains `heightmap`: `cols` x `rows`
samples (2 to 2049 a side; 1-4 m is usual), interpolated over the square exactly as the grid is,
with the sculpt edits, the paint and the rules on top as before.

- **The document names the samples and does not hold them.** `world.json` says `cols`, `rows` and
  `content`, the hash of the samples; a kilometre at 2 m is half a million numbers, too many for
  text and for the copy of the world every command works on. The host hands them to the engine
  (`Session::set_heightmap`), which takes only what the document names: other samples, or another
  shape, are an error that says both. The text, the revision and the history do not move: the
  samples are not an edit.
- **A source is a file of the project**, committed with it: `worlds/<world>/terrain/heightmap/
  tile_<tx>_<tz>.bin`, tiles of 128 cells (129 x 129 samples, neighbours sharing a border row), in
  the shape of the derived field's cache (`GCHEIGHT1`, the tile's place, f32 samples). Not the
  cache: that is thrown away at will, this is made only by importing again. A tile that is missing,
  cut short, a link out of the pack or of another content than the document names is said when
  the project opens, and the project opens without ground; the derived cache is not used either,
  since the next sculpt would rebuild from nothing.
- **`terrain.import` reads what needs no new dependency**: a PGM (P5, 8 or 16 bits, which every
  GIS tool exports), rows of text, raw 16-bit integers and raw 32-bit floats (little-endian);
  `scale_m` and `offset_m` turn raw numbers into metres (height = number x scale + offset). The
  square is centred on the origin as every terrain is, of side `size_m`. A georeference (origin,
  projection) is the importer's business, not the engine's; GeoTIFF needs a decoder the engine
  does not have (open, roadmap item 5).
- **Until the samples arrive there is no ground**, and whatever needs it says so (`terrain.set`
  with kind `heightmap` names one; `terrain.import` names it and hands the samples over in one
  step). A running world takes the samples between ticks as any change. `sim.start` and
  `sim.step` refuse meanwhile, with that sentence: without ground every character falls
  forever, silently (a review fix of the same day).
- **Nothing is lifted for edits**: what the survey already holds belongs in the file (bake it
  there), the 1 024 edits are for what a game master changes over it. Open: a command that bakes
  a world's edits into its heightmap.
- **The samples are plain numbers**, so the simulation's determinism is not touched: no
  trigonometry, a `BTreeMap` for the tiles, no file and no clock below the host. The state hash
  covers the document, so two worlds that name one content hash the same; for the samples the
  host is trusted, as for a model's triangles.

### 2026-11-25: A cell's static geometry is merged for the distance, in memory

Per-model levels (the decision below on levels of detail) still draw every instance: a district
of 300 000 instances is 300 000 instances to submit whatever their level. So the ground is cut in
cells of 128 m and each cell's **static** shapes and models (characters excluded: they move) are
merged into one mesh per material, every model at its coarsest level, as most engines do offline
for their far levels (Unreal's HLOD, clusters and impostors). The merged mesh takes over where the
cell is farther than the largest draw distance of its own items (at least 300 m) and draws until
20 km, the far plane of a view. Chosen 128 m because a cell of a project (512 m) can hold a whole
district, and 64 merged meshes for a kilometre squared are few enough; chosen per cell, not per
block within it, for the same reason.

- **No hole at a cell's corner, and no flicker where both draw.** The renderer measures an item's
  distance to the box of its cluster of instances (8 m) and the merged mesh's to the cell's box: a
  cluster at a cell's far corner is up to a cell's diagonal farther than the cell. So the cell's own
  items draw until a diagonal past the take-over distance, and every merged entity is pulled
  towards its own origin by 0.25 % a side, or 2 cm on an axis longer than 8 m, so where both
  draw the copy stays inside the real geometry. (At first every axis lost 0.25 %: half a
  percent of a 200 m wall is half a metre, and beyond the band its merged copy showed a gap
  where it met a block; a review fix of the same day, checked on pictures.) The depth buffer
  cannot tell the copy from the real one 300 m away on a thin shape, so in the band their
  faces fight faintly; that is for a reversed-Z depth in VisionCraft, as for roads. A
  VisionCraft item's first level starts at distance 0, so the near level of a merged item is
  an empty mesh.
- **A take-over follows the cell's farthest-drawing item**, so a landmark that draws to 3 km holds
  its whole cell at full detail until then: the items are grouped per asset, distances and
  take-over distance, not per cell, so a cell cannot cut one of its items short and an item draws
  on only where merged meshes take over (a cell with a landmark in it does not hold every copy of a
  model in the world). The distances of the size rule are the cell's to move; a distance given in
  metres (the component's, the file's) stays where it is when it is nearer: what a file says to
  stop drawing is not drawn, merged or not. Chosen over per-cell items (an item per cell per
  asset: many more items for the near field) and over cutting a user's `draw_distance_m`.
- **Built again only where something changed.** A cell's content is hashed (its entities' ids,
  poses, shapes or models, and the assets revision); a rebuild makes a cell's merged mesh again only
  when its hash, its assets or its take-over distance changed, and keeps the rest as uploaded. The
  meshes stay in memory: a copy in the cooked cache of a project, keyed by the same hash, is where
  most engines put it and is not built yet (open).
- Not merged: plants (items per species, seed and growth; a district's trees matter as much as its
  buildings), roads and terrain; a cell over 4 M vertices keeps its own items and says so.

### 2026-11-25: A bake replaces what the script owns, and the result is kept, not the script

A game that generates a large world (62 000 entities from an OpenStreetMap extract, 295 000
with prefab children) kept its script: one file, unreadable diffs, a full rebuild on every
change. `gamecraft-cli project bake SCRIPT --into DIR` now runs such a script and saves the
result as a project of cells in one step. The script is a build artefact (`.gitignore`); the
generator and its sources and the cells are committed (`guide/generated-worlds.md`).

- **What a re-run replaces: the layers the script writes, and its own terrain edits.** Chosen
  over replacing the whole world (hand edits in other layers, prefabs and assets would be lost
  every time) and over tracking every entity a script made by an id of its own (a new id scheme
  for one command). The bake writes what it owns in `worlds/<world>/bake.json`: the layers
  (every layer a step names, the default layer, the layers of the previous bake) and the places
  of the terrain edits it added. A re-run removes those entities and those edits, then runs the
  script; when the terrain was sculpted by hand after the last bake it refuses, since a re-run
  would stack its edits.
- **The same script gives the same files.** That needed one change in the project's save: the
  string id of an entity of the root pack that is gone is free again (the ids of dependencies'
  entities stay taken, for their removal lines). Before, a re-bake moved every entity to a new
  id (`box-0` became `box-0-2`) and rewrote every cell. In cells the runtime ids follow the
  order of the cells, so a script run directly hashes the same as its bake only with one file
  per layer.
- **Nothing is written while a step can still fail.** A new project is made after the whole
  script ran; an existing one is saved after. A failing step names the script line, the step
  and the command.
- **The script is read one step at a time** (a reader over the bytes between the brackets of
  the JSON list, with the line of every step), so a hundred thousand steps cost no more than
  one: 100 000 entities bake in about 14 s and 580 MB.
- **`entity.remove_many {ids}`** (one pass, checked once) because `entity.remove` per entity
  walks the world once (a removal makes the session validate every entity, and
  `World::remove_entity` scans for children): removing a layer of a hundred thousand one by one
  took over an hour; in one command it takes seconds. An id or a name that no entity has is an
  error naming it: an id used to be skipped in silence, so a typo in a generated list removed
  nothing and said nothing (a review fix of the same day). The bake passes ids it has just
  read from the document, so a re-run never meets one that is gone.

### 2026-11-25: Models change level per ground cell, at distances from the file, the component or their size

One 100 x 100 m block drew 360-430 k triangles per view; a district must not scale linearly.
The mechanism was already in VisionCraft's renderer (`add_lod_item`): the parts of an item
that end at the same distance form one level, the instances of an item are grouped in cells
of 8 m on the ground, each cell takes the level for the distance from the eye to its box (1 m
of hysteresis, a dissolve between two levels), a cell beyond the last distance is not drawn,
and cells and items are culled against the six clip planes. GameCraft's adapter handed every
model one level with an infinite distance. What was missing was the content and the distances.

- **Which geometry is which level** is read from the file by two conventions: names ending in
  `_LOD<n>` (Unity, Unreal, most exporters) and the `MSFT_lod` extension. Both are found by one
  walk in `gamecraft-project::lod` that the host's `inspect` and the renderer's parse share,
  so `asset.list` reports the levels the renderer draws, to the bit. Chosen over one
  convention (files in the wild use either) and over a way of GameCraft's own to mark levels
  (nothing should have to be added to a model by hand). Every level of a group is drawn where
  the group's finest node in the scene stands, since that node is the one the author placed; a
  level's own transform is ignored. What is broken (a coarser level with more triangles, a
  missing level, an id that points nowhere) is a warning naming the asset, never an error:
  the model is drawn as well as it can be.
- **The distances** come from three places, each winning over the next: the `model` component
  (`lod_distances_m`, `draw_distance_m`), the file (`asset.extras.gamecraft` with the same two
  fields, else `MSFT_screencoverage`), and a rule from the model's size. For the file's
  distances, glTF `extras` was chosen because it is the standard place for what an
  application adds to a file: the artist's distances travel with the artist's model, through
  `asset.import` and mods, and the browser build reads them from the model itself. A file of
  its own beside the model would have needed a place in the licence gate; the licence record
  is a licence record.
- **The rule** is a screen-size rule at a reference view, 1080 pixels high at 60 degrees: a
  level ends where the model's extent across (the diagonal of its box) spans a quarter of the
  view's height for the first change, and a quarter of the previous share for each next one;
  the model is drawn while its extent spans 4 pixels. In metres: 3.5, 14, 55, ... times its
  extent, drawn to 234 times its extent (a crate 0.9 m on a side to 364 m, a house 15 m
  across to 3.5 km). Chosen over distances that follow the picture's size and field of view:
  the renderer takes one distance per level per item, a scene is not rebuilt when a window
  resizes, and an agent's numbers should not change with the size of its pictures. Pictures
  are always drawn at the highest quality, where the metres are as written; a live frame at a
  lower quality switches sooner, as it does for everything in the renderer (40 % at low,
  50 % at medium).
- **No mesh is simplified** (it would need a new dependency): a model without levels keeps one
  level and is still culled beyond its draw distance. Automatic levels are the next step if
  the packs' models turn out not to carry any.
- **What was drawn**, per model and per level, is counted again on the CPU by the renderer's
  own rule (`gamecraft-render/src/culling.rs`) and comes with every picture (`view.render`'s
  `stats.models`): the renderer's own statistics are frame totals. An agent cannot look at
  pictures; numbers are how it checks. The count of a picture is exact, since a picture starts
  settled (no hysteresis, no dissolve); a live frame differs for the few frames a hand-over
  dissolves over.
- **A character the camera follows is never culled by distance**: it has an item of its own
  without an end, built again when the followed character changes.

The review round of the same day settled the rest:

- **Scattered copies share items by buckets of scale.** A `scatter` with `scale_range` gives
  every copy a scale of its own; one item per copy would have put a thousand draws in every
  pass and shadow cascade. Entities that show an asset with the same component distances now
  share one item per bucket of scale (steps of 1.25) wherever a distance follows the scale
  (the rule's, or the file's screen coverage); an item's distances are computed for its
  bucket's upper scale, so no copy disappears early. Distances given in metres, on the
  component or in the file, do not follow the scale: every scale then shares one item.
- **A live view is counted at its quality.** The renderer divides every distance by the live
  quality's share (40 % low, 50 % medium, 100 % high); `count_models` and `frame_stats` do the
  same, and a picture, always drawn at the highest quality, counts at 1.
- **A level without geometry is left out.** As the last level of a file (common with
  `MSFT_lod`, whose last id is often a node without a mesh) it means "draw nothing from
  here": the model ends where that level would have started. In any other place the level
  after it starts where it would have. A problem line names the asset either way; left in,
  the empty level would have shifted the renderer's levels.
- **A coarser level keeps its own rotation and scale** and loses only its translation, since
  a modelling tool lays the levels out side by side.
- **The box of a model is its finest level's**, measured over every vertex of that level,
  strips and fans included, by the host's reader and the renderer's alike (one size for both):
  what most engines collide with. So the collider bounds of an asset with levels differ from
  a box over every level's vertices, and a journal recorded with such an asset before this
  change replays to another hash. None of the pinned examples uses an asset with levels: the
  CLI's golden hashes did not move (`cargo xtask ci`). `lod_distances_m` and `draw_distance_m`
  change a picture, not what moves next, and are left out of the simulation's static hash.
- `MSFT_screencoverage` is kept as the file gives it and turned into metres for the size the
  model has in the world, as the rule is.
- **Strips and fans are drawn**, as the triangles they stand for (a strip's every second
  triangle wound the other way, as the format says). Before, the host counted them and the
  renderer dropped them, so a level made of a strip was listed and never drawn.
- **The live counts are the live frames' own.** The renderer writes its statistics in the
  frame path that a picture also goes through, so the adapter keeps the counts of the last
  live frame itself (`frame_stats`); a picture leaves them as they were.
- What the distances say of an asset is said once, however many items its entities fill.

### 2026-11-24: Shared textures are files of the pack, named by their content hash

Every imported glTF model embedded its own copy of every image: a pack of 29 models sharing four
texture sets was 20.6 MiB, 18 of them the same 12 JPEGs, and a district of house variants would
have grown with its variants for no visual gain. `asset.import` now takes a model's images into
a pack-level texture store, `assets/textures/<content-hash>.<ext>`, one file per distinct
image, and rewrites the model so that each image refers to it by a relative URI
(`../textures/<name>`), as glTF files refer to files beside them and as engines share images
between materials (Unity's and Godot's textures are assets of their own that materials
reference; glTF tooling deduplicates textures by content into a folder). A model without images
is stored as it came; models with embedded images, from before, load as before.

Chosen over a registry file that maps hashes to files (the file name is the registry, and the
licence gate already checks files); over converting every model to `.gltf` with a `.bin` (two
files per model and new paths for existing packs, where the format lets a `.glb` refer to
images outside it); over keying the store by image name (names collide and change, bytes do
not). The hash is FNV-1a, 64 bits, the workspace's own `gamecraft_math::StateHasher`: a pack
holds a few hundred textures at most, where a collision's odds are negligible, and an import
checks an existing file of the store against its name, so one is refused rather than reused.
A cryptographic hash (`blake3`) is the upgrade should packs ever be merged from untrusted
sources; the names would change with it.

A texture is an asset with a licence record of its own, so `asset.credits` names it once
however many models use it, and its record may differ from a model's (a CC-BY texture set on
CC0 models): the first import of an image writes it, a later one keeps it. The URIs climb with
`..`, so the gate and the renderer resolve a URI against the model's folder inside `assets/`
(`gamecraft_project::model::resolve_uri`) instead of refusing `..` outright; the rule that a
model refers to nothing outside `assets/` stands. A failed model keeps the list of files it
waited for, so a texture that arrives later (the browser fetches one by one) wakes it.

The views that held only images leave the model, and only those: a buffer that loses a view is
rebuilt from the views it keeps (a `.glb`'s chunk, a `.gltf`'s buffer file alike), one that keeps
none goes, one that loses nothing is copied as it came. Renumbering buffer views is safe only
where the engine knows every reference: a rewritten model is refused when its document names
`EXT_meshopt_compression` (compressed geometry the engine cannot read), when an extension object
anywhere names a buffer, or when anything but the accessors and the images names a buffer view,
with a sentence, not guessed at. A model's buffers must live in its own folder, where the gate
lets its record cover them: an import refuses one anywhere else instead of writing a file the
next open would refuse the pack for. A buffer file's `byteLength` is touched only when the import
rewrites that file. The licence gate lets a model's record cover the files it names inside its
own folder only: a `..` reference reaches the texture store (records of their own) or a file
that must bring its record, so a model cannot cover a folder it does not live in. A model's
`bytes` are its own files: a texture shared by many is counted once, as its own asset.

### 2026-11-23: Roads are a component draped on the terrain, overlaps ordered by a tier

A real-city pack laid its streets as about 55 000 yaw-only boxes; on slopes every box was a step,
and where boxes met their tops sat centimetres apart and the renderer showed dark lines and
stripes. What most engines do instead is a road as a polyline (or spline) with a width, turned
by the engine into one mesh draped on the terrain (`gamecraft-world/src/road.rs`).

- **A component, not a world-level list.** A road is an entity with a `road` component: it gets
  ids, names, layers, undo, prefabs, `entity.find`, replication and pack overrides for free.
  Its points are offsets in the entity's frame (position, yaw and scale apply), so `entity.set`
  moves a road; `road.add` takes world points and makes the offsets.
- **The surface is derived, exactly.** The footprint (a trapezoid per segment, a sector on the
  outer side of each bend, round ends) is clipped against the terrain's own triangles and every
  clipped polygon is lifted by the road's offset, so the surface is the ground plus the offset
  everywhere, with no sampling density to tune and no ridge poking through. Never stored;
  rebuilt from the recipe and the height field, deterministic to the bit.
- **Junctions by a tier, kept in the document.** Overlapping roads get different tiers (0 to 7,
  1 cm each), so no two surfaces are coplanar. `road.add` picks the lowest free one,
  `retier_all` redoes all of them in id order, and an author can set it (main road over side
  street), as decals and road splines have a sort priority in most engines. Chosen over junction
  patches (trimming roads and building a junction polygon: robust only with a full road graph,
  which OSM ways do not give) and over a tier computed on the fly (it depends on every other
  road, so each ray would need the whole network); with the tier in the document a ray only
  looks at the roads near it.
- **Rays meet the surface as drawn.** The ground lifted by a road's offset is raycast (exact on
  the triangles) once per distinct offset, and the hit counts where it lies in a footprint: no
  mesh, cache or index is kept, and the sim, `query.raycast`, lines of sight and projectiles all
  see roads through `gamecraft_sim::ray::cast`.
- **Drawn per terrain chunk and colour,** so thousands of roads are a few dozen items the renderer
  culls by chunk; roughness 0.85, puddles when wet. Roads cast and receive shadows like the
  ground: a 3 cm lift shades a strip a few centimetres wide beside the road's sunny edge.
- **Characters walk on the terrain** beneath the road: the lift is at most a few centimetres, and
  a second ground would add a step at every road edge.

Left for later: flattening the terrain across a road (a terrain edit derived from roads would
make the height field depend on entities; it belongs with the generators of M3, "roads and
rivers along splines that shape the terrain"); road markings and materials by id; a per-material
depth bias or reversed-Z depth in VisionCraft, which would let the lift shrink (with a standard
depth buffer and a near plane of 5 cm, 3 cm of lift separates road and ground to about 150 m;
beyond that a distant road can flicker into the ground).

### 2026-11-23: A prefab instance's children are derived data in the file

A real-city world (47 612 courtyard lots from 10 prefabs, 287 015 entities) was 137.6 MB because
every child of every instance was written with all its components, and the engine could not
reopen what it had built. The children follow from the prefab and the instance's placement, so
by rule 2 they are not stored: the file keeps the instance (prefab, placement, overrides) and only
the children that were changed or removed, as modifications by the child's index in the prefab,
which is what scene files of most engines do with a prefab instance (`docs/design/journal.md`,
*Children are derived data*).

Ids: the children keep the consecutive ids `prefab.spawn` reserves (the instance's id plus 1, 2,
... depth first), so the common case writes no id at all; an instance whose children are not at
those ids writes the list. Chosen over always writing a list (about 45 bytes per instance for 6
children at six-digit ids, a third of what a lot costs now) and over ids derived from a hash
(collisions, and ids that mean nothing in a diff). The format version stays 1, with a document
field `derived_children: true`, so worlds without prefab children keep their text and their
state hash; an earlier engine refuses a new file instead of dropping children.

`prefab.set` brings children along by the rule of components, recognising a child by its unique
name before its index. Prefabs may now contain prefabs (8 levels, 4096 entities per instance).
Project layer files still write every child as an entity with its string id: their pack overrides
work by string id, and deriving children there is the next step for the cell files.

### 2026-11-23: A cooked binary cache of a world, as derived data

Text stays the source; a compact binary form is "cooked" from it for fast loading and can always
be rebuilt (`gamecraft-world/src/cooked.rs`). `file.open` hashes the bytes of the `.gamecraft`
file (FNV-1a) and uses `.gamecraft-cache/<file>.cooked` beside it when it was made from exactly
those bytes by this format version; a missing, stale, damaged or other-version cache is ignored
and the text read, never an error, and the cache is written again where the host may write
(inside the write root, or beside any file for the command line; `GAMECRAFT_NO_CACHE=1` turns
that off). `file.save` cooks what it wrote. The cache folder holds a `.gitignore` of `*`: cooked
files are derived and never committed to a pack; deleting them costs one slow load.

Format: postcard (MIT/Apache-2.0, no_std and wasm-safe, a stable wire format; bincode is no
longer maintained), the developer's choice. Components are internally tagged in serde, which a
format that does not describe itself cannot read, so each distinct component is stored once as
JSON and entities refer to it by index, and a prefab child that is exactly what its prefab
places is stored as its id, parent and index. The Fallujah world (297 535 entities, 66.3 MB of
minified text) cooks to 12.3 MB and reads in about 0.37 s instead of 0.7 s, to the same state
hash. Projects are not cooked yet: their load also builds where every entity came from.

### 2026-11-23: A large world is stored in cells

A big map is divided into a regular grid of cells, each cell a file of its layer
(`layers/<layer>/<cx>_<cz>.json`), as large outdoor engines store their worlds: files stay small
and diffable, an edit touches the cells it changed, and a streamer can load the cells near a
player. The cell size is data (`cell_size_m` in `world.json`, default constant 512 m, usual for a
map of several km). The entity at the top of an entity's parents decides its cell, so a prefab
instance stays in one file. Loading reads every cell for now (the authoritative simulation needs
the whole world); `project.open {cells}` and `project.cells` are what a streamer needs, and
streaming in the renderer is M3 work (`roadmap.md`). Cells were chosen over one file per entity
(hundreds of thousands of files) and over splitting by count (files that change when anything
before them changes).

### 2026-11-23: The world-file cap is a safety net the host sets

`file.open` refused files over 64 MiB, a cap meant for assets that also stopped the engine
reading its own worlds. The world-file cap (and the cap on a file of a project's pack) is now
512 MiB by default and set by the host: `gamecraft-cli --max-world-mb N` on every subcommand,
`GAMECRAFT_MAX_WORLD_MB`, or `Access::max_world_bytes`. It protects against a file that would
exhaust memory; it is not a design limit (the world bounds itself: a million entities). Models
and textures keep their 64 MiB cap. The error names the cap and how to raise it.

### 2026-11-21: Real ground enters as a control grid, not as a bitmap

A map of a real place needs real elevation, and `TerrainSource` had only `flat` and `noise`. It
gains `grid`: a rectangular control grid of heights carried in the document itself (at most
513 × 513), interpolated bilinearly over the square, with the sculpt edits on top as before.
Reasons: it keeps a world text — diffable, mergeable, no loader change, the same in wasm and over
MCP — and a control grid at 30-60 m spacing carries every landform a map of 2-16 km needs, while
street-level detail belongs to buildings and not to the ground. Binary terrain tiles stay what the
2026-11-18 decision made them: the *derived* height field and painted weights of a large world,
streamed by distance. Importing a 4 m DEM as text would have been 4-16 MB for detail the engine
cannot use anyway.

### 2026-11-18: Text files are the source; a database only as a cache or on a server

The source of a project stays text files plus binary tiles and assets, not a database: they go
into git (history, diff, review, branches, pull requests), two people or agents can merge their
work, a mod is a directory layered over another (a database cannot be layered that simply), an
agent reads and writes them with any tool, and they need nothing compiled in C, which keeps the
browser build simple. A database has two places:

- **An index, derived and thrown away at will** (rule 2): when a project is large, the engine
  builds an embedded store (pure Rust, chosen when needed) next to it for what scanning files
  would make slow: finding entities by name, tag or area across all layers, which prefab is used
  where, which asset is referenced by what. Deleting it loses nothing.
  Chosen: Turso (MIT), the pure-Rust, SQLite-compatible database, because agents and developers
  already speak SQL, it needs no C code (so the browser build stays simple) and the developer
  trusts the team behind it. Its 0.7 release (July 2026) dropped the beta label but is not 1.0;
  for a derived index that is enough, since a broken index is rebuilt from the files.
  As built: `default-features = false`; one piece of C remains (`simsimd`, Apache-2.0, vector
  kernels that Turso turns on and a user cannot turn off). Accepted, since the index is native
  only and the browser build does not compile it. Not SurrealDB: its Business Source
  Licence does not fit an MIT engine whose games anyone must be free to ship.
- **Persistent servers** (M9): a world that runs for weeks with many players keeps its live state
  and the players' progress in a server database, and writes snapshots and journals. That is
  operational data, not the game's source.

### 2026-11-18: One JSON file is M0's format, not the game's

A single `.gamecraft` JSON file is fine for a test valley and wrong for a large game: one change
rewrites the whole file, two people (or agents) editing different places conflict, and loading
means parsing everything. The format for M3 and later:

- **A project is a directory**, and a pack is one too. A world is split into **layers** (one
  file per area or per kind: `layers/village.json`, `layers/forest-north.json`), each a list of
  entities, so an edit touches one small file and diffs and merges stay readable.
- **Bulk data is binary, beside the text**: terrain heights and painted layers as tiles
  (`terrain/tile_x_z.bin`, a few hundred kilobytes each, loaded by distance), models and textures
  in their own formats (glTF, KTX2). Text files refer to them by path.
- **Recipes over results** (rule 2): a forest of 100 000 trees is one generator entry with a seed
  and an area, not 100 000 entities; the trees exist only in memory and on the GPU.
- **At runtime nothing is JSON**: the engine holds compact structures in memory, streams tiles in
  and out, and the simulation never touches a file.
- **Saved games are not the world file**: a save is the base world's identity plus the changes
  made while playing (and later the journal), in a compact binary form.
- **Distribution** packs a project or a pack into one bundle file.

What has to change in the code before worlds grow (also under *Structural debt* in
`architecture.md`): undo keeps whole copies of the world and must keep changes instead; a live
command copies the running world and must apply a change set; the renderer rebuilds on change
and must update what changed.

### 2026-11-18: The command line is the main door, MCP the second

An agent with a shell drives GameCraft through `gamecraft-cli`, as with 3DCraft's
`craft3d-cli`: `run` with `--cmd` and `--params` or a script of steps, one line of JSON per
result, `info`, `render`, `play`, exit codes 0, 1 and 2. MCP stays for tools that have no shell.
Both run the same commands through the same host, so nothing can be done in one that cannot be
done in the other. Reason: the developer asked for it; a command line is scriptable, testable
and visible in a terminal log, and needs no client configuration.

### 2026-11-18: Engine, packs, games: mechanisms in the engine, things in packs

Two games as far apart as a military sandbox and a falling-block puzzle must both be buildable,
and the puzzle must not carry rifles. So there are three layers. The **engine** (this
repository) holds mechanisms that name no theme: entities and components, the simulation,
collisions, input, audio, networking, running scripts, scenarios and triggers, the game master.
A **pack** holds content and rules for a family of games: models, materials, sounds, prefabs,
scripts (a "nature" pack with terrain materials and plant sets; a "military" pack with weapons,
ballistics, damage, vehicles). A **game** is a list of packs plus its own data, scripts and
scenarios. Packs, games and mods are the same thing technically: a directory of data in the
engine's own formats with a small manifest, layered over the ones below it. A pack holds no
Rust and is no codebase of its own; a single-file bundle of one is for distribution, later.

The test for where something goes: does it name a *thing in a game world* (a weapon, a soldier,
a block) or a *capability* (to collide, to hear, to replicate, to trigger)? Things go into a
pack, capabilities into the engine. A mechanism moves from a pack into the engine only when a
second, unrelated pack needs it. The engine therefore reaches up to and including the layering
of packs and the running of scripts, because packs must be able to bring behaviour.

What this means for VisionCraft: its renderer and its plant generator are mechanisms and are
used by the engine; its asset library (models, materials and skies under CC0) is content and
becomes the first pack, built with import commands modelled on VisionCraft's asset tool. One
exception stands for now: the plant species are compiled into the generator and so come with
the engine; they can move to the nature pack when packs exist (M3).

A small puzzle game of GameCraft's own design is kept as a second example beside the outdoor
one: the day it needs terrain or a weapon to start, something has leaked into the engine.

### 2026-11-18: Three promises: anyone builds, everything mods, a running game is steered

The developer's aim is the experience of the large military sandbox games (he named the Arma
series and its engine, Enfusion): highly customisable through mods, but such that *anyone* can
build a game in it, and with what those games do best, scenarios steered live by people, "which
can provide total mayhem". So three promises outrank other wishes: anyone can build (every
action is a command, so an agent does the building for a person who describes it), everything
can be changed (games and mods are layers of data and scripts), and a running game can be
steered (a game master, person or agent, changes the live world). The last one is why world
commands act on a running simulation from M0 on (rule 6), and why scenarios and the game master
come right after the first walkable window in the roadmap.

### 2026-11-18: An engine, not one game

GameCraft is a platform (rule 16). The reference class is the engine behind large outdoor
sandbox games: big terrains, entities and prefabs as data, an authoritative simulation for many
players, mods, and a workbench. The developer named Enfusion as the example of that class. What
is taken from that class is the list of problems to solve; every design here is GameCraft's own
(rule 14).

### 2026-11-18: The workbench is a set of commands

Where such engines have editors with windows, GameCraft has commands with JSON parameters,
reachable from the CLI, a JSON-lines channel and MCP, exactly as 3DCraft and VisionCraft do it.
A window comes second and is thin. Reason: the first user of the engine is an agent.

### 2026-11-18: VisionCraft as git dependencies behind an adapter

`visioncraft-render` and `visioncraft-flora` are used directly, pinned to one rev, and only the
crates of rule 13 may touch them. Reason: it is the fastest start, and VisionCraft is changing
quickly, so the surface that can break is kept to one adapter. Pulling the shared parts into a
repository of their own waits until it is clear which parts the engine really uses.

### 2026-11-18: Own world model, no engine framework, no ECS crate

Entities and components are GameCraft's own data types. Reason: the document must be
serialisable, inspectable, ordered and deterministic by construction, and those are properties
of the storage itself.

### 2026-11-18: Determinism from the first line

Fixed step, `f64`, portable maths, ordered iteration, seeded randomness, a state hash. Reason:
it is what lets an agent test a game without a person (replay, compare, bisect), and it cannot
be added afterwards.

### 2026-11-18: The never-crash lints of 3DCraft

`unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `unreachable` and
`indexing_slicing` are denied in non-test code. Reason: an engine runs other people's data.

### 2026-11-18: MIT, public on GitHub

The repository is `github.com/Skediaio/gamecraft`, public, MIT, as VisionCraft.

## Open

### Physics: own or an existing library (blocks M4)

Writing rigid-body physics is a large work of its own; a pure-Rust library with a deterministic
mode exists and builds for wasm. Using one means a dependency at the heart of the simulation and
proving that its determinism holds across targets. The character movement of M0 is GameCraft's
own and does not depend on this.

### Scripting: how game rules are written (blocks M5)

Candidates: rules as data only (triggers and actions that are commands); WebAssembly modules
with a narrow host interface (sandboxed, deterministic, any source language, runs in the browser
too); an embedded scripting language. An agent writes any of them; what matters is sandboxing,
determinism, hot reload and what a mod may do.

### Networking model (blocks M9)

An authoritative server with replication and prediction is assumed. Open: the transport for
browsers, and whether determinism is used for more than testing (rollback, lockstep).

### A game's own assets, and attachment points (blocks the first game outside this repository)

The engine repository holds only the small example packs that test it (rule 16), under CC0 or
CC-BY (rule 15). A real game lives in a repository of its own, with its own art. Open:

- The loader refuses every asset that is not CC0 or CC-BY, in *every* pack. That is right for
  this repository but blocks a game that ships its own or bought art. Proposed: the record stays
  required and `asset.credits` lists it, but the licence is any SPDX id or `proprietary`; the
  CC0/CC-BY rule is checked for the repository's own examples by a test, not by the loader.
- Equipment made of parts (a scope on a rail, a part swapped for another) needs attachment
  points kept from a model's nodes and a slot mechanism in the engine (M5); today the glTF
  reader merges the nodes away.
- Terrains of several kilometres a side need streaming of terrain, vegetation and entities; no
  milestone plans it yet.

### Toolchain pin

VisionCraft follows stable Rust without a pin; 3DCraft pins a version. A pin keeps new clippy
lints from breaking CI at random; no pin keeps the setup simple.

### A shared automation crate

The command spec, the JSON-lines protocol and the MCP server exist three times now (3DCraft,
VisionCraft, GameCraft), ported each time. One shared crate would end that, at the price of a
dependency between the projects.
