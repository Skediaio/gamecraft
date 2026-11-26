# Attachments

A model mounted on a named socket of another model: a sight on a rail, a suppressor on a
muzzle, an optic on a riser mount that is itself mounted. Status: built 2026-11-26 (roadmap item
14). Decisions in `decisions.md` (*Attachments are a tree on the model component*).

## The recipe

A model's mount points are glTF nodes named `socket_<name>` (`socket_rail_top`,
`socket_muzzle`). The node's position and full rotation are the mount's frame: an attachment's
origin is where it meets the socket, its local +Y is the outward normal and its +Z runs along
the parent's length. What each asset's sockets are is derived data, measured by the host as the
bounding boxes are ([`Socket`], `World::set_model_sockets`); a node not in the scene has no
place and is no socket.

An attachment is a model of its own, on the `model` component of an entity:

```json
{"type": "model", "asset": "models/launcher.glb", "attachments": [
  {"socket": "rail_top", "asset": "models/riser.glb", "attachments": [
    {"socket": "top", "asset": "models/optic.glb"}]}]}
```

- `socket`: the mount's name on the parent's model (the part after `socket_`), 1 to 64 ASCII
  letters, digits, `-` and `_`. A socket holds one attachment.
- `asset`: the attachment's model.
- `scale`: on top of the entity's and of every attachment of the chain; written only when not 1.
- `attachments`: what is mounted on this attachment's own sockets, written only when there is
  one: the tree reads as it mounts.

A mount is at most **8 levels deep** (`MAX_ATTACHMENT_DEPTH`) and an entity carries at most
**64 attachments** (`MAX_ATTACHMENTS`). A world without attachments keeps its text and its
hash: the field is written only when there is one. Prefab overrides replace the whole component,
attachments included, as for any component.

## Where it lands

The attachment's frame is composed in `f64` through `glam`'s `libm` maths, the same bits on
every platform (`gamecraft_world::attachments::place`): the entity's frame (its position, yaw
and scale, the model's scale), then for each attachment of the chain the socket's frame in the
parent's asset and the attachment's scale. The answer gives the position, the rotation as the
turns alone (the entity's yaw and every socket's turn, multiplied: read back from the frame it
would be wrong wherever a scale per axis meets a turn that is not a right angle), and the
scale along the attachment's own axes as the lengths of the frame's axes when they are at
right angles to each other. When they are not (the entity scaled per axis, a socket turned by
another angle than a right one), there is no such scale: `world_scale` is `null` and the frame
itself is right all the same. The same in the glTF: a socket's turn is the turns of its node
chain, whatever scales stand above it.

The simulation treats an attachment as part of its root entity: no collider of its own, no
mass yet. The state hash covers the document, so a world with attachments hashes as it reads.

## Asking and changing

- `attachment.list {entity?}`: the mount trees, each attachment with its `path` (the chain of
  socket names from the entity's model), where it landed, the sockets its own model has and
  what is mounted on those.
- `attachment.add {entity, path?, socket, asset, scale?}`: mounts a model on `socket` of the
  attachment at `path` (the entity's own model when empty). The socket must be one the parent's
  model has (the error lists them) and free; an asset whose sockets were not measured (a world
  opened alone, without its pack) is an error saying so.
- `attachment.set {entity, path, asset, scale?}`: swaps the model, and sets the scale when one is
  given; what it holds stays on its sockets, so the new model must have them. A model without
  them is refused by name (with the sockets it has); a model whose sockets were not measured
  is refused too. A path has at most 8 names and addresses the deepest attachment; nothing
  mounts on it.
- `attachment.remove {entity, path, cascade?}`: refused while the attachment holds attachments
  of its own, or takes them along with `cascade: true`.

## The picture

An attachment is drawn as its own item at the socket's full rotation (the entity itself only
turns about +Y until full orientation, M3), with the levels of detail the rule gives its size
(it has no distances of its own in the data), and never beyond where its root entity's model
stops drawing: that model's own draw distance, else the rule's for it. On a character it
follows the character between ticks as the character's own model does, on the frame that model
is drawn in, and lies with it when it is down. A model that cannot be drawn (missing or
broken) is a small magenta stand-in: at the entity for its own model, on the attachment's
socket for an attachment, as the engine places it. The far levels of a cell (HLOD) merge the
entity's model, not its attachments: where the merged mesh takes over, the attachment is not
drawn either.

## Open

- Full orientation of entities (M3), so the simulation can reason about attachments too.
- Mass, a collider of its own, and what an attachment changes (a suppressor's sound range): when
  the first pack needs it.
- Attachments in the merged far levels; a view model of its own for a first-person view (today
  a hidden character keeps its attachments, drawn where its body would be).
- Animation of a model's nodes (roadmap item 13).
