# GameCraft

An open-source game engine built with and for AI agents, in Rust.

- **Anyone can build a game in it.** Every action is a command with JSON parameters, reachable
  from `gamecraft-cli` and over MCP. You say what you want; an agent builds the world, plays it,
  looks at it and tests it.
- **Everything can be changed.** A game is data layered over the engine, and a mod is one more
  layer.
- **A running game can be steered.** The commands that build a world also change it while it is
  being played: a game master, person or agent, places things, moves them and turns the weather.

It draws with the wgpu renderer of [VisionCraft](https://github.com/Skediaio/visioncraft) and
follows the principles of [3DCraft](https://github.com/johancoppens/3dcraft): everything is a
command, never crash, deterministic, tested by machines.

Status: version 0.0.0, milestone M0 (the foundation). See the [roadmap](docs/roadmap.md).

## Workspace layout

Dependencies point downwards only.

| Path | What it is |
|---|---|
| `apps/gamecraft-cli` | `gamecraft-cli`: headless command line, JSON-lines server and MCP server. |
| `crates/gamecraft-automation` | Command execution over a host, the JSON-lines and MCP protocols. |
| `crates/gamecraft-render` | Draws a world with VisionCraft's renderer and plant generator. No UI. |
| `crates/gamecraft-engine` | Session and command registry: every action is a command. Headless. |
| `crates/gamecraft-sim` | The deterministic fixed-step simulation. |
| `crates/gamecraft-world` | The world document: terrain, environment, entities, components. |
| `crates/gamecraft-math` | Portable maths, seeded randomness, noise, the state hash. |
| `xtask` | The workspace's own checks: `cargo xtask layers`, `wasm`, `ci`. |

Rules are in [AGENTS.md](AGENTS.md), the map in [docs/architecture.md](docs/architecture.md),
decisions in [docs/decisions.md](docs/decisions.md). Code attribution is in
[ATTRIBUTION.md](ATTRIBUTION.md).

## Build and check

Needs a recent stable Rust toolchain and the `wasm32-unknown-unknown` target. The VisionCraft
crates are fetched by Cargo from GitHub.

```sh
cargo xtask ci
```

runs formatting, clippy, the tests, the layering check and the wasm check. Details and the
VisionCraft dependency: [docs/development.md](docs/development.md).

## Licence

MIT, see [LICENSE](LICENSE).
