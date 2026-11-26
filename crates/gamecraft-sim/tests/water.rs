//! Water in the simulation: a character wades, slower by the depth, at the documented numbers;
//! deep water is refused; a character in deep water gets out; a world without water is
//! unchanged; hostile outlines never panic.

// Integration tests are compiled as their own crates, outside `cfg(test)`, so clippy.toml's
// test allowances do not reach their helper functions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, flat_world, pos, settle, walk};
use gamecraft_sim::{
    DT, Input, Sim, WADE_FREE_DEPTH_M, WADE_MAX_DEPTH_M, WADE_SLOWEST_SHARE, wade_share,
};
use gamecraft_world::{Component, EntityDraft, EntityId, Water, World};

/// A flat world with a pond: water `level` high (so `level` deep over the flat ground at 0)
/// from x = 2 to x = 12, z from -20 to 20. Its west edge is at x = 2.
fn pond(level: f64) -> World {
    let mut world = flat_world(0.0);
    world
        .add_entity(EntityDraft {
            name: "Pond".into(),
            position: [7.0, level, 0.0],
            components: vec![Component::Water(Water::new(
                0.0,
                vec![[-5.0, -20.0], [5.0, -20.0], [5.0, 20.0], [-5.0, 20.0]],
            ))],
            ..EntityDraft::default()
        })
        .unwrap();
    world
}

/// A default character's walking speed, metres per second.
fn speed() -> f64 {
    gamecraft_world::Character::default().walk_speed_mps
}

/// Where a walker is after 10 s walking east from x = -3 through the pond's water at the share
/// `share` of its speed: 188 ticks of dry land (the 189th starts inside the pond), then 412
/// ticks of water.
fn after_ten_seconds(share: f64) -> f64 {
    let dry = (5.0 / (speed() * DT)).ceil();
    let wet = 600.0 - dry;
    -3.0 + dry * speed() * DT + wet * share * speed() * DT
}

/// Walks east (yaw -90) from x = -3 for `ticks` and returns where the character is.
fn walk_east(world: World, ticks: u32) -> (Sim, EntityId) {
    let mut world = world;
    let id = add_character(&mut world, [-3.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(id, walk(-90.0)).unwrap();
    sim.step(ticks);
    (sim, id)
}

#[test]
fn the_documented_depths_slow_a_walker_as_written() {
    let ticks = 600;
    // 10 s: 5 m of dry land from x = -3 to the pond's edge at x = 2, then water.
    for (level, share) in [
        (0.0, 1.0),
        (WADE_FREE_DEPTH_M, 1.0),
        (1.0, 0.7),
        (1.4, 0.46),
    ] {
        assert!(
            (wade_share(Some(level)) - share).abs() < 1e-12,
            "{level}: {share}"
        );
    }
    // Knee-deep: as fast as on dry land (16 m in 10 s, past the pond's far end at x = 12).
    let (sim, id) = walk_east(pond(WADE_FREE_DEPTH_M), ticks);
    let [x, y, z] = pos(&sim, id);
    assert!((x - 13.0).abs() < 1e-9, "x {x}");
    assert!(y.abs() < 1e-9 && z.abs() < 1e-9);
    // Waist-deep (1.0 m): 0.7 of the speed in the water.
    let (sim, id) = walk_east(pond(1.0), ticks);
    let [x, _, _] = pos(&sim, id);
    let expected = after_ten_seconds(0.7);
    assert!((x - expected).abs() < 1e-9, "x {x}, expected {expected}");
    assert_eq!(sim.water_depth(id), Some(1.0));
    // 1.4 m: 0.46 of the speed.
    let (sim, id) = walk_east(pond(1.4), ticks);
    let [x, _, _] = pos(&sim, id);
    let expected = after_ten_seconds(wade_share(Some(1.4)));
    assert!((x - expected).abs() < 1e-9, "x {x}, expected {expected}");
}

#[test]
fn deep_water_is_refused_and_a_character_in_it_gets_out() {
    // 2 m deep: the walker stops at the pond's edge and stays there.
    let (sim, id) = walk_east(pond(2.0), 600);
    let [x, _, _] = pos(&sim, id);
    assert!(x < 2.0 && x > 1.9, "x {x}");
    assert_eq!(sim.water_depth(id), None, "still on dry land");
    // Exactly at the limit the water is wadeable (the slowest share).
    let (sim, id) = walk_east(pond(WADE_MAX_DEPTH_M), 600);
    let [x, _, _] = pos(&sim, id);
    let expected = after_ten_seconds(WADE_SLOWEST_SHARE);
    assert!((x - expected).abs() < 1e-9, "x {x}, expected {expected}");
    // A character dropped in the middle of the deep pond moves at the slowest share and
    // walks out onto dry land, where it is not held back any more.
    let mut world = pond(2.0);
    let id = add_character(&mut world, [7.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(id, walk(90.0)).unwrap();
    assert_eq!(sim.water_depth(id), Some(2.0));
    sim.step(150);
    // 2.5 s at 0.4 x 1.6 m/s = 1.6 m west: still in the water, wading.
    let [x, _, _] = pos(&sim, id);
    assert!((x - (7.0 - 0.4 * 1.6 * 2.5)).abs() < 1e-9, "x {x}");
    sim.step(600);
    let [x, y, _] = pos(&sim, id);
    // Then 3.4 m of water at 0.64 m/s and the rest (4.69 s) on dry land at full speed:
    // about 7.5 m past the edge.
    assert!((x + 5.5).abs() < 0.05, "x {x}");
    assert!(x < 2.0, "out of the water: x {x}");
    assert!(y.abs() < 1e-9);
    assert_eq!(sim.water_depth(id), None);
    // Back east it is refused at the edge again.
    sim.set_input(id, walk(-90.0)).unwrap();
    sim.step(600);
    let [x, _, _] = pos(&sim, id);
    assert!(x < 2.0 && x > 1.9, "x {x}");
}

#[test]
fn water_everywhere_and_the_highest_surface_count() {
    // The sea at y = 0.3 over flat ground: 0.3 m deep everywhere, wadeable at full speed...
    let mut world = flat_world(0.0);
    world
        .add_entity(EntityDraft {
            name: "Sea".into(),
            position: [0.0, 0.3, 0.0],
            components: vec![Component::Water(Water::default())],
            ..EntityDraft::default()
        })
        .unwrap();
    let (sim, id) = walk_east(world, 600);
    let [x, _, _] = pos(&sim, id);
    assert!((x - 13.0).abs() < 1e-9, "x {x}");
    assert_eq!(sim.water_depth(id), Some(0.3));
    // ... and a deep pond inside it is refused.
    let mut world = flat_world(0.0);
    world
        .add_entity(EntityDraft {
            name: "Sea".into(),
            position: [0.0, 0.3, 0.0],
            components: vec![Component::Water(Water::default())],
            ..EntityDraft::default()
        })
        .unwrap();
    world
        .add_entity(EntityDraft {
            name: "Pit".into(),
            position: [7.0, 0.3, 0.0],
            components: vec![Component::Water(Water::new(
                2.0,
                vec![[-5.0, -20.0], [5.0, -20.0], [5.0, 20.0], [-5.0, 20.0]],
            ))],
            ..EntityDraft::default()
        })
        .unwrap();
    let (sim, id) = walk_east(world, 600);
    let [x, _, _] = pos(&sim, id);
    assert!(x < 2.0 && x > 1.9, "x {x}");
    assert_eq!(sim.water_depth(id), Some(0.3));
    assert!(sim.has_water());
}

#[test]
fn a_world_without_water_moves_as_before_to_the_bit() {
    // The same walk with and without a pond the walker never enters: every position and the
    // state hash are the same, tick by tick.
    let mut dry = flat_world(0.0);
    let a = add_character(&mut dry, [-3.0, 0.0, 0.0]);
    let mut wet = dry.clone();
    wet.add_entity(EntityDraft {
        name: "Far pond".into(),
        position: [100.0, 1.0, 100.0],
        components: vec![Component::Water(Water::new(
            0.0,
            vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]],
        ))],
        ..EntityDraft::default()
    })
    .unwrap();
    let mut sims = [Sim::new(dry).unwrap(), Sim::new(wet).unwrap()];
    assert!(!sims[0].has_water() && sims[1].has_water());
    for sim in &mut sims {
        sim.set_input(
            a,
            Input {
                forward: 1.0,
                right: 0.3,
                yaw_deg: 20.0,
                jump: true,
                ..Input::default()
            },
        )
        .unwrap();
    }
    for _ in 0..120 {
        for sim in &mut sims {
            sim.step(1);
        }
        assert_eq!(
            pos(&sims[0], a),
            pos(&sims[1], a),
            "tick by tick, to the bit"
        );
        assert_eq!(sims[0].body(a), sims[1].body(a));
    }
    assert_eq!(sims[0].water_depth(a), None);
    // An untouched walker settles the same on dry land.
    let mut world = pond(1.0);
    let far = add_character(&mut world, [-60.0, 5.0, 40.0]);
    let mut sim = Sim::new(world).unwrap();
    settle(&mut sim);
    sim.step(180);
    let [_, y, _] = pos(&sim, far);
    assert!(y.abs() < 1e-9 && sim.water_depth(far).is_none(), "y {y}");
}

#[test]
fn hostile_outlines_and_depths_never_panic() {
    let world = flat_world(0.0);
    // A self-intersecting outline, huge coordinates and a bow-tie: covered or not, never a panic.
    for outline in [
        vec![[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]],
        vec![[-1e6, -1e6], [1e6, 1e6], [1e6, -1e6], [-1e6, 1e6]],
        vec![[0.0, 0.0], [0.01, 0.0], [0.0, 0.01]],
    ] {
        let mut w = world.clone();
        w.add_entity(EntityDraft {
            name: "Odd".into(),
            position: [0.0, 1e6, 0.0],
            components: vec![Component::Water(Water::new(0.0, outline))],
            ..EntityDraft::default()
        })
        .unwrap();
        let id = add_character(&mut w, [0.0, 0.0, 0.0]);
        let mut sim = Sim::new(w).unwrap();
        sim.set_input(
            id,
            Input {
                forward: 1.0,
                right: 1.0,
                yaw_deg: 1e300,
                ..Input::default()
            },
        )
        .unwrap();
        sim.step(30);
        let [x, y, z] = pos(&sim, id);
        assert!(x.is_finite() && y.is_finite() && z.is_finite());
        let _ = sim.water_depth(id);
    }
    // A character at the coordinate limit with a surface beyond it.
    let mut w = flat_world(0.0);
    w.add_entity(EntityDraft {
        name: "High".into(),
        position: [0.0, 1e6, 0.0],
        components: vec![Component::Water(Water::new(1.0, Vec::new()))],
        ..EntityDraft::default()
    })
    .unwrap();
    let sim = Sim::new(w).unwrap();
    assert!(!sim.has_water(), "a surface beyond the limit is no water");
    assert_eq!(sim.water_depth(EntityId(1)), None);
}
