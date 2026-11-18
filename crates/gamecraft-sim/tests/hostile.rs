//! Hostile input and odd worlds never panic and never write a non-finite number.

// Integration tests are compiled as their own crates, outside `cfg(test)`, so clippy.toml's
// test allowances do not reach their helper functions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, add_shape, flat_world, pos, walk};
use gamecraft_sim::{FALL_FLOOR_M, Input, MAX_STEP_TICKS, Sim, SimError};
use gamecraft_world::{EntityId, ShapeKind, World};

#[test]
fn non_finite_input_fields_are_refused_naming_the_field() {
    let mut world = flat_world(0.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    for (field, input) in [
        (
            "forward",
            Input {
                forward: f64::NAN,
                ..Input::default()
            },
        ),
        (
            "right",
            Input {
                right: f64::INFINITY,
                ..Input::default()
            },
        ),
        (
            "yaw_deg",
            Input {
                yaw_deg: f64::NEG_INFINITY,
                ..Input::default()
            },
        ),
    ] {
        let err = sim.set_input(a, input).unwrap_err();
        match &err {
            SimError::BadInput(why) => assert!(why.contains(field), "{why}"),
            other => panic!("{other:?}"),
        }
        assert!(sim.input(a).is_none());
    }
}

#[test]
fn input_is_clamped_to_the_unit_disc() {
    let mut world = flat_world(0.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(
        a,
        Input {
            forward: 30.0,
            right: -40.0,
            yaw_deg: 725.0,
            ..Input::default()
        },
    )
    .unwrap();
    let input = sim.input(a).unwrap();
    assert!((input.forward - 0.6).abs() < 1e-12);
    assert!((input.right - -0.8).abs() < 1e-12);
    assert!((input.yaw_deg - 5.0).abs() < 1e-9);
    // A small input is kept as it is.
    sim.set_input(
        a,
        Input {
            forward: 0.25,
            right: 0.5,
            ..Input::default()
        },
    )
    .unwrap();
    assert_eq!(sim.input(a).unwrap().forward, 0.25);
    assert_eq!(sim.input(a).unwrap().right, 0.5);
}

#[test]
fn input_for_a_missing_or_non_character_entity_is_refused() {
    let mut world = flat_world(0.0);
    let wall = add_shape(
        &mut world,
        ShapeKind::Box,
        [1.0; 3],
        [5.0, 0.0, 5.0],
        0.0,
        true,
    );
    let mut sim = Sim::new(world).unwrap();
    assert_eq!(
        sim.set_input(EntityId(999), Input::default()),
        Err(SimError::NoEntity(EntityId(999)))
    );
    assert_eq!(
        sim.set_input(wall, Input::default()),
        Err(SimError::NotACharacter(wall))
    );
}

#[test]
fn step_u32_max_advances_max_step_ticks() {
    let mut world = flat_world(0.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(
        a,
        Input {
            run: true,
            ..walk(45.0)
        },
    )
    .unwrap();
    sim.step(u32::MAX);
    assert_eq!(sim.tick(), u64::from(MAX_STEP_TICKS));
    assert!(pos(&sim, a).iter().all(|v| v.is_finite()));
    sim.world().validate().unwrap();
}

#[test]
fn a_world_without_terrain_falls_to_the_floor() {
    let mut world = World::new("void", 1);
    let a = add_character(&mut world, [0.0, 10.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    assert!(sim.heights().is_none());
    sim.set_input(a, walk(0.0)).unwrap();
    sim.step(MAX_STEP_TICKS);
    let [x, y, z] = pos(&sim, a);
    assert!(x.is_finite() && z.is_finite());
    assert_eq!(y, FALL_FLOOR_M);
    assert_eq!(sim.body(a).unwrap().velocity[1], 0.0);
    sim.world().validate().unwrap();
}

#[test]
fn a_box_in_the_void_is_ground() {
    let mut world = World::new("void", 1);
    add_shape(
        &mut world,
        ShapeKind::Box,
        [4.0, 1.0, 4.0],
        [0.0, 0.0, 0.0],
        0.0,
        true,
    );
    let a = add_character(&mut world, [0.0, 5.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.step(300);
    assert_eq!(pos(&sim, a)[1], 1.0);
    assert!(sim.body(a).unwrap().grounded);
}

#[test]
fn an_empty_world_steps() {
    let mut sim = Sim::new(World::new("", 0)).unwrap();
    sim.step(100);
    assert_eq!(sim.tick(), 100);
    assert_eq!(sim.bodies().count(), 0);
}

#[test]
fn extreme_characters_stay_finite_and_valid() {
    let mut world = World::new("extreme", 1);
    let a = world
        .add_entity(gamecraft_world::EntityDraft {
            position: [999_999.0, 999_999.0, -999_999.0],
            components: vec![gamecraft_world::Component::Character(
                gamecraft_world::Character {
                    walk_speed_mps: 1e300,
                    run_speed_mps: 1e300,
                    jump_speed_mps: 1e300,
                    ..gamecraft_world::Character::default()
                },
            )],
            ..gamecraft_world::EntityDraft::default()
        })
        .unwrap();
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(
        a,
        Input {
            forward: 1.0,
            right: 1.0,
            yaw_deg: 10.0,
            run: true,
            jump: true,
        },
    )
    .unwrap();
    for _ in 0..50 {
        sim.step(7);
        assert!(pos(&sim, a).iter().all(|v| v.is_finite()));
        assert!(sim.body(a).unwrap().velocity.iter().all(|v| v.is_finite()));
        sim.world().validate().unwrap();
    }
}

#[test]
fn a_character_entity_with_its_own_shape_is_not_blocked_by_it() {
    let mut world = flat_world(0.0);
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    world
        .entity_mut(a)
        .unwrap()
        .set_component(gamecraft_world::Component::Shape(gamecraft_world::Shape {
            kind: ShapeKind::Cylinder,
            size: [0.6, 1.8, 0.6],
            color: [1.0, 0.0, 0.0],
            solid: true,
        }));
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(a, walk(0.0)).unwrap();
    sim.step(60);
    assert!((pos(&sim, a)[2] - -1.6).abs() < 1e-9);
}

#[test]
fn input_round_trips_through_json_and_refuses_unknown_fields() {
    let input = Input {
        forward: 0.5,
        right: -0.25,
        yaw_deg: 90.0,
        run: true,
        jump: false,
    };
    let text = serde_json::to_string(&input).unwrap();
    assert_eq!(serde_json::from_str::<Input>(&text).unwrap(), input);
    assert_eq!(
        serde_json::from_str::<Input>("{\"forward\": 1}").unwrap(),
        Input {
            forward: 1.0,
            ..Input::default()
        }
    );
    assert!(serde_json::from_str::<Input>("{\"fwd\": 1}").is_err());
}
