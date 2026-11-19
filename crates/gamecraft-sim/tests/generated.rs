//! Generated instances in the simulation: trunks block characters and shots, rays hit them,
//! scattered solids block like shapes, a forest is the same on every run.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, pos, settle, walk};
use gamecraft_sim::ray::{self, GeneratedHit};
use gamecraft_sim::{EventKind, Sim};
use gamecraft_world::{
    Area, Circle, Component, EntityDraft, Generator, GeneratorItem, GeneratorKind, Instance,
    Launcher, Plant, Shape, ShapeKind, Terrain, TerrainSource, Trunk, World,
};

fn world() -> World {
    let mut w = World::new("woods", 3);
    w.terrain = Some(Terrain {
        size_m: 512.0,
        cell_m: 4.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    w.environment.wind_speed_mps = 0.0;
    w
}

fn oak() -> Component {
    Component::Plant(Plant {
        species: "quercus-robur".into(),
        seed: 1,
        growth: 1.0,
    })
}

/// A forest of thick trunks (0.5 m radius, 8 m tall) around `center`.
fn forest(center: [f64; 2], radius_m: f64, density: f64) -> Generator {
    Generator {
        id: "woods".into(),
        kind: GeneratorKind::Forest,
        area: Area::Circle(Circle { center, radius_m }),
        seed: 9,
        density_per_ha: density,
        min_distance_m: Some(4.0),
        slope_max_deg: None,
        height_range_m: None,
        exclude: Vec::new(),
        items: vec![GeneratorItem {
            trunk: Some(Trunk {
                radius_m: 0.5,
                height_m: 8.0,
            }),
            ..GeneratorItem::of(1.0, vec![oak()])
        }],
    }
}

/// The running world's instances of `id`.
fn trees(sim: &Sim, id: &str) -> Vec<Instance> {
    sim.world()
        .generated()
        .set(id)
        .unwrap()
        .instances()
        .to_vec()
}

#[test]
fn a_character_walking_into_a_trunk_is_stopped() {
    let mut w = world();
    // One tree: a forest whose area holds about one.
    let mut g = forest([0.0, 0.0], 2.0, 2000.0);
    g.min_distance_m = Some(100.0);
    w.generators.push(g);
    let sim = Sim::new(w.clone()).unwrap();
    let tree = trees(&sim, "woods");
    assert_eq!(tree.len(), 1, "{tree:?}");
    let [tx, _, tz] = tree[0].position;
    // Ten metres north of it, walking south, straight at it.
    let hero = add_character(&mut w, [tx, 0.0, tz - 10.0]);
    let mut sim = Sim::new(w).unwrap();
    settle(&mut sim);
    sim.set_input(hero, walk(180.0)).unwrap();
    for _ in 0..600 {
        sim.step(1);
        let [x, _, z] = pos(&sim, hero);
        let d = ((x - tx) * (x - tx) + (z - tz) * (z - tz)).sqrt();
        assert!(d >= 0.5 + 0.3 - 1e-6, "inside the trunk: {d}");
    }
    let [_, _, z] = pos(&sim, hero);
    assert!(z < tz, "stopped north of the trunk: {z} vs {tz}");
}

#[test]
fn rays_and_shots_hit_a_trunk() {
    let mut w = world();
    let mut g = forest([0.0, -50.0], 2.0, 2000.0);
    g.min_distance_m = Some(100.0);
    w.generators.push(g);
    let mut sim = Sim::new(w.clone()).unwrap();
    let tree = trees(&sim, "woods")[0];
    let [tx, _, tz] = tree.position;
    // A ray at chest height from the south, straight at the trunk.
    let origin = [tx, 1.5, tz + 30.0];
    let found = ray::cast(
        sim.world(),
        sim.heights(),
        origin,
        [0.0, 0.0, -1.0],
        100.0,
        |_| false,
    )
    .unwrap();
    assert_eq!(found.entity, None);
    assert_eq!(
        found.generated,
        Some(GeneratedHit {
            set: 0,
            number: tree.number
        })
    );
    assert!((found.hit.distance_m - 29.5).abs() < 1e-9, "{found:?}");
    // Above the trunk the ray passes.
    let over = ray::cast(
        sim.world(),
        sim.heights(),
        [tx, 9.0, tz + 30.0],
        [0.0, 0.0, -1.0],
        100.0,
        |_| false,
    );
    assert!(over.is_none_or(|f| f.generated.is_none()));
    // A shot from the south stops at the trunk.
    let gun = sim_add_gun(&mut w, [tx, 0.0, tz + 40.0]);
    sim = Sim::new(w).unwrap();
    sim.fire(gun, Some(0.0), Some(0.0)).unwrap();
    let mut hit = None;
    for _ in 0..120 {
        sim.step(1);
        if let Some(e) = sim
            .take_events()
            .into_iter()
            .find(|e| matches!(e.kind, EventKind::Hit { .. }))
        {
            hit = Some(e.kind);
            break;
        }
    }
    let Some(EventKind::Hit { point, target, .. }) = hit else {
        panic!("no hit");
    };
    assert_eq!(target, None);
    let [px, _, pz] = point;
    let d = ((px - tx) * (px - tx) + (pz - tz) * (pz - tz)).sqrt();
    assert!(
        (d - 0.5).abs() < 0.01,
        "on the trunk's surface: {d} at {point:?}"
    );
    assert!(sim.projectiles().is_empty());
}

fn sim_add_gun(w: &mut World, at: [f64; 3]) -> gamecraft_world::EntityId {
    w.add_entity(EntityDraft {
        name: "gun".into(),
        position: at,
        components: vec![Component::Launcher(Launcher {
            muzzle_velocity_mps: 800.0,
            dispersion_mrad: 0.0,
            muzzle_height_m: 1.5,
            ..Launcher::default()
        })],
        ..EntityDraft::default()
    })
    .unwrap()
}

#[test]
fn scattered_solids_block_like_shapes() {
    let mut w = world();
    let mut g = forest([0.0, 0.0], 2.0, 2000.0);
    g.kind = GeneratorKind::Scatter;
    g.min_distance_m = Some(100.0);
    g.items = vec![GeneratorItem {
        random_rotation: Some(false),
        ..GeneratorItem::of(
            1.0,
            vec![Component::Shape(Shape {
                kind: ShapeKind::Box,
                size: [4.0, 3.0, 4.0],
                color: [0.5, 0.5, 0.5],
                solid: true,
            })],
        )
    }];
    w.generators.push(g);
    let rock = trees(&Sim::new(w.clone()).unwrap(), "woods")[0];
    let [rx, _, rz] = rock.position;
    let hero = add_character(&mut w, [rx, 0.0, rz - 10.0]);
    let mut sim = Sim::new(w).unwrap();
    settle(&mut sim);
    sim.set_input(hero, walk(180.0)).unwrap();
    sim.step(600);
    let [_, _, z] = pos(&sim, hero);
    assert!(
        (z - (rz - 2.0 - 0.3)).abs() < 1e-6,
        "against the box's north face: {z}"
    );
}

#[test]
fn a_dense_forest_is_the_same_on_every_run_and_live_changes_take_effect() {
    let mut w = world();
    w.generators.push(forest([0.0, 0.0], 200.0, 400.0));
    let hero = add_character(&mut w, [0.0, 0.0, 0.0]);
    let run = |w: &World| {
        let mut sim = Sim::new(w.clone()).unwrap();
        settle(&mut sim);
        sim.set_input(hero, walk(37.0)).unwrap();
        sim.step(1200);
        (sim.state_hash(), pos(&sim, hero))
    };
    let (a, pa) = run(&w);
    let (b, pb) = run(&w);
    assert_eq!((a, pa), (b, pb));
    let mut sim = Sim::new(w.clone()).unwrap();
    assert!(trees(&sim, "woods").len() > 3000);
    // The game master grows a second wood while it runs.
    let mut live = sim.world().clone();
    let mut more = forest([150.0, 150.0], 40.0, 100.0);
    more.id = "copse".into();
    live.generators.push(more);
    sim.replace_world(live).unwrap();
    assert!(!trees(&sim, "copse").is_empty());
    assert!(sim.world().generated_is_current());
    // And the other trees stay where they stood.
    assert_eq!(trees(&sim, "woods"), trees(&Sim::new(w).unwrap(), "woods"));
}
