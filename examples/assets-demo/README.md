# Assets demo

A small pack that tests models in packs (`docs/design/projects.md`, *Assets*): a yard of 64 m
with three wooden crates (solid, one stacked on the others), three fence segments that are not
solid, and two characters, the Walker in front of the crates and the Hiker in front of the
fence.

| File | What it is |
|---|---|
| `pack.json` | The pack `assets-demo` with the world `yard` |
| `worlds/yard/` | The world: a flat terrain, an afternoon sun, and the layer `yard` with the crates, the fences and the two characters |
| `assets/models/crate.glb`, `fence.glb` | The two models, GLB with one textured material each (a 0.9 m crate of 12 triangles, a 2 m fence segment of 48), and their licence records `*.license.json` (CC0-1.0, the GameCraft project as author) |
| `tools/make_models.py` | The script that made them, standard library only: `python3 examples/assets-demo/tools/make_models.py` writes the same bytes again |

```sh
gamecraft-cli run examples/assets-demo --cmd asset.list
gamecraft-cli render examples/assets-demo --out yard.png
gamecraft examples/assets-demo        # walk it in the window

A `.gamecraft` file saved inside the pack (say `assets-demo/yard.gamecraft`) finds the pack above
it and draws its models; the same file elsewhere needs `--pack examples/assets-demo`.
```

A test walks both characters north (`crates/gamecraft-automation/tests/assets.rs`): the Walker
stops at the crate's face, the Hiker walks through the fence.
