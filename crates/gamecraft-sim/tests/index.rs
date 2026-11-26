//! The spatial index finds what a walk over every entity finds, to the bit: on random rays and
//! lines of sight over a mixed world (with entities skipped), and after edits, where an index
//! updated in place must give the candidates a fresh one gives.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use gamecraft_math::{Portable, Rng};
use gamecraft_sim::index::SpatialIndex;
use gamecraft_sim::ray;
use gamecraft_world::{
    Component, EntityDraft, Model, ModelBounds, ModelCollider, Prefab, PrefabChild, Road, Scale,
    Shape, ShapeKind, Spawn, Terrain, TerrainSource, World,
};
use std::collections::BTreeMap;

/// A world of everything a ray can meet, on rolling ground: turned boxes (solid or not,
/// some scaled per axis), spheres, cylinders, models with bounds, roads, prefab instances
/// with children, a character, and two shapes too large for a cell.
fn mixed(seed: u64) -> (World, gamecraft_world::HeightField) {
    let mut world = World::new("mixed", seed);
    world.terrain = Some(Terrain {
        size_m: 512.0,
        cell_m: 4.0,
        source: TerrainSource::Noise {
            seed,
            relief_m: 12.0,
            feature_m: 160.0,
            octaves: 3,
            roughness: 0.5,
        },
        ..Default::default()
    });
    let field = world.terrain.clone().unwrap().build().unwrap();
    world.set_model_bounds(
        [(
            "models/crate.glb".to_string(),
            ModelBounds {
                min: [-0.5, 0.0, -0.4],
                max: [0.5, 1.0, 0.6],
            },
        )]
        .into_iter()
        .collect::<BTreeMap<_, _>>(),
    );
    world.prefabs = vec![Prefab {
        id: "gate".into(),
        name: "Gate".into(),
        components: Vec::new(),
        children: vec![
            PrefabChild {
                name: "Post".into(),
                offset: [-2.0, 0.0, 0.0],
                components: vec![Component::Shape(Shape {
                    visible: true,
                    kind: ShapeKind::Cylinder,
                    size: [0.4, 3.0, 0.4],
                    ..Shape::default()
                })],
                rotation_y_deg: 0.0,
                prefab: None,
            },
            PrefabChild {
                name: "Post".into(),
                offset: [2.0, 0.0, 0.0],
                components: vec![Component::Shape(Shape {
                    visible: true,
                    kind: ShapeKind::Cylinder,
                    size: [0.4, 3.0, 0.4],
                    ..Shape::default()
                })],
                rotation_y_deg: 0.0,
                prefab: None,
            },
            PrefabChild {
                name: "Beam".into(),
                offset: [0.0, 3.0, 0.0],
                components: vec![Component::Shape(Shape {
                    visible: true,
                    kind: ShapeKind::Box,
                    size: [4.4, 0.4, 0.4],
                    ..Shape::default()
                })],
                rotation_y_deg: 0.0,
                prefab: None,
            },
        ],
    }];
    let mut rng = Rng::new(seed ^ 0x5eed);
    let at = |rng: &mut Rng| {
        [
            rng.range(-200.0, 200.0),
            rng.range(0.0, 6.0),
            rng.range(-200.0, 200.0),
        ]
    };
    let shape = |kind, size: [f64; 3], solid| {
        Component::Shape(Shape {
            visible: true,
            kind,
            size,
            solid,
            ..Shape::default()
        })
    };
    for _ in 0..120 {
        world
            .add_entity(EntityDraft {
                position: at(&mut rng),
                rotation_y_deg: rng.range(0.0, 360.0),
                scale: if rng.next_u32().is_multiple_of(4) {
                    Scale::per_axis(
                        rng.range(0.5, 3.0),
                        rng.range(0.5, 3.0),
                        rng.range(0.5, 3.0),
                    )
                } else {
                    Scale::uniform(rng.range(0.5, 4.0))
                },
                components: vec![shape(
                    ShapeKind::Box,
                    [
                        rng.range(1.0, 6.0),
                        rng.range(0.5, 5.0),
                        rng.range(1.0, 6.0),
                    ],
                    !rng.next_u32().is_multiple_of(3),
                )],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    for _ in 0..30 {
        world
            .add_entity(EntityDraft {
                position: at(&mut rng),
                components: vec![shape(ShapeKind::Sphere, [rng.range(0.5, 2.0); 3], true)],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    for _ in 0..30 {
        let d = rng.range(0.4, 2.0);
        world
            .add_entity(EntityDraft {
                position: at(&mut rng),
                components: vec![shape(
                    ShapeKind::Cylinder,
                    [d, rng.range(1.0, 6.0), d],
                    true,
                )],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    for _ in 0..20 {
        world
            .add_entity(EntityDraft {
                position: at(&mut rng),
                rotation_y_deg: rng.range(0.0, 360.0),
                scale: Scale::uniform(rng.range(0.5, 3.0)),
                components: vec![Component::Model(Model {
                    collider: if rng.next_u32().is_multiple_of(2) {
                        ModelCollider::Box
                    } else {
                        ModelCollider::None
                    },
                    ..Model::of("models/crate.glb")
                })],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    for i in 0..6 {
        let x = -180.0 + i as f64 * 60.0;
        world
            .add_entity(EntityDraft {
                name: "Road".into(),
                position: [x, 0.0, -240.0],
                components: vec![Component::Road(Road::new(vec![
                    [0.0, 0.0],
                    [10.0, 120.0],
                    [-5.0, 240.0],
                    [0.0, 480.0],
                ]))],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    for _ in 0..8 {
        let y = field
            .height_at(rng.range(-180.0, 180.0), rng.range(-180.0, 180.0))
            .unwrap_or(0.0);
        let spawn = Spawn {
            position: [rng.range(-180.0, 180.0), y, rng.range(-180.0, 180.0)],
            rotation_y_deg: rng.range(0.0, 360.0),
            ..Spawn::default()
        };
        world.spawn_prefab("gate", spawn).unwrap();
    }
    for (position, size) in [
        ([0.0, 20.0, 0.0], [400.0, 0.2, 400.0]),
        ([150.0, 0.0, 150.0], [1.0, 60.0, 1.0]),
    ] {
        world
            .add_entity(EntityDraft {
                position,
                components: vec![shape(ShapeKind::Box, size, true)],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    common::add_character(&mut world, [3.0, 20.5, 3.0]);
    (world, field)
}

/// A hit as bits: the distance, the normal and the entity.
fn bits(found: Option<ray::Found>) -> Option<(u64, [u64; 3], Option<u64>)> {
    found.map(|f| {
        (
            f.hit.distance_m.to_bits(),
            f.hit.normal.map(f64::to_bits),
            f.entity.map(|e| e.0),
        )
    })
}

/// Random rays: where the ground is not too far away to matter.
fn rays(seed: u64, n: usize) -> Vec<([f64; 3], [f64; 3], f64, u32)> {
    let mut rng = Rng::new(seed);
    (0..n)
        .map(|_| {
            let origin = [
                rng.range(-220.0, 220.0),
                rng.range(0.3, 45.0),
                rng.range(-220.0, 220.0),
            ];
            let yaw = deg_to_rad(rng.range(0.0, 360.0)).psin_cos();
            let pitch = deg_to_rad(rng.range(-80.0, 30.0)).psin_cos();
            let dir = [yaw.0 * pitch.1, pitch.0, -yaw.1 * pitch.1];
            let max_m = [5.0, 50.0, 300.0, 2000.0][(rng.next_u32() % 4) as usize];
            (origin, dir, max_m, rng.next_u32())
        })
        .collect()
}

fn deg_to_rad(deg: f64) -> f64 {
    gamecraft_math::deg_to_rad(deg)
}

#[test]
fn rays_find_the_same_through_the_index_as_over_every_entity() {
    let (world, field) = mixed(11);
    let index = SpatialIndex::build(&world);
    // The boxes, spheres, cylinders, roads, the gates' children and the two large shapes;
    // models count when they have a box collider (the generator flips a coin).
    assert!(index.len() >= 120 + 30 + 30 + 6 + 24 + 2, "{}", index.len());
    let mut looked = 0;
    let mut hits = 0;
    for (origin, dir, max_m, salt) in rays(3, 800) {
        let skip = |id: gamecraft_world::EntityId| salt.is_multiple_of(2) && id.0.is_multiple_of(3);
        let walked = ray::cast(&world, Some(&field), origin, dir, max_m, skip);
        let (found, seen) =
            ray::cast_counted(&world, Some(&field), Some(&index), origin, dir, max_m, skip);
        assert_eq!(bits(found), bits(walked), "{origin:?} {dir:?} {max_m}");
        assert!(seen <= index.len());
        looked += seen;
        hits += usize::from(found.is_some());
    }
    assert!(hits > 300, "the rays meet things: {hits}");
    // The work: a share of the world per ray, not the world.
    assert!(looked / 800 < 150, "{} looked at per ray", looked / 800);
    // A ray from high above everything, down: the cell under it and what is everywhere.
    let (_, seen) = ray::cast_counted(
        &world,
        Some(&field),
        Some(&index),
        [100.0, 500.0, 100.0],
        [0.0, -1.0, 0.0],
        600.0,
        |_| false,
    );
    assert!(seen < 60, "{seen}");
}

#[test]
fn lines_of_sight_find_the_same_through_the_index() {
    let (world, field) = mixed(12);
    let index = SpatialIndex::build(&world);
    let mut clear = 0;
    for (origin, dir, _, salt) in rays(4, 400) {
        let to = ray::along(origin, dir, 40.0 + (salt % 7) as f64 * 40.0);
        let skip = |id: gamecraft_world::EntityId| salt.is_multiple_of(3) && id.0.is_multiple_of(5);
        let walked = ray::line_of_sight(&world, Some(&field), origin, to, skip);
        let (found, _) =
            ray::line_of_sight_counted(&world, Some(&field), Some(&index), origin, to, skip);
        assert_eq!(found, walked, "{origin:?} {to:?}");
        clear += usize::from(found.as_ref().is_some_and(|s| s.blocked_by.is_none()));
    }
    assert!(clear > 100, "{clear}");
}

#[test]
fn an_index_updated_in_place_is_one_made_again() {
    let (world, _) = mixed(13);
    let mut index = SpatialIndex::build(&world);
    let mut before = world.clone();
    let mut rng = Rng::new(5);
    let mut current = world.clone();
    // Edits that add, move, turn, scale and remove, each followed by the index's update.
    let ids: Vec<gamecraft_world::EntityId> = current.entities().map(|e| e.id).collect();
    for step in 0..12 {
        match step % 4 {
            0 => {
                for _ in 0..5 {
                    current
                        .add_entity(EntityDraft {
                            position: [rng.range(-200.0, 200.0), 0.0, rng.range(-200.0, 200.0)],
                            components: vec![Component::Shape(Shape {
                                visible: true,
                                kind: ShapeKind::Box,
                                size: [3.0, 2.0, 3.0],
                                ..Shape::default()
                            })],
                            ..EntityDraft::default()
                        })
                        .unwrap();
                }
            }
            1 => {
                let id = ids.get(step * 7).copied().expect("ids");
                let e = current.entity(id).expect("there");
                let to = [e.position[0] + 33.0, e.position[1], e.position[2] - 70.0];
                current
                    .place_entity(id, to, e.rotation_y_deg + 40.0)
                    .unwrap();
            }
            2 => {
                let id = ids.get(step * 5).copied().expect("ids");
                current
                    .set_scale(id, Scale::per_axis(2.0, 1.0, 30.0))
                    .unwrap();
            }
            _ => {
                let id = ids.get(step * 11).copied().expect("ids");
                current.remove_entity(id);
            }
        }
        index.update(&before, &current);
        before = current.clone();
    }
    let fresh = SpatialIndex::build(&current);
    assert_eq!(index.len(), fresh.len());
    for (origin, dir, max_m, _) in rays(6, 300) {
        assert_eq!(
            index.ray_candidates(origin, dir, max_m),
            fresh.ray_candidates(origin, dir, max_m),
            "{origin:?} {dir:?} {max_m}"
        );
    }
    for i in 0..80 {
        let p = [rng.range(-220.0, 220.0), rng.range(-220.0, 220.0)];
        let reach = [0.0, 1.5, 20.0][i % 3];
        assert_eq!(index.near(p, reach), fresh.near(p, reach), "{p:?} {reach}");
    }
}

/// A flat world of 8 km with three roads: one winding for six kilometres (more cells than a
/// footprint may touch), one short, and one with more cells than any road is placed in.
fn roads_world() -> (
    World,
    gamecraft_world::HeightField,
    [gamecraft_world::EntityId; 3],
) {
    let mut world = World::new("roads", 5);
    world.terrain = Some(Terrain {
        size_m: 8192.0,
        cell_m: 8.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    let field = world.terrain.clone().unwrap().build().unwrap();
    let winding = Road::new(
        (0..200_usize)
            .map(|i| {
                let x = -3000.0 + 30.0 * i as f64;
                let z = if i.is_multiple_of(2) { -60.0 } else { 60.0 };
                [x, z]
            })
            .collect(),
    );
    let short = Road::new(vec![[1500.0, 1500.0], [1540.0, 1500.0]]);
    let huge = Road::new(
        (0..40_usize)
            .map(|i| {
                let x = 20_000.0 * i as f64;
                let z = if i.is_multiple_of(2) { -5000.0 } else { 5000.0 };
                [x, z]
            })
            .collect(),
    );
    let mut ids = [gamecraft_world::EntityId(0); 3];
    for (slot, road) in [winding, short, huge].into_iter().enumerate() {
        ids[slot] = world
            .add_entity(EntityDraft {
                name: "Road".into(),
                components: vec![Component::Road(road)],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    (world, field, ids)
}

#[test]
fn a_long_road_is_found_in_the_cells_of_its_segments_not_everywhere() {
    let (world, field, [long, short, huge]) = roads_world();
    let index = SpatialIndex::build(&world);
    assert_eq!(index.len(), 3);
    // The winding road touches more cells than a footprint may hold, and is placed all the
    // same: only the road with more cells than any road is placed in is found everywhere.
    let (e, r) = (
        world.entity(long).unwrap(),
        world.entity(long).unwrap().road().unwrap(),
    );
    let (lo, hi) = r.bounds(e).unwrap();
    let across = ((hi[0] - lo[0]) / 32.0).ceil() * ((hi[1] - lo[1]) / 32.0 + 1.0).ceil();
    assert!(across > 256.0, "{across}");
    assert_eq!(index.found_everywhere(), 1);
    // Three kilometres from the first point the road runs: it is found there, and nothing
    // else is within 200 m.
    let line = r.centre_line(e);
    let on = line.get(100).copied().expect("points");
    assert_eq!(index.near(on, 1.0), vec![long, huge]);
    assert_eq!(index.near([on[0], on[1] + 200.0], 1.0), vec![huge]);
    assert_eq!(index.near([on[0], on[1] - 200.0], 1.0), vec![huge]);
    // Far from every road: nothing but the road found everywhere.
    assert_eq!(index.near([0.0, 3900.0], 2.0), vec![huge]);
    assert_eq!(
        index.ray_candidates([0.0, 1.0, 3900.0], [1.0, 0.0, 0.0], 200.0),
        Some(vec![huge])
    );
    // A ray across the road looks at it; one that runs 200 m beside it for a kilometre does
    // not; the short road is found where it is.
    let crossing = index
        .ray_candidates([on[0], 1.0, on[1] - 500.0], [0.0, 0.0, 1.0], 1000.0)
        .unwrap();
    assert!(crossing.contains(&long), "{crossing:?}");
    let beside = index
        .ray_candidates([on[0] - 500.0, 1.0, on[1] + 200.0], [1.0, 0.0, 0.0], 1000.0)
        .unwrap();
    assert_eq!(beside, vec![huge], "{beside:?}");
    let at_short = index
        .ray_candidates([1400.0, 1.0, 1500.0], [1.0, 0.0, 0.0], 200.0)
        .unwrap();
    assert_eq!(at_short, vec![short, huge], "{at_short:?}");
    // Near the roads the hits are the ones of a walk over every entity, to the bit.
    let mut checked = 0;
    let mut rng = Rng::new(77);
    // Rays from above, sloping down along or across the winding road.
    for _ in 0..300 {
        let k = rng.range(0.0, 199.0).floor();
        let a = line.get(k as usize).copied().expect("points");
        let origin = [
            a[0] + rng.range(-150.0, 150.0),
            20.0 + rng.range(0.0, 200.0),
            a[1] - 400.0,
        ];
        let yaw = deg_to_rad(rng.range(-20.0, 20.0)).psin_cos();
        let dir = [yaw.0, -0.05, yaw.1];
        let max_m = 900.0;
        let walked = ray::cast(&world, Some(&field), origin, dir, max_m, |_| false);
        let (found, seen) = ray::cast_counted(
            &world,
            Some(&field),
            Some(&index),
            origin,
            dir,
            max_m,
            |_| false,
        );
        assert_eq!(bits(found), bits(walked), "{origin:?} {dir:?}");
        assert!(seen <= 2, "{seen} looked at");
        checked += usize::from(walked.is_some());
    }
    assert!(checked > 30, "the rays meet the road: {checked}");
}

#[test]
fn what_is_around_a_place_holds_every_solid_that_could_meet_it() {
    let (world, field) = mixed(14);
    let index = SpatialIndex::build(&world);
    let mut rng = Rng::new(21);
    for i in 0..200 {
        let p = [rng.range(-220.0, 220.0), rng.range(-220.0, 220.0)];
        let reach = [0.0, 0.5, 2.0, 33.0][i % 4];
        let near = index.near(p, reach);
        // Every entity whose footprint meets the square is a candidate: by the footprint
        // itself, walked over every entity (a road by the boxes of its segments).
        let meets = |(lo, hi): ([f64; 2], [f64; 2])| {
            lo[0] <= p[0] + reach
                && hi[0] >= p[0] - reach
                && lo[1] <= p[1] + reach
                && hi[1] >= p[1] - reach
        };
        for e in world.entities() {
            let could = match e.road() {
                Some(road) => road.segment_boxes(e).iter().any(|b| meets(*b)),
                None => gamecraft_sim::index::footprint(&world, e).is_some_and(meets),
            };
            assert!(!could || near.contains(&e.id), "{p:?} {reach}: {:?}", e.id);
        }
        assert!(near.windows(2).all(|w| w[0] < w[1]), "sorted, each once");
    }
    let _ = field;
}
