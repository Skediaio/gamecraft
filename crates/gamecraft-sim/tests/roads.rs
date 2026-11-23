//! Roads in the simulation: rays and lines of sight meet the road surface and name the road,
//! characters walk along a road on the terrain beneath it, and the state hash holds.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, noise_world, pos, walk};
use gamecraft_sim::{Sim, ray};
use gamecraft_world::{Component, EntityDraft, EntityId, Road, World};

const TOL: f64 = 1e-9;

/// Hilly ground with one road running north from z 60 to z -60 along x 10.
fn road_world() -> (World, EntityId) {
    let mut world = noise_world(3, 30.0, 80.0);
    let id = world
        .add_entity(EntityDraft {
            name: "Main Street".into(),
            position: [10.0, 0.0, 60.0],
            components: vec![Component::Road(Road::new(vec![
                [0.0, 0.0],
                [0.0, -60.0],
                [3.0, -120.0],
            ]))],
            ..EntityDraft::default()
        })
        .unwrap();
    (world, id)
}

#[test]
fn a_ray_down_onto_a_road_names_it_and_beside_it_the_terrain() {
    let (world, road) = road_world();
    let sim = Sim::new(world).unwrap();
    let field = sim.heights().unwrap();
    let offset = sim
        .world()
        .entity(road)
        .unwrap()
        .road()
        .unwrap()
        .surface_offset_m();
    let down = [0.0, -1.0, 0.0];
    let found = ray::cast(
        sim.world(),
        Some(field),
        [11.0, 80.0, 20.0],
        down,
        200.0,
        |_| false,
    )
    .unwrap();
    assert_eq!(found.entity, Some(road));
    let ground = field.height_at(11.0, 20.0).unwrap();
    assert!((80.0 - found.hit.distance_m - (ground + offset)).abs() < TOL);
    assert!(found.hit.normal[1] > 0.5);
    let beside = ray::cast(
        sim.world(),
        Some(field),
        [20.0, 80.0, 20.0],
        down,
        200.0,
        |_| false,
    )
    .unwrap();
    assert_eq!(beside.entity, None);
    // Skipping the road sees the terrain beneath it.
    let under = ray::cast(
        sim.world(),
        Some(field),
        [11.0, 80.0, 20.0],
        down,
        200.0,
        |id| id == road,
    )
    .unwrap();
    assert_eq!(under.entity, None);
    assert!((80.0 - under.hit.distance_m - ground).abs() < TOL);
}

#[test]
fn the_road_surface_is_seen_and_hides_the_ground_beneath_it() {
    let (world, road) = road_world();
    let sim = Sim::new(world).unwrap();
    let field = sim.heights().unwrap();
    let offset = sim
        .world()
        .entity(road)
        .unwrap()
        .road()
        .unwrap()
        .surface_offset_m();
    let eye = [30.0, field.height_at(30.0, 0.0).unwrap() + 20.0, 0.0];
    let ground = field.height_at(10.5, -5.0).unwrap();
    let on_road = [10.5, ground + offset, -5.0];
    let sight = ray::line_of_sight(sim.world(), Some(field), eye, on_road, |_| false).unwrap();
    assert_eq!(
        sight.blocked_by, None,
        "a point on the road surface can be seen"
    );
    let beneath = [10.5, ground, -5.0];
    let sight = ray::line_of_sight(sim.world(), Some(field), eye, beneath, |_| false).unwrap();
    assert_eq!(sight.blocked_by.map(|f| f.entity), Some(Some(road)));
}

#[test]
fn a_character_walks_along_a_sloped_road_on_the_ground_beneath_it() {
    let (mut world, road) = road_world();
    let hero = add_character(&mut world, [10.0, 50.0, 50.0]);
    let mut sim = Sim::new(world).unwrap();
    let offset = sim
        .world()
        .entity(road)
        .unwrap()
        .road()
        .unwrap()
        .surface_offset_m();
    sim.set_input(hero, walk(0.0)).unwrap();
    let mut climbed: f64 = 0.0;
    let mut last: Option<f64> = None;
    for _ in 0..600 {
        sim.step(1);
        let p = pos(&sim, hero);
        let field = sim.heights().unwrap();
        let ground = field.height_at(p[0], p[2]).unwrap();
        if sim.body(hero).unwrap().grounded {
            // The feet are on the terrain, the road surface `offset` above them.
            assert!((p[1] - ground).abs() < 1e-6, "{p:?} over {ground}");
            assert!(offset < 0.05);
        }
        if let Some(y) = last {
            climbed += (p[1] - y).abs();
        }
        last = Some(p[1]);
    }
    let p = pos(&sim, hero);
    assert!(p[2] < 40.0, "walked north: {p:?}");
    assert!(climbed > 0.5, "the road goes up and down: {climbed}");
}

#[test]
fn a_world_with_roads_hashes_the_same_every_run() {
    let run = || {
        let (mut world, _) = road_world();
        let hero = add_character(&mut world, [10.0, 50.0, 50.0]);
        let mut sim = Sim::new(world).unwrap();
        sim.set_input(hero, walk(10.0)).unwrap();
        sim.step(120);
        sim.state_hash()
    };
    assert_eq!(run(), run());
    let (with, _) = road_world();
    let without = noise_world(3, 30.0, 80.0);
    assert_ne!(
        Sim::new(with).unwrap().state_hash(),
        Sim::new(without).unwrap().state_hash(),
        "the road is part of the world"
    );
}
