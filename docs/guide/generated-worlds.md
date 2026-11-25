# Generated worlds

*A world that a program builds, from thousands to hundreds of thousands of entities, is kept in
git as the project `gamecraft-cli project bake` writes, never as the program's output.*

## Commit the result, not the script

An importer or a generator writes a script of commands (a JSON list of steps for `gamecraft-cli
run --script`). That script is a build artefact: one large file, unreadable diffs, and a world
rebuilt from scratch on every change. What belongs in git is:

- **the generator and its sources** (the program, its inputs and its licence records), so the
  world can be made again;
- **the project folder** the bake writes: `pack.json`, `prefabs/`, `worlds/<world>/world.json`
  and the layers as cells (`worlds/<world>/layers/<layer>/<cx>_<cz>.json`,
  [`design/projects.md`](../design/projects.md)), each file small, one entity per line, so an
  edit, a review and a merge show exactly what changed.

Add the generated script (and any other intermediate output) to `.gitignore`.

```sh
# make the script, then bake it into the project `world` as cells of 512 m
my-importer sources/ > build/gen.json
gamecraft-cli project bake build/gen.json --into world --id world --cell-size-m 512

# bake it again after the sources, the importer or the script changed
gamecraft-cli project bake build/gen.json --into world
git diff world
```

## What a bake replaces

A bake into a project that does not exist yet makes it (it needs `--id`); nothing is written
until every step has run. A bake into an existing project replaces **what the script owns** and
nothing else, so hand edits survive:

- **the layers the script writes**: every layer a step names (`"layer": "roads"`), the project's
  default layer (where steps that name no layer add their entities) and the layers the previous
  bake owned. Every entity of those layers is removed first, then the script runs. An entity
  added by hand in another layer, a prefab, an asset, a world field the script does not set: all
  stay.
- **the terrain edits the script added**: the bake records where in the list of edits its own
  edits begin and end (`worlds/<world>/bake.json`) and removes them before it runs again. When
  something was sculpted by hand after the last bake, it refuses and says so (a re-run would
  stack its edits on top); remove those edits with `terrain.remove_edit`, or bake into another
  folder.

The record `worlds/<world>/bake.json` lists the owned layers; it is a file of the project like
any other and is committed with it.

## Regenerating

The same script gives byte for byte the same files: ids, order and text are stable and nothing
holds a clock reading. So `git diff` after a re-run shows only what the script changed: a moved
entity changes the one or two cells it left and entered, nothing else. Edit the project by hand
in other layers, never in the layers a script owns (the next bake would remove the edit).

The ids of a celled project follow the order of its cells (x, then z), so a script that adds
entities in another order gives other runtime ids, and another state hash, than the same script
run directly; with one file per layer (`--cell-size-m 0`) the order is the script's.

## Large worlds

The script is read one step at a time; progress goes to stderr every 5 000 steps. A hundred
thousand entities bake in about 15 seconds and 600 MB of memory, and a re-bake of the same
script into the same project takes about as long and writes nothing.
