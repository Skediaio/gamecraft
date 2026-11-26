//! Collision from a model's own geometry (`collider: "mesh"`): the host hands over the asset's
//! parts (one box per mesh, or per `*_collider` node of the file) and a character walks through
//! a doorway of a building model and stops at its walls; a ray passes through the opening; a
//! model with the `box` collider, or one part that is the box, behaves as it did, with the same
//! hashes; parts are placed by the entity's yaw and scale; a part outside the visual bounds is
//! found through the spatial index.

// Integration tests are compiled as their own crates, outside `cfg(test)`, so clippy.toml's
// test allowances do not reach their helper functions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, flat_world, pos, walk};
use gamecraft_sim::{Sim, ray};
use gamecraft_world::{Component, EntityDraft, Model, ModelBounds, ModelBox, ModelCollider, World};
use std::collections::BTreeMap;

const HOUSE: &str = "models/house.glb";

/// The facade's three meshes as local boxes (the test of the project crate reads the same
/// ones from a glTF file).
fn facade_parts() -> Vec<ModelBox> {
    vec![
        ModelBox {
            center: [-2.5, 2.0, 0.0],
            half: [1.5, 2.0, 0.15],
            yaw_deg: 0.0,
        },
        ModelBox {
            center: [2.5, 2.0, 0.0],
            half: [1.5, 2.0, 0.15],
            yaw_deg: 0.0,
        },
        ModelBox {
            center: [0.0, 3.25, 0.0],
            half: [1.0, 0.75, 0.15],
            yaw_deg: 0.0,
        },
    ]
}

fn bounds() -> ModelBounds {
    ModelBounds {
        min: [-4.0, 0.0, -0.15],
        max: [4.0, 4.0, 0.15],
    }
}

/// A flat world with a solid house model of `collider`, its parts handed over when `parts` is
/// `Some`; the entity turned by `yaw` and scaled by `scale`.
fn house(
    collider: ModelCollider,
    parts: Option<Vec<ModelBox>>,
    yaw: f64,
    scale: [f64; 3],
) -> World {
    let mut world = flat_world(0.0);
    world.set_model_bounds(BTreeMap::from([(HOUSE.to_string(), bounds())]));
    if let Some(parts) = parts {
        world.set_model_parts(BTreeMap::from([(HOUSE.to_string(), parts)]));
    }
    world
        .add_entity(EntityDraft {
            name: "House".into(),
            position: [0.0, 0.0, 0.0],
            rotation_y_deg: yaw,
            scale: gamecraft_world::Scale::per_axis(scale[0], scale[1], scale[2]),
            components: vec![Component::Model(Model {
                solid: true,
                collider,
                ..Model::of(HOUSE)
            })],
            ..EntityDraft::default()
        })
        .unwrap();
    world
}

fn walker(world: &mut World, at: [f64; 3]) -> gamecraft_world::EntityId {
    add_character(world, at)
}

fn walk_north(world: World, id: gamecraft_world::EntityId, ticks: u32) -> Sim {
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(id, walk(0.0)).unwrap();
    sim.step(ticks);
    sim
}

#[test]
fn a_walker_passes_through_the_doorway_and_stops_at_the_walls() {
    let mut world = house(ModelCollider::Mesh, Some(facade_parts()), 0.0, [1.0; 3]);
    let through = walker(&mut world, [0.0, 0.0, 6.0]);
    let wall = walker(&mut world, [2.5, 0.0, 6.0]);
    let jamb = walker(&mut world, [-1.25, 0.0, 6.0]);
    let sim = walk_north(world, through, 600);
    // 10 s at 1.6 m/s from z = 6: through the 2 m doorway and 10 m beyond it.
    assert!(
        (pos(&sim, through)[2] + 10.0).abs() < 1e-9,
        "{:?}",
        pos(&sim, through)
    );
    assert_eq!(pos(&sim, through)[1], 0.0);
    // The walls: the walker stops at the facade's south face, its radius away.
    let sim = walk_north(sim.into_world(), wall, 600);
    assert!(
        (pos(&sim, wall)[2] - 0.45).abs() < 1e-9,
        "{:?}",
        pos(&sim, wall)
    );
    // The doorway is 2 m wide: a walker 1.25 m from the middle is inside the wall's reach.
    let sim = walk_north(sim.into_world(), jamb, 600);
    assert!(
        (pos(&sim, jamb)[2] - 0.45).abs() < 1e-9,
        "{:?}",
        pos(&sim, jamb)
    );
}

#[test]
fn a_ray_passes_through_the_opening_and_hits_the_walls() {
    let world = house(ModelCollider::Mesh, Some(facade_parts()), 0.0, [1.0; 3]);
    let sim = Sim::new(world).unwrap();
    let cast = |origin: [f64; 3], direction: [f64; 3]| {
        ray::cast_in(
            sim.world(),
            sim.heights(),
            Some(sim.index()),
            origin,
            direction,
            12.0,
            |_| false,
        )
    };
    assert_eq!(
        cast([0.0, 1.0, 6.0], [0.0, 0.0, -1.0]),
        None,
        "through the doorway"
    );
    let hit = cast([0.0, 3.5, 6.0], [0.0, 0.0, -1.0]).expect("the lintel");
    assert!((hit.hit.distance_m - 5.85).abs() < 1e-9, "{hit:?}");
    let hit = cast([2.5, 1.0, 6.0], [0.0, 0.0, -1.0]).unwrap();
    assert!((hit.hit.distance_m - 5.85).abs() < 1e-9, "{hit:?}");
    assert_eq!(hit.entity, Some(gamecraft_world::EntityId(1)));
    let hit = cast([0.0, 3.0, 6.0], [0.0, 0.0, -1.0]).unwrap();
    assert!(
        (hit.hit.distance_m - 5.85).abs() < 1e-9,
        "the lintel: {hit:?}"
    );
    // A line of sight through the doorway is clear.
    let sight = ray::line_of_sight_in(
        sim.world(),
        sim.heights(),
        Some(sim.index()),
        [0.0, 1.0, 6.0],
        [0.0, 1.0, -6.0],
        |_| false,
    )
    .unwrap();
    assert!(sight.blocked_by.is_none());
    let sight = ray::line_of_sight_in(
        sim.world(),
        sim.heights(),
        Some(sim.index()),
        [3.0, 1.0, 6.0],
        [3.0, 1.0, -6.0],
        |_| false,
    )
    .unwrap();
    assert!(sight.blocked_by.is_some());
}

#[test]
fn the_box_collider_is_unchanged_and_one_part_moves_as_the_box() {
    // The `box` collider: one solid, the bounding box. Its hashes do not depend on the parts
    // the host hands over for the asset (they are the `mesh` collider's).
    let run = |collider: ModelCollider, parts: Option<Vec<ModelBox>>| {
        let mut world = house(collider, parts, 0.0, [1.0; 3]);
        let a = walker(&mut world, [0.0, 0.0, 6.0]);
        let b = walker(&mut world, [2.5, 0.0, 6.0]);
        let mut sim = Sim::new(world).unwrap();
        for id in [a, b] {
            sim.set_input(id, walk(0.0)).unwrap();
        }
        let mut hashes = Vec::new();
        for _ in 0..10 {
            sim.step(60);
            hashes.push(sim.state_hash());
        }
        (hashes, pos(&sim, a), pos(&sim, b))
    };
    let before = run(ModelCollider::Box, None);
    let with_parts = run(ModelCollider::Box, Some(facade_parts()));
    assert_eq!(
        before, with_parts,
        "a `box` collider's hashes ignore the parts"
    );
    assert!(
        (before.1[2] - 0.45).abs() < 1e-9 && (before.2[2] - 0.45).abs() < 1e-9,
        "both stopped at the facade: {:?} {:?}",
        before.1,
        before.2
    );
    // The `mesh` collider with one part that is the bounding box moves as the box does; its
    // hash is another, as its document is (it says `mesh`).
    let one_part = run(
        ModelCollider::Mesh,
        Some(vec![ModelBox {
            center: [0.0, 2.0, 0.0],
            half: [4.0, 2.0, 0.15],
            yaw_deg: 0.0,
        }]),
    );
    assert_eq!((one_part.1, one_part.2), (before.1, before.2));
    assert_ne!(one_part.0, before.0);
    // `mesh` before the host handed the parts over: the bounding box, as before.
    let not_yet = run(ModelCollider::Mesh, None);
    assert_eq!((not_yet.1, not_yet.2), (before.1, before.2));
    // The `none` collider: through.
    let none_collider = run(ModelCollider::None, None);
    assert!((none_collider.1[2] + 10.0).abs() < 1e-9);
}

#[test]
fn parts_follow_the_entity_s_turn_and_scale() {
    // Turned 90 degrees, the doorway faces east and west; scaled twice along the entity's z
    // (now east-west) the facade is 0.6 m thick.
    let world = house(
        ModelCollider::Mesh,
        Some(facade_parts()),
        90.0,
        [1.0, 1.0, 2.0],
    );
    let sim = Sim::new(world).unwrap();
    let cast = |origin: [f64; 3], direction: [f64; 3]| {
        ray::cast_in(
            sim.world(),
            sim.heights(),
            Some(sim.index()),
            origin,
            direction,
            12.0,
            |_| false,
        )
    };
    assert_eq!(
        cast([6.0, 1.0, 0.0], [-1.0, 0.0, 0.0]),
        None,
        "through the doorway"
    );
    let hit = cast([6.0, 1.0, 2.5], [-1.0, 0.0, 0.0]).unwrap();
    assert!(
        (hit.hit.distance_m - 5.7).abs() < 1e-9,
        "0.3 m thick: {hit:?}"
    );
    assert!(
        (hit.hit.normal[0] - 1.0).abs() < 1e-9 && hit.hit.normal[1].abs() < 1e-9,
        "{hit:?}"
    );
}

#[test]
fn a_part_outside_the_visual_bounds_is_met_through_the_index() {
    // Collider nodes may sit outside what is drawn (a fence line around a garden): the index
    // holds the parts' footprint, not only the bounds.
    let far = vec![ModelBox {
        center: [100.5, 2.0, 0.0],
        half: [0.5, 2.0, 0.15],
        yaw_deg: 0.0,
    }];
    let mut world = house(ModelCollider::Mesh, Some(far), 0.0, [1.0; 3]);
    let id = walker(&mut world, [100.5, 0.0, 6.0]);
    let sim = walk_north(world, id, 600);
    assert!(
        (pos(&sim, id)[2] - 0.45).abs() < 1e-9,
        "{:?}",
        pos(&sim, id)
    );
}
