# Code attribution

Where code and methods in this repository come from. GameCraft is MIT; nothing here is copied
from closed software. Papers and articles are cited for the published method; the code is written
here unless the licence column says otherwise. A change that ports code adds its row in the same
change.

## From VisionCraft (MIT, <https://github.com/Skediaio/visioncraft>)

| Where | Origin | Licence |
|---|---|---|
| root `Cargo.toml` | `visioncraft-render` and `visioncraft-flora` as git dependencies, pinned to one rev | MIT |
| `AGENTS.md`, `docs/` | The rules and the shape of the documents follow VisionCraft's `AGENTS.md` | MIT |
| `crates/gamecraft-engine/src/spec.rs` | Ported from `crates/visioncraft-core/src/spec.rs` (command specs, parameter checks, nearest name) and the params notation of `crates/visioncraft-automation/src/docs.rs` | MIT |
| `crates/gamecraft-render/src/light.rs` | `auto_ev`, `MIST_EV`, `NIGHT_SKY_LUX`, the natural look's grade and `haze_extinction` from `apps/visioncraft/src/studio.rs`; `daylight_lux` and `usno_atmos` (after USNO Circular 171) from `crates/visioncraft-core/src/sun.rs` | MIT |
| `crates/gamecraft-render/src/renderer.rs` | `plant_material_desc` from `apps/visioncraft/src/scene.rs`; plant parts as meshes following `apps/visioncraft/src/plants.rs` | MIT |
| `crates/gamecraft-engine/src/session.rs` | Modelled on `crates/visioncraft-core/src/session.rs` (registry, checks before the handler, snapshot undo, transactions) | MIT |

## From 3DCraft (MIT, <https://github.com/johancoppens/3dcraft>)

| Where | Origin | Licence |
|---|---|---|
| `AGENTS.md` | The golden rules (never crash, everything is a command, determinism, tests as the gate, platform not applications) follow 3DCraft's `AGENTS.md` | MIT |
| root `Cargo.toml`, `clippy.toml` | The never-crash clippy lints and the list of disallowed platform maths functions follow 3DCraft's | MIT |

## Published methods

| Where | Method | Source |
|---|---|---|
| `crates/gamecraft-math/src/rng.rs` | SplitMix64 | G. Steele, D. Lea, C. Flood, "Fast Splittable Pseudorandom Number Generators", OOPSLA 2014; constants as in S. Vigna's public-domain `splitmix64.c` |
| `crates/gamecraft-math/src/hash.rs` | FNV-1a, 64 bit | G. Fowler, L. C. Noll, K.-P. Vo, "The FNV Non-Cryptographic Hash Algorithm" (IETF draft), public domain |
| `crates/gamecraft-math/src/noise.rs` | Value noise with a quintic fade, fractal sum | K. Perlin, "Improving Noise", SIGGRAPH 2002 (the fade curve); written here |
| `crates/gamecraft-world/src/environment.rs` | Solar declination, hour angle, elevation and azimuth | P. I. Cooper, "The absorption of radiation in solar stills", Solar Energy 1969 (the declination formula); standard spherical astronomy |
