//! The state hash is the oracle: same history, same hash.

// Integration tests are compiled as their own crates, outside `cfg(test)`, so clippy.toml's
// test allowances do not reach their helper functions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, add_shape, noise_world, walk};
use gamecraft_math::Rng;
use gamecraft_sim::{Input, Sim};
use gamecraft_world::{EntityId, ShapeKind, World};

/// A fixed scenario: noise terrain, two characters, a wall, a cylinder and a step.
fn scenario() -> (World, EntityId, EntityId) {
    let mut world = noise_world(11, 30.0, 60.0);
    add_shape(
        &mut world,
        ShapeKind::Box,
        [4.0, 3.0, 1.0],
        [0.0, 0.0, -6.0],
        15.0,
        true,
    );
    add_shape(
        &mut world,
        ShapeKind::Cylinder,
        [1.0, 2.0, 1.0],
        [3.0, 0.0, -2.0],
        0.0,
        true,
    );
    add_shape(
        &mut world,
        ShapeKind::Sphere,
        [1.5, 1.5, 1.5],
        [-3.0, 0.0, -3.0],
        0.0,
        true,
    );
    let a = add_character(&mut world, [0.0, 20.0, 0.0]);
    let b = add_character(&mut world, [2.0, 20.0, 2.0]);
    (world, a, b)
}

/// The scripted inputs of tick `tick` (changing every 20 ticks), from a seeded stream.
fn script(rng: &mut Rng) -> Input {
    Input {
        forward: rng.range(-1.0, 1.0),
        right: rng.range(-1.0, 1.0),
        yaw_deg: rng.range(0.0, 360.0),
        run: rng.below(2) == 0,
        jump: rng.below(5) == 0,
        interact: false,
        pitch_deg: 0.0,
        fire: false,
        face: None,
    }
}

fn run_scripted(sim: &mut Sim, a: EntityId, b: EntityId, ticks: u32, mut each: impl FnMut(&Sim)) {
    let mut rng = Rng::new(99);
    for tick in 0..ticks {
        if tick % 20 == 0 {
            sim.set_input(a, script(&mut rng)).unwrap();
            sim.set_input(b, script(&mut rng)).unwrap();
        }
        sim.step(1);
        each(sim);
    }
}

#[test]
fn same_world_same_inputs_same_hash_every_tick() {
    let (world, a, b) = scenario();
    let mut one = Sim::new(world.clone()).unwrap();
    let mut two = Sim::new(world).unwrap();
    assert_eq!(one.state_hash(), two.state_hash());
    let mut hashes = Vec::new();
    run_scripted(&mut one, a, b, 1200, |s| hashes.push(s.state_hash()));
    let mut i = 0;
    run_scripted(&mut two, a, b, 1200, |s| {
        assert_eq!(s.state_hash(), hashes[i], "tick {i}");
        i += 1;
    });
}

#[test]
fn a_different_input_gives_a_different_hash() {
    let (world, a, _) = scenario();
    let mut one = Sim::new(world.clone()).unwrap();
    let mut two = Sim::new(world).unwrap();
    one.set_input(a, walk(0.0)).unwrap();
    two.set_input(a, walk(0.5)).unwrap();
    assert_ne!(
        one.state_hash(),
        two.state_hash(),
        "the input is part of the hash"
    );
    one.step(10);
    two.step(10);
    assert_ne!(one.state_hash(), two.state_hash());
}

#[test]
fn the_hash_moves_with_the_tick() {
    let (world, _, _) = scenario();
    let mut sim = Sim::new(world).unwrap();
    let h0 = sim.state_hash();
    sim.step(1);
    assert_ne!(sim.state_hash(), h0);
}

#[test]
fn many_ticks_in_one_call_equal_one_tick_per_call() {
    let (world, a, b) = scenario();
    let mut one = Sim::new(world.clone()).unwrap();
    let mut two = Sim::new(world).unwrap();
    for sim in [&mut one, &mut two] {
        sim.set_input(a, walk(10.0)).unwrap();
        sim.set_input(
            b,
            Input {
                right: 1.0,
                run: true,
                jump: true,
                yaw_deg: 200.0,
                ..Input::default()
            },
        )
        .unwrap();
    }
    one.step(600);
    for _ in 0..600 {
        two.step(1);
    }
    assert_eq!(one.tick(), 600);
    assert_eq!(one.state_hash(), two.state_hash());
    assert_eq!(one.world(), two.world());
}

/// The hash of the fixed scenario after 600 scripted ticks. It must be the same on every
/// platform, natively and in wasm: a change here means the simulation's results changed.
const GOLDEN_HASH: u64 = 0x8244_A854_0D07_9D88;

#[test]
fn golden_hash_of_a_fixed_scenario() {
    let (world, a, b) = scenario();
    let mut sim = Sim::new(world).unwrap();
    run_scripted(&mut sim, a, b, 600, |_| {});
    assert_eq!(
        sim.state_hash(),
        GOLDEN_HASH,
        "golden hash changed: {:#018x}",
        sim.state_hash()
    );
}
