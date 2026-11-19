# Generators

A forest of tens of thousands of trees, or a scatter of rocks, as one entry of the world
document: an area, a density, the items with their weights and a seed. What it places is derived
(AGENTS.md rule 2), the same everywhere, seen by everything, editable while the world runs and
stored in projects as a recipe. Status: built 2026-11-19 (M3, `roadmap.md`).

## The entry

```json
{"id": "beech-wood", "kind": "forest",
 "area": {"circle": {"center": [420, 330], "radius_m": 470}},
 "seed": 202, "density_per_ha": 220, "min_distance_m": 4, "slope_max_deg": 30,
 "height_range_m": [-50, 120],
 "exclude": [{"circle": {"center": [300, 250], "radius_m": 70}}, {"polygon": [[x, z], ...]}],
 "items": [
   {"weight": 4, "components": [{"type": "plant", "species": "fagus-sylvatica"}],
    "scale_range": [0.8, 1.2], "growth_range": [0.6, 1.0], "variants": 3,
    "trunk": {"radius_m": 0.35, "height_m": 15}},
   {"weight": 2, "prefab": "oak"}
 ]}
```

- **`kind`**: `forest` (every item has a `plant`; every tree has a trunk that blocks) or
  `scatter` (anything: rocks, crates, bushes; each instance is what its components say).
- **`area`**: `{"circle": {center, radius_m}}` or `{"polygon": [[x, z], ...]}` (3 to 1024
  corners, the even-odd rule). `exclude` takes the same areas, at most 256.
- **`density_per_ha`**: instances per hectare of the area (above 0, at most 10 000);
  `requested` is the density times the area, at most 250 000 per generator and 1 000 000 for all
  the generators of a world. Fewer stand where the spacing, the ground or an exclude turns points
  away.
- **Items** (1 to 64): `weight` (the share), `components` (as in `entity.add`) or `prefab` (a
  prefab of the world; its children are not placed), `scale_range` (default `[1, 1]`),
  `random_rotation` (default true), and for plants `growth_range` (cut into 3 growth classes),
  `variants` (the plant seeds the instances share, the plant's seed and the ones after it, 1 to
  16, default 3) and `trunk` (`{radius_m, height_m}` at scale 1 and growth 1; a forest item
  without one gets 0.2 m by 6 m from a file, and its species' from `generator.add`; radius 0 is
  no trunk). No `character` (generated things do not move).
- **Ids**: the instances are `<id>#<n>`, `n` from 1, stable (see below). A generator id is 1 to
  64 ASCII letters, digits, `-` and `_`.

The `.gamecraft` file holds the list as `generators` (left out when empty, so other worlds write
and hash as before). A project holds it in `worlds/<world>/generators.json`.

## Placing

Placing runs in two stages, so that a change touches only what it should
(`gamecraft_world::Generated`, `generated.rs`):

1. **Candidates** come from the recipe alone: `requested` points drawn evenly over the area by
   a stream forked from the seed (inside a circle by radius and angle, inside a polygon by
   rejection from its bounding rectangle), each kept only when it is at least `min_distance_m`
   from every point kept before it (at most 30 draws per wanted point). Each kept point draws
   five numbers from a second stream: its item (by weight), its plant seed, its growth class,
   its scale and its turn, five whatever the item uses, so one item's settings never shift
   another's numbers. Candidate `n` is instance `<id>#<n>`.
2. **Placing** tests each candidate on its own: not inside an exclude, ground under it (the
   terrain's `HeightField`), no steeper than `slope_max_deg` (from the triangle's normal, with
   `patan`), within `height_range_m`. It stands on the ground there.

Only `+ - * /`, `sqrt`, `floor`, `psin_cos` and `patan`: the same bits on every platform. The
same entry on the same terrain gives the same instances in the same order.

What follows from the two stages:

- **Editing the terrain** under a forest re-runs the second stage over the kept candidates:
  every tree away from the edit keeps its place and number; those under it move up or down,
  appear or go (a test raises a hill under a forest and compares every other tree bit for bit).
- **Excludes and ground limits** are second-stage tests too, so keeping a clearing, or baking a
  spot, leaves every other instance with its number.
- **Spacing does not see the terrain**: a candidate turned away by a steep slope leaves a gap
  rather than letting a neighbour move in. Density is held over open ground: a test asks for 120
  per hectare over 50 ha of flat ground with a 3 m spacing and gets them within 2 %.
- **A world without terrain** places nothing (the summary says why).

## Derived, never stored

`World::generated()` holds the sets, one per generator in order, behind `Arc`s: a world is
cloned for every command, every undo step and the running simulation, and the sets are shared,
not copied. They are derived data like the model bounds: not written to the file, not compared
by `World == World` (they follow from the generators and the terrain, which are), not in the
state hash (the recipe and the terrain are, through the world's text).

`World::regenerate(heights)` derives them. A generator whose entry and terrain are unchanged
keeps its set (the `Arc`); one whose terrain changed keeps its candidates and is placed again;
a new or changed one is derived from the start. Who calls it: the session after every change of
its document (a command, undo, redo, a new document, model bounds), the simulation in
`Sim::replace_world` (so a live command that grows a wood takes effect between two ticks), the
handlers that read the instances they just changed (`generator.bake`), and the renderer for a
world handed to it without them (`World::generated_is_current`).

A set also keeps a grid of 16 m cells of its instances: `near(point, r)` and `along(ray)` find
the few instances a character or a ray can meet in a forest of tens of thousands.

## Everything sees them

- **The simulation**: a tree's trunk is an upright cylinder (radius and height times the
  instance's scale and growth) that blocks characters as a solid cylinder does, and projectiles
  and sight as a shape does; a scattered thing blocks as its components say (a solid shape, a
  solid model with a box collider; a low shape is a step). The canopy blocks nothing: shots and
  sight pass between the trunks. Chosen because a dense forest without obstacles cannot be
  fought in, and a cylinder per trunk is what a character and a bullet meet; leaves that stop
  sight would need perception through foliage, later. The simulation keeps the generated
  solids in a grid of 16 m cells and meets only those near a character.
- **Rays** (`gamecraft_sim::ray::cast`: `query.raycast`, `query.line_of_sight`, projectiles,
  perception) test the trunks, shapes and model boxes of the instances along the ray (from the
  grid); a hit names the instance (`Found::generated`: the set and the number).
- **Queries**: `entity.find` lists instances with the entities (`id` `<generator>#<n>`,
  `generated: true`; their name is their id; `generated: false` leaves them out), `entity.get`
  takes `<generator>#<n>`, `query.raycast` answers `kind: "generated"` with the instance,
  `gm.overview` counts each generator's instances (near a point too), `world.inspect` lists the
  generators and the total.
- **The renderer** draws them instanced: one item per plant variant (species, seed, growth
  class) with an instance per tree, its three levels of detail from the plant library (`Lod0`
  to `Lod2`) and, for a tree (3 m and taller), an impostor baked from the middle level beyond
  them, with VisionCraft's distances (`apps/visioncraft/src/plants.rs`: levels ending at
  `4 + h`, `10 + 2 h`, `25 + 3 h` metres, the impostor to where the tree is 1.5 pixels tall).
  The variants are grown once and kept from one rebuild to the next (a rebuild for another
  change grows nothing again). Scattered shapes and models join the entities' shape and model
  items. A forest of 30 000 trees of a few species is a few hundred draws in all passes, as is
  one of 10 000 (a test). Plants placed as entities are drawn as before.

## Commands

All in `gamecraft-engine/src/generator_cmds.rs`. The changing ones are World commands: one undo
step each, and they work on a running world (the game master grows a wood in the fog).

| Command | What it does |
|---|---|
| `generator.add` | Adds an entry; answers `{id, kind, requested, candidates, placed, per_item, bounds, note?}`. A forest item without `trunk` gets its species' from the plant library (radius 1.4 % of the mature height, 0.05 to 1.5 m; height 60 % of it; none for shrubs and smaller plants), written into the entry so the recipe says it |
| `generator.set` | The fields given replace the entry's (`null` clears an optional one) |
| `generator.remove` | Removes the entry and what it placed; baked entities stay |
| `generator.list` | Every generator with its counts, and its entry (`recipes`) |
| `generator.preview` | The counts and bounds of an existing generator (`id`) or of an entry not yet added (`recipe`), without placing anything |
| `generator.bake` | Turns the instances inside a circle into ordinary entities (named `<id>#<n>`, with their place, turn, scale and components; a prefab item's stays an instance of the prefab) and adds the circle to `exclude`, so the rest keep their numbers; without a circle all of them, and the generator goes. At most 20 000 at once |

A test bakes a spot and checks that the baked entities and the instances left are the forest
as it was, each once; the panic hunt runs every command with hostile parameters.

## Projects

`worlds/<world>/generators.json` is `{"gamecraft_generators": 1, "generators": [...]}`, one
entry per line. Packs layer it by id, like prefabs: a later pack's entry with an earlier id
replaces it in its place, a new id is added after the others, and `{"id": "...", "removed":
true}` removes an earlier pack's generator. `project.save` writes the root pack's file as the
difference from what the dependencies give (the entries that are new or differ, and removals),
only when the generators changed; a mod that changes a game's forest holds one line. Bad files
(not JSON, another version, an unknown field, a bad entry, an id twice) are errors that name the
file; every truncation of one is an error, never a panic (tests).

The project index (`gamecraft-index`) does not list the instances yet: that crate was outside
this work (see *Open*).

## Measured

`examples/forest`: a 2 km valley (2048 m at 4 m, 513 x 513 samples, noise with a lowered basin
and a road ramped across it) with three forests and a scatter of boulders.

Measured 2026-11-19 on this machine (release build; the GPU an Intel Iris Xe, TGL GT2, Mesa
26.2.6 on Vulkan), by `the_forest_example_is_measured` (`crates/gamecraft-render/tests/forest.rs`, ignored by
default: `cargo test -p gamecraft-render --release --test forest -- --ignored --nocapture`) and
`gamecraft-cli`:

| What | Number |
|---|---|
| Instances | 50 396: 49 812 trees (`spruce-slopes` 23 903, `beech-wood` 14 222, `birch-meadow` 11 687) and 584 boulders, from four entries; `world.gamecraft` is 10 KB |
| Deriving them all | 0.08 s (the terrain builds in 0.06 s); about 3.4 MB of resident memory |
| `gamecraft-cli run` of `build.json` | 0.25 s and 19.6 MB at most (0.18 s and 16.5 MB without the generators) |
| The renderer's first sync | 4.6 s: 48 plant variants grown at three levels and their impostors baked; a rebuild for another change grows nothing again |
| A picture, 1280 x 720 (`out/forest.png`) | 3.28 million triangles, 9 675 draws in all passes, 29 135 instances drawn (the valley without its forests: 1.75 million, 215, 43) |
| A live frame, 1280 x 720, each waited for | 22.6 ms at LOW, 26.5 ms at MEDIUM (11.8 and 14.6 ms without the forests) |

The draws follow the variants and the forest's extent, not the trees: VisionCraft draws, per
item, part and pass, each run of neighbouring 8 m cells that share a level of detail. Three
times the trees in the same circle (10 053 to 30 159) went from 455 to 538 draws (a test); the
48 variants over 2 km here make 9 675. Fewer variants (`variants`, `growth_range`) mean fewer
draws.

## Open

- The project index: a table `generated(key, generator, number, item, x, y, z, rotation_y_deg,
  scale, components)` filled from `World::generated`, so `project.query` finds instances
  (`gamecraft-index`).
- Roads and rivers along splines that shape the terrain and keep forests clear.
- Foliage that hides (sight through leaves), wind-thrown trees, felling a tree (bake it first).
- The renderer rebuilds every item when anything changes (the plants themselves are kept);
  with a 50 000-tree forest a rebuild re-sorts the instances, which a change set would avoid
  (`architecture.md`, *Structural debt*).
- Instances of a hit (`hit` events) name no generated instance yet: the event's `target` is
  null, as for the terrain.
