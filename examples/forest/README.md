# Forest

A valley 2 km across with about 50 000 trees in three forests: spruces, firs and pines on the
northern slopes, beeches, oaks and hornbeams in the east, birches, rowans and pines in the west,
a road across the valley floor that every forest keeps clear, two clearings, boulders scattered
over the whole map, and a ranger standing on the road. It exists to test the generators
(`docs/design/generators.md`): the whole forest is four entries of the world, not 50 000
entities.

Everything here was made with engine commands only, nothing else.

| File | What it is |
|---|---|
| `build.json` | The `gamecraft-cli run` script that builds the world: the terrain (noise, a lowered basin, a ramped road, painted layers), the weather, three `forest` generators and one `scatter` of boulders, and the ranger |
| `world.gamecraft` | The result of that script: the recipes only (a test checks that building it again gives the same world); the trees are derived when it opens |

## Build

```sh
gamecraft-cli run --new '{"name": "Forest", "seed": 5}' \
    --script examples/forest/build.json --out examples/forest/world.gamecraft
```

## Look at it

```sh
gamecraft-cli render examples/forest/world.gamecraft --out out/forest.png --size 1280x720 \
    --camera '{"position": [-700, 140, 120], "target": [100, 0, -150], "fov_y_deg": 50}'
gamecraft-cli run examples/forest/world.gamecraft --cmd generator.list --params '{"recipes": false}'
```

## Measured

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
