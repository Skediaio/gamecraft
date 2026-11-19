# Skirmish

A small map, 384 m across, with rolling hills, a few trees, a low wall line to the south, two
block houses and a barn as cover, a light wind (5 m/s from the west) and some haze. You are the
blue rifleman, with a rifle and 100 health. Four red enemies hold the map: two patrol routes,
one guards the houses, one sits on the hill. Clear the village and stay alive.

Everything here was made with engine commands only, nothing else.

| File | What it is |
|---|---|
| `build.json` | The `gamecraft-cli run` script that builds the whole world: terrain, environment, trees, cover, player, enemies with their behaviour, objectives, variables and triggers. The enemies are placed from one prefab, `enemy-soldier` (the patrols override its `behaviour`), and the houses from a prefab `house` (House 2 overrides its `shape`) |
| `world.gamecraft` | The result of that script (a test checks that building it again gives the same world) |
| `playthrough.json` | A `gamecraft-cli play` script: a scripted player that moves from cover to cover, keeps each enemy in its sights with `face_entity`, and wins at 53.3 s with 43 health left |
| `directed.journal.json` | The journal of the same playthrough directed by a scripted game master (`apps/gamecraft-cli/tests/game_master.rs`): reinforcements behind the village after two enemies are down, fog when the player is below half health, the wind turned when the player reaches the houses. This time the player goes down and both objectives fail. `gamecraft-cli replay examples/skirmish/directed.journal.json --check` |

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

`playthrough.json` holds the inputs only (`sim.input` with `face`, `face_entity`, `fire`,
`sim.step`), as an agent or a keyboard would send them. `face [x, z]` is held, so one input walks
the player to the next cover; `face_entity` keeps an enemy in the sights, turning every tick and
aiming the pitch at its chest with the drop allowed for, so `fire` hits. Within 75 m the wind
moves the bullet by about a centimetre, so the script needs no wind correction. A script is a
fixed list and cannot use the answer of a query; an agent that plays over MCP can ask
`query.aim {"from": "Player", "to": "Hill Sentry"}` for the exact yaw and pitch against drag
and wind (and `drop_m`, `drift_m`, `time_of_flight_s`) and send them as `yaw_deg` and
`pitch_deg`.

The triggers count the enemies down in one message (`"{target} is down, {var:enemies_left}
left"`), and the end triggers read the objectives: the village is only cleared while `alive`
is open, and the player is only lost while `clear` is open. The trees keep the wall line, the
village, the barn and the start clear (`exclude` of `entity.scatter`).

## Record and replay a session

```sh
# Writes the journal of the run: every command with its tick and the state hash after it.
gamecraft-cli play examples/skirmish/world.gamecraft \
    --script examples/skirmish/playthrough.json --seconds 60 --journal skirmish.journal.json
# Runs it again from the world it embeds and compares every hash; exit 1 names the first tick
# that differs. --to-tick N stops there and prints sim.state, to bisect.
gamecraft-cli replay skirmish.journal.json --check
```

## Open it in the window

```sh
gamecraft examples/skirmish/world.gamecraft
```

## What a game master can do while it runs

Start `gamecraft-cli serve --file examples/skirmish/world.gamecraft` (or `gamecraft-cli mcp`),
send `sim.start`, then look with `gm.overview`, step and steer. The game master's guide,
[`docs/guide/game-master.md`](../../docs/guide/game-master.md), has ten such moves as exact
commands; a few:

- Turn the weather against the shooter: `environment.set {"wind_speed_mps": 18, "rain": 0.6,
  "mist": 0.8}` bends the shots and cuts how far the enemies (and you) see.
- Send reinforcements: `prefab.spawn {"prefab": "enemy-soldier", "at": [60, -40], "name":
  "Reinforcement", "overrides": [{"type": "behaviour", "mode": "patrol", "route": [[60, -40],
  [20, -40]], "team": "red"}]}` drops a soldier who patrols from the next tick, then
  `var.set {"name": "enemies_left", "add": 1}` so the count stays right.
- Move a squad: `gm.order {"entity": ["Patrol Alpha", "Patrol Bravo"], "order": "move",
  "point": [4, -62]}` pulls both patrols back to the houses; `"order": "patrol"` sends them on
  their rounds again.
- Take an enemy over and play it: `sim.input {"entity": "Hill Sentry", "face": [0, 48],
  "fire": true}` makes it fire where you say, and `sim.release {"entity": "Hill Sentry"}` gives
  it back to its behaviour.
