//! Attachments: models mounted on the named sockets of other models
//! (`docs/design/attachments.md`).
//!
//! An **attachment** is a model of its own ([`Attachment`]: a socket, an asset, a scale, and
//! attachments of its own) on the `model` component of an entity. A model names its mount
//! points as glTF nodes `socket_<name>`; the node's position and full rotation are the
//! mount's frame ([`Socket`]), derived data that the host measures as the bounds
//! ([`World::set_model_sockets`]). An attachment's origin meets the socket and its axes are
//! the socket's: its +Y is the outward normal, its +Z runs along the parent's length. So a
//! mount is a tree, at most [`MAX_ATTACHMENT_DEPTH`] deep: a base, an adapter on it, an
//! attachment on the adapter.
//!
//! **Where an attachment lands** ([`place`]) is composed in `f64` through glam's `libm`
//! maths, the same bits on every platform: the entity's frame (its position, yaw and scale,
//! and the model's scale), then for each step of the chain the socket's frame in the parent's
//! asset and the attachment's scale. The simulation treats an attachment as part of its root
//! entity (it has no collider of its own); the renderer draws it as its own item at the full
//! rotation. A world without attachments keeps its text and its hash: the field is written
//! only when there is one.

use crate::error::Ctx;
use crate::limits::{SCALE_MAX, SCALE_MIN};
use crate::model::{Model, is_model_path};
use crate::{Entity, WorldError};
use gamecraft_math::{DMat4, DQuat, DVec3, deg_to_rad};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The start of the name of a socket node in a glTF file: `socket_rail_top`.
pub const SOCKET_PREFIX: &str = "socket_";
/// Most levels of attachments on one entity (an attachment on the entity's model is the
/// first).
pub const MAX_ATTACHMENT_DEPTH: usize = 8;
/// Most attachments on one entity, at every depth.
pub const MAX_ATTACHMENTS: usize = 64;
/// Most characters in a socket's name.
pub const MAX_SOCKET_NAME_CHARS: usize = 64;

/// An attachment: a model mounted on a named socket of its parent entity's model, or of
/// another attachment's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    /// The mount's name on the parent's model: the glTF node `socket_<name>` of the parent's
    /// asset, 1 to 64 ASCII letters, digits, `-` and `_`.
    pub socket: String,
    /// The attachment's asset, a path inside a pack's `assets/` folder ending in `.glb` or
    /// `.gltf`.
    pub asset: String,
    /// Its scale, on top of the entity's, 0.001 to 1000. Default 1; written only then.
    #[serde(default = "default_scale", skip_serializing_if = "is_one")]
    pub scale: f64,
    /// What is mounted on this attachment's own sockets, in the order given. Default none;
    /// written only when there is one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

fn default_scale() -> f64 {
    1.0
}

fn is_one(v: &f64) -> bool {
    v.to_bits() == 1.0f64.to_bits()
}

impl Attachment {
    /// The attachment `asset` on the socket `socket`, at scale 1, with nothing on its own
    /// sockets.
    pub fn new(socket: impl Into<String>, asset: impl Into<String>) -> Attachment {
        Attachment {
            socket: socket.into(),
            asset: asset.into(),
            scale: 1.0,
            attachments: Vec::new(),
        }
    }

    /// How many attachments this one holds, itself included.
    pub fn size(&self) -> usize {
        1 + self.attachments.iter().map(Attachment::size).sum::<usize>()
    }

    /// How deep the mount runs below and including this one.
    pub fn depth(&self) -> usize {
        1 + self
            .attachments
            .iter()
            .map(Attachment::depth)
            .max()
            .unwrap_or(0)
    }

    /// Checks the attachment and everything below it: `depth` is its level (1 on the entity's
    /// own model), `field` the path of the error (`model.attachments[0]`), `count` the
    /// attachments met so far.
    pub(crate) fn validate(
        &self,
        ctx: Ctx,
        depth: usize,
        field: &str,
        count: &mut usize,
    ) -> Result<(), WorldError> {
        *count += 1;
        if *count > MAX_ATTACHMENTS {
            return Err(ctx.invalid(
                field,
                format!("{count} attachments"),
                format!("an entity has at most {MAX_ATTACHMENTS} attachments, at every depth"),
            ));
        }
        if depth > MAX_ATTACHMENT_DEPTH {
            return Err(ctx.invalid(
                field,
                format!("level {depth}"),
                format!(
                    "attachments mount at most {MAX_ATTACHMENT_DEPTH} levels deep (a base, an \\
                     adapter on it, an attachment on the adapter, ...)"
                ),
            ));
        }
        check_socket_name(ctx, format!("{field}.socket"), &self.socket)?;
        if !is_model_path(&self.asset) {
            let shown: String = self.asset.chars().take(208).collect();
            return Err(ctx.invalid(
                format!("{field}.asset"),
                format!("{shown:?}"),
                "it must be a path inside the pack's assets/ folder ending in .glb or .gltf, \
                 such as \"models/sight.glb\"",
            ));
        }
        ctx.range(
            format!("{field}.scale"),
            self.scale,
            SCALE_MIN,
            SCALE_MAX,
            "",
        )?;
        for (i, child) in self.attachments.iter().enumerate() {
            child.validate(ctx, depth + 1, &format!("{field}.attachments[{i}]"), count)?;
        }
        Ok(())
    }
}

/// The frame of a mount point of a model asset, in the asset's own axes: where a glTF node
/// `socket_<name>` stands and how it is turned (the full rotation, as a unit quaternion
/// `[x, y, z, w]`). Derived data, measured by the host; never written to a file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Socket {
    /// The node's position, metres.
    pub position: [f64; 3],
    /// The node's rotation: `[x, y, z, w]`.
    pub rotation: [f64; 4],
}

/// Every socket of every model asset the host measured: by asset path, then by name.
pub type Sockets = BTreeMap<String, BTreeMap<String, Socket>>;

/// The socket a glTF node's name marks: the name after [`SOCKET_PREFIX`], if it is a plain
/// socket name.
pub fn socket_name_of(node_name: &str) -> Option<&str> {
    node_name
        .strip_prefix(SOCKET_PREFIX)
        .filter(|n| is_socket_name(n))
}

/// Whether `name` is a usable socket name: 1 to [`MAX_SOCKET_NAME_CHARS`] ASCII letters,
/// digits, `-` and `_`.
pub fn is_socket_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SOCKET_NAME_CHARS
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `socket` must be a plain socket name ([`is_socket_name`]); the error names `field`.
pub(crate) fn check_socket_name(
    ctx: Ctx,
    field: impl Into<String>,
    socket: &str,
) -> Result<(), WorldError> {
    if is_socket_name(socket) {
        return Ok(());
    }
    let shown: String = socket.chars().take(MAX_SOCKET_NAME_CHARS + 8).collect();
    Err(ctx.invalid(
        field,
        format!("{shown:?}"),
        format!(
            "a socket is named by the part after `socket_` in its node's name: 1 to \\
             {MAX_SOCKET_NAME_CHARS} ASCII letters, digits, `-` and `_`, such as \"rail_top\""
        ),
    ))
}

/// The names of the sockets a model asset has, as a sentence for an error: `rail_top,
/// rail_left` or `none`.
pub fn list_sockets(sockets: &BTreeMap<String, Socket>) -> String {
    if sockets.is_empty() {
        "none".to_string()
    } else {
        sockets.keys().cloned().collect::<Vec<_>>().join(", ")
    }
}

/// The attachment `path` points to below `model` (each name a socket of the attachment it is
/// on; empty: the model itself), with the chain of attachments along it.
pub fn walk<'a>(model: &'a Model, path: &[String]) -> Option<Vec<&'a Attachment>> {
    let mut chain = Vec::with_capacity(path.len());
    let mut below = &model.attachments;
    for socket in path {
        let found = below.iter().find(|a| a.socket == *socket)?;
        chain.push(found);
        below = &found.attachments;
    }
    Some(chain)
}

/// The entity's model frame: its position, yaw and scale, and the model's scale, as a
/// column-major matrix.
pub fn entity_frame(entity: &Entity, model: &Model) -> DMat4 {
    DMat4::from_scale_rotation_translation(
        DVec3::from_array(entity.scale.times([model.scale; 3])),
        DQuat::from_rotation_y(deg_to_rad(entity.rotation_y_deg)),
        DVec3::from_array(entity.position),
    )
}

/// The socket's frame as a matrix: its rotation, then its position.
fn socket_frame(socket: &Socket) -> DMat4 {
    DMat4::from_rotation_translation(
        DQuat::from_array(socket.rotation),
        DVec3::from_array(socket.position),
    )
}

/// Where an attachment lands in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// The origin of the attachment (where it meets its socket), world metres.
    pub position: [f64; 3],
    /// Its turn in the world: a unit quaternion `[x, y, z, w]`, the entity's yaw and every
    /// socket's turn of the chain, multiplied.
    pub rotation: [f64; 4],
    /// Its scale along its own axes: the lengths of the frame's axes, when they are at right
    /// angles to each other. `None` when the entity is scaled per axis and a socket of the
    /// chain turns by another angle than a right angle: the frame is then skewed, and no
    /// single scale along its own axes describes it ([`Placed::matrix`] is right all the
    /// same).
    pub scale: Option<[f64; 3]>,
    /// Its model-to-world matrix, column-major.
    pub matrix: [f64; 16],
}

/// The chain from the entity's frame to the attachment at `path`: its matrix (the entity's
/// frame, then for each attachment the socket's frame and the attachment's scale) and its
/// turn (the entity's yaw, then each socket's turn). The turn is composed on its own: read
/// back from the matrix it would be wrong wherever a scale per axis meets a turn.
fn compose(
    entity: &Entity,
    model: &Model,
    sockets: &Sockets,
    path: &[String],
) -> Option<(DMat4, DQuat)> {
    let chain = walk(model, path)?;
    let mut frame = entity_frame(entity, model);
    let mut turn = DQuat::from_rotation_y(deg_to_rad(entity.rotation_y_deg));
    let mut parent_asset = model.asset.as_str();
    for attachment in chain {
        let socket = sockets.get(parent_asset)?.get(&attachment.socket)?;
        frame *= socket_frame(socket);
        frame *= DMat4::from_scale(DVec3::splat(attachment.scale));
        turn *= DQuat::from_array(socket.rotation);
        parent_asset = attachment.asset.as_str();
    }
    // A turn is a unit quaternion; the sockets' turns are too, up to rounding.
    let length_sq = turn.length_squared();
    if !length_sq.is_finite() || length_sq <= 0.0 {
        return None;
    }
    if length_sq != 1.0 {
        turn *= 1.0 / length_sq.sqrt();
    }
    Some((frame, turn))
}

/// Where the attachment at `path` below `entity`'s model lands (the origin of the attachment
/// is its socket; [`Placed::matrix`] is its model-to-world matrix). `None` for an empty `path`,
/// when the path names no attachment, when a socket of the chain was not measured (the host
/// has not handed the sockets of that asset over), or when a number is not finite.
pub fn place(entity: &Entity, model: &Model, sockets: &Sockets, path: &[String]) -> Option<Placed> {
    if path.is_empty() {
        return None;
    }
    let (frame, rotation) = compose(entity, model, sockets, path)?;
    let position = frame.w_axis.truncate().to_array();
    let matrix = frame.to_cols_array();
    // The frame's axes: their lengths are the scale along the attachment's own axes when the
    // axes are at right angles to each other. A scale per axis under a turn that is not a
    // right angle skews them ([`Placed::scale`] is then `None`): the frame is right, a single
    // turn and scale read from it would not be.
    let (x, y, z) = (
        frame.x_axis.truncate(),
        frame.y_axis.truncate(),
        frame.z_axis.truncate(),
    );
    let at_right_angles = |a: DVec3, b: DVec3| a.dot(b).abs() <= 1e-9 * a.length() * b.length();
    let scale = (at_right_angles(x, y) && at_right_angles(x, z) && at_right_angles(y, z))
        .then(|| [x.length(), y.length(), z.length()]);
    let finite = position
        .iter()
        .chain(&rotation.to_array())
        .all(|v| v.is_finite())
        && scale.is_none_or(|s| s.iter().all(|v| v.is_finite()))
        && matrix.iter().all(|v| v.is_finite());
    if !finite {
        return None;
    }
    Some(Placed {
        position,
        rotation: rotation.to_array(),
        scale,
        matrix,
    })
}

/// One attachment of a model, by its path: what shows its asset.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    /// Its path: the chain of socket names from the entity's model.
    pub path: Vec<String>,
    /// Its asset.
    pub asset: String,
    /// Its own scale.
    pub scale: f64,
}

/// Every attachment of `model`, depth first, in the order the file lists them: what the
/// renderer and the browser fetch, each once.
pub fn shown(model: &Model) -> Vec<Shown> {
    let mut out = Vec::new();
    fn below(prefix: &[String], list: &[Attachment], out: &mut Vec<Shown>) {
        for a in list {
            let mut path = prefix.to_vec();
            path.push(a.socket.clone());
            out.push(Shown {
                path: path.clone(),
                asset: a.asset.clone(),
                scale: a.scale,
            });
            below(&path, &a.attachments, out);
        }
    }
    below(&[], &model.attachments, &mut out);
    out
}

/// The scale the levels of detail of the attachment at `path` follow: the entity's largest,
/// the model's and every attachment's of the chain; for an empty `path` the entity's own model.
pub fn chain_scale(entity: &Entity, model: &Model, path: &[String]) -> f64 {
    walk(model, path)
        .map(|chain| chain.iter().map(|a| a.scale).product::<f64>())
        .unwrap_or(1.0)
        * model.scale
        * entity.scale.largest()
}
