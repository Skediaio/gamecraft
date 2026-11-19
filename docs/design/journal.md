# The journal and prefabs

How a directed session is kept so it replays to the same hash, and how an entity described once
is placed many times. Status: built 2026-11-19 for M2 (`roadmap.md`).

## The journal

### What is recorded

While a simulation runs, the session records every command that reaches it from outside and
changes the running world or the simulation:

- every world command (scope `world`: `entity.*`, `environment.set`, `prefab.spawn`,
  `var.set`, `objective.set`, ...),
- `sim.input`, `sim.fire`, `sim.release`, `message.show` and `sim.step`.

Each entry is `{tick, command, params, end_tick?, hash}`: the tick at which it took effect (for
`sim.step` the tick it started from), the command and its parameters exactly as sent, for a step
the tick it ended on, and the state hash after it. Only a command that succeeded is recorded; a
failed command changes nothing.

Not recorded: queries (they change nothing), `sim.start` and `sim.stop` (the journal begins and
ends with them), and the commands of triggers. Those run inside a step or inside another command
(`Session::run` nested in a handler) and follow from the rest: the same world and the same
commands give the same events, so the same triggers. The session records only the outermost
`Session::run`.

`sim.start` begins a new journal and embeds a copy of the document the simulation starts from.
After `sim.stop` the last journal stays readable (`journal.get`, `journal.save`) until the next
`sim.start`. A transaction that fails puts the journal back as it was, also when it started a new
simulation inside. A journal holds at most 1,000,000 entries; beyond that it stops recording and
is marked `truncated`, and `replay --check` refuses to check it to its end.

### Why a hash after every entry

The brief asked for one in each `sim.step`. Every entry has one because inputs and world commands
are part of the state hash at once (an input record, a held facing, the world's text in its
static part): a journal edited by hand diverges at the edited entry itself, so the first tick
named is the tick of the change, not the end of the next step. The cost is one hash per command,
which is small next to a step.

### The file

One JSON document, version 1, written with the world and each entry on a line of their own so a
person can read, diff and edit it:

```json
{"gamecraft_journal": 1,
"world": {"gamecraft": 1, "name": "Skirmish", ...},
"entries": [
{"tick":0,"command":"sim.input","params":{"entity":"Player","face":[0,50.5],"forward":1},"hash":"..."},
{"tick":0,"command":"sim.step","params":{"ticks":708},"end_tick":708,"hash":"..."}
],
"final": {"tick":3600,"hash":"0221de111eb7dd5d"}
}
```

JSON rather than JSON lines: the world embedded is a document of its own, and one document is
read and checked in one go (`Journal::from_json` refuses unknown fields and another version, and
reads the world with all of its checks). `final` is the tick and hash after the last entry (after
`sim.start` when there is none).

### Replay

`Journal::replay(to_tick)` (and `gamecraft-cli replay`) starts a new session on the embedded
world, runs `sim.start`, then every entry in order, and compares: the tick the entry was recorded
at with the tick the replay is at, a step's end tick, and the hash after it. The first difference
is kept (`Divergence {entry, tick, message}`); the replay goes on, so its session ends where the
journal ends and can still be inspected. A recorded command that fails in the replay is a
divergence too, never a crash. Without `to_tick` the final tick and hash are compared as well.

`to_tick` stops the replay when the simulation reaches that tick, before the commands recorded at
it; a step that crosses it is cut short (`sim.step {ticks}` up to it). Bisecting a bug is then:
replay to the middle, look at `sim.state`, halve.

### Where it lives

The engine (L3) records and replays (`gamecraft_engine::journal`), with no files: `journal.get`
is an engine query. Writing a file is the host's (`journal.save`, inside the write root, `.json`,
through a temporary file and a rename). The CLI reads files (`replay`) and writes the journal of
`play --journal`.

## Prefabs

### The model

A prefab is `{id, name, components, children}` in the world's `prefabs` list. An entity placed
from it has `prefab: id`; its **effective components** are the prefab's, each replaced by the
override of the same type, followed by overrides of types the prefab lacks. Children are
`{name, offset, rotation_y_deg, components}`: `prefab.spawn` creates each as an entity of its own,
placed by the instance's position and turn (`offset` is in the parent's frame: -Z its forward, +X
its right), with `parent` set to the instance. Removing an entity removes everything below it;
`entity.set` (through `World::place_entity`) moves and turns everything below it rigidly.

Format version 1 is kept: `prefabs`, `prefab`, `parent` and `overrides` are optional, and a
document without them is written exactly as before (the same text, so the same static hash).

### Materialised, not resolved on read

The effective components are **materialised** in `gamecraft-world`: in memory an instance's
`Entity::components` holds its effective components, computed when the document is read
(`Prefab::effective`) and when a prefab is spawned or changed. Only the file keeps the
difference: an instance is written as `prefab` and `overrides`, and the overrides are computed on
writing as the components that differ from the prefab's (`Prefab::overrides_of`).

The alternative, `Entity::components_effective(&World)` resolved on every read, was rejected:
every reader (the simulation, every query, the renderer) would have to pass the world along and
call it, and one that forgot would silently see only the overrides. Materialising keeps every
reader as it is, the renderer's API unchanged (it reads `World` and sees the effective components),
and the simulation's hot path free of lookups. What it costs:

- **Changing a prefab updates its instances.** `World::put_prefab` (`prefab.set`) takes each
  instance's overrides against the old prefab and applies them to the new one.
- **An instance keeps every component type of its prefab.** Overrides can replace and add, not
  remove, so a removal could not be written; `World::validate` refuses an instance without one
  of its prefab's component types, naming the entity, the prefab and the type.
- **An override equal to the prefab's component is no override.** Setting an instance's
  component to exactly the prefab's value makes it follow the prefab again.

Changes made while playing work the same way: a trigger that switches a beacon's light on
changes the instance's components, and on `sim.stop {keep: true}` that light is written as an
override of that beacon.

### Limits

At most 1000 prefabs, 64 children per prefab, and 16 levels of parents (a loop in a bad file is
an error that names the entity). Children are plain entities: they do not refer to the prefab,
so `prefab.set` changes the children of later spawns, not those already placed. Children follow
`entity.set`, not the simulation: a character that walks leaves its children where they were
(hierarchies that move with simulated bodies come with full orientation, M3).
