# Decisions

Decisions taken, newest first, each with its reason; then the open ones. The developer steers the
product and the architecture and leaves how it is built to the orchestrator (2026-11-18): an
implementation decision is taken by the orchestrator, written down here with its reason, and
stands until the developer overrules it. A fork in the architecture or the product goes to the
developer as a recommendation.

## Taken

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
where the engine knows every reference: a document where anything but the accessors and the images
names one (an extension such as `EXT_meshopt_compression`, a buffer's fallback view) is refused
with a sentence, not guessed at. A buffer file's `byteLength` is touched only when the import
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
