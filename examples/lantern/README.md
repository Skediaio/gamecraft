# Lantern

Night in a small valley, 256 m across, with gentle hills, oaks, birches and pines (real species
of the plant library) and a few rocks. You carry the only light, a lantern. Five dark beacons
stand over the valley, one of them on the hill in the east. Light all five before the mist
comes: it thickens at 120, 180 and 240 s, and at 300 s the game is lost if a beacon is still
dark.

Everything here was made with engine commands only, nothing else.

| File | What it is |
|---|---|
| `build.json` | The `gamecraft-cli run` script that builds the whole world from nothing: terrain, environment, trees, rocks, the player, the beacons, objectives, variables and triggers |
| `world.gamecraft` | The result of that script (a test checks that building it again gives the same world) |
| `playthrough.json` | A `gamecraft-cli play` script: a player that walks to each beacon with `face` and `forward` and lights it with `interact`. It wins at 86.9 s |

## Build

```sh
gamecraft-cli run --new '{"name": "Lantern", "seed": 11}' \
    --script examples/lantern/build.json --out examples/lantern/world.gamecraft
```

## Play in the CLI

```sh
# The scripted player wins; the state line ends with every objective "done" and the hash.
gamecraft-cli play examples/lantern/world.gamecraft \
    --script examples/lantern/playthrough.json --seconds 125
# Standing still loses at 300 s: the main objective is "failed".
gamecraft-cli play examples/lantern/world.gamecraft --seconds 305
```

The hash of the first run is the same on every run and every machine (the test pins it).

## Open it in the window

```sh
gamecraft examples/lantern/world.gamecraft
```

## What a game master can do while it runs

Start `gamecraft-cli serve --file examples/lantern/world.gamecraft` (or `gamecraft-cli mcp`),
send `sim.start`, then step and steer with the same commands that built the world:

- Bring the mist early: `environment.set {"mist": 0.9}` (the player still has until 300 s;
  add `"wind_speed_mps": 8` for a wind with it).
- Move a beacon: `entity.set {"id": "Beacon Hill", "at": [60, 40]}` puts the hill beacon
  somewhere easier (or harder) to reach.
- Hand out a shortcut or a goal: `var.set {"name": "lit", "add": 1}` counts one more beacon as
  lit without anybody walking there, and `objective.add {"id": "bonus", "title": "Reach the
  pines by 60 s"}` gives the player a new goal.

## The numbers

Valley: noise terrain 256 m, 2 m cells, relief 30 m, seed 11. Player at the centre, walk speed
1.6 m/s, run speed 5 m/s (the playthrough runs). Beacons: Meadow (-45, -35), Pines (-30, 60),
Ford (60, 75), Hill (100, 5), Ridge (30, -85); each lights when the player faces it within 2 m
and interacts.
