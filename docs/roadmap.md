# Roadmap

What is planned, in order. Updated 2026-11-18. Nothing here is a promise of a date. A milestone
is done when its *Done when* holds and `cargo xtask ci` is green; one with something to see is
done when it also runs in Chromium on WebGPU.

The order follows the three promises of `AGENTS.md`: first a world an agent can build and play
(M0, M1), then a running game that can be steered (M2), then size, depth and mods.

## Current focus

**M0, the foundation.** Then M1, so there is something to walk in.

## M0: Foundation (in progress)

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
| `gamecraft-cli`: `commands`, `run`, `batch`, `serve`, `mcp`, `render` | open |
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

**Done when:** an agent plays each from start to finish over MCP, a second run gives the same
state hash, and a picture of each exists.

## M1: Walk

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

## M2: Direct

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

## M3: World

Worlds of real size, made of reusable parts, by anyone.

- Entities get full orientation; layers.
- Packs: a directory of text files (worlds, prefabs, resources, a manifest) in the engine's own
  formats. A game is a list of packs plus its own data, and a mod is one more pack layered on
  top (`decisions.md`, *Engine, packs, games*).
- Resources: glTF models and textures brought in by command, with licence, author and source
  recorded.
- The first pack, "nature": terrain materials, skies and props from VisionCraft's asset library,
  and the plant species moved out of the engine.
- A second example game, a small puzzle of GameCraft's own design that loads no pack: the guard
  that the engine assumes no genre.
- Generators as document entries: forests and scatter by area, density and slope; roads and
  rivers along splines that shape the terrain.
- Terrain: sculpting as edits over the source, painted material layers, tiles streamed and
  refined by distance, positions relative to the camera.
- Water: sea level, lakes, rivers, through VisionCraft's water.
- The renderer adapter updates only what changed; plants as instanced items with levels of
  detail and impostors.

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
