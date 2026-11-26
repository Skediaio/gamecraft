//! Collision geometry of a model (`gamecraft_project::model::Collider`): nodes named
//! `*_collider` are the colliders and are not drawn (not counted, not in the bounds); without
//! them every mesh of the finest level that draws is a box, in the entity's frame; too many fall
//! back to the bounding box with a sentence. The models are built in the test
//! (`common/gltf.rs`): meshes of unit cubes, scaled and moved by their nodes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

mod common;

use common::gltf::{Mesh, Model, Node};
use gamecraft_project::inspect;
use gamecraft_project::model::{
    COLLIDER_SUFFIXES, ColliderSource, MAX_MODEL_PARTS, is_collider_name,
};
use gamecraft_world::ModelBox;

const CRATE: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/crate.glb");

/// Within a micrometre: the test models' vertices are `f32` (0.15 is not exact there).
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6
}

fn close_box(a: &ModelBox, center: [f64; 3], half: [f64; 3]) -> bool {
    a.center
        .iter()
        .zip(center)
        .chain(a.half.iter().zip(half))
        .all(|(x, y)| close(*x, y))
        && a.yaw_deg == 0.0
}

fn close_bounds(min: [f64; 3], max: [f64; 3], want_min: [f64; 3], want_max: [f64; 3]) -> bool {
    min.iter()
        .zip(want_min)
        .chain(max.iter().zip(want_max))
        .all(|(a, b)| close(*a, b))
}

/// A unit-cube mesh placed as a box from `lo` to `hi`.
fn wall(name: &str, lo: [f64; 3], hi: [f64; 3]) -> Node {
    Node::new(name, Some(0)).at(lo[0], lo[1], lo[2]).scaled3([
        hi[0] - lo[0],
        hi[1] - lo[1],
        hi[2] - lo[2],
    ])
}

/// A facade 8 m wide, 4 m high, 0.3 m thick with a doorway 2 m wide and 2.5 m high in the
/// middle: three meshes.
fn facade() -> Model {
    Model::new(
        &[Mesh(1)],
        vec![
            wall("Left", [-4.0, 0.0, -0.15], [-1.0, 4.0, 0.15]),
            wall("Right", [1.0, 0.0, -0.15], [4.0, 4.0, 0.15]),
            wall("Lintel", [-1.0, 2.5, -0.15], [1.0, 4.0, 0.15]),
        ],
        &[0, 1, 2],
    )
}

/// The same facade in one visual mesh that shows a doorway only on the outside, with three
/// collider nodes 100 m to the side (where they would be seen if they were drawn): the two
/// spellings of the suffix, in any case.
fn facade_with_colliders() -> Model {
    let mut m = facade();
    m.nodes[0].name = "Facade".into();
    m.nodes[0].mesh = Some(0);
    m.nodes[0].translation = [-4.0, 0.0, -0.15];
    m.nodes[0].scale = [8.0, 4.0, 0.3];
    m.nodes.truncate(1);
    for (i, name) in ["left_collider", "right-collider", "Lintel_COLLIDER"]
        .iter()
        .enumerate()
    {
        let mut n = wall(
            name,
            [100.0 + i as f64, 0.0, -0.15],
            [101.0 + i as f64, 4.0, 0.15],
        );
        n.name = (*name).to_string();
        m.nodes.push(n);
    }
    m.roots = vec![0, 1, 2, 3];
    m
}

#[test]
fn collider_nodes_are_the_colliders_and_are_not_drawn() {
    let m = facade_with_colliders();
    let s = m.inspect().unwrap();
    // Drawn: the facade only: 12 triangles, and bounds that stop at x = 4.
    assert_eq!(s.triangles, 12, "{s:?}");
    assert!(close_bounds(
        s.bounds.min,
        s.bounds.max,
        [-4.0, 0.0, -0.15],
        [4.0, 4.0, 0.15]
    ));
    assert_eq!(s.meshes, 1);
    // The colliders: the three collider nodes, one box each, where they stand in the file.
    assert_eq!(s.collider.source, ColliderSource::Colliders);
    assert!(s.collider.note.is_none(), "{:?}", s.collider.note);
    assert_eq!(s.collider.parts.len(), 3, "{:?}", s.collider.parts);
    assert!(close_box(
        &s.collider.parts[0],
        [100.5, 2.0, 0.0],
        [0.5, 2.0, 0.15]
    ));
    assert!(close_box(
        &s.collider.parts[2],
        [102.5, 2.0, 0.0],
        [0.5, 2.0, 0.15]
    ));
}

#[test]
fn without_collider_nodes_each_mesh_of_the_finest_level_is_a_part() {
    let s = facade().inspect().unwrap();
    assert_eq!(s.collider.source, ColliderSource::Meshes);
    assert_eq!(s.collider.parts.len(), 3, "{:?}", s.collider.parts);
    let left = s
        .collider
        .parts
        .iter()
        .find(|p| p.center[0] < 0.0 && p.half[1] > 1.9)
        .unwrap();
    assert!(
        close_box(left, [-2.5, 2.0, 0.0], [1.5, 2.0, 0.15]),
        "{left:?}"
    );
    let lintel = s.collider.parts.iter().find(|p| p.center[1] > 2.0).unwrap();
    assert!(
        close_box(lintel, [0.0, 3.25, 0.0], [1.0, 0.75, 0.15]),
        "{lintel:?}"
    );
    // The parts lie inside the bounds, and together they cover them.
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &s.collider.parts {
        for k in 0..3 {
            lo[k] = lo[k].min(p.center[k] - p.half[k]);
            hi[k] = hi[k].max(p.center[k] + p.half[k]);
        }
    }
    assert!(
        close_bounds(lo, hi, s.bounds.min, s.bounds.max),
        "{lo:?} {hi:?}"
    );
    // A model with levels: the finest level's meshes only (the coarser levels stand in for
    // them, they are not more parts).
    let s = Model::named().inspect().unwrap();
    assert_eq!(s.collider.source, ColliderSource::Meshes);
    assert_eq!(s.collider.parts.len(), 1, "{:?}", s.collider.parts);
    assert!(close_box(
        &s.collider.parts[0],
        [1.5, 0.5, 0.5],
        [1.5, 0.5, 0.5]
    ));
    // One mesh: its one part is the bounding box, the collider models had before.
    let c = inspect(CRATE, &|_: &str| None).unwrap();
    assert_eq!(c.collider.source, ColliderSource::Meshes);
    assert_eq!(c.collider.parts.len(), 1);
    let p = &c.collider.parts[0];
    for k in 0..3 {
        assert!(
            close(p.center[k] - p.half[k], c.bounds.min[k])
                && close(p.center[k] + p.half[k], c.bounds.max[k]),
            "{p:?} vs {:?}..{:?}",
            c.bounds.min,
            c.bounds.max
        );
    }
}

#[test]
fn a_turned_mesh_is_a_box_in_the_entity_s_frame() {
    // A wall turned 30 degrees in the file: its part is the smallest box around it in the
    // entity's frame (fatter than the wall), alike for every reader.
    let m = Model::new(
        &[Mesh(1)],
        vec![
            Node::new("Wall", Some(0))
                .turned(30.0)
                .scaled3([4.0, 3.0, 0.2])
                .at(0.0, 0.0, 0.0),
        ],
        &[0],
    );
    let s = m.inspect().unwrap();
    assert_eq!(s.collider.parts.len(), 1);
    let p = &s.collider.parts[0];
    assert_eq!(p.yaw_deg, 0.0);
    // Fatter than the wall's 0.1 m of thickness, narrower than its 2 m of length: the box
    // around the turned slab, and exactly the bounds.
    assert!(
        p.half[2] > 0.5 && p.half[0] > 1.7 && p.half[0] < 1.9,
        "{p:?}"
    );
    let lo: Vec<f64> = p.center.iter().zip(p.half).map(|(c, h)| c - h).collect();
    let hi: Vec<f64> = p.center.iter().zip(p.half).map(|(c, h)| c + h).collect();
    assert!(
        close_bounds(
            [lo[0], lo[1], lo[2]],
            [hi[0], hi[1], hi[2]],
            s.bounds.min,
            s.bounds.max
        ),
        "{lo:?} {hi:?} {:?} {:?}",
        s.bounds.min,
        s.bounds.max
    );
}

#[test]
fn too_many_parts_are_the_bounding_box_with_a_sentence() {
    let n = MAX_MODEL_PARTS + 1;
    let mut nodes = Vec::new();
    for i in 0..n {
        nodes.push(wall(
            &format!("Wall{i}"),
            [i as f64, 0.0, 0.0],
            [i as f64 + 1.0, 1.0, 1.0],
        ));
    }
    let m = Model::new(&[Mesh(1)], nodes, &(0..n).collect::<Vec<_>>());
    let s = m.inspect().unwrap();
    assert_eq!(s.collider.source, ColliderSource::Box);
    assert_eq!(s.collider.parts.len(), 1);
    let note = s.collider.note.as_deref().unwrap();
    assert!(
        note.contains(&format!("{n}")) && note.contains("bounding box"),
        "{note}"
    );
    assert!(close_bounds(
        s.bounds.min,
        s.bounds.max,
        [0.0, 0.0, 0.0],
        [n as f64, 1.0, 1.0]
    ));
}

#[test]
fn both_suffixes_are_collider_in_any_case() {
    assert_eq!(COLLIDER_SUFFIXES, ["_collider", "-collider"]);
    for name in [
        "wall_collider",
        "Wall_Collider",
        "UBX_Wall_COLLIDER",
        "wall-collider",
        "Wall-Collider",
        "wall-COLLIDER",
        "LOD0_collider",
    ] {
        assert!(is_collider_name(name), "{name}");
    }
    for name in [
        "collider",
        "wall_colliders",
        "wallcollider",
        "wall-colliders",
        "wall collider",
        "wall",
        "",
        "_collider",
        "-collider",
    ] {
        assert!(!is_collider_name(name), "{name}");
    }
}
