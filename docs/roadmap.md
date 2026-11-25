# Roadmap

What is planned, in order. Updated 2026-11-18. Nothing here is a promise of a date. A milestone
is done when its *Done when* holds and `cargo xtask ci` is green; one with something to see is
done when it also runs in Chromium on WebGPU.

The order follows the three promises of `AGENTS.md`: first a world an agent can build and play
(M0, M1), then a running game that can be steered (M2), then size, depth and mods.

## Current focus

**Prototype P1 (Lantern)**: scenarios are being built. Then M1, so a person can walk in it too.

## M0: Foundation (done 2026-11-18)

One path works end to end, from the command line and over MCP: a command edits the world, the
simulation steps it deterministically, commands change the world while it runs, a picture is
drawn with VisionCraft's renderer.

| Part | Status |
|---|---|
| Workspace, rules, layers, CI (`cargo xtask layers`, `wasm`, `ci`) | done 2026-11-18 |
| `gamecraft-math`: portable maths, seeded randomness, noise, state hash | done 2026-11-18 |
| `gamecraft-world`: the document, its JSON format, validation, the height field, the sun | done 2026-11-18 |
| `gamecraft-sim`: fixed step, input, a character that walks, falls and is stopped by solids; taking over a changed world between ticks | done 2026-11-18 |
| `gamecraft-engine`: session, command specs, world commands on the document and on a running world, queries, undo, play commands | done 2026-11-18 |
| `gamecraft-render`: terrain, shapes, plants, sky and weather through VisionCraft; PNG out | done 2026-11-18 |
| `gamecraft-automation`: JSON lines, MCP, host commands (`file.*`, `edit.*`, `view.render`) | done 2026-11-18 |
| `gamecraft-cli`: `run`, `play`, `info`, `render`, `commands`, `new`, `serve`, `mcp` (`docs/guide/command-line.md`) | done 2026-11-18 |
| `docs/agent.md` with a worked example that a test runs | done 2026-11-18 |

**Done when:** an agent with only the MCP server builds a small valley with trees and a
character, walks the character across it by input, drops a wall in its way while it walks, reads
its position and the state hash, gets the same hash on a second run, and looks at a picture of
the result.

## Prototypes: two small games that test the engine

The developer wants the engine's reach tested early with two games, built as packs outside the
engine's crates (`examples/`), each playable by an agent over MCP and checked by a state hash.
They run alongside the milestones and pull work forward where they need it.

- **P1, something simple and of our own.** *Lantern*: at night in a small valley, a character
  carries the only light and must find and light a handful of beacons before the mist closes
  in; a game master (the agent) moves the mist and the wind. It needs only what M0 and M2
  give: terrain, weather, a character, zones, objectives, triggers. No weapon, no enemy.
- **P2, a skirmish.** A small map, one rifle, a few enemies that patrol and react. It needs
  projectiles as a mechanism of the engine: gravity, drag, and the wind and weather of the
  world acting on the flight (rain and mist shorten sight, wind bends the shot), hits on
  shapes and characters, health as data of a pack, enemies moved by routes and simple
  behaviour. The rifle, its cartridge and the enemies are things of the pack.

Status: the scenario mechanism of P1 (zones, interaction, variables, objectives, triggers whose
actions are commands, messages; `docs/design/scenarios.md`) is done 2026-11-18, with a miniature
of Lantern as its test (`crates/gamecraft-engine/tests/lantern.rs`). Lights are drawn since 2026-11-18.

The mechanisms of P2 (projectiles with G7 drag, air density and wind; health; sight reduced by
night, mist and rain; hearing; patrol and engage behaviour; `docs/design/ballistics.md`) are done
2026-11-18, with a miniature skirmish as their test (`crates/gamecraft-engine/tests/skirmish.rs`).
Tracers, impacts, downed characters, a crosshair and firing with the mouse are drawn since
2026-11-19 (live frames; not yet in `view.render` pictures).

Both games exist as `examples/lantern` and `examples/skirmish` (2026-11-18): built only with
commands (`build.json`), won by a scripted player (`playthrough.json`) with pinned hashes.
What they found awkward in the engine, and what follows from it:

- `sim.input {face}` turned once instead of tracking the point: now a held facing, also of an
  entity (`face_entity`), done 2026-11-19.
- Messages were static text and trigger conditions read only variables: `{var:name}`,
  `{who}`, `{target}` in trigger messages and `{objective, status}` conditions, done 2026-11-19.
- No way to ask for a ballistic aim: `query.aim`, flown with the simulation's own model, done
  2026-11-19.
- Scatter could not keep areas clear: `exclude` circles, done 2026-11-19 (names were already
  numbered).
- Scripts cannot react (a player script is a fixed list): scripting, M5.
- Characters get stuck on solids without waypoints: navigation, M7.

**Done when:** an agent plays each from start to finish over MCP, a second run gives the same
state hash, and a picture of each exists.

## M1: Walk (in progress)

Done 2026-11-18: `apps/gamecraft`, the window on winit and wgpu, native and in the browser
(Trunk), with a fixed-step loop and interpolated display, keyboard and mouse to input records,
first- and third-person cameras, lights drawn, and `--stdin` so an agent drives the window a
person watches; a headless Chromium smoke test walks in the browser. Gamepads since 2026-11-19 (in the browser always; natively behind the feature `native-gamepad`,
which needs the system's libudev development files). The hash is proven equal natively and in wasm (`cargo xtask wasm-hash`, 6 scenarios, in CI)
since 2026-11-19. Open: the socket control channel and `--bridge`, a real native window tried by
a person.


A person and an agent are in the same world, in a window and in the browser.

- `apps/gamecraft`: the window on wgpu, native and `wasm32-unknown-unknown`, with the game loop
  (fixed simulation step, interpolated display).
- Input devices mapped to input records: keyboard and mouse first, then gamepad.
- First-person and third-person cameras that follow a character.
- The control channel: an agent drives the window the developer is watching
  (`gamecraft-cli mcp --bridge`), and takes its screenshot.
- The state hash proven equal natively and in wasm, in CI.
- A browser smoke test in headless Chromium.

**Done when:** the M0 valley is walked with the keyboard in Chromium, and an agent changes the
world over MCP while the developer walks in it.

## M2: Direct (in progress)

Done 2026-11-19: the journal (every command and input of a session with its tick and hash;
`journal.get`, `journal.save`, `gamecraft-cli play --journal` and `replay --check` that names the
tick where a run diverges) and prefabs (describe once, place many, with overrides and children;
`prefab.spawn` works on a running world). Done earlier for P1 and P2: zones, objectives,
triggers, characters that patrol and engage. Done 2026-11-19: the game master's tools
(`gm.overview`, `gm.sight`, `gm.order` for one character or a squad by tag, messages to a tag,
the MCP prompt `direct_scenario` and `docs/guide/game-master.md` with ten moves that a test runs),
and a scripted game master who directs the skirmish into a loss for the player, saved as a
journal that replays to the same hash. Open: the window shows each player only its own messages.


Scenarios, and the game master who steers them.

- The journal: every command and input of a session with its tick, so a directed session is
  saved, replayed to the same state hash and bisected.
- Prefabs: an entity described once and placed many times, with overrides; `prefab.spawn` on a
  running world.
- Scenario data in the world: zones, objectives, timers and triggers whose actions are commands.
- Characters that move without a player: follow a route, go to a place, keep a formation.
- The game master's view: commands to list what is happening (who is where, which objectives
  are open), to take over and release a character, to speak to players.
- An agent as game master: an MCP prompt that briefs it on a scenario, and a guide.

**Done when:** an agent runs a scenario for a walking player: it places and moves a dozen
characters, opens and closes objectives as the player reaches them, changes the weather, and
the saved journal replays to the same hash.

## M3: World (in progress)

Done 2026-11-19: projects and packs as directories (`docs/design/projects.md`): a manifest,
worlds split into layer files with one entity per line, prefab files, packs layered by id so a
mod changes a game, saves that write each change back to the file it came from (and edits to a
dependency into the mod), `project.*` commands and `gamecraft-cli project`, and the Turso index
with read-only SQL (`project.query`). Example: `examples/skirmish/project` and the mod
`examples/skirmish-night-mod`. Terrain editing (2026-11-19): sculpting (raise, lower,
flatten, smooth, ramp, noise) and painted material layers with slope and height rules, as edits
over the recipe that rebuild only the tiles they touch, a binary tile cache in projects, terrains
of 4 km at 2 m (built in about 2 s, a crater in 45 ms). Large worlds (2026-11-23): a prefab
instance's children are derived data in the `.gamecraft` file (only changed or removed children
are written; prefabs inside prefabs), the world-file cap is a host setting (512 MiB default),
and a project's layers can be split into spatial cells (`cell_size_m`, `project.cells`,
`project.open {cells}`), and a world file loads from a cooked binary cache when it is current.
Roads (2026-11-23): a `road` component (a polyline with a width) whose surface is draped on
the terrain's triangles, junctions ordered by a tier so no two surfaces are coplanar, drawn per
terrain chunk, hit by rays, lines of sight and projectiles with the road's id (`road.add`,
`road.set`; `decisions.md`, *Roads are a component*). Open: streaming cells near the player in
the renderer and the simulation, derived children in layer files, cooking projects, flattening
the terrain under a road.


Worlds of real size, made of reusable parts, by anyone.

- Entities get full orientation; layers.
- Packs: a directory of text files (worlds, prefabs, resources, a manifest) in the engine's own
  formats. A game is a list of packs plus its own data, and a mod is one more pack layered on
  top (`decisions.md`, *Engine, packs, games*).
- Resources: glTF models and textures brought in by command, with licence, author and source
  recorded: done 2026-11-19 (`model` component, `asset.import`, `asset.list`, `asset.credits`,
  solid models in the simulation, PBR materials in the renderer; `examples/assets-demo`).
- The first pack, "nature": terrain materials, skies and props from VisionCraft's asset library,
  and the plant species moved out of the engine.
- A second example game, a small puzzle of GameCraft's own design that loads no pack: the guard
  that the engine assumes no genre.
- Generators as document entries: forests and scatter by area, density and slope; roads and
  rivers along splines that shape the terrain (roads that lie on the terrain: done 2026-11-23;
  shaping the terrain under them is open).
- Terrain: sculpting as edits over the source, painted material layers, tiles streamed and
  refined by distance, positions relative to the camera.
- Water: sea level, lakes, rivers, through VisionCraft's water.
- The renderer adapter updates only what changed; plants as instanced items with levels of
  detail and impostors.

### Open: found by building one showcase block (2026-11-24)

One 100 × 100 m courtyard-house block was built with the current engine and no engine work.
What it still lacks, in the order it hurts the picture (details, commands and proposals:
the project's `ENGINE-WISHLIST.md`):

- **Decals** for dirt, stains, bullet pocks, posters: walls read freshly built without them.
- **Road and terrain surface detail:** roads are one flat colour (no grain, patches, markings);
  terrain layers are colour plus roughness, with no detail texture or ruts in lanes.
- **Ambient occlusion** (SSAO or baked per-vertex): facade recesses, lane corners and the ground
  under cars and gates read flat.
- ~~**Shared textures between model assets**~~: done 2026-11-25. `asset.import` moves a model's
  images into the pack's texture store (`assets/textures/<content hash>`), each with its own
  licence record, listed once by `asset.list` and `asset.credits` (`docs/design/projects.md`).
- ~~**Distance LOD and culling for models**~~: done 2026-11-25. A glTF model's levels
  (`_LOD<n>` names, `MSFT_lod`) are drawn by distance and the model is culled beyond a draw
  distance, both from the file (`asset.extras.gamecraft`, `MSFT_screencoverage`), the
  `model` component (`lod_distances_m`, `draw_distance_m`) or a rule from its size;
  `asset.list` lists the levels and `view.render` reports what each model drew per level
  (`docs/design/projects.md`; `decisions.md`). No mesh is simplified yet; plants still have
  one level in the adapter (VisionCraft's application gives them levels and impostors).
- **Model collision finer than the bounding box:** recessed doorways and balconies are cosmetic.
- **Wires and cables as splines** instead of fixed-length models.
- **Emissive windows** and a cheap interior suggestion for dusk.
- **Export that carries assets:** `project.export` yields a file whose models draw as magenta
  boxes.
- **Cameras as world data**, **prefab children that can be overridden per instance**,
  `entity.scatter` with a layer parameter and a rotation bias, and plants for dry regions
  (a true date palm, dry scrub).

### Open: found by building a large real-place map (2026-11-25)

A game built an 8 km map at 4 m from a DEM and an OpenStreetMap extract (about 62 000 entities,
295 000 with prefab children, 288 000 instances and 38 M triangles in a wide view). What it needs
from the engine, as mechanisms with no game in them, in the order they are scheduled:

1. ~~**Generated worlds are stored as cells, not as scripts.**~~ Done 2026-11-25: the guideline
   `guide/generated-worlds.md` (commit the generator and its sources, and the cells; the
   generated build script is a build artefact) and `gamecraft-cli project bake`, which runs a
   script and saves the result as a celled project in one step, replacing only what the script
   owns on a re-run (`decisions.md`).
2. ~~**Far levels of detail for groups (HLOD):**~~ Done 2026-11-25, in memory: the static shapes
   and models of a cell of 128 m merged into one mesh per material (models at their coarsest
   level), drawn beyond the cell's own items' draw distances and rebuilt only for the cells an
   edit touches; `view.render`'s `stats.hlod` says what they drew (`decisions.md`). Open: a copy
   in the project's cooked cache, plants merged too.
3. ~~**Terrain from a DEM at full resolution:**~~ Done 2026-11-25: a heightmap in tiles of 128 cells
   is a terrain source named in `world.json` by its shape and content hash, its samples files of
   the project (`worlds/<world>/terrain/heightmap/`), read by `terrain.import` from a PGM, text or
   raw 16/32-bit file, with sculpting, paint and rules over it as over any terrain
   (`decisions.md`). Open: baking a world's edits into its heightmap, a GeoTIFF decoder.
4. **A navigation mesh per cell**, built from solids and terrain, rebuilt for the cells an edit
   touches and queried by the simulation (walls, gates, bridges, rooftops); part of M7, after
   cells are streamed near the player.
5. **A generic geodata importer** (optional): a DEM and an OpenStreetMap extract to a pack
   (frame, terrain, roads, building footprints, water, licence records); rules of a place stay in
   the game.
6. ~~**A ground-height query while building**, and spans (decks) whose ends snap to the
   ground,~~ done 2026-11-25: `query.ground {points}` gives the ground (height, normal, slope)
   at up to 65 536 points in one call, from the document as it is being built; `at` takes an
   `offset_m`; `entity.add` and `prefab.spawn` take `span {from, to, offset_m?, length_m?}`,
   which places a deck level on the higher end's ground, facing `from` to `to`, scaled to span
   the ends, and says in `ends` how far each end is above its ground (`decisions.md`). Open: a
   span scales uniformly until item 7, and cannot tilt (full orientation, M3).
7. **Non-uniform scale for prefab instances** (one deck prefab for every span length).
8. **Raycasts and simulation at scale:** a spatial index for rays, lines of sight and solids, so a
   world of about 300 000 entities (prefab children included) raycasts in well under a
   millisecond and simulates faster than real time.
9. **Filtered shadows** (PCF, soft edges) instead of stair-stepped shadow edges at eye level.
10. **Water as a surface kind in the simulation:** a character in water wades (slower, not
    standing on the ground); part of the water item above.
11. **Collision from a model's own geometry:** per-mesh or convex-part colliders, or a collision
    mesh shipped in the glTF (nodes named `*-collider`), so a building can be walked under and
    onto (see also *Model collision finer than the bounding box* above).
12. **A world finds its pack's assets:** `view.render` and `project.open` of a world file take the
    pack (`--pack DIR`) or the world records which pack it belongs to, so models do not fall back
    to placeholder cubes.
13. **Weapon animation without a skeleton:** a model's parts move on keyframes, with no skinning.
    The glTF node animations (translation, rotation, scale) are read and played. Clips are
    started by events: `fire` on a launcher's shot, `reload` on a reload, `dry` on an empty
    magazine. Recoil is a short kick on the weapon and the first-person camera that springs back.
    Muzzle flash is a light and an emissive quad for one or two frames.
    All of it is render-side: the simulation only gives the moment of the event, so the pinned
    hashes stay. The keyframe playback in the renderer may need VisionCraft, as the shadow
    filter did. Skinned characters stay in M6.

## M4: Physics

Rigid bodies, colliders from shapes and models, ray and shape casts, joints, then vehicles. All
deterministic. Whether GameCraft writes its own or wraps an existing pure-Rust library is an open
decision (`decisions.md`).

## M5: Gameplay and scripts

Game rules as data and code that a pack can bring: inventories, damage, equipment, the game
mode. The scripting model is an open decision (`decisions.md`). With it, the first "military"
pack: weapons, ballistics, damage.

## M6: Characters and animation

Skinned models, animation clips and blending, a locomotion graph, inverse kinematics for feet on
terrain. Needs skinning in VisionCraft's renderer.

## M7: Navigation and AI

Navigation data derived from terrain and solids, path finding, steering, behaviours; characters
driven by an agent outside the engine through the same input records as a player.

## M8: Audio

Positional sound, ambience that follows weather and place, with a description an agent can read
of what is audible where.

## M9: Many players

A headless authoritative server, replication of entities and components, prediction on the
client, joining from a browser; several game masters and many players in one world.

## M10: Interface

Menus and a heads-up display described as data, drawn in the window, readable and operable
through commands.

## M11: Mods and distribution

Packaging a game and its mods, a web export, versioning and migration of the formats; a place
in the browser where a person describes a game and plays what an agent built.
