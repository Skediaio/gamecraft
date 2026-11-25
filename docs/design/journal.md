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
`{name, offset, rotation_y_deg, prefab?, components}`: `prefab.spawn` creates each as an entity of its own,
placed by the instance's position and turn (`offset` is in the parent's frame: -Z its forward, +X
its right), with `parent` set to the instance. Removing an entity removes everything below it;
`entity.set` (through `World::place_entity`) moves and turns everything below it rigidly.

**An instance's scale is its children's, in the instance's frame** (2026-11-25): each stands
at its offset times the instance's scale, axis by axis, then turned, and its own scale is the
instance's turned into its own frame (a right angle swaps x and z; any other turn keeps the
scale along the child's own axes: `Scale::in_turned_frame`). `World::set_scale` rescales what is
below an entity the same way. An unedited child is still derived data, to the bit, so nothing
more is written for it; a child given a scale of its own keeps it when the instance's changes,
multiplied by the ratio in its own frame. Before this a scaled instance's children stayed where
the prefab put them, at scale 1. A scale is `x`, `y`, `z` along the entity's own axes
(`gamecraft_world::Scale`): one number in a file when the three are equal, else `[x, y, z]`.

**Spans** (`span` on `prefab.spawn` and `entity.add`; `decisions.md`, *The ground is asked for
while building*): the prefab's extent is measured from its children when the span needs a
length, as the smallest box in the instance's frame around every child's shape and every model
whose asset has bounds, each turned by the child's rotation and moved to its offset, a nested
prefab's children included (`gamecraft_world::prefab_box`).

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

### Children are derived data

An instance's children follow from its prefab and its placement, so the file does not store
them (rule 2), as scene files of most engines store a prefab instance: a reference, a placement
and a list of modifications. In memory nothing changes for readers: each child is a real entity
with materialised components, marked with the index of the prefab child it was placed as
(`Entity::prefab_child`, shown by `entity.get`). In the file (`gamecraft-world/src/instance.rs`):

- An instance is written as before (`prefab`, placement, `overrides`) and its children are left
  out. Reading it places them again from the prefab, relative to the instance, in its layer.
- **Ids.** `prefab.spawn` gives an instance and its children consecutive ids: the instance's,
  then each child's in the prefab's order, depth first through prefabs inside prefabs. Those are
  the ids reading gives them, so the file needs none of them. When they are not those ids (a
  child removed, a child the prefab gained later, which gets a fresh id), the instance writes
  `children: [id | null, ...]`, one per prefab child, `null` for a removed one. Ids are never
  handed out again, so a reserved id cannot collide with another entity, and they survive save,
  load, undo (a snapshot of the document) and the removal of other entities.
- **Edited children.** A child changed after spawning is written on its instance as
  `child_edits: [{index, ...}]`, holding only what differs from what the prefab places: `name`,
  `position` and `rotation_y_deg` (its own world placement), `scale`, `layer`, `overrides` and
  `removed_components`, and for a child that is itself an instance its own `children` and
  `child_edits`. Numbers are compared bit for bit, so a read world writes the same text again.
  Moving an instance (`entity.set`) places an untouched child where the prefab puts it, to the
  bit, so it stays derived.
- **Entities added below an instance** that are not from its prefab are written as entities of
  their own with their `parent`, as before.
- **Prefabs inside prefabs.** A child with `prefab` is an instance of that prefab: its
  `components` are its overrides, and it places that prefab's children in turn. At most 8
  levels (`MAX_PREFAB_NESTING`) and 4096 entities per instance (`MAX_INSTANCE_ENTITIES`); a
  prefab that contains itself, or names a prefab that does not exist, is refused, and a prefab
  placed inside another cannot be removed.
- **`prefab.set` and children.** Children follow the rule of components, child by child: a child
  keeps what was edited in it and takes the rest from the new prefab child (moving what is below
  it when its placement changes); a child the prefab no longer has is removed when it was not
  edited and has nothing of its own below it, and otherwise stays as an entity of its own; a
  child the prefab gained is placed in every instance; a child removed from an instance stays
  removed. A child is recognised by its name when the name is unique in the old and the new
  prefab, otherwise by its index with the same name, so removing a wall from the middle of the
  list does not hand its edits to the next wall.
- **Older files.** A file says `"derived_children": true` when it stores children this way
  (the format version stays 1, so the text and the static hash of a world without prefab
  children do not change). A file without it, from an earlier engine or built from a project's
  layer files (which write every child), holds its children in full; a child at the id its
  instance would give it, with the instance as parent, is taken as that prefab child, and a
  missing one as removed. A child written in full in a new file is merged, not doubled. An
  earlier engine refuses a new file (the unknown field) rather than losing its children.

A 6-child prefab costs 220 bytes per instance in the pretty-printed file instead of 3392 (the
`bytes_per_instance_of_a_six_child_prefab` test counts them).

### Limits

At most 1000 prefabs, 64 children per prefab, and 16 levels of parents (a loop in a bad file is
an error that names the entity); prefabs nest 8 levels deep and one instance places at most 4096
entities. Children follow
`entity.set`, not the simulation: a character that walks leaves its children where they were
(hierarchies that move with simulated bodies come with full orientation, M3).
