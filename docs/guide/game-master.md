# Directing a game: the game master's guide

A game master changes a world while it is being played: drops in enemies, turns the weather,
moves a squad, talks to the players. In GameCraft that is not a separate tool: it is whoever
sends world commands to a running world (AGENTS.md rule 6), a person at the command line or an
agent over MCP. This guide is for both. It assumes the conventions of [`../agent.md`](../agent.md)
(metres, Y up, north is -Z, entities by id or name) and uses the Skirmish prototype
(`examples/skirmish`) for its examples.

The MCP server has a prompt for it, `direct_scenario {goal}`, and serves this guide as the
resource `gamecraft://guide/game-master`.

## Start

```sh
gamecraft-cli serve --file examples/skirmish/world.gamecraft --automation-write-root /tmp/gm
# or, for an agent:
gamecraft-cli mcp --file examples/skirmish/world.gamecraft --automation-write-root /tmp/gm
```

Send `sim.start`. The player is a character like any other: a person drives it from the window,
a script or another agent with `sim.input`, and you direct around it.

## The loop

1. **Look.** `gm.overview` answers what is going on in one go: the tick and time, every
   character (team, position, health, down, its behaviour's `mode` and `state`, its `target`,
   whom it `sees`, whether an agent `taken_over` it), every team with its characters alive and
   down, the objectives, the last 50 events with their ticks (who entered which zone, who hit and
   downed whom, who heard a shot), the messages showing, the weather with how far one sees
   (`visibility_m`, `light`) and the variables. Numbers are rounded to centimetres; `names`
   gives the name of every id an event mentions. `near: [x, z]` with `radius_m`, and `team`,
   narrow the characters listed when there are many.
2. **Decide.** What does the game need now: more pressure, a breather, a twist, a word? Keep it
   fair: the player must be able to see a threat coming and answer it.
3. **Act** with ordinary commands (below). Give each an `intent`, a sentence saying why: it
   tells the player's side of the story and the next reader of the journal.
4. **Step.** `sim.step {seconds}` advances the game; a few seconds of game time per round is a
   good rhythm. Each step answers with its events and the triggers that fired, so you see what
   your move did. Then look again.

`gm.sight {from}` tells who and what one character sees now, with the distance, using the
simulation's own perception: the view range shortened by night, mist and rain, the field of
view and a clear line of sight. Ask it before you place a sniper or a surprise: "would the
player see this coming?"

Everything you send while the game runs goes into the session's journal with its tick and the
state hash after it (`docs/design/journal.md`). `journal.save {path}` writes it, and
`gamecraft-cli replay FILE --check` plays the directed game again to the same hash.

## The commands

| What | Command |
|---|---|
| The view | `gm.overview {near?, radius_m?, team?}`, `gm.sight {from}`, `sim.state {for?}`, `scenario.inspect`, `entity.get`, `entity.find` |
| Move people | `gm.order {entity, order, route? \| point?}`: `patrol` a route, `move` to a point, `hold` where they stand, `guard` their post; `entity` is a character, `tag:<tag>` or a list, so one command moves a squad |
| Add people | `prefab.spawn {prefab, at, name?, rotation_y_deg?, overrides?}`; `entity.add` |
| The weather | `environment.set {time_of_day_h?, mist?, rain?, cloud_cover?, wind_direction_deg?, wind_speed_mps?, ...}` |
| Talk | `message.show {text, to?, seconds?}`: `to` a character or `tag:<tag>` (`tag:player`); everyone when left out. `sim.state {for: "Player"}` shows what that player sees |
| The story | `objective.add`, `objective.set`, `var.set`, `trigger.add` |
| Play a character | `sim.input {entity, ...}` takes a character with a behaviour over; `sim.release {entity}` gives it back |
| Keep it | `journal.save {path}`; `sim.stop {keep: true}` makes the directed world the document |

`gm.order` changes only the mode and route of a behaviour and keeps the rest (team, ranges,
aim). A character that is fighting or searching finishes that first, then follows its order;
one taken over by an agent follows it after `sim.release`.

## Ten moves for the Skirmish

Each move below is the exact commands, one per line, as `command_run` takes them over MCP
(`{command, params, intent}`; over JSON lines wrap each in `engine.execute`). A test runs every
block on the running Skirmish, so they stay true. The map: the player starts at the south
`[0, 110]`, the wall line runs along z 48, the village (House 1 and 2) is at z -50 to -58, the
barn at `[40, -10]`, the Hill Sentry at `[90, -30]`; the enemies are of the prefab
`enemy-soldier`, tagged `enemy` and `team:red`.

**1. A fog bank rolls in.** Mist and drizzle cut how far anyone sees to about 85 m (the enemies
see 120 m in clear air), so the player can slip closer, and so can they.

```jsonl gamecraft-gm
{"command": "environment.set", "params": {"mist": 1, "rain": 1, "cloud_cover": 1}, "intent": "a fog bank rolls in from the river: sight drops to 85 m"}
```

**2. Reinforcements drop behind the village.** Two soldiers walk in from the north. Count them
in, so the village is cleared only when they are down too.

```jsonl gamecraft-gm
{"command": "prefab.spawn", "params": {"prefab": "enemy-soldier", "at": [-10, -85], "name": "Reinforcement 1", "overrides": [{"type": "behaviour", "mode": "patrol", "route": [[-10, -85], [-10, -62]], "team": "red", "view_range_m": 120, "fov_deg": 120, "engage_range_m": 100, "aim_error_mrad": 8, "reaction_s": 1.0, "search_s": 10}]}, "intent": "reinforcements arrive behind the village"}
{"command": "prefab.spawn", "params": {"prefab": "enemy-soldier", "at": [20, -85], "name": "Reinforcement 2", "overrides": [{"type": "behaviour", "mode": "patrol", "route": [[20, -85], [20, -68]], "team": "red", "view_range_m": 120, "fov_deg": 120, "engage_range_m": 100, "aim_error_mrad": 8, "reaction_s": 1.0, "search_s": 10}]}, "intent": "reinforcements arrive behind the village"}
{"command": "var.set", "params": {"name": "enemies_left", "add": 2}, "intent": "the village is clear only when they are down too"}
{"command": "message.show", "params": {"text": "HQ: two more hostiles behind the village.", "to": "tag:player", "seconds": 8}, "intent": "warn the player, fairly"}
```

**3. A sniper on the hill.** A sharpshooter far east who sees farther and aims better, but
reacts slowly: the player has two seconds to find cover once seen.

```jsonl gamecraft-gm
{"command": "prefab.spawn", "params": {"prefab": "enemy-soldier", "at": [110, 10], "name": "Sniper", "rotation_y_deg": 90, "overrides": [{"type": "behaviour", "mode": "guard", "team": "red", "view_range_m": 250, "fov_deg": 90, "engage_range_m": 240, "aim_error_mrad": 2, "reaction_s": 2.0, "search_s": 20}]}, "intent": "a sniper takes the hill east of the barn"}
{"command": "gm.sight", "params": {"from": "Sniper"}, "intent": "check what the sniper covers before the player meets it"}
```

**4. The wind turns.** A strong wind from the east bends every long shot: the player's 75 m
shots move a few centimetres, the sniper's much more.

```jsonl gamecraft-gm
{"command": "environment.set", "params": {"wind_direction_deg": 90, "wind_speed_mps": 16}, "intent": "the wind turns east and rises"}
```

**5. A message from HQ.** Only the player reads it (`tag:player`); `sim.state {for}` shows it as
the player sees it.

```jsonl gamecraft-gm
{"command": "message.show", "params": {"text": "HQ: the village guard is your priority. Out.", "to": "tag:player", "seconds": 10}, "intent": "give the player a lead"}
{"command": "sim.state", "params": {"for": "Player"}, "intent": "see the messages as the player does"}
```

**6. Night falls.** The light drops to 15 % of the view range: the enemies see the player only
within about 18 m.

```jsonl gamecraft-gm
{"command": "environment.set", "params": {"time_of_day_h": 21.5, "cloud_cover": 0.8}, "intent": "dusk becomes night: a stealth phase"}
```

**7. The patrol pulls back to the village.** One command per squad: both patrols fall back
between the houses.

```jsonl gamecraft-gm
{"command": "gm.order", "params": {"entity": ["Patrol Alpha", "Patrol Bravo"], "order": "move", "point": [4, -62]}, "intent": "the patrols fall back to defend the houses"}
```

**8. The whole red team holds.** Everyone stops where they stand: a breather for a player under
pressure. `patrol` with no route sends the patrols back on their own routes later.

```jsonl gamecraft-gm
{"command": "gm.order", "params": {"entity": "tag:enemy", "order": "hold"}, "intent": "a lull: the enemy holds its ground"}
{"command": "gm.order", "params": {"entity": ["Patrol Alpha", "Patrol Bravo"], "order": "patrol"}, "intent": "the patrols resume their rounds"}
```

**9. A new objective: reach the barn.** A zone at the barn and a trigger that completes the
objective when the player enters it.

```jsonl gamecraft-gm
{"command": "objective.add", "params": {"id": "barn", "title": "Reach the barn"}, "intent": "a side task that draws the player east"}
{"command": "entity.add", "params": {"name": "Barn yard", "at": [40, -2], "components": [{"type": "zone", "shape": "circle", "radius_m": 6, "height_m": 4}]}, "intent": "the yard in front of the barn"}
{"command": "trigger.add", "params": {"id": "barn-reached", "when": {"event": "enter", "zone": "Barn yard", "who": "tag:player"}, "do": [{"command": "objective.set", "params": {"id": "barn", "status": "done"}}, {"command": "message.show", "params": {"text": "Barn reached.", "to": "$who", "seconds": 5}}]}, "intent": "complete it when the player arrives"}
{"command": "message.show", "params": {"text": "HQ: reach the barn.", "to": "tag:player", "seconds": 8}, "intent": "tell the player"}
```

**10. Take an enemy over.** Play the Village Guard yourself for a moment: it turns on the player
and fires, then its behaviour takes over again.

```jsonl gamecraft-gm
{"command": "sim.input", "params": {"entity": "Village Guard", "face_entity": "Player", "fire": true}, "intent": "the guard opens fire on my word"}
{"command": "sim.step", "params": {"seconds": 2}, "intent": "two seconds of it"}
{"command": "sim.release", "params": {"entity": "Village Guard"}, "intent": "back to its own behaviour"}
```

More: a medic reaches the player (`entity.set_component {"id": "Player", "component": {"type":
"health", "max": 100, "current": 100}}`), the sun comes up (`environment.set
{"time_of_day_h": 6}`), a guard is called away (`gm.order {"entity": "Hill Sentry", "order":
"move", "point": [60, -40]}`).

## Fair and fun

- **Telegraph.** Announce a threat before it bites (a message, a sound, a patrol that is seen
  first). `gm.sight` from the player tells you what it can see.
- **Push, then breathe.** After a hard push (reinforcements, fog), give a lull (`hold`) or help
  (a medic, a lead).
- **Keep the count right.** When you add enemies, add them to the variables the triggers count
  (`var.set {"name": "enemies_left", "add": 2}`), or the game ends early.
- **Do not cheat the rules.** Change the world, not a hit's outcome: the simulation stays
  deterministic, and a journal of your game replays to the same hash.

## An example: a scripted game master

`apps/gamecraft-cli/tests/game_master.rs` directs the Skirmish while a scripted player plays it,
deciding only from `gm.overview`: when the player has downed two enemies it drops two
reinforcements behind the village (move 2), when the player falls below half health it rolls in
the fog (move 1), and when the player reaches the houses it turns the wind (move 4). Its journal
is `examples/skirmish/directed.journal.json`:

```sh
gamecraft-cli replay examples/skirmish/directed.journal.json --check
```
