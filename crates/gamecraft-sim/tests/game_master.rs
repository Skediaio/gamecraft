//! The game master's hand: `replace_world` between two ticks.

// Integration tests are compiled as their own crates, outside `cfg(test)`, so clippy.toml's
// test allowances do not reach their helper functions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, add_shape, flat_world, pos, settle, walk};
use gamecraft_sim::{Input, Sim, SimError};
use gamecraft_world::{ShapeKind, Terrain, TerrainSource};

#[test]
fn an_added_character_gets_a_body_at_rest_and_others_keep_their_motion() {
    let mut world = flat_world(0.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    settle(&mut sim);
    sim.set_input(
        a,
        Input {
            jump: true,
            ..walk(0.0)
        },
    )
    .unwrap();
    sim.step(5);
    let before = *sim.body(a).unwrap();
    assert!(!before.grounded && before.velocity[1] > 0.0, "mid-jump");

    let mut next = sim.world().clone();
    let b = add_character(&mut next, [5.0, 0.0, 5.0]);
    sim.replace_world(next).unwrap();
    assert_eq!(*sim.body(a).unwrap(), before);
    assert!(sim.input(a).unwrap().jump);
    let body_b = sim.body(b).unwrap();
    assert_eq!(body_b.velocity, [0.0, 0.0, 0.0]);
    assert_eq!(sim.tick(), 6);
}

#[test]
fn a_removed_character_loses_body_and_input() {
    let mut world = flat_world(0.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(a, walk(0.0)).unwrap();
    sim.step(3);
    let mut next = sim.world().clone();
    next.remove_entity(a);
    sim.replace_world(next).unwrap();
    assert!(sim.body(a).is_none());
    assert!(sim.input(a).is_none());
    assert_eq!(sim.bodies().count(), 0);
    sim.step(3);
}

#[test]
fn a_moved_character_uses_the_new_position() {
    let mut world = flat_world(0.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    settle(&mut sim);
    let mut next = sim.world().clone();
    next.entity_mut(a).unwrap().position = [10.0, 0.0, -7.0];
    sim.replace_world(next).unwrap();
    assert_eq!(pos(&sim, a), [10.0, 0.0, -7.0]);
    sim.step(10);
    assert_eq!(pos(&sim, a), [10.0, 0.0, -7.0]);
}

#[test]
fn a_new_wall_blocks_from_the_next_tick() {
    let mut world = flat_world(0.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(a, walk(0.0)).unwrap();
    sim.step(60);
    let z_now = pos(&sim, a)[2];
    let mut next = sim.world().clone();
    // A wall whose near face is 0.5 m ahead of the character's circle.
    add_shape(
        &mut next,
        ShapeKind::Box,
        [4.0, 3.0, 1.0],
        [0.0, 0.0, z_now - 0.3 - 0.5 - 0.5],
        0.0,
        true,
    );
    sim.replace_world(next).unwrap();
    sim.step(600);
    let z = pos(&sim, a)[2];
    assert!(
        (z - (z_now - 0.5)).abs() < 1e-9,
        "stopped at the wall: {z} vs {}",
        z_now - 0.5
    );
}

#[test]
fn a_changed_terrain_changes_the_ground() {
    let mut world = flat_world(0.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    settle(&mut sim);
    let mut next = sim.world().clone();
    next.terrain = Some(Terrain {
        size_m: 256.0,
        cell_m: 4.0,
        source: TerrainSource::Flat { height_m: 2.0 },
        ..Default::default()
    });
    sim.replace_world(next).unwrap();
    assert_eq!(sim.heights().unwrap().height_at(0.0, 0.0), Some(2.0));
    sim.step(1);
    assert_eq!(pos(&sim, a)[1], 2.0);

    // Lowered: it falls to the new ground.
    let mut next = sim.world().clone();
    next.terrain = Some(Terrain {
        size_m: 256.0,
        cell_m: 4.0,
        source: TerrainSource::Flat { height_m: -3.0 },
        ..Default::default()
    });
    sim.replace_world(next).unwrap();
    sim.step(1);
    assert!(!sim.body(a).unwrap().grounded);
    sim.step(300);
    assert_eq!(pos(&sim, a)[1], -3.0);
}

#[test]
fn an_invalid_world_is_refused_and_nothing_changes() {
    let mut world = flat_world(0.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(a, walk(0.0)).unwrap();
    sim.step(10);
    let hash = sim.state_hash();
    let before = sim.world().clone();
    let mut bad = sim.world().clone();
    bad.entity_mut(a).unwrap().position = [f64::NAN, 0.0, 0.0];
    let err = sim.replace_world(bad).unwrap_err();
    assert!(matches!(err, SimError::World(_)), "{err:?}");
    assert!(!err.to_string().is_empty());
    assert_eq!(sim.state_hash(), hash);
    assert_eq!(sim.world(), &before);
}

#[test]
fn replacing_with_the_same_world_keeps_the_hash() {
    let mut world = flat_world(0.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(a, walk(0.0)).unwrap();
    sim.step(10);
    let hash = sim.state_hash();
    sim.replace_world(sim.world().clone()).unwrap();
    assert_eq!(sim.state_hash(), hash);
}

#[test]
fn a_changed_static_world_changes_the_hash() {
    let mut world = flat_world(0.0);
    add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    let hash = sim.state_hash();
    let mut next = sim.world().clone();
    next.environment.time_of_day_h = 3.0;
    sim.replace_world(next).unwrap();
    assert_ne!(sim.state_hash(), hash);
}
