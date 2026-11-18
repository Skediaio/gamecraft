# Decisions

Decisions taken, newest first, each with its reason; then the open ones. The developer steers the
product and the architecture and leaves how it is built to the orchestrator (2026-11-18): an
implementation decision is taken by the orchestrator, written down here with its reason, and
stands until the developer overrules it. A fork in the architecture or the product goes to the
developer as a recommendation.

## Taken

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

### The project on disk (blocks M3)

One file per world works for M0. A game needs worlds, prefabs, resources and mods: a directory
of text files with a layering rule, and perhaps a single-file bundle for distribution.

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
