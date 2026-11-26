//! Attachments in the picture ([`gamecraft_world::attachments`]): the matrix of a model's
//! attachment at a path, so the renderer draws it as its own item at the socket's full
//! rotation, on the frame in which the entity's own model is drawn. A character's attachments
//! follow it between ticks as its own model does, and lie with it when it is down.

use gamecraft_world::attachments;
use gamecraft_world::{Entity, Model, World};
use glam::{DMat4, Mat4};

/// The instance matrix of the attachment at `path` below `e`'s `model`, drawn on `own`: the
/// frame in which the item draws the entity's own model (standing or lying, scaled as that
/// item scales it). The chain is the one the engine composes ([`attachments::place`]), taken
/// relative to the entity's model frame, so the attachment follows the body as the body is
/// drawn. `None` when a socket of the chain was not measured or the path names no attachment.
pub(crate) fn pose(
    world: &World,
    e: &Entity,
    model: &Model,
    path: &[String],
    own: Mat4,
) -> Option<Mat4> {
    let placed = attachments::place(e, model, world.all_model_sockets(), path)?;
    let relative =
        attachments::entity_frame(e, model).inverse() * DMat4::from_cols_array(&placed.matrix);
    Some((own.as_dmat4() * relative).as_mat4())
}
