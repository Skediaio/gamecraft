//! Attachments: where a two-deep chain lands (the composed transform, to a micrometre and a
//! hundredth of a degree, with a socket turned 90 degrees about the length axis), the checks of
//! the tree, the file format with and without attachments, and what gives no place.

use crate::attachments::{
    Attachment, Attachment as A, MAX_ATTACHMENT_DEPTH, MAX_ATTACHMENTS, SOCKET_PREFIX, Socket,
    Sockets, is_socket_name, is_socket_node, list_sockets, place, socket_name_of, walk,
};
use crate::{Component, EntityDraft, Model, World};
use gamecraft_math::{DMat4, DQuat, DVec3, Portable, deg_to_rad};
use std::collections::BTreeMap;

/// A socket at `position`, turned by the unit quaternion `rotation` (`[x, y, z, w]`).
fn socket(position: [f64; 3], rotation: [f64; 4]) -> Socket {
    Socket { position, rotation }
}

/// A turn of 90 degrees about +Z (the length axis of a mount).
const ROLL_Z: [f64; 4] = [
    0.0,
    0.0,
    std::f64::consts::FRAC_1_SQRT_2,
    std::f64::consts::FRAC_1_SQRT_2,
];
/// No turn at all.
const IDENTITY: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

/// A base with `rail_top` (on top, rolled 90 degrees) and `rail_left`, and an adapter with
/// `top` a little forward and up.
fn sockets() -> Sockets {
    [
        (
            "models/base.glb".to_string(),
            [
                ("rail_top".to_string(), socket([0.0, 1.0, 0.0], ROLL_Z)),
                ("rail_left".to_string(), socket([-0.3, 0.6, 0.1], IDENTITY)),
            ]
            .into_iter()
            .collect(),
        ),
        (
            "models/adapter.glb".to_string(),
            [("top".to_string(), socket([0.0, 0.05, 0.2], IDENTITY))]
                .into_iter()
                .collect(),
        ),
    ]
    .into_iter()
    .collect()
}

/// The base with an adapter (scale 2) on `rail_top` that carries an optic (scale 0.5) on `top`.
fn chain() -> Model {
    Model {
        attachments: vec![Attachment {
            socket: "rail_top".to_string(),
            asset: "models/adapter.glb".to_string(),
            scale: 2.0,
            attachments: vec![Attachment {
                socket: "top".to_string(),
                asset: "models/optic.glb".to_string(),
                scale: 0.5,
                attachments: Vec::new(),
            }],
        }],
        ..Model::of("models/base.glb")
    }
}

/// The entity of the tests: at [10, 0, 20], turned 30 degrees, its model twice as large, its
/// sockets measured.
fn rifle(world: &mut World) -> crate::Entity {
    world
        .add_entity(EntityDraft {
            name: "Rifle".into(),
            position: [10.0, 0.0, 20.0],
            rotation_y_deg: 30.0,
            components: vec![Component::Model(Model {
                scale: 2.0,
                ..chain()
            })],
            ..EntityDraft::default()
        })
        .unwrap();
    world.set_model_sockets(sockets());
    world.entity(crate::EntityId(1)).unwrap().clone()
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn a_two_deep_chain_lands_at_the_composed_transform() {
    let mut world = World::new("w", 1);
    let rifle = rifle(&mut world);
    let model = rifle.model().unwrap();
    let sockets = world.all_model_sockets();

    // Depth 1: on top of the base, at the model's scale, rolled 90 degrees, turned with it.
    let adapter = place(&rifle, model, sockets, &["rail_top".to_string()]).unwrap();
    for k in 0..3 {
        assert!(
            close(adapter.position[k], [10.0, 2.0, 20.0][k], 1e-9),
            "{adapter:?}"
        );
        assert!(close(adapter.scale[k], 4.0, 1e-9), "{adapter:?}");
    }
    // The entity's yaw and the socket's roll: ry(30) * rz(90), its parts from the half angles.
    let (s, c) = deg_to_rad(15.0).psin_cos();
    let want = [
        s * std::f64::consts::FRAC_1_SQRT_2,
        s * std::f64::consts::FRAC_1_SQRT_2,
        c * std::f64::consts::FRAC_1_SQRT_2,
        c * std::f64::consts::FRAC_1_SQRT_2,
    ];
    for k in 0..4 {
        assert!(close(adapter.rotation[k], want[k], 1e-12), "{adapter:?}");
    }
    // The same composed with glam, operation for operation: the same bits.
    let frame = DMat4::from_scale_rotation_translation(
        DVec3::splat(2.0),
        DQuat::from_rotation_y(deg_to_rad(30.0)),
        DVec3::new(10.0, 0.0, 20.0),
    );
    let with_glam = frame
        * DMat4::from_rotation_translation(
            DQuat::from_xyzw(ROLL_Z[0], ROLL_Z[1], ROLL_Z[2], ROLL_Z[3]),
            DVec3::new(0.0, 1.0, 0.0),
        )
        * DMat4::from_scale(DVec3::splat(2.0));
    assert_eq!(adapter.matrix, with_glam.to_cols_array());

    // Depth 2: on the adapter's `top`, in the adapter's (rolled) frame, half the adapter's size.
    let optic = place(
        &rifle,
        model,
        sockets,
        &["rail_top".to_string(), "top".to_string()],
    )
    .unwrap();
    // The roll turns the adapter's +y (0.05 up) into the base's -x, before the entity's yaw:
    // to a millimetre, 10 + 4 (0.05 sin30 + 0.2 cos30), 2, 20 + 4 (-0.05 cos30 + 0.2 sin30).
    assert!(close(optic.position[0], 10.2268, 1e-3), "{optic:?}");
    assert!(close(optic.position[1], 2.0, 1e-3), "{optic:?}");
    assert!(close(optic.position[2], 20.7928, 1e-3), "{optic:?}");
    for k in 0..3 {
        assert!(close(optic.scale[k], 2.0, 1e-9), "{optic:?}");
    }
    for k in 0..4 {
        assert!(close(optic.rotation[k], want[k], 1e-12), "{optic:?}");
    }
    let with_glam2 = with_glam
        * DMat4::from_rotation_translation(
            DQuat::from_xyzw(IDENTITY[0], IDENTITY[1], IDENTITY[2], IDENTITY[3]),
            DVec3::new(0.0, 0.05, 0.2),
        )
        * DMat4::from_scale(DVec3::splat(0.5));
    assert_eq!(optic.matrix, with_glam2.to_cols_array());
    let (s2, r2, t2) = with_glam2.to_scale_rotation_translation();
    assert_eq!(optic.position, t2.to_array());
    assert_eq!(optic.rotation, r2.to_array());
    assert_eq!(optic.scale, s2.to_array());
}

#[test]
fn what_gives_no_place() {
    let mut world = World::new("w", 1);
    let rifle = rifle(&mut world);
    let model = rifle.model().unwrap();
    let sockets = world.all_model_sockets();
    // No path: the entity itself is not an attachment.
    assert!(place(&rifle, model, sockets, &[]).is_none());
    // A socket nobody mounted anything on, a wrong name, a wrong second step.
    assert!(place(&rifle, model, sockets, &["rail_left".to_string()]).is_none());
    assert!(
        place(
            &rifle,
            model,
            sockets,
            &["rail_top".to_string(), "side".to_string()]
        )
        .is_none()
    );
    // The walk and the accessor agree with the tree.
    assert_eq!(walk(model, &[]).unwrap().len(), 0);
    assert_eq!(walk(model, &["rail_top".to_string()]).unwrap().len(), 1);
    assert!(
        model
            .attachment(&["rail_top".to_string(), "top".to_string()])
            .is_some()
    );
    assert!(model.attachment(&["top".to_string()]).is_none());
    assert_eq!(list_sockets(None), "");
    assert_eq!(list_sockets(Some(&BTreeMap::new())), "none");
    assert_eq!(
        list_sockets(world.model_sockets("models/base.glb")),
        "rail_left, rail_top"
    );
    // The size and depth of a tree.
    assert_eq!(model.attachments[0].size(), 2);
    assert_eq!(model.attachments[0].depth(), 2);
    // Sockets not measured (a world opened alone): no place.
    let mut alone = World::new("alone", 1);
    alone
        .add_entity(EntityDraft {
            name: "Rifle".into(),
            components: vec![Component::Model(chain())],
            ..EntityDraft::default()
        })
        .unwrap();
    let e = alone.entity(crate::EntityId(1)).unwrap();
    assert!(
        place(
            e,
            e.model().unwrap(),
            alone.all_model_sockets(),
            &["rail_top".to_string()]
        )
        .is_none()
    );
}

#[test]
fn names_are_checked() {
    assert_eq!(SOCKET_PREFIX, "socket_");
    assert!(is_socket_name("rail_top") && is_socket_name("a") && is_socket_name("x-1_Y"));
    assert!(
        !is_socket_name("") && !is_socket_name("with space") && !is_socket_name(&"x".repeat(65))
    );
    assert!(is_socket_node("socket_rail_top"));
    assert!(!is_socket_node("socket_"));
    assert!(!is_socket_node("rail_top") && !is_socket_node("sockets_a"));
    assert_eq!(socket_name_of("socket_rail_top"), Some("rail_top"));
    assert_eq!(socket_name_of("rail_top"), None);
}

#[test]
fn the_tree_round_trips_and_an_empty_one_writes_nothing() {
    let plain = serde_json::to_string(&Component::Model(Model::of("models/base.glb"))).unwrap();
    assert_eq!(
        plain,
        r#"{"type":"model","asset":"models/base.glb","scale":1.0,"solid":false,"collider":"box"}"#
    );
    let text = serde_json::to_string(&Component::Model(chain())).unwrap();
    assert_eq!(
        text,
        r#"{"type":"model","asset":"models/base.glb","scale":1.0,"solid":false,"collider":"box","attachments":[{"socket":"rail_top","asset":"models/adapter.glb","scale":2.0,"attachments":[{"socket":"top","asset":"models/optic.glb","scale":0.5}]}]}"#
    );
    let back: Component = serde_json::from_str(&text).unwrap();
    assert_eq!(back, Component::Model(chain()));
    // A default scale is not written, and an unknown field is refused.
    let one = serde_json::to_string(&Component::Model(Model {
        attachments: vec![A::new("rail_top", "models/sight.glb")],
        ..Model::of("models/base.glb")
    }))
    .unwrap();
    assert!(
        one.contains(r#"{"socket":"rail_top","asset":"models/sight.glb"}"#),
        "{one}"
    );
    assert!(
        serde_json::from_str::<Component>(
            r#"{"type":"model","asset":"models/a.glb","attachments":[{"socket":"a","asset":"models/b.glb","mount":"x"}]}"#
        )
        .is_err()
    );
    // A world without attachments keeps its text and its hash: the field is written only when
    // there is one.
    let mut world = World::new("w", 1);
    world
        .add_entity(EntityDraft {
            name: "Rifle".into(),
            components: vec![Component::Model(Model::of("models/base.glb"))],
            ..EntityDraft::default()
        })
        .unwrap();
    let before = world.to_json().unwrap();
    assert!(!before.contains("attachments"), "{before}");
}

#[test]
fn a_broken_tree_is_refused_by_name() {
    let mut world = World::new("w", 1);
    let mut add = |attachments: Vec<Attachment>| {
        world.add_entity(EntityDraft {
            name: "Rifle".into(),
            components: vec![Component::Model(Model {
                attachments,
                ..Model::of("models/base.glb")
            })],
            ..EntityDraft::default()
        })
    };
    // Two on one socket.
    let e = add(vec![
        A::new("rail_top", "models/a.glb"),
        A::new("rail_top", "models/b.glb"),
    ])
    .unwrap_err()
    .to_string();
    assert!(
        e.contains("model.attachments[1].socket") && e.contains("holds one attachment"),
        "{e}"
    );
    // A bad name, a bad asset, a bad scale.
    let e = add(vec![A::new("with space", "models/a.glb")])
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("model.attachments[0].socket") && e.contains("socket_"),
        "{e}"
    );
    let e = add(vec![A::new("rail_top", "models/a.obj")])
        .unwrap_err()
        .to_string();
    assert!(e.contains("model.attachments[0].asset"), "{e}");
    let e = add(vec![Attachment {
        scale: 0.0,
        ..A::new("rail_top", "models/a.glb")
    }])
    .unwrap_err()
    .to_string();
    assert!(e.contains("model.attachments[0].scale"), "{e}");
    // Too deep: a chain of MAX_ATTACHMENT_DEPTH levels is fine, one more is not.
    let mut deep = A::new("s0", "models/a.glb");
    for i in 1..MAX_ATTACHMENT_DEPTH {
        deep = Attachment {
            attachments: vec![deep],
            ..A::new(format!("s{i}"), "models/a.glb")
        };
    }
    assert_eq!(deep.depth(), MAX_ATTACHMENT_DEPTH);
    assert!(add(vec![deep.clone()]).is_ok());
    let too_deep = Attachment {
        attachments: vec![deep],
        ..A::new("top", "models/a.glb")
    };
    let e = add(vec![too_deep]).unwrap_err().to_string();
    assert!(
        e.contains(&format!("level {}", MAX_ATTACHMENT_DEPTH + 1))
            && e.contains(&format!("{MAX_ATTACHMENT_DEPTH} levels")),
        "{e}"
    );
    // Too many: MAX_ATTACHMENTS is fine, one more is not.
    let many: Vec<Attachment> = (0..MAX_ATTACHMENTS)
        .map(|i| A::new(format!("s{i}"), "models/a.glb"))
        .collect();
    assert!(add(many.clone()).is_ok());
    let mut too_many = many;
    too_many.push(A::new("more", "models/a.glb"));
    let e = add(too_many).unwrap_err().to_string();
    assert!(
        e.contains(&format!("{}", MAX_ATTACHMENTS + 1))
            && e.contains(&format!("at most {MAX_ATTACHMENTS}")),
        "{e}"
    );
}

#[test]
fn sockets_handed_over_are_kept_and_the_odd_ones_dropped() {
    let mut world = World::new("w", 1);
    let _ = rifle(&mut world);
    assert_eq!(
        world.model_sockets("models/base.glb").map(BTreeMap::len),
        Some(2)
    );
    assert!(world.model_sockets("models/none.glb").is_none());
    let mut odd = sockets();
    odd.get_mut("models/base.glb")
        .unwrap()
        .insert("far".to_string(), socket([f64::NAN, 0.0, 0.0], IDENTITY));
    world.set_model_sockets(odd);
    assert_eq!(
        world.model_sockets("models/base.glb").map(BTreeMap::len),
        Some(2)
    );
    // Derived data: the text does not change.
    let text = world.to_json().unwrap();
    assert!(
        text.contains("rail_top"),
        "the attachment is in the file: {text}"
    );
    world.set_model_sockets(sockets());
    assert_eq!(world.to_json().unwrap(), text);
}
