# Skirmish at night (a mod)

A mod of the [Skirmish](../skirmish/README.md): a pack that depends on the game and changes it
by id, without touching a file of the game (`docs/design/projects.md`).

| File | What it changes |
|---|---|
| `pack.json` | The pack `skirmish-night`, depending on `skirmish` in `../skirmish/project` |
| `worlds/village/world.json` | Night with mist: `time_of_day_h` 23, `mist` 0.5, `cloud_cover` 0.6. Only these fields; the rest of the game's `world.json` stays |
| `prefabs/enemy-soldier.json` | The game's soldier prefab with a `light` added: every enemy carries a lantern |
| `worlds/village/layers/night-watch.json` | A new layer with one more enemy, the Night Watch, between the houses |

```sh
gamecraft-cli project info examples/skirmish-night-mod
gamecraft-cli project export examples/skirmish-night-mod --out night.gamecraft
gamecraft-cli render examples/skirmish-night-mod --out night.png
```

Edit the game's entities with the mod open (`project.open {path: "skirmish-night-mod"}`, then
`entity.set`, `project.save`) and the changes are written here as overrides, one line each in a
layer of the same name; the game's files stay as they are. A test checks that the mod changes
exactly this and nothing else (`apps/gamecraft-cli/tests/projects.rs`).
