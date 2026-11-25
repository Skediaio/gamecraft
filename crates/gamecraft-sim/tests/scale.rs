//! A scale per axis in the simulation: a box blocks and is hit to its scaled size on every
//! axis, round shapes take the middle of the three scales, and a uniform scale behaves as
//! before.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, flat_world, pos, walk};
use gamecraft_sim::{Sim, ray};
use gamecraft_world::{Component, EntityDraft, EntityId, Scale, Shape, ShapeKind, World};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// A solid shape of `size` at `position`, turned by `yaw`, scaled as `scale`.
fn solid(
    world: &mut World,
    kind: ShapeKind,
    size: [f64; 3],
    position: [f64; 3],
    scale: Scale,
) -> EntityId {
    world
        .add_entity(EntityDraft {
            name: "solid".into(),
            position,
            scale,
            components: vec![Component::Shape(Shape {
                kind,
                size,
                color: [0.5, 0.5, 0.5],
                solid: true,
            })],
            ..EntityDraft::default()
        })
        .unwrap()
}

/// A ray down from 40 m at `x`, `z`: how far it fell and what it hit (the ground: `None`).
fn down(sim: &Sim, x: f64, z: f64) -> Option<(f64, Option<EntityId>)> {
    ray::cast(
        sim.world(),
        sim.heights(),
        [x, 40.0, z],
        [0.0, -1.0, 0.0],
        100.0,
        |_| false,
    )
    .map(|f| (f.hit.distance_m, f.entity))
}

#[test]
fn a_stretched_box_blocks_and_is_hit_on_every_scaled_axis() {
    let mut world = flat_world(0.0);
    // A box 2 x 1 x 10, scaled [3, 2, 0.5]: 6 m along x, 2 m high, 5 m along z.
    let id = solid(
        &mut world,
        ShapeKind::Box,
        [2.0, 1.0, 10.0],
        [0.0, 0.0, 0.0],
        Scale::per_axis(3.0, 2.0, 0.5),
    );
    // One walking in from the east, one falling over it.
    let walker = add_character(&mut world, [8.0, 0.0, 0.0]);
    let lander = add_character(&mut world, [0.0, 5.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    // Its top is at 2 m; beside it on either axis there is the ground.
    assert_eq!(down(&sim, 2.9, 2.4), Some((38.0, Some(id))));
    assert_eq!(down(&sim, 3.1, 0.0), Some((40.0, None)));
    assert_eq!(down(&sim, 0.0, 2.6), Some((40.0, None)));
    // Its side is at x = 3.
    let side = ray::cast(
        sim.world(),
        sim.heights(),
        [10.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        100.0,
        |_| false,
    )
    .unwrap();
    assert!(close(side.hit.distance_m, 7.0) && side.entity == Some(id));
    // The walker stops at the side, its radius away.
    sim.set_input(walker, walk(90.0)).unwrap();
    sim.step(180);
    let p = pos(&sim, walker);
    assert!(close(p[0], 3.3) && close(p[2], 0.0), "{p:?}");
    // The other, falling from 5 m over the box, landed on its top.
    assert_eq!(pos(&sim, lander)[1], 2.0);
}

#[test]
fn a_uniform_scale_is_what_it_was() {
    let mut world = flat_world(0.0);
    let id = solid(
        &mut world,
        ShapeKind::Box,
        [2.0, 1.0, 2.0],
        [0.0, 0.0, 0.0],
        Scale::uniform(2.0),
    );
    let sim = Sim::new(world).unwrap();
    // Twice the size: 4 m wide, 2 m high, as before per-axis scales.
    assert_eq!(down(&sim, 1.9, 0.0), Some((38.0, Some(id))));
    assert_eq!(down(&sim, 2.1, 0.0), Some((40.0, None)));
}

#[test]
fn round_shapes_take_the_middle_scale() {
    let mut world = flat_world(0.0);
    // A cylinder 1 m across and 2 m tall: scaled [1, 3, 10] it is 3 m across (the middle of
    // the three) and 6 m tall (the y scale).
    let cylinder = solid(
        &mut world,
        ShapeKind::Cylinder,
        [1.0, 2.0, 1.0],
        [0.0, 0.0, 0.0],
        Scale::per_axis(1.0, 3.0, 10.0),
    );
    // A sphere 2 m across: scaled [1, 1, 10] it stays 2 m across.
    let sphere = solid(
        &mut world,
        ShapeKind::Sphere,
        [2.0, 2.0, 2.0],
        [20.0, 0.0, 0.0],
        Scale::per_axis(1.0, 1.0, 10.0),
    );
    let walker = add_character(&mut world, [5.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    assert_eq!(
        down(&sim, 1.4, 0.0),
        Some((34.0, Some(cylinder))),
        "the top at 6 m"
    );
    assert_eq!(down(&sim, 1.6, 0.0), Some((40.0, None)));
    assert_eq!(
        down(&sim, 0.0, 1.6),
        Some((40.0, None)),
        "as round along z as along x"
    );
    assert_eq!(
        down(&sim, 20.0, 0.0),
        Some((38.0, Some(sphere))),
        "the top at 2 m"
    );
    assert_eq!(down(&sim, 20.0, 1.1), Some((40.0, None)));
    // It blocks as it is hit: a character walking in from the east stops 1.5 m + its radius
    // from the axis.
    sim.set_input(walker, walk(90.0)).unwrap();
    sim.step(120);
    let p = pos(&sim, walker);
    assert!(close(p[0], 1.8) && close(p[2], 0.0), "{p:?}");
}
