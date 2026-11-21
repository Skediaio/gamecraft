//! Tree trunks: derived collision columns that block characters, rays, projectiles, sight
//! and give cover, with a grid so that thousands stay cheap, and that leave the hash of a
//! world without trees as it was.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use std::sync::Arc;

use common::{add_character, flat_world, pos, settle, walk};
use gamecraft_sim::cover::{CoverAsk, CoverSource, Threat};
use gamecraft_sim::ray;
use gamecraft_sim::{Event, EventKind, Sim, Trunk, TrunkSource, Trunks};
use gamecraft_world::{
    Character, Component, EntityDraft, EntityId, Health, Launcher, Plant, World,
};

const TOL: f64 = 1e-9;
const TREE_RADIUS_M: f64 = 0.4;
const TREE_HEIGHT_M: f64 = 5.0;

/// A source that answers with the same trunks for any world: the engine's plant library is not
/// part of the simulation, so the tests put the trunks in themselves.
#[derive(Debug)]
struct Fixed(Arc<Trunks>);

impl TrunkSource for Fixed {
    fn trunks_of(&self, _world: &World) -> Arc<Trunks> {
        self.0.clone()
    }
}

/// A plant entity with health at `[x, 0, z]`: what a hit on its trunk must not hurt.
fn add_tree(world: &mut World, x: f64, z: f64) -> EntityId {
    world
        .add_entity(EntityDraft {
            name: "oak".into(),
            position: [x, 0.0, z],
            components: vec![
                Component::Plant(Plant {
                    species: "quercus-robur".into(),
                    seed: 1,
                    growth: 1.0,
                }),
                Component::Health(Health {
                    max: 100.0,
                    current: 100.0,
                    armor: 0.0,
                }),
            ],
            ..EntityDraft::default()
        })
        .unwrap()
}

fn trunk_of(id: EntityId, x: f64, z: f64) -> Trunk {
    Trunk {
        entity: id,
        position: [x, 0.0, z],
        radius_m: TREE_RADIUS_M,
        height_m: TREE_HEIGHT_M,
    }
}

/// A simulation of `world` whose trunks are `trunks`.
fn sim_with(world: World, trunks: Vec<Trunk>) -> Sim {
    let mut sim = Sim::new(world).unwrap();
    sim.set_trunk_source(Arc::new(Fixed(Arc::new(Trunks::new(trunks)))));
    sim
}

fn rifle() -> Launcher {
    Launcher {
        muzzle_velocity_mps: 800.0,
        mass_kg: 0.0109,
        ballistic_coefficient: 200.0,
        dispersion_mrad: 0.0,
        rate_s: 0.5,
        magazine: 5,
        reload_s: 2.0,
        damage_per_joule: 0.05,
        muzzle_height_m: 1.5,
        max_flight_s: 60.0,
        sound_range_m: 400.0,
        ..Launcher::default()
    }
}

fn distance(p: [f64; 3], x: f64, z: f64) -> f64 {
    let (dx, dz) = (p[0] - x, p[2] - z);
    (dx * dx + dz * dz).sqrt()
}

#[test]
fn a_character_walking_into_a_trunk_stops_at_its_surface() {
    let mut world = flat_world(0.0);
    let tree = add_tree(&mut world, 0.0, -4.0);
    let me = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = sim_with(world, vec![trunk_of(tree, 0.0, -4.0)]);
    let radius = Character::default().radius_m;
    sim.set_input(me, walk(0.0)).unwrap();
    for tick in 0..600 {
        sim.step(1);
        let d = distance(pos(&sim, me), 0.0, -4.0);
        assert!(d >= TREE_RADIUS_M + radius - TOL, "tick {tick}: {d}");
    }
    let end = pos(&sim, me);
    assert!(
        (end[2] - (-4.0 + TREE_RADIUS_M + radius)).abs() < 1e-6,
        "{end:?}"
    );
}

#[test]
fn a_character_slides_round_a_trunk_it_meets_off_centre() {
    let mut world = flat_world(0.0);
    let tree = add_tree(&mut world, 0.0, -4.0);
    let me = add_character(&mut world, [0.15, 0.0, 0.0]);
    let mut sim = sim_with(world, vec![trunk_of(tree, 0.0, -4.0)]);
    let radius = Character::default().radius_m;
    sim.set_input(me, walk(0.0)).unwrap();
    for tick in 0..900 {
        sim.step(1);
        let d = distance(pos(&sim, me), 0.0, -4.0);
        assert!(d >= TREE_RADIUS_M + radius - TOL, "tick {tick}: {d}");
    }
    assert!(pos(&sim, me)[2] < -5.0, "it got past: {:?}", pos(&sim, me));
}

#[test]
fn without_the_trunk_the_character_walks_through() {
    let mut world = flat_world(0.0);
    add_tree(&mut world, 0.0, -4.0);
    let me = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(me, walk(0.0)).unwrap();
    sim.step(600);
    assert!(pos(&sim, me)[2] < -5.0);
}

#[test]
fn a_bullet_stops_at_the_surface_of_a_trunk_and_hurts_nothing() {
    let mut world = flat_world(0.0);
    let tree = add_tree(&mut world, 0.0, -20.0);
    let gun = world
        .add_entity(EntityDraft {
            name: "gun".into(),
            components: vec![Component::Launcher(rifle())],
            ..EntityDraft::default()
        })
        .unwrap();
    let mut sim = sim_with(world, vec![trunk_of(tree, 0.0, -20.0)]);
    sim.fire(gun, Some(0.0), Some(0.0)).unwrap();
    let mut hit = None;
    for _ in 0..60 {
        sim.step(1);
        let events: Vec<Event> = sim.take_events();
        hit = events.into_iter().find_map(|e| match e.kind {
            EventKind::Hit { target, point, .. } => Some((target, point)),
            _ => None,
        });
        if hit.is_some() {
            break;
        }
    }
    let (target, point) = hit.expect("the shot hit the trunk");
    assert_eq!(target, Some(tree));
    assert!(
        (point[2] - (-20.0 + TREE_RADIUS_M)).abs() < 0.02,
        "stopped at the near surface: {point:?}"
    );
    assert!(sim.projectiles().is_empty(), "the bullet ended");
    let health = sim.world().entity(tree).unwrap().health().unwrap();
    assert_eq!(health.current, 100.0, "no damage to the tree");
}

#[test]
fn a_shot_above_the_trunk_passes() {
    let mut world = flat_world(0.0);
    let tree = add_tree(&mut world, 0.0, -20.0);
    let gun = world
        .add_entity(EntityDraft {
            name: "gun".into(),
            components: vec![Component::Launcher(rifle())],
            ..EntityDraft::default()
        })
        .unwrap();
    let mut short = trunk_of(tree, 0.0, -20.0);
    short.height_m = 1.0;
    let mut sim = sim_with(world, vec![short]);
    sim.fire(gun, Some(0.0), Some(0.0)).unwrap();
    sim.step(3);
    let hit = sim
        .take_events()
        .iter()
        .any(|e| matches!(e.kind, EventKind::Hit { .. }));
    assert!(!hit, "the muzzle is above a trunk of 1 m");
}

#[test]
fn a_ray_hits_a_trunk_and_skip_ignores_it() {
    let mut world = flat_world(0.0);
    let tree = add_tree(&mut world, 0.0, -10.0);
    let trunks = Trunks::new([trunk_of(tree, 0.0, -10.0)]);
    let field = world.terrain.as_ref().unwrap().build().unwrap();
    let from = [0.0, 1.5, 0.0];
    let dir = [0.0, 0.0, -1.0];
    let found = ray::cast(&world, Some(&field), &trunks, from, dir, 100.0, |_| false).unwrap();
    assert!(found.trunk && found.entity == Some(tree), "{found:?}");
    assert!((found.hit.distance_m - (10.0 - TREE_RADIUS_M)).abs() < TOL);
    assert!((found.hit.normal[2] - 1.0).abs() < TOL);
    let skipped = ray::cast(&world, Some(&field), &trunks, from, dir, 100.0, |id| {
        id == tree
    });
    assert!(
        skipped.is_none(),
        "a level ray over flat ground: {skipped:?}"
    );
    let sight =
        ray::line_of_sight(&world, None, &trunks, from, [0.0, 1.5, -20.0], |_| false).unwrap();
    assert!(sight.blocked_by.is_some_and(|f| f.trunk));
    let beside = ray::line_of_sight(
        &world,
        None,
        &trunks,
        [2.0, 1.5, 0.0],
        [2.0, 1.5, -20.0],
        |_| false,
    )
    .unwrap();
    assert!(beside.blocked_by.is_none());
}

#[test]
fn a_trunk_hides_a_character_from_sight() {
    let mut world = flat_world(0.0);
    let tree = add_tree(&mut world, 0.0, -10.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let b = add_character(&mut world, [0.0, 0.0, -20.0]);
    let mut open = Sim::new(world.clone()).unwrap();
    settle(&mut open);
    assert!(open.sees(a, b), "nothing between them");
    let mut hidden = sim_with(world, vec![trunk_of(tree, 0.0, -10.0)]);
    settle(&mut hidden);
    assert!(!hidden.sees(a, b), "the trunk is between them");
}

#[test]
fn the_far_side_of_a_trunk_is_cover() {
    let mut world = flat_world(0.0);
    let tree = add_tree(&mut world, 0.0, -2.0);
    let me = add_character(&mut world, [0.0, 0.0, 0.0]);
    let sim = sim_with(world, vec![trunk_of(tree, 0.0, -2.0)]);
    let threats = [Threat {
        point: [0.0, 1.4, 30.0],
        entity: None,
    }];
    let ask = CoverAsk {
        me: Some(me),
        feet: [0.0, 0.0, 0.0],
        height_m: 1.8,
        radius_m: 0.3,
        eye_height_m: 1.65,
        search_m: 8.0,
        threats: &threats,
        friends: &[],
    };
    let found = sim.cover(&ask);
    let best = found[0];
    assert_eq!(best.protection, 1.0, "{best:?}");
    assert_eq!(best.source, CoverSource::Solid, "{best:?}");
    assert!(best.point[2] < -2.0, "behind the trunk: {best:?}");
}

/// A grove of `n` by `n` trunks 5 m apart.
fn grove(n: u64) -> Vec<Trunk> {
    (0..n * n)
        .map(|i| trunk_of(EntityId(i + 1), (i % n) as f64 * 5.0, (i / n) as f64 * 5.0))
        .collect()
}

#[test]
fn the_grid_keeps_the_work_small_for_thousands_of_trunks() {
    let trunks = Trunks::new(grove(100));
    assert_eq!(trunks.len(), 10_000);
    // A character looks at a few trunks, not at ten thousand.
    let near = trunks.candidates_near(250.2, 250.2, 0.3);
    assert!((1..=16).contains(&near), "{near} candidates of 10000");
    // A ray along a row of trunks tests the cells it crosses: 100 m is some 13 cells.
    let hit = trunks
        .cast([-50.0, 1.0, 250.0], [1.0, 0.0, 0.0], 500.0, |_| false)
        .unwrap();
    assert_eq!(hit.0, EntityId(50 * 100 + 1), "the first trunk of row 50");
    assert!((hit.1.distance_m - (50.0 - TREE_RADIUS_M)).abs() < TOL);
    // The whole walk of a character through the grove keeps every step cheap and legal.
    let mut world = flat_world(0.0);
    let me = add_character(&mut world, [2.5, 0.0, 2.5]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_trunk_source(Arc::new(Fixed(Arc::new(trunks))));
    sim.set_input(me, walk(0.0)).unwrap();
    let radius = Character::default().radius_m;
    for _ in 0..300 {
        sim.step(1);
        let p = pos(&sim, me);
        for t in sim.trunks().within(p[0], p[2], 2.0) {
            assert!(t.1 >= radius - 1e-6, "inside a trunk: {t:?} at {p:?}");
        }
    }
}

#[test]
fn trunks_do_not_change_the_hash_of_a_world_without_trees_and_are_deterministic() {
    let build = || {
        let mut world = flat_world(0.0);
        let me = add_character(&mut world, [0.0, 0.0, 0.0]);
        (world, me)
    };
    let (world, me) = build();
    let mut plain = Sim::new(world).unwrap();
    let (world, _) = build();
    let mut empty = sim_with(world, Vec::new());
    for sim in [&mut plain, &mut empty] {
        sim.set_input(me, walk(0.0)).unwrap();
        sim.step(120);
    }
    assert_eq!(plain.state_hash(), empty.state_hash());

    let run = || {
        let mut world = flat_world(0.0);
        let tree = add_tree(&mut world, 0.0, -4.0);
        let me = add_character(&mut world, [0.1, 0.0, 0.0]);
        let mut sim = sim_with(world, vec![trunk_of(tree, 0.0, -4.0)]);
        sim.set_input(me, walk(0.0)).unwrap();
        sim.step(300);
        sim.state_hash()
    };
    assert_eq!(run(), run());
    // A world with trees hashes differently from the same world without them.
    let mut world = flat_world(0.0);
    add_tree(&mut world, 0.0, -4.0);
    let me2 = add_character(&mut world, [0.1, 0.0, 0.0]);
    let mut bare = Sim::new(world).unwrap();
    bare.set_input(me2, walk(0.0)).unwrap();
    bare.step(300);
    assert_ne!(run(), bare.state_hash());
}
