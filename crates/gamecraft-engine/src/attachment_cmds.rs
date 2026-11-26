//! Attachments: `attachment.add`, `attachment.set`, `attachment.remove` and
//! `attachment.list` (`docs/design/attachments.md`).
//!
//! An attachment is a model of its own on a named socket of an entity's model, or of another
//! attachment's (`gamecraft_world::attachments`): a mount is a tree, and a command names the
//! attachment it acts on by its **path**, the chain of socket names from the entity's model
//! down. A socket holds one attachment: `attachment.add` to an occupied socket is refused, and
//! an attachment is swapped with `attachment.set`. Removing one that holds attachments is
//! refused, or takes them along with `cascade`.
//!
//! Where an attachment lands is derived: the entity's placement, the sockets the host measured
//! from the assets and the attachments' scales. A socket that the asset does not have is an
//! error listing the sockets it has; an asset whose sockets were not measured (a world opened
//! alone, without its pack) is an error saying so.

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Edit, Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, kind_of, param, spec};
use gamecraft_world::attachments::{
    self, Attachment, MAX_ATTACHMENT_DEPTH, MAX_ATTACHMENTS, MAX_SOCKET_NAME_CHARS, Placed,
    is_socket_name, list_sockets, place, walk,
};
use gamecraft_world::{Component, EntityId, Model, Socket, is_model_path};
use serde_json::{Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: add_spec,
        handler: Handler::World(add),
    },
    Registration {
        spec: set_spec,
        handler: Handler::World(set),
    },
    Registration {
        spec: remove_spec,
        handler: Handler::World(remove),
    },
    Registration {
        spec: list_spec,
        handler: Handler::Query(list),
    },
];

const ENTITY_DOC: &str = "The entity whose model carries the attachments: its id as a number, \
     or its name as a string (any case). It needs a `model` component.";
const PATH_DOC: &str = "The attachment to act on: the chain of socket names from the entity's \
     own model down, [socket, socket, ...]: the first is a socket of the entity's model, the \
     next a socket of the attachment on it, and so on.";
const ASSET_DOC: &str = "The attachment's model: a path inside a pack's assets/ folder ending \
     in .glb or .gltf (models/sight.glb). Its origin meets the socket and its axes are the \
     socket's: its +Y is the outward normal, its +Z runs along the parent's length.";

fn entity_param() -> crate::spec::ParamSpec {
    param("entity", T::Entity, ENTITY_DOC).required()
}

fn asset_param() -> crate::spec::ParamSpec {
    param("asset", T::String, ASSET_DOC).required()
}

const SCALE_DOC: &str = "The attachment's scale, on top of the entity's and of every \
     attachment of the chain: 0.001 to 1000.";
const SCALE_SET_DOC: &str = "Its new scale, on top of the entity's and of every attachment of \
     the chain: 0.001 to 1000. Unchanged when left out.";

fn scale_param() -> crate::spec::ParamSpec {
    param("scale", T::Number, SCALE_DOC).default(json!(1))
}

fn path_param(required: bool) -> crate::spec::ParamSpec {
    let p = param("path", T::Array, PATH_DOC);
    if required {
        p.required()
    } else {
        p.default(json!([]))
    }
}

fn placed_json(
    entity: EntityId,
    path: &[String],
    attachment: &Attachment,
    placed: &Placed,
) -> Value {
    json!({
        "entity": entity.0,
        "path": path,
        "socket": attachment.socket,
        "asset": attachment.asset,
        "scale": attachment.scale,
        "position": placed.position,
        "rotation": placed.rotation,
        "world_scale": placed.scale,
    })
}

const PLACED: &str = "{entity, path: [socket, ...], socket, asset, scale, position, rotation \
     ([x, y, z, w]), world_scale: [x, y, z]}: where the attachment landed";

fn add_spec() -> CommandSpec {
    spec(
        "attachment.add",
        "Add Attachment",
        "Mounts a model on a socket of an entity's model, or of one of its attachments: the \
         socket `socket` of the attachment at `path` (the entity's own model when `path` is \
         empty). The socket must be one the parent's model has (a glTF node `socket_<name>`; \
         the error lists them) and free: swap a mounted one with attachment.set. A mount is a \
         tree at most 8 levels deep and an entity carries at most 64 attachments. Drawn as its \
         own item at the socket's full rotation; the simulation counts it as part of its root \
         entity.",
        vec![
            entity_param(),
            path_param(false),
            param(
                "socket",
                T::String,
                "The mount's name on the parent's model: the part after `socket_` in its glTF \
                 node's name, such as \"rail_top\".",
            )
            .required(),
            asset_param(),
            scale_param(),
        ],
        PLACED,
        json!({"entity": "Rifle", "socket": "rail_left", "asset": "models/torch.glb"}),
    )
}

fn set_spec() -> CommandSpec {
    spec(
        "attachment.set",
        "Swap Attachment",
        "Swaps the model of the attachment at `path`, and sets its `scale` when one is given. \
         What it holds on its own sockets stays.",
        vec![
            entity_param(),
            path_param(true),
            param(
                "asset",
                T::String,
                "Its new model, as attachment.add takes it.",
            )
            .required(),
            param("scale", T::Number, SCALE_SET_DOC),
        ],
        PLACED,
        json!({"entity": "Rifle", "path": ["rail_top"], "asset": "models/suppressor.glb"}),
    )
}

fn remove_spec() -> CommandSpec {
    spec(
        "attachment.remove",
        "Remove Attachment",
        "Takes the attachment at `path` off its socket. Refused while it holds attachments of \
         its own (remove those first), unless `cascade` takes them along with it. The parent's \
         model and the other mounts are untouched.",
        vec![
            entity_param(),
            path_param(true),
            param(
                "cascade",
                T::Boolean,
                "Take the attachments it holds along with it.",
            )
            .default(json!(false)),
        ],
        "{entity, path, removed: how many attachments went, the one named included}",
        json!({"entity": "Rifle", "path": ["rail_top", "top"]}),
    )
}

fn list_spec() -> CommandSpec {
    spec(
        "attachment.list",
        "List Attachments",
        "The mount trees of the world shown: of one entity, or of every entity with a model \
         and attachments. Each attachment with its path (the chain of socket names from the \
         entity's model), its model, its scale, where it landed (position, rotation as [x, y, \
         z, w], world scale), the sockets its own model has, and what is mounted on those.",
        vec![param(
            "entity",
            T::Entity,
            "Only this entity's tree (it needs a `model`).",
        )],
        "{entities: [{entity, name, asset, sockets: [names], attachments: [{path, socket, \
         asset, scale, position, rotation, world_scale, sockets, attachments: [...]}]}]}",
        json!({}),
    )
    .query()
}

/// `path` as the docs give it: an array of socket names, each a plain name, at most as many as
/// a mount is deep.
fn path_of(p: Params<'_>) -> Result<Vec<String>, CommandError> {
    let Some(v) = p.value("path") else {
        return Ok(Vec::new());
    };
    let items = v.as_array().ok_or_else(|| {
        bad(format!(
            "`path` must be a list of socket names, [socket, ...], got {}",
            kind_of(v)
        ))
    })?;
    if items.len() >= MAX_ATTACHMENT_DEPTH {
        return Err(bad(format!(
            "`path` names {} levels; attachments mount at most {MAX_ATTACHMENT_DEPTH} deep",
            items.len()
        )));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let name = item.as_str().ok_or_else(|| {
                bad(format!(
                    "`path[{i}]` must be a socket name (a string), got {}",
                    kind_of(item)
                ))
            })?;
            if !is_socket_name(name) {
                return Err(bad(format!(
                    "`path[{i}]` is not a socket name: 1 to {MAX_SOCKET_NAME_CHARS} ASCII \
                     letters, digits, `-` and `_`"
                )));
            }
            Ok(name.to_string())
        })
        .collect()
}

/// `scale` as the commands take it: a number within the range a world checks, named when not.
fn scale_of(p: Params<'_>) -> Result<Option<f64>, CommandError> {
    let scale = p.f64("scale")?;
    if let Some(v) = scale
        && !(gamecraft_world::limits::SCALE_MIN..=gamecraft_world::limits::SCALE_MAX).contains(&v)
    {
        return Err(bad(format!(
            "`scale` is {v}; it must be from {} to {}",
            gamecraft_world::limits::SCALE_MIN,
            gamecraft_world::limits::SCALE_MAX
        )));
    }
    Ok(scale)
}

/// The path as text for an error: `rail_top > top`.
fn shown_path(path: &[String]) -> String {
    if path.is_empty() {
        "the entity's own model".to_string()
    } else {
        path.join(" > ")
    }
}

/// The model of `entity` and the chain of attachments along `path` (empty: none): errors name
/// the entity, the missing model and the first socket of the path that is not there.
fn chain<'a>(
    edit: &'a Edit<'_>,
    id: EntityId,
    path: &[String],
) -> Result<(&'a Model, Vec<&'a Attachment>), CommandError> {
    let entity = edit
        .world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    let model = entity
        .model()
        .ok_or_else(|| failed(format!("entity {id} has no `model` component to mount on")))?;
    let chain = walk(model, path).ok_or_else(|| {
        // How much of the path exists: the first step that is not there is named.
        let mut found = 0;
        while let Some(head) = path.get(..found + 1)
            && walk(model, head).is_some()
        {
            found += 1;
        }
        let at = match path.get(..found) {
            None | Some([]) => format!("the model `{}`", model.asset),
            Some(head) => format!("the attachment at `{}`", shown_path(head)),
        };
        failed(format!(
            "no attachment is mounted on `{}` of {at}; attachment.list shows the tree",
            path.get(found).map_or("", String::as_str)
        ))
    })?;
    Ok((model, chain))
}

/// The socket `socket` of the asset `asset`, as the host measured it; the error lists the
/// sockets the asset has, or says they are unknown.
fn socket_of<'a>(
    edit: &'a Edit<'_>,
    asset: &str,
    socket: &str,
) -> Result<&'a Socket, CommandError> {
    let sockets = edit.world.model_sockets(asset).ok_or_else(|| {
        failed(format!(
            "the sockets of the model `{asset}` are not known: the host measures them when it \
             opens the world's pack (project.open, file.open with `pack`); a world opened alone \
             has none"
        ))
    })?;
    sockets.get(socket).ok_or_else(|| {
        failed(format!(
            "the model `{asset}` has no socket `{socket}`; its sockets are: {}",
            list_sockets(Some(sockets))
        ))
    })
}

/// A clone of `entity`'s model with `path` (empty: the list of the model's own attachments)
/// pointing at the list to add to or remove from.
fn model_of(edit: &Edit<'_>, id: EntityId) -> Result<Model, CommandError> {
    let entity = edit
        .world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    entity
        .model()
        .cloned()
        .ok_or_else(|| failed(format!("entity {id} has no `model` component to mount on")))
}

fn add(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = p.required_entity("entity", &edit.world)?;
    let path = path_of(p)?;
    let socket = p.str("socket")?.ok_or_else(|| bad("missing `socket`"))?;
    if !is_socket_name(socket) {
        return Err(bad(format!(
            "`socket` is not a socket name: 1 to {MAX_SOCKET_NAME_CHARS} ASCII letters, \
             digits, `-` and `_`, such as \"rail_top\""
        )));
    }
    let asset = p.str("asset")?.ok_or_else(|| bad("missing `asset`"))?;
    if !is_model_path(asset) {
        return Err(bad(format!(
            "`asset` must be a path inside a pack's assets/ folder ending in .glb or .gltf, \
             such as \"models/sight.glb\"; got {asset:?}"
        )));
    }
    let scale = scale_of(p)?.unwrap_or(1.0);
    let (model, chain) = chain(edit, id, &path)?;
    if path.len() + 1 > MAX_ATTACHMENT_DEPTH {
        return Err(bad(format!(
            "`path` names level {}; attachments mount at most {MAX_ATTACHMENT_DEPTH} levels deep",
            path.len() + 1
        )));
    }
    let count: usize = model.attachments.iter().map(Attachment::size).sum();
    if count + 1 > MAX_ATTACHMENTS {
        return Err(bad(format!(
            "the entity already carries {count} attachments, and an entity has at most \
             {MAX_ATTACHMENTS}; remove one first"
        )));
    }
    let parent_asset = chain
        .last()
        .map_or(model.asset.as_str(), |a| a.asset.as_str());
    socket_of(edit, parent_asset, socket)?;
    // The socket must be free.
    let taken = match chain.last() {
        None => model.attachments.iter().any(|a| a.socket == socket),
        Some(a) => a.attachments.iter().any(|x| x.socket == socket),
    };
    if taken {
        return Err(failed(format!(
            "the socket `{socket}` of {} already holds an attachment; swap it with \
             attachment.set, or remove it first",
            if chain.is_empty() {
                format!("the model `{}`", model.asset)
            } else {
                format!("the attachment at `{}`", shown_path(&path))
            }
        )));
    }
    let attachment = Attachment {
        socket: socket.to_string(),
        asset: asset.to_string(),
        scale,
        attachments: Vec::new(),
    };
    let mut edited = model_of(edit, id)?;
    // The list of what is mounted on the attachment at `path`, or the model's own list.
    edited
        .attachments_at_mut(&path)
        .ok_or_else(|| {
            failed(format!(
                "no attachment is mounted at `{}`",
                shown_path(&path)
            ))
        })?
        .push(attachment);
    let entity = edit
        .world
        .entity_mut(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    entity.set_component(Component::Model(edited));
    finish(edit, id, &path, socket)
}

/// The answer for the attachment at `path` (its last socket included): where it landed.
fn finish(
    edit: &Edit<'_>,
    id: EntityId,
    path: &[String],
    socket: &str,
) -> Result<Value, CommandError> {
    let entity = edit
        .world
        .entity(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    let model = entity
        .model()
        .ok_or_else(|| failed(format!("entity {id} has no `model` component")))?;
    let mut full = path.to_vec();
    full.push(socket.to_string());
    let found = walk(model, &full)
        .and_then(|chain| chain.last().copied())
        .ok_or_else(|| {
            failed(format!(
                "the attachment at `{}` is not there",
                shown_path(&full)
            ))
        })?;
    let placed = place(entity, model, edit.world.all_model_sockets(), &full).ok_or_else(|| {
        failed(format!(
            "the attachment at `{}` has no place: a socket of the chain was not measured",
            shown_path(&full)
        ))
    })?;
    Ok(placed_json(id, &full, found, &placed))
}

fn set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = p.required_entity("entity", &edit.world)?;
    let path = path_of(p)?;
    let (_, parent) = path
        .split_last()
        .ok_or_else(|| bad("`path` names the attachment to swap: at least one socket name"))?;
    let asset = p
        .str("asset")?
        .map(str::to_string)
        .filter(|a| is_model_path(a))
        .ok_or_else(|| {
            bad(
                "`asset` must be a path inside a pack's assets/ folder ending in .glb or .gltf, \
                 such as \"models/sight.glb\"",
            )
        })?;
    let scale = scale_of(p)?;
    let (_, chain) = chain(edit, id, &path)?;
    let socket = chain
        .last()
        .map(|a| a.socket.clone())
        .ok_or_else(|| failed("the path names no attachment"))?;
    let mut edited = model_of(edit, id)?;
    let target = edited.attachment_mut(&path).ok_or_else(|| {
        failed(format!(
            "no attachment is mounted at `{}`",
            shown_path(&path)
        ))
    })?;
    target.asset = asset;
    if let Some(scale) = scale {
        target.scale = scale;
    }
    let entity = edit
        .world
        .entity_mut(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    entity.set_component(Component::Model(edited));
    finish(edit, id, parent, &socket)
}

fn remove(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = p.required_entity("entity", &edit.world)?;
    let path = path_of(p)?;
    let (socket, parent) = path
        .split_last()
        .ok_or_else(|| bad("`path` names the attachment to remove: at least one socket name"))?;
    let cascade = p.bool("cascade")?.unwrap_or(false);
    chain(edit, id, &path)?;
    let mut edited = model_of(edit, id)?;
    let list = edited.attachments_at_mut(parent).ok_or_else(|| {
        failed(format!(
            "no attachment is mounted at `{}`",
            shown_path(parent)
        ))
    })?;
    let at = list
        .iter()
        .position(|a| a.socket == *socket)
        .ok_or_else(|| {
            failed(format!(
                "no attachment is mounted at `{}`",
                shown_path(&path)
            ))
        })?;
    let held = list.get(at).map_or(0, |a| a.size().saturating_sub(1));
    if held > 0 && !cascade {
        return Err(failed(format!(
            "the attachment at `{}` holds {held} attachments of its own; remove those first \
             (attachment.list shows them), or give cascade: true to take them along",
            shown_path(&path)
        )));
    }
    let removed = list.remove(at).size();
    let entity = edit
        .world
        .entity_mut(id)
        .ok_or_else(|| failed(format!("no entity has the id {id}")))?;
    entity.set_component(Component::Model(edited));
    Ok(json!({"entity": id.0, "path": path, "removed": removed}))
}

/// The tree of the attachments in `list` (what is mounted on the entity's model, or on the
/// attachment at `prefix`), each with its path and its place, and what is mounted on it in turn.
fn tree(
    entity: &gamecraft_world::Entity,
    model: &Model,
    sockets: &attachments::Sockets,
    prefix: &[String],
    list: &[Attachment],
) -> Value {
    list.iter()
        .map(|a| {
            let mut path = prefix.to_vec();
            path.push(a.socket.clone());
            let placed = place(entity, model, sockets, &path);
            json!({
                "path": path,
                "socket": a.socket,
                "asset": a.asset,
                "scale": a.scale,
                "position": placed.map(|p| p.position),
                "rotation": placed.map(|p| p.rotation),
                "world_scale": placed.map(|p| p.scale),
                "sockets": sockets.get(&a.asset).map(|s| s.keys().cloned().collect::<Vec<_>>()),
                "attachments": tree(entity, model, sockets, &path, &a.attachments),
            })
        })
        .collect::<Vec<_>>()
        .into()
}

fn list(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let world = session.current_world();
    let sockets = world.all_model_sockets();
    let only = p.entity("entity", world)?;
    if let Some(id) = only
        && world.entity(id).and_then(|e| e.model()).is_none()
    {
        return Err(failed(format!(
            "entity {id} has no `model` component, so it carries no attachments"
        )));
    }
    let mut entities = Vec::new();
    for e in world.entities() {
        let Some(model) = e.model() else { continue };
        if only.is_some_and(|id| id != e.id) {
            continue;
        }
        if model.attachments.is_empty() {
            continue;
        }
        entities.push(json!({
            "entity": e.id.0,
            "name": e.name,
            "asset": model.asset,
            "sockets": sockets.get(&model.asset).map(|s| s.keys().cloned().collect::<Vec<_>>()),
            "attachments": tree(e, model, sockets, &[], &model.attachments),
        }));
    }
    Ok(json!({ "entities": entities }))
}
