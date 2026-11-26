//! Sockets of a model as the host reads them (`gamecraft_project::lod::sockets`,
//! `ModelSummary::sockets`): every node named `socket_<name>` of the scene with its position and
//! full rotation in the asset's own axes, nested nodes included; a name used twice keeps the
//! first; a node outside the scene is no socket. The models are built in the test
//! (`common/gltf.rs`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

mod common;

use common::gltf::{Mesh, Model, Node};
use gamecraft_math::{DMat4, DQuat, DVec3};
use gamecraft_project::{ColliderSource, inspect};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6
}

/// A unit quaternion `[x, y, z, w]` for a turn of `deg` about the axis (`ax`, `ay`, `az`).
fn turn(deg: f64, ax: f64, ay: f64, az: f64) -> [f64; 4] {
    let half = deg * std::f64::consts::PI / 360.0;
    let (s, c) = gamecraft_math::Portable::psin_cos(half);
    [ax * s, ay * s, az * s, c]
}

#[test]
fn socket_nodes_give_their_frames_in_the_asset_s_axes() {
    // A body (turned 10 degrees) carries `socket_rail` on top, rolled 90 degrees about its
    // length, and under it `socket_top`, a little forward; `socket_side` hangs off the body.
    let m = Model::new(
        &[Mesh(1)],
        vec![
            Node::new("Body", Some(0))
                .at(1.0, 2.0, 3.0)
                .turned(10.0)
                .children(&[1, 3]),
            Node::new("socket_rail", None)
                .at(0.0, 1.0, 0.0)
                .rotated(turn(90.0, 0.0, 0.0, 1.0))
                .children(&[2]),
            Node::new("socket_top", None).at(0.0, 0.05, 0.2),
            Node::new("socket_side", None).at(-0.3, 0.6, 0.1),
        ],
        &[0],
    );
    let s = m.inspect().unwrap();
    let names: Vec<&str> = s.sockets.keys().map(String::as_str).collect();
    assert_eq!(names, ["rail", "side", "top"], "{:?}", s.sockets);
    let rail = &s.sockets["rail"];
    let top = &s.sockets["top"];
    // Composed with glam, operation for operation: the body's frame, then the socket's.
    let body = DMat4::from_rotation_translation(
        DQuat::from_rotation_y(gamecraft_math::deg_to_rad(10.0)),
        DVec3::new(1.0, 2.0, 3.0),
    );
    let roll = turn(90.0, 0.0, 0.0, 1.0);
    let rail_frame = body
        * DMat4::from_rotation_translation(
            DQuat::from_xyzw(roll[0], roll[1], roll[2], roll[3]),
            DVec3::new(0.0, 1.0, 0.0),
        );
    let (_, r, t) = rail_frame.to_scale_rotation_translation();
    for k in 0..4 {
        assert!(
            (rail.rotation[k] - r.to_array()[k]).abs() < 1e-5,
            "the roll is kept whole: {rail:?}"
        );
    }
    for k in 0..3 {
        assert!(close(rail.position[k], t.to_array()[k]), "{rail:?}");
    }
    assert!(
        close(rail.position[0], 1.0)
            && close(rail.position[1], 3.0)
            && close(rail.position[2], 3.0),
        "{rail:?}"
    );
    // The top: under the rolled rail, so its 0.05 up is the body's -x (before its turn).
    let top_frame =
        rail_frame * DMat4::from_rotation_translation(DQuat::IDENTITY, DVec3::new(0.0, 0.05, 0.2));
    let (_, r2, t2) = top_frame.to_scale_rotation_translation();
    for k in 0..4 {
        assert!((top.rotation[k] - r2.to_array()[k]).abs() < 1e-5, "{top:?}");
    }
    for k in 0..3 {
        assert!(close(top.position[k], t2.to_array()[k]), "{top:?}");
    }
    assert!(
        top.position[0] < 1.0 && close(top.position[1], 3.0) && top.position[2] > 3.2,
        "forward and to the left: {top:?}"
    );
    // Sockets are not drawn and are not colliders: the triangles, the bounds and the collider
    // are the body's alone.
    assert_eq!(s.triangles, 12);
    assert_eq!(s.meshes, 1);
    let body_only = Model::new(
        &[Mesh(1)],
        vec![Node::new("Body", Some(0)).at(1.0, 2.0, 3.0).turned(10.0)],
        &[0],
    )
    .inspect()
    .unwrap();
    assert_eq!(s.bounds.min, body_only.bounds.min);
    assert_eq!(s.bounds.max, body_only.bounds.max);
    assert_eq!(s.collider.source, ColliderSource::Meshes);
    assert_eq!(s.collider.parts.len(), 1);
}

#[test]
fn a_name_used_twice_keeps_the_first_and_a_node_outside_the_scene_is_no_socket() {
    let mut m = Model::new(
        &[Mesh(1)],
        vec![
            Node::new("socket_rail", None).at(0.0, 1.0, 0.0),
            Node::new("socket_rail", None).at(5.0, 5.0, 5.0),
            Node::new("socket_lost", None).at(9.0, 9.0, 9.0),
            Node::new("Body", Some(0)).children(&[0, 1]),
        ],
        &[3],
    );
    let s = m.inspect().unwrap();
    assert_eq!(s.sockets.len(), 1, "{:?}", s.sockets);
    assert_eq!(s.sockets["rail"].position, [0.0, 1.0, 0.0]);
    // A socket whose name is not a plain name is no socket; so is `socket_` alone.
    m.nodes[2].name = "socket_".into();
    m.nodes[0].name = "socket_with space".into();
    m.nodes[3].children = vec![0, 2];
    m.roots = vec![3];
    let s = m.inspect().unwrap();
    assert!(s.sockets.is_empty(), "{:?}", s.sockets);
    // The crate and the models of the demo: no sockets.
    let crate_: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/crate.glb");
    assert!(inspect(crate_, &|_: &str| None).unwrap().sockets.is_empty());
}

#[test]
fn a_socket_under_a_parent_scaled_per_axis_keeps_the_turns_and_its_place() {
    // A body scaled [2, 1, 0.5] and turned 20 degrees carries `socket_arm` at (1, 2, 3) in
    // the body, turned 30 degrees about z (a right angle would only swap the axes, and
    // leave no shear to find).
    let m = Model::new(
        &[Mesh(1)],
        vec![
            Node::new("Body", Some(0))
                .at(5.0, 0.0, -4.0)
                .turned(20.0)
                .scaled3([2.0, 1.0, 0.5])
                .children(&[1]),
            Node::new("socket_arm", None)
                .at(1.0, 2.0, 3.0)
                .rotated(turn(30.0, 0.0, 0.0, 1.0)),
        ],
        &[0],
    );
    let s = m.inspect().unwrap();
    let arm = &s.sockets["arm"];
    // The turn is the body's and then the socket's own: the scales play no part. (Read
    // back from the matrix they shear, the turn would be neither.)
    let want = DQuat::from_rotation_y(gamecraft_math::deg_to_rad(20.0))
        * DQuat::from_array(turn(30.0, 0.0, 0.0, 1.0));
    for k in 0..4 {
        assert!(close(arm.rotation[k], want.to_array()[k]), "{arm:?}");
    }
    // The place is in the scaled body.
    let body = DMat4::from_scale_rotation_translation(
        DVec3::new(2.0, 1.0, 0.5),
        DQuat::from_rotation_y(gamecraft_math::deg_to_rad(20.0)),
        DVec3::new(5.0, 0.0, -4.0),
    );
    let at = body.transform_point3(DVec3::new(1.0, 2.0, 3.0));
    for k in 0..3 {
        assert!(close(arm.position[k], at.to_array()[k]), "{arm:?}");
    }
}
