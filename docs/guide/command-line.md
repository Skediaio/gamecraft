# The command line

*Everything an agent does through MCP you can do from a shell: run commands and scripts, look at a
world, play it and draw it, with `gamecraft-cli`.*

`gamecraft-cli` runs in a terminal (on Windows: Command Prompt or PowerShell) and needs no window.
Only pictures need a GPU. An agent with a shell uses it directly; a tool without a shell uses the
same commands over [MCP](../control-protocol.md) (`gamecraft-cli mcp`). `gamecraft-cli --help`
lists the subcommands, `gamecraft-cli <subcommand> --help` explains one.

| Subcommand | What it does |
|---|---|
| `commands` | Lists every command: id, parameters, summary |
| `run` | Opens or creates a world (a file or a project folder), runs commands and scripts, saves |
| `info` | Prints a world file's (or a project's) state as JSON |
| `render` | Draws a world file (or a project) to a PNG (GPU) |
| `play` | Plays a world file (or a project) for a while and prints the state hash |
| `replay` | Runs a journal of a play session again and compares its state hashes |
| `new` | Writes an empty world file |
| `project` | Makes, opens, describes, exports and queries a project folder (packs, layers, mods) |
| `serve` | The JSON-lines protocol on stdin and stdout |
| `mcp` | The MCP server on stdin and stdout |

Results go to stdout as JSON, diagnostics to stderr.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | A command or a file failed: the error is a sentence on stderr that names the thing, the value and what is allowed. A missing, unreadable or invalid file is such an error, never a crash |
| 2 | A mistake in the command line (unknown subcommand or flag, a missing argument, JSON in `--params` that does not parse): the usage is on stderr |

## Listing the commands: `commands`

```sh
gamecraft-cli commands --filter sim
gamecraft-cli commands --json > commands.json
```

One block per command with its parameters in a one-line notation and its summary; `--filter` keeps
the commands whose id or label contains the text. `--json` prints what the protocol method
`engine.commands` gives: every parameter with its type, unit, default and allowed values, the
result's shape and a tested example.

## Running commands: `run`

```sh
gamecraft-cli run --new '{"name": "Green valley", "seed": 7}' \
    --cmd terrain.set --params '{"kind": "noise", "size_m": 128, "cell_m": 2, "relief_m": 6}' \
    --cmd world.inspect --params '{"entities": false}' \
    --out valley.gamecraft
gamecraft-cli run valley.gamecraft --cmd environment.set --params '{"time_of_day_h": 17.5}' --out valley.gamecraft
```

`run` opens a file (or starts a world with `--new`, the parameters of `file.new`; with neither, an
empty world), runs the steps in the order given, prints one line of JSON per result, and saves the
document with `--out`. It stops at the first step that fails, saves nothing and exits with code 1.
With `--keep-going` it carries on, prints `{"failed": id, "step": n, "error": text}` for a failed
step (`n` counts from 0), saves what was built and still exits with code 1.

Play commands work inside one run: `sim.start`, `sim.input`, `sim.step`, `sim.state`, then
`sim.stop`. `--out` saves the document, never the running world.

`run` may read the folder of its FILE and write the folder of its `--out`, and nothing else.

> **Quotes on Windows:** in PowerShell and Command Prompt the JSON in `--params` needs other
> quoting (in Command Prompt, double quotes doubled or escaped as `\"`). A script file (below)
> avoids the problem altogether.

## Scripts

A script is a JSON list of steps, `[["id", {params}], ...]`, the format of 3DCraft's scripts. The
form `[{"command": "id", "params": {...}}, ...]` is accepted too. `--script` may be given several
times and mixed with `--cmd`; the steps run in the order of the command line.

```sh
gamecraft-cli run --new '{"name": "Meadow"}' --script meadow.json --out meadow.gamecraft
```

A step that is not an array or an object, an unknown key in an object, or a file that is not JSON,
is an error (exit code 1) before the first step runs.

## Looking inside a file: `info`

```sh
gamecraft-cli info valley.gamecraft
```

Prints the whole state of the file as JSON, as `world.inspect` gives it: the terrain with its height
range, the environment and the sun, and up to 1000 entities with their components.

## Pictures: `render`

```sh
gamecraft-cli render valley.gamecraft --out valley.png --size 1280x720
gamecraft-cli render valley.gamecraft --out scout.png --follow Scout --view third --distance 8
gamecraft-cli render valley.gamecraft --out eyes.png --follow Scout --view first
gamecraft-cli render valley.gamecraft --out top.png --camera '{"position": [0, 120, 40], "target": [0, 0, 0], "fov_y_deg": 50}'
```

Draws the world to a PNG on the GPU and prints what the renderer says: the camera it used, what it
drew (`stats`) and `problems`, for example an unknown plant species. Without a camera the whole
world is framed. `--follow` takes an entity's id or name: `--view third` (the default) looks at it
from behind, `--view first` through a character's eyes. Without a GPU it fails with code 1 and says
so; everything else works without one.

## Playing: `play`

```sh
gamecraft-cli play valley.gamecraft --script walk.json --seconds 6 --out-state state.json --picture end.png
```

Starts the simulation of the file, runs the script (steps as for `run`, typically `sim.input` and
`sim.step`), steps on until `--seconds` (or `--ticks`) of simulated time in total have passed, and
prints `sim.state` as one line of JSON: the tick, the **state hash** and where the characters are.
Two runs of the same arguments print the same hash, on every machine. `--out-state` also writes the
state to a file; `--picture` draws the last state (the state line gets a `picture` key);
`--journal FILE` writes the journal of the run (below). The file is never changed.

## Replaying a session: `replay`

```sh
gamecraft-cli play valley.gamecraft --script walk.json --seconds 6 --journal walk.journal.json
gamecraft-cli replay walk.journal.json --check
gamecraft-cli replay walk.journal.json --to-tick 180
```

A journal holds the world a simulation started from and every command that changed it while it
ran (world commands, `sim.input`, `sim.fire`, `sim.release`, `message.show`, `sim.step`), each
with the tick it took effect at and the state hash after it (`docs/design/journal.md`). `play
--journal` writes one, and so does the command `journal.save` over the protocol or MCP.
`replay` starts the embedded world, sends every command again at its tick and compares the
hashes. It prints `{tick, hash, entries, entries_run, hashes_checked, final, matches,
divergence}`; `divergence` names the entry and the first tick where the hash differs. With
`--check` a difference exits 1 with that sentence on stderr. `--to-tick N` stops when the
simulation reaches tick N, before the commands recorded at it, and prints `sim.state` there
(with a `replay` key): halve N until the tick where things go wrong is found.

## Making a world file: `new`

```sh
gamecraft-cli new empty.gamecraft --name Empty --seed 3
```

Writes an empty world and prints `{path, bytes, name, seed}`. It does not replace an existing file.

## Projects: `project`

A game bigger than a test valley lives in a **project**: a folder of text files
([`design/projects.md`](../design/projects.md)). `pack.json` names the pack and what it depends
on, `prefabs/<id>.json` holds one prefab each, and a world is `worlds/<world>/world.json` (terrain,
environment, scenario) plus `worlds/<world>/layers/<layer>.json`, one entity per line, each with a
string `id`. A mod is a pack that depends on the game and changes it by id; editing a game you
depend on and saving writes the change into your own pack, never into the game.

```sh
# The Skirmish as a project, and its night mod layered over it.
gamecraft-cli project info examples/skirmish/project
gamecraft-cli project open examples/skirmish-night-mod
# Make a project from a world file (one file per layer and prefab).
gamecraft-cli project new my-skirmish --id my-skirmish --name "My skirmish" --world village \
    --from examples/skirmish/world.gamecraft
# One .gamecraft file of what a project resolves to.
gamecraft-cli project export examples/skirmish-night-mod --out night.gamecraft
# Ask the index: every enemy within 100 m of the houses.
gamecraft-cli project query examples/skirmish/project \
    "SELECT e.name, e.x, e.z FROM entities e JOIN entity_tags t ON t.entity_id = e.id
     WHERE t.tag = 'enemy' AND (e.x - 4) * (e.x - 4) + (e.z + 54) * (e.z + 54) <= 100 * 100"
```

`run`, `play`, `info` and `render` take a project folder wherever they take a `.gamecraft` file
(`gamecraft-cli play examples/skirmish/project --script examples/skirmish/playthrough.json`); the
folder that holds the project is where its dependencies are found (`"path": "../skirmish/project"`
in the night mod's `pack.json`). The index is a Turso database in `.gamecraft-index/` inside the
project folder, rebuilt from the files and ignored by git; `project query` takes one `SELECT` on
the tables `entities` (`id, key, name, layer, pack, prefab, parent, x, y, z, rotation_y_deg, scale,
tags, components`), `entity_tags` (`entity_id, tag`), `entity_components` (`entity_id, type,
data`) and `prefabs` (`id, name, pack, components, instances`), and refuses anything else. Over
`serve` and `mcp` the same is `project.new`, `project.open`, `project.save`, `project.export`,
`project.info`, `pack.add_dependency` and `project.query`.

## Servers: `serve` and `mcp`

```sh
gamecraft-cli serve --automation-read-root worlds --automation-write-root out
gamecraft-cli mcp --automation-write-root out --file worlds/valley.gamecraft
```

`serve` speaks the JSON-lines protocol and `mcp` the Model Context Protocol, one message per line
on stdin and stdout ([`control-protocol.md`](../control-protocol.md)). Nothing listens on a network
port. Files are touched only inside the roots given: `file.open` reads inside
`--automation-read-root`, `file.save` and `view.render` with a `path` write inside
`--automation-write-root`. With no root there is no file access, and the error names the flag that
grants it. `--file` starts with a world open (read once; its path is not remembered, so `file.save`
needs a `path`).

## An example: a small valley

A world with a hilly terrain, a stand of oaks and a character, built with `run`:

```sh gamecraft-example
gamecraft-cli run --new '{"name": "Green valley", "seed": 7}' \
    --cmd terrain.set --params '{"kind": "noise", "size_m": 128, "cell_m": 2, "relief_m": 6, "feature_m": 60, "seed": 7}' \
    --cmd environment.set --params '{"time_of_day_h": 17.5, "cloud_cover": 0.3}' \
    --cmd entity.scatter --params '{"count": 25, "name": "Oak", "min_distance_m": 6, "seed": 3, "components": [{"type": "plant", "species": "quercus-robur", "growth": 1}]}' \
    --cmd entity.add --params '{"name": "Scout", "at": [0, 0], "components": [{"type": "character"}]}' \
    --out valley.gamecraft
```

Look at what was built, and at the picture (the picture needs a GPU):

```sh gamecraft-example
gamecraft-cli info valley.gamecraft

gamecraft-cli render valley.gamecraft --out valley.png --size 960x540
```

Make Scout walk north for two seconds with a script, and play on to six seconds in total:

```sh gamecraft-example
cat > walk.json <<'EOF'
[["sim.input", {"entity": "Scout", "forward": 1, "yaw_deg": 0}], ["sim.step", {"seconds": 2}]]
EOF

gamecraft-cli play valley.gamecraft --script walk.json --seconds 6 --out-state state.json
```

The last line prints the state, with the same hash on every run:

```json
{"playing":true,"tick":360,"time_s":6.0,"hash":"...","characters":[{"id":26,"name":"Scout","position":[...],"grounded":true}],"inputs":[...]}
```
