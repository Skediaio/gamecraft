//! Models in the simulation: a solid model's bounding box blocks like a box, a model that is
//! not solid is walked through, and rays hit a model's collider box.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, box_distance, flat_world, pos, walk};
use gamecraft_math::Portable;
use gamecraft_sim::{Sim, ray};
use gamecraft_world::{
    Character, Component, EntityDraft, EntityId, Model, ModelBounds, ModelCollider, World,
};
use std::collections::BTreeMap;

const TOL: f64 = 1e-9;

/// A crate asset of 1.2 x 1.0 x 0.8 m standing on its origin, off-centre in x by 0.1 m.
const CRATE: ModelBounds = ModelBounds {
    min: [-0.5, 0.0, -0.4],
    max: [0.7, 1.0, 0.4],
};

fn with_model(
    world: &mut World,
    at: [f64; 3],
    yaw: f64,
    solid: bool,
    collider: ModelCollider,
) -> EntityId {
    world
        .add_entity(EntityDraft {
            name: "Crate".into(),
            position: at,
            rotation_y_deg: yaw,
            scale: gamecraft_world::Scale::ONE,
            components: vec![Component::Model(Model {
                asset: "models/crate.glb".into(),
                scale: 2.0,
                solid,
                collider,
                lod_distances_m: None,
                draw_distance_m: None,
            })],
            layer: None,
        })
        .unwrap()
}

fn bounds() -> BTreeMap<String, ModelBounds> {
    BTreeMap::from([("models/crate.glb".to_string(), CRATE)])
}

fn walk_north(world: World, ticks: u32) -> ([f64; 3], Vec<[f64; 3]>) {
    let mut world = world;
    let id = add_character(&mut world, [0.2, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(id, walk(0.0)).unwrap();
    let mut path = Vec::new();
    for _ in 0..ticks {
        sim.step(1);
        path.push(pos(&sim, id));
    }
    (pos(&sim, id), path)
}

#[test]
fn a_solid_model_blocks_like_its_turned_bounding_box() {
    for yaw in [30.0, 90.0] {
        let mut world = flat_world(0.0);
        with_model(&mut world, [0.0, 0.0, -4.0], yaw, true, ModelCollider::Box);
        world.set_model_bounds(bounds());
        let (end, path) = walk_north(world, 600);
        // Scaled by 2: half extents 1.2 and 0.8, the centre 0.2 m along the model's +X,
        // turned counter-clockwise seen from above.
        let (s, c) = gamecraft_math::deg_to_rad(yaw).psin_cos();
        let centre = [0.2 * c, -4.0 - 0.2 * s];
        for p in &path {
            let d = box_distance([p[0], p[2]], centre, [1.2, 0.8], yaw);
            assert!(
                d >= Character::default().radius_m - 1e-6,
                "{yaw}: {p:?}: {d}"
            );
        }
        if yaw == 90.0 {
            // Its local +X points north, so the centre is 0.2 m north of the base and the
            // near face 1.2 m south of the centre, at z = -3.0.
            let stop = -4.2 + 1.2 + Character::default().radius_m;
            assert!((end[2] - stop).abs() < 1e-6, "{end:?}");
        }
    }
}

#[test]
fn a_model_that_is_not_solid_or_has_no_bounds_is_walked_through() {
    for (solid, collider, give_bounds) in [
        (false, ModelCollider::Box, true),
        (true, ModelCollider::None, true),
        (true, ModelCollider::Box, false),
    ] {
        let mut world = flat_world(0.0);
        with_model(&mut world, [0.0, 0.0, -4.0], 0.0, solid, collider);
        if give_bounds {
            world.set_model_bounds(bounds());
        }
        let (end, _) = walk_north(world, 600);
        assert!(
            end[2] < -10.0,
            "{solid} {collider:?} {give_bounds}: {end:?}"
        );
    }
}

#[test]
fn a_low_solid_model_is_a_step_to_stand_on() {
    let mut world = flat_world(0.0);
    let id = with_model(&mut world, [0.0, 0.0, -3.0], 0.0, true, ModelCollider::Box);
    world.set_model_bounds(BTreeMap::from([(
        "models/crate.glb".to_string(),
        ModelBounds {
            min: [-1.0, 0.0, -1.0],
            max: [1.0, 0.1, 1.0],
        },
    )]));
    let _ = id;
    let (_, path) = walk_north(world, 120);
    let top = path.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
    assert!((top - 0.2).abs() < TOL, "it stood on the 0.2 m top: {top}");
}

#[test]
fn rays_hit_a_model_s_collider_box() {
    let mut world = flat_world(0.0);
    let id = with_model(&mut world, [0.0, 0.0, -4.0], 0.0, false, ModelCollider::Box);
    world.set_model_bounds(bounds());
    let found = ray::cast(
        &world,
        None,
        [0.4, 0.5, 0.0],
        [0.0, 0.0, -1.0],
        100.0,
        |_| false,
    )
    .unwrap();
    assert_eq!(found.entity, Some(id));
    assert!(
        (found.hit.distance_m - (4.0 - 0.8)).abs() < TOL,
        "{found:?}"
    );
    let mut none = flat_world(0.0);
    with_model(&mut none, [0.0, 0.0, -4.0], 0.0, true, ModelCollider::None);
    none.set_model_bounds(bounds());
    assert!(
        ray::cast(
            &none,
            None,
            [0.4, 0.5, 0.0],
            [0.0, 0.0, -1.0],
            100.0,
            |_| false
        )
        .is_none()
    );
}

#[test]
fn the_bounds_of_solid_models_are_part_of_the_state_hash() {
    let make = |b: ModelBounds| {
        let mut world = flat_world(0.0);
        with_model(&mut world, [0.0, 0.0, -4.0], 0.0, true, ModelCollider::Box);
        world.set_model_bounds(BTreeMap::from([("models/crate.glb".to_string(), b)]));
        Sim::new(world).unwrap().state_hash()
    };
    let mut wider = CRATE;
    wider.max[0] = 0.9;
    assert_ne!(make(CRATE), make(wider));
    assert_eq!(make(CRATE), make(CRATE));
    // A world without models hashes as before: bounds of assets nobody uses change nothing.
    let mut plain = flat_world(0.0);
    let before = Sim::new(plain.clone()).unwrap().state_hash();
    plain.set_model_bounds(bounds());
    assert_eq!(Sim::new(plain).unwrap().state_hash(), before);
}

#[test]
fn the_distances_of_a_model_s_levels_are_no_part_of_the_state_hash() {
    // What only changes a picture (where a model changes level, where it stops being drawn)
    // does not decide what moves next.
    let make = |lod: bool| {
        let mut world = flat_world(0.0);
        let id = with_model(&mut world, [0.0, 0.0, -4.0], 0.0, true, ModelCollider::Box);
        if lod {
            let mut m = world.entity(id).unwrap().model().unwrap().clone();
            m.lod_distances_m = Some(vec![40.0, 150.0]);
            m.draw_distance_m = Some(800.0);
            world
                .entity_mut(id)
                .unwrap()
                .set_component(Component::Model(m));
        }
        world.set_model_bounds(bounds());
        let text = world.to_json().unwrap();
        let hash = Sim::new(world).unwrap().state_hash();
        (hash, text)
    };
    let (plain, plain_text) = make(false);
    let (lod, lod_text) = make(true);
    assert_ne!(plain_text, lod_text, "the two worlds differ in their files");
    assert_eq!(plain, lod, "but not in what the simulation hashes");
}
