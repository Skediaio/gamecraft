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

## Open

- Terrain tiles and painted layers on disk (binary, later in M3).
- Asset import commands and licence records.
- Version ranges and conflicts between two mods that change the same thing (last one wins, with a
  warning naming both).
