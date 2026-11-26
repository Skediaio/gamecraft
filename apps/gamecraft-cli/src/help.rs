//! The help texts.

pub const HELP: &str = "\
gamecraft-cli: the command line of GameCraft

Usage: gamecraft-cli <subcommand> [options]
       gamecraft-cli <subcommand> --help
       gamecraft-cli --version | --help

Subcommands:
  commands   List every command: id, parameters, summary
  run        Open or create a world, run commands and scripts, save with --out
  info       Print a world file's (or a project's) state as JSON
  render     Draw a world file (or a project) to a PNG (needs a GPU)
  play       Play a world file (or a project) for a while and print the state hash (nothing
             is changed)
  replay     Run a journal of a play session again and compare its state hashes
  new        Write an empty world file
  project    Make, open, describe, export and query a project folder (packs, layers, mods)
  serve      JSON-lines server on stdin and stdout, for scripts
  mcp        Model Context Protocol server on stdin and stdout, for agents
  help       This text; `help <subcommand>` explains one

Global option (any subcommand, anywhere on the line):
  --max-world-mb N   Read world files (and project pack files) of up to N MiB; the default
                     is 512, a safety net against a file that would exhaust memory. The
                     environment variable GAMECRAFT_MAX_WORLD_MB sets it too; the flag wins.

Results go to stdout as JSON, diagnostics to stderr.
Exit codes: 0 success, 1 a command or file failed, 2 a mistake in the command line.";

pub const COMMANDS: &str = "\
Usage: gamecraft-cli commands [--filter TEXT] [--json]

Lists the commands, one block each: id, parameters and summary. These are the ids that
`run --cmd`, `serve` and `mcp` accept.

Options:
  --filter TEXT   Only commands whose id or label contains TEXT (any case)
  --json          The list as JSON, as the method engine.commands gives it";

pub const RUN: &str = "\
Usage: gamecraft-cli run [FILE] [--new JSON] [--cmd ID [--params JSON]]... [--script FILE]...
                         [--out FILE] [--keep-going] [--pack DIR]

Opens the world FILE (a .gamecraft file, or a project folder) or, with --new, starts a world with the parameters of
file.new (without either, an empty world), runs the steps in the order given, prints one line of
JSON per result, and saves the document with --out. It stops at the first step that fails, saves
nothing and exits 1. Play commands work inside one run: sim.start, sim.input, sim.step,
sim.state, sim.stop.

Options:
  --new JSON      Parameters of file.new, e.g. '{\"name\": \"Valley\", \"seed\": 7}'. Not with FILE
  --cmd ID        A command to run; may be repeated
  --params JSON   The parameters of the --cmd right before it, a JSON object
  --script FILE   A JSON list of steps, [[\"id\", {params}], ...] or
                  [{\"command\": \"id\", \"params\": {...}}, ...]; may be repeated and mixed with --cmd
  --out FILE      Save the document afterwards (a .gamecraft file; replaces an existing one)
  --keep-going    Carry on after a failing step: its line is
                  {\"failed\": id, \"step\": n, \"error\": text}, --out still saves, exit code 1
  --pack DIR      With a world FILE: the pack whose assets/ holds its models (a folder with a
                  pack.json), instead of the nearest pack.json above the file

Example:
  gamecraft-cli run --new '{\"name\": \"Valley\"}' \\
      --cmd terrain.set --params '{\"kind\": \"noise\", \"size_m\": 128, \"cell_m\": 2, \"relief_m\": 8}' \\
      --out valley.gamecraft";

pub const INFO: &str = "\
Usage: gamecraft-cli info FILE [--pack DIR]

Prints the state of a .gamecraft file, or of a project folder (its world layered from its
packs), as JSON, as the command world.inspect gives it (at most 1000 entities). --pack DIR,
with a world file, is the pack whose assets/ holds its models (default: the nearest pack.json
above the file).";

pub const RENDER: &str = "\
Usage: gamecraft-cli render FILE --out PNG [--size WxH] [--camera JSON]
                            [--follow ENTITY [--view first|third] [--distance M]] [--pack DIR]

Draws the world to a PNG on the GPU and prints what the renderer says (the camera it used, what
it drew, problems) as one line of JSON. Without a camera the whole world is framed. Without a GPU
it fails (exit 1) and says so; everything else works without one. A world file's models come from
the nearest pack above it; without one they are magenta boxes and `problems` says, in one
sentence, what is missing.

Options:
  --out PNG        The picture to write (replaced if it exists)
  --pack DIR       With a world FILE: the pack whose assets/ holds its models (a folder with a
                   pack.json), instead of the nearest pack.json above the file
  --size WxH       Pixels, 16 to 3840 wide and 16 to 2160 tall (default 960x540)
  --camera JSON    '{\"position\": [x, y, z], \"target\": [x, y, z], \"fov_y_deg\": 60}', metres
  --follow ENTITY  Through the eyes of an entity (id or name) or behind it
  --view V         With --follow: first (the character's eyes) or third (the default)
  --distance M     With --follow and the third person: metres behind the entity (default 6)";

pub const PLAY: &str = "\
Usage: gamecraft-cli play FILE [--script FILE] [--seconds S | --ticks N]
                          [--out-state FILE] [--picture PNG] [--journal FILE] [--pack DIR]

Starts the simulation of the world FILE, runs the script (steps as for run, typically sim.input
and sim.step), steps on until the total time given is reached, and prints sim.state as one line
of JSON, the state hash included. Two runs of the same arguments print the same hash. The file is
never changed.

Options:
  --script FILE    Steps to run after the simulation starts
  --seconds S      Total simulated time, in seconds of 60 ticks each (stepping stops at the
                   script's own end when that is later)
  --ticks N        The same in ticks
  --out-state FILE Also write the state to FILE as JSON
  --picture PNG    Draw the last state to PNG (needs a GPU); the state line gets a \"picture\" key
  --journal FILE   Also write the journal of the run (every command with its tick and the state
                   hash after it; docs/design/journal.md), for `gamecraft-cli replay`
  --pack DIR       With a world FILE: the pack whose assets/ holds its models, so solid ones
                   collide (default: the nearest pack.json above the file)";

pub const REPLAY: &str = "\
Usage: gamecraft-cli replay JOURNAL [--to-tick N] [--check]

Runs a journal (written by `play --journal` or the command journal.save) again from the world it
embeds: sim.start, then every recorded command at its tick, comparing the state hash after each
with the recorded one. Prints one line of JSON: {tick, hash, entries, entries_run, hashes_checked,
final (the journal's), matches, divergence: null | {entry, tick, message}}. The first divergence
names the tick where the hash first differs.

Options:
  --to-tick N   Stop when the simulation reaches tick N, before the commands recorded at it (a
                step that crosses it is cut short), and print sim.state there instead, with a
                \"replay\" key ({tick, entries_run, hashes_checked, divergence}); halve N to
                bisect a bug
  --check       Exit 1 when the replay diverges, naming the tick (with --to-tick: up to N)";

pub const NEW: &str = "\
Usage: gamecraft-cli new FILE [--name NAME] [--seed N]

Writes an empty world (no terrain, the default environment, no entities) to FILE, a .gamecraft
file that must not exist yet, and prints {path, bytes, name, seed}.";

pub const SERVE: &str = "\
Usage: gamecraft-cli serve [--automation-read-root DIR] [--automation-write-root DIR]
                        [--file FILE [--pack DIR]]

The JSON-lines protocol on stdin and stdout: one request per line, one reply per line
({\"id\", \"method\", \"params\"}; docs/control-protocol.md). Nothing listens on a network port.
Files are touched only inside the roots given: file.open reads inside --automation-read-root,
file.save and view.render with a path write inside --automation-write-root. Without them there
is no file access.

Options:
  --automation-read-root DIR   Where file.open may read
  --automation-write-root DIR  Where file.save and pictures may write
  --file FILE                  Start with this world open (its path is not remembered: file.save
                               needs a path). A world file's models come from the nearest
                               pack.json above it, or from --pack DIR
  --pack DIR                   With --file: the pack whose assets/ holds the world's models (a
                               folder with a pack.json), for the start and for every world file
                               opened later without `pack`";

pub const MCP: &str = "\
Usage: gamecraft-cli mcp [--automation-read-root DIR] [--automation-write-root DIR]
                      [--file FILE [--pack DIR]]

The Model Context Protocol server (JSON-RPC 2.0, one message per line) on stdin and stdout, over
the same commands as serve. The flags are those of serve.

Options:
  --automation-read-root DIR   Where file.open may read
  --automation-write-root DIR  Where file.save and pictures may write
  --file FILE                  Start with this world open. A world file's models come from the
                               nearest pack.json above it, or from --pack DIR
  --pack DIR                   With --file: the pack whose assets/ holds the world's models (a
                               folder with a pack.json), for the start and for every world file
                               opened later without `pack`";

pub const PROJECT: &str = "\
Usage: gamecraft-cli project new DIR --id ID [--name NAME] [--world W] [--from FILE]
                                   [--cell-size-m M]
       gamecraft-cli project open DIR [--world W]
       gamecraft-cli project info DIR [--world W]
       gamecraft-cli project export DIR --out FILE
       gamecraft-cli project query DIR \"SELECT ...\" [--limit N]
       gamecraft-cli project bake SCRIPT --into DIR [--cell-size-m M] [--id ID --name NAME]
                                                           [--world W]

A project is a folder of text files: pack.json, prefabs/<id>.json and
worlds/<world>/{world.json, layers/<layer>.json}, layered over the packs it depends on
(docs/design/projects.md). run, play, info and render take a project folder wherever they take
a .gamecraft file; the folder that holds the project is where its dependencies are looked for.

Actions:
  new      Make the project DIR (new or empty) from FILE (a .gamecraft file) or an empty world:
           the pack ID with world W (default main), one file per layer and per prefab; with
           --cell-size-m M each layer is a folder of cells of M metres (layers/<layer>/<cx>_<cz>.json)
  open     Open the project and its dependencies and print what was loaded (packs in load
           order, world, entities, the index)
  info     Print project.info: packs, worlds, layers with their entity counts, prefabs, warnings
  export   Write the project's world as one .gamecraft file
  query    Run one read-only SQL SELECT on the project's index (kept in DIR/.gamecraft-index):
           tables entities, entity_tags, entity_components, prefabs; at most 1000 rows
  bake     Run the script SCRIPT (the JSON list of steps run --script takes, read one step at a
           time) and save the result as the project DIR in one step, its layers in cells of M
           metres (default 512). A new project needs --id and is written once every step
           succeeded; in an existing one the bake replaces only what the script owns (the layers
           its steps add entities to and the terrain edits it added, recorded in
           worlds/<world>/bake.json), so hand edits in other layers survive. The same script
           gives the same files; a failing step names its line and writes nothing.";
