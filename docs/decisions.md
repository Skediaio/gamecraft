# Decisions

Decisions taken, newest first, each with its reason; then the open ones. The developer steers the
product and the architecture and leaves how it is built to the orchestrator (2026-11-18): an
implementation decision is taken by the orchestrator, written down here with its reason, and
stands until the developer overrules it. A fork in the architecture or the product goes to the
developer as a recommendation.

## Taken

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
  Preferred: a SQLite-compatible store, because agents and developers already speak SQL; best
  the pure-Rust rewrite from the libSQL/Turso people (MIT) if it is mature when M3 needs it,
  otherwise SQLite natively and no index in the browser. Not SurrealDB: its Business Source
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

### Toolchain pin

VisionCraft follows stable Rust without a pin; 3DCraft pins a version. A pin keeps new clippy
lints from breaking CI at random; no pin keeps the setup simple.

### A shared automation crate

The command spec, the JSON-lines protocol and the MCP server exist three times now (3DCraft,
VisionCraft, GameCraft), ported each time. One shared crate would end that, at the price of a
dependency between the projects.
