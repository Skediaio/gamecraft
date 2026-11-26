//! Rays, lines of sight and simulation steps on a world of about 300 000 entities: boxes and
//! prefab instances with their children spread over 8 km, and 300 roads crossing it (as a
//! real place has). What each of them costs is counted
//! (entities looked at: a number a machine can check) and timed (a number for the report; not
//! asserted, since a shared build server's clock is not).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, pos, walk};
use gamecraft_math::{Portable, Rng, deg_to_rad};
use gamecraft_sim::{Sim, ray};
use gamecraft_world::{
    Component, EntityDraft, Prefab, PrefabChild, Road, Shape, ShapeKind, Spawn, Terrain,
    TerrainSource, World,
};
use std::time::Instant;

/// The side of the world, metres.
const SIZE_M: f64 = 8192.0;
/// Boxes on their own.
const BOXES: usize = 60_000;
/// Instances of the six-entity prefab.
const LOTS: usize = 40_000;
/// Characters that walk while the world is stepped.
const WALKERS: usize = 8;
/// Roads that cross the world, each with forty segments.
const ROADS: usize = 300;

/// A world of `BOXES` boxes, `LOTS` instances of a prefab with five children and `ROADS` roads,
/// spread over the square of `SIZE_M`, on flat ground: 300 000 entities.
pub fn world_at_scale() -> World {
    let mut world = World::new("at scale", 2026);
    world.terrain = Some(Terrain {
        size_m: SIZE_M,
        cell_m: 8.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    world.prefabs = vec![Prefab {
        id: "lot".into(),
        name: "Lot".into(),
        components: Vec::new(),
        children: (0..5)
            .map(|i| PrefabChild {
                name: format!("House {i}"),
                offset: [8.0 * (i as f64 - 2.0), 0.0, 0.0],
                rotation_y_deg: 0.0,
                prefab: None,
                components: vec![Component::Shape(Shape {
                    visible: true,
                    kind: ShapeKind::Box,
                    size: [6.0, 4.0 + i as f64, 6.0],
                    color: [0.7, 0.6, 0.5],
                    solid: true,
                })],
            })
            .collect(),
    }];
    let mut rng = Rng::new(7);
    let half = SIZE_M / 2.0;
    let at = |rng: &mut Rng| [rng.range(-half, half), 0.0, rng.range(-half, half)];
    for _ in 0..BOXES {
        let size = rng.range(4.0, 24.0);
        world
            .add_entity(EntityDraft {
                name: "Block".into(),
                position: at(&mut rng),
                rotation_y_deg: rng.range(0.0, 360.0),
                components: vec![Component::Shape(Shape {
                    visible: true,
                    kind: ShapeKind::Box,
                    size: [size, size / 2.0, size],
                    color: [0.5, 0.5, 0.55],
                    solid: true,
                })],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    for _ in 0..LOTS {
        world
            .spawn_prefab(
                "lot",
                Spawn {
                    position: at(&mut rng),
                    rotation_y_deg: rng.range(0.0, 360.0),
                    ..Spawn::default()
                },
            )
            .unwrap();
    }
    // Roads that cross the world: forty segments of 220 m from a scattered start, each turned a
    // little, 6 to 14 m wide.
    for i in 0..ROADS {
        let mut x = rng.range(-half, half);
        let mut z = if i.is_multiple_of(2) {
            -half
        } else {
            rng.range(-half, half)
        };
        let mut heading = if i.is_multiple_of(2) {
            rng.range(60.0, 120.0)
        } else {
            rng.range(-30.0, 30.0)
        };
        let mut points = Vec::with_capacity(40);
        for _ in 0..40 {
            points.push([x, z]);
            let (s, c) = deg_to_rad(heading).psin_cos();
            x += 220.0 * s;
            z += 220.0 * c;
            heading += rng.range(-12.0, 12.0);
        }
        world
            .add_entity(EntityDraft {
                name: "Road".into(),
                components: vec![Component::Road(Road {
                    width_m: rng.range(6.0, 14.0),
                    ..Road::new(points)
                })],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    world
}

/// Seconds per call over `n` calls: a number for the report.
fn per_call(n: usize, elapsed: std::time::Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0 / n as f64
}

/// Most entities a ray or a line of sight of these lengths looks at on this world: the goal
/// is "well under a millisecond", and a few hundred tests are. The walk over every entity
/// looks at 300 000.
const MAX_LOOKED_AT: usize = 4_000;
/// Most entities a ray or a line of sight looks at with the roads placed in the cells of their
/// segments: a few hundred of the world's, and the few roads the ray's cells touch. Found
/// everywhere, the 300 roads would be looked at by every ray.
const MAX_LOOKED_AT_WITH_ROADS: usize = 400;
/// Most solids a tick with eight walkers tests: a few dozen solids around each walker, in a
/// handful of passes (the first tick, with walkers placed inside boxes, a few hundred). The
/// walk over every solid tests more than two million.
const MAX_SOLID_TESTS_PER_TICK: u64 = 10_000;

#[test]
fn rays_sight_and_steps_on_three_hundred_thousand_entities() {
    let build = Instant::now();
    let mut world = world_at_scale();
    assert_eq!(world.entity_count(), BOXES + LOTS * 6 + ROADS);
    let mut walkers = Vec::new();
    for i in 0..WALKERS {
        let x = -3000.0 + i as f64 * 700.0;
        walkers.push(add_character(&mut world, [x, 0.0, 123.4]));
    }
    println!(
        "world: {} entities, built in {:.2} s",
        world.entity_count(),
        build.elapsed().as_secs_f64()
    );
    let start = Instant::now();
    let mut sim = Sim::new(world).unwrap();
    println!("sim.start: {:.0} ms", per_call(1, start.elapsed()));
    // The instances themselves have no shape, nor have the walkers: only the boxes, the
    // instances' five children each and the roads are something a ray can meet. Each road is
    // in the cells of its segments: none of the 300 is found everywhere.
    assert_eq!(sim.index().len(), BOXES + LOTS * 5 + ROADS);
    assert_eq!(sim.index().found_everywhere(), 0, "a road found everywhere");
    let (world, heights, index) = (sim.world(), sim.heights(), sim.index());
    let mut rng = Rng::new(99);
    // Rays from 100 m up, sloping down over a kilometre, from scattered places.
    let rays: usize = 200;
    let mut rays_hit = 0;
    let mut looked = 0;
    let mut most = 0;
    let start = Instant::now();
    for _ in 0..rays {
        let half = SIZE_M / 2.0 - 600.0;
        let origin = [rng.range(-half, half), 100.0, rng.range(-half, half)];
        let yaw = rng.range(0.0, 360.0);
        let (s, c) = deg_to_rad(yaw).psin_cos();
        let dir = [s, -0.1, c];
        let (found, seen) =
            ray::cast_counted(world, heights, Some(index), origin, dir, 1000.0, |_| false);
        looked += seen;
        most = most.max(seen);
        rays_hit += usize::from(found.is_some());
    }
    println!(
        "ray (1 km, {rays} of them): {:.3} ms per ray, {rays_hit} hit something, {} entities \
         looked at per ray, {most} at most",
        per_call(rays, start.elapsed()),
        looked / rays
    );
    assert!(
        looked / rays < MAX_LOOKED_AT_WITH_ROADS,
        "{} per ray",
        looked / rays
    );
    assert!(most < MAX_LOOKED_AT, "{most}");
    // Forty of them again without the index, over every entity: the same hits, to the bit.
    let mut rng = Rng::new(99);
    let mut same = 0;
    for _ in 0..40 {
        let half = SIZE_M / 2.0 - 600.0;
        let origin = [rng.range(-half, half), 100.0, rng.range(-half, half)];
        let yaw = rng.range(0.0, 360.0);
        let (s, c) = deg_to_rad(yaw).psin_cos();
        let dir = [s, -0.1, c];
        let walked = ray::cast(world, heights, origin, dir, 1000.0, |_| false);
        let (found, _) =
            ray::cast_counted(world, heights, Some(index), origin, dir, 1000.0, |_| false);
        let as_bits =
            |f: Option<ray::Found>| f.map(|f| (f.hit.distance_m.to_bits(), f.entity.map(|e| e.0)));
        assert_eq!(as_bits(found), as_bits(walked), "{origin:?} {dir:?}");
        same += usize::from(found.is_some());
    }
    println!("{same} of 40 rays hit something, the same without the index");
    // Lines of sight between two points 300 m apart at eye height.
    let mut clear = 0;
    let mut looked = 0;
    let mut most = 0;
    let start = Instant::now();
    for _ in 0..rays {
        let half = SIZE_M / 2.0 - 600.0;
        let from = [rng.range(-half, half), 1.6, rng.range(-half, half)];
        let yaw = rng.range(0.0, 360.0);
        let (s, c) = deg_to_rad(yaw).psin_cos();
        let to = [from[0] + s * 300.0, 1.6, from[2] + c * 300.0];
        let (Some(sight), seen) =
            ray::line_of_sight_counted(world, heights, Some(index), from, to, |_| false)
        else {
            panic!("the points are finite");
        };
        looked += seen;
        most = most.max(seen);
        clear += usize::from(sight.blocked_by.is_none());
    }
    println!(
        "line of sight (300 m, {rays} of them): {:.3} ms each, {clear} clear, {} entities \
         looked at per line, {most} at most",
        per_call(rays, start.elapsed()),
        looked / rays
    );
    assert!(
        looked / rays < MAX_LOOKED_AT_WITH_ROADS,
        "{} per line",
        looked / rays
    );
    assert!(most < MAX_LOOKED_AT, "{most}");
    // Characters walking east among it all: sixty ticks.
    for &id in &walkers {
        sim.set_input(id, walk(270.0)).unwrap();
    }
    let start = Instant::now();
    sim.step(60);
    let step = per_call(60, start.elapsed());
    let tests = sim.solid_tests() / 60;
    println!(
        "sim.step: {step:.3} ms per tick with {WALKERS} walkers, {tests} solid tests per \
         tick; first walker at {:?}",
        pos(&sim, walkers[0])
    );
    assert!(tests > 0, "the counter counts");
    assert!(
        tests < MAX_SOLID_TESTS_PER_TICK,
        "{tests} solid tests per tick"
    );
    // An edit in one place touches the index for that entity only.
    let id = sim
        .world()
        .entities()
        .nth(1000)
        .map(|e| e.id)
        .expect("entities");
    let entity = sim.world().entity(id).expect("it");
    let mut moved = sim.world().clone();
    moved
        .place_entity(
            id,
            [
                entity.position[0] + 3.0,
                entity.position[1],
                entity.position[2],
            ],
            entity.rotation_y_deg,
        )
        .unwrap();
    let mut index = gamecraft_sim::index::SpatialIndex::build(sim.world());
    index.update(sim.world(), &moved);
    assert_eq!(index.len(), sim.index().len());
    let fresh = gamecraft_sim::index::SpatialIndex::build(&moved);
    let p = [entity.position[0] + 3.0, entity.position[2]];
    assert_eq!(index.near(p, 4.0), fresh.near(p, 4.0));
}
