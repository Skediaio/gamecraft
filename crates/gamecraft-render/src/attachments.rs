//! Attachments in the picture ([`gamecraft_world::attachments`]): the matrix of a model's
//! attachment at a path, so the renderer draws it as its own item at the socket's full
//! rotation. A static one stands where the engine places it ([`frame`]); a character's is
//! taken relative to the entity's model frame and put on the frame the character's item
//! draws that model in ([`pose`]), so it follows it between ticks and lies with it when it
//! is down.

use gamecraft_world::attachments;
use gamecraft_world::{Entity, Model, World};
use glam::{DMat4, Mat4};

/// The instance matrix of the attachment at `path` below `e`'s `model` in the world: the
/// frame the engine composes ([`attachments::place`]), as it is. `None` when a socket of the
/// chain was not measured or the path names no attachment.
pub(crate) fn frame(world: &World, e: &Entity, model: &Model, path: &[String]) -> Option<Mat4> {
    let placed = attachments::place(e, model, world.all_model_sockets(), path)?;
    let mut matrix = [0.0f32; 16];
    for (to, from) in matrix.iter_mut().zip(placed.matrix) {
        *to = from as f32;
    }
    Some(Mat4::from_cols_array(&matrix))
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use gamecraft_world::attachments::Socket;
    use gamecraft_world::{Component, EntityDraft};
    use std::collections::BTreeMap;

    /// A static attachment's frame is the engine's: a rig far from the origin, with a mount
    /// on a socket of its model, stands where `place` says, to well under a centimetre.
    #[test]
    fn a_static_attachment_stands_where_the_engine_places_it() {
        let mut world = World::new("rig", 1);
        let mut model = Model::of("models/base.glb");
        model.attachments = vec![attachments::Attachment::new("side", "models/slab.glb")];
        world
            .add_entity(EntityDraft {
                name: "Rig".into(),
                position: [50_000.0, 0.0, -70_000.0],
                rotation_y_deg: 30.0,
                components: vec![Component::Model(model)],
                ..EntityDraft::default()
            })
            .unwrap();
        world.set_model_sockets(
            [(
                "models/base.glb".to_string(),
                [(
                    "side".to_string(),
                    Socket {
                        position: [0.5, 2.0, -1.5],
                        rotation: [
                            0.0,
                            0.0,
                            std::f64::consts::FRAC_1_SQRT_2,
                            std::f64::consts::FRAC_1_SQRT_2,
                        ],
                    },
                )]
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
            )]
            .into_iter()
            .collect(),
        );
        let e = world.entities().next().unwrap();
        let model = e.model().unwrap();
        let path = ["side".to_string()];
        let placed = attachments::place(e, model, world.all_model_sockets(), &path).unwrap();
        let at = frame(&world, e, model, &path).unwrap();
        // The frame's translation column, in single precision, against the engine's.
        let found = at.w_axis.truncate().to_array().map(f64::from);
        for (found, want) in found.into_iter().zip(placed.position) {
            assert!(
                (found - want).abs() < 1e-2,
                "{found} against {want}: the engine's {:?}",
                placed.position
            );
        }
        assert!(frame(&world, e, model, &["nowhere".to_string()]).is_none());
    }
}
