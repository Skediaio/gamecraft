//! The example world with a model whose sockets a host measured and a two-deep mount on it:
//! what the attachment commands' examples run on. Built as data (no undo steps), so a test can
//! count the steps its own commands take.

use gamecraft_engine::{Session, example_world};
use gamecraft_world::attachments::Attachment;
use gamecraft_world::{Component, EntityDraft, Model, Socket, World};
use std::collections::BTreeMap;

/// The example world with the mount tree of the attachment commands' examples: the entity
/// `Rifle` (a model with the sockets `rail_top` and `rail_left`), a sight on `rail_top` whose
/// own model has the socket `top`, and an optic on it.
pub fn world() -> World {
    let mut world = example_world();
    let mut rifle = Model::of("models/launcher.glb");
    rifle.attachments.push(Attachment {
        attachments: vec![Attachment::new("top", "models/optic.glb")],
        ..Attachment::new("rail_top", "models/sight.glb")
    });
    world
        .add_entity(EntityDraft {
            name: "Rifle".into(),
            position: [4.0, 12.5, -3.0],
            components: vec![Component::Model(rifle)],
            ..EntityDraft::default()
        })
        .expect("the example world takes an entity");
    let identity = [0.0, 0.0, 0.0, 1.0];
    let socket = |position: [f64; 3]| Socket {
        position,
        rotation: identity,
    };
    let sockets: BTreeMap<String, BTreeMap<String, Socket>> = [
        (
            "models/launcher.glb".to_string(),
            BTreeMap::from([
                ("rail_top".to_string(), socket([0.0, 0.9, 0.0])),
                ("rail_left".to_string(), socket([-0.04, 0.8, 0.1])),
            ]),
        ),
        (
            "models/sight.glb".to_string(),
            BTreeMap::from([("top".to_string(), socket([0.0, 0.06, 0.05]))]),
        ),
    ]
    .into_iter()
    .collect();
    world.set_model_sockets(sockets);
    world
}

/// A session of that world, editing.
pub fn session() -> Session {
    Session::new(world())
}

/// A session of that world, playing.
pub fn playing() -> Session {
    let mut s = session();
    s.run("sim.start", &serde_json::Value::Null)
        .expect("sim.start runs");
    s
}
