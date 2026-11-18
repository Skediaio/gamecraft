# Skirmish

A small map, 384 m across, with rolling hills, a few trees, a low wall line to the south, two
block houses and a barn as cover, a light wind (5 m/s from the west) and some haze. You are the
blue rifleman, with a rifle and 100 health. Four red enemies hold the map: two patrol routes,
one guards the houses, one sits on the hill. Clear the village and stay alive.

Everything here was made with engine commands only, nothing else.

| File | What it is |
|---|---|
| `build.json` | The `gamecraft-cli run` script that builds the whole world: terrain, environment, trees, cover, player, enemies with their behaviour, objectives, variables and triggers |
| `world.gamecraft` | The result of that script (a test checks that building it again gives the same world) |
| `playthrough.json` | A `gamecraft-cli play` script: a scripted player that moves from cover to cover, aims every shot with a correction for drop and wind, and wins at 52.8 s with 43 health left |

## The weapons

- **Player rifle: 7.62 x 51 mm**, 168 grain (10.9 g) bullet at 800 m/s, G7 ballistic
  coefficient 0.223 lb/in2 (199.6 kg/m2 in the engine's units), 0.3 mrad dispersion, one shot
  every 0.8 s, 10 rounds, 3 s reload. A hit carries about 3.5 kJ; at `damage_per_joule` 0.05 one
  hit takes more than an enemy's 100 health.
- **Enemy rifles:** lighter, 4 g at 715 m/s, BC 100 kg/m2, 2 mrad dispersion, a shot every
  1.2 s. A hit takes about 10 health (`damage_per_joule` 0.01).

## Tuning (so a plain script can win)

The enemies see 120 m and engage within 100 m, react after 1.0 s and their aim error is 8
mrad (a cone of 0.8 m radius at 100 m); the player's rifle is lethal in one hit while theirs
takes ten. The player's script only shoots targets within 75 m with a clear line, and goes
round the wall line (which is solid) to reach each cover.

## Build

```sh
gamecraft-cli run --new '{"name": "Skirmish", "seed": 21}' \
    --script examples/skirmish/build.json --out examples/skirmish/world.gamecraft
```

## Play in the CLI

```sh
# The scripted player wins: both objectives "done", the player not down.
gamecraft-cli play examples/skirmish/world.gamecraft \
    --script examples/skirmish/playthrough.json --seconds 60
# Doing nothing useful loses: enemies that see you shoot you down, "Stay alive" and
# "Clear the village" both fail.
gamecraft-cli play examples/skirmish/world.gamecraft --seconds 120
```

`playthrough.json` holds the inputs only (`sim.input` with `face`, `yaw_deg`, `pitch_deg`,
`fire`, `sim.step`), as an agent or a keyboard would send them. Each shot's `yaw_deg` and
`pitch_deg` were worked out by flying a test shot with the engine's own G7 drag, air density and
wind profile until it hit the target's chest, leading a moving target by its velocity.

## Open it in the window

```sh
gamecraft examples/skirmish/world.gamecraft
```

## What a game master can do while it runs

Start `gamecraft-cli serve --file examples/skirmish/world.gamecraft` (or `gamecraft-cli mcp`),
send `sim.start`, then step and steer:

- Turn the weather against the shooter: `environment.set {"wind_speed_mps": 18, "rain": 0.6,
  "mist": 0.8}` bends the shots and cuts how far the enemies (and you) see.
- Send reinforcements: `entity.add` a copy of an enemy (a `character`, `health`, `launcher`,
  `behaviour` with `"team": "red"` and the tag `enemy`) with `"at": [60, -40]`, then
  `var.set {"name": "enemies_left", "add": 1}` so the count stays right.
- Take an enemy over and play it: `sim.input {"entity": "Hill Sentry", "face": [0, 48],
  "fire": true}` makes it fire where you say, and `sim.release {"entity": "Hill Sentry"}` gives
  it back to its behaviour.
