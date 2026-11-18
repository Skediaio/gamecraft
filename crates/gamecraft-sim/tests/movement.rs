//! Characters on terrain: walking, falling, jumping, slopes, the edge of the world.

// Integration tests are compiled as their own crates, outside `cfg(test)`, so clippy.toml's
// test allowances do not reach their helper functions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, add_character_with, flat_world, noise_world, pos, settle, walk};
use gamecraft_math::{Portable, Rng, rad_to_deg};
use gamecraft_sim::{DT, GRAVITY_MPS2, Input, Sim};
use gamecraft_world::{Character, EntityId};

const TOL: f64 = 1e-9;

fn walk_case(input: Input, expect_dx: f64, expect_dz: f64) {
    let mut world = flat_world(0.0);
    let id = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(id, input).unwrap();
    sim.step(600);
    let [x, y, z] = pos(&sim, id);
    assert!((x - expect_dx).abs() < TOL, "x {x} wanted {expect_dx}");
    assert!(y.abs() < TOL, "y {y}");
    assert!((z - expect_dz).abs() < TOL, "z {z} wanted {expect_dz}");
}

#[test]
fn walk_north_closed_form() {
    let walk_speed = Character::default().walk_speed_mps;
    walk_case(walk(0.0), 0.0, -walk_speed * 600.0 * DT);
}

#[test]
fn walk_with_yaw_90_goes_west() {
    let walk_speed = Character::default().walk_speed_mps;
    walk_case(walk(90.0), -walk_speed * 600.0 * DT, 0.0);
}

#[test]
fn strafe_right_goes_east() {
    let walk_speed = Character::default().walk_speed_mps;
    let input = Input {
        right: 1.0,
        ..Input::default()
    };
    walk_case(input, walk_speed * 600.0 * DT, 0.0);
}

#[test]
fn run_uses_run_speed() {
    let run_speed = Character::default().run_speed_mps;
    let input = Input {
        run: true,
        ..walk(0.0)
    };
    walk_case(input, 0.0, -run_speed * 600.0 * DT);
}

#[test]
fn facing_follows_the_input_and_wraps() {
    let mut world = flat_world(0.0);
    let id = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(id, walk(-90.0)).unwrap();
    assert_eq!(sim.input(id).unwrap().yaw_deg, 270.0);
    sim.step(1);
    assert_eq!(sim.world().entity(id).unwrap().rotation_y_deg, 270.0);
}

#[test]
fn diagonal_is_as_fast_as_straight() {
    let mut world = flat_world(0.0);
    let id = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(
        id,
        Input {
            forward: 1.0,
            right: 1.0,
            yaw_deg: 30.0,
            ..Input::default()
        },
    )
    .unwrap();
    sim.step(600);
    let [x, _, z] = pos(&sim, id);
    let distance = (x * x + z * z).sqrt();
    let expected = Character::default().walk_speed_mps * 600.0 * DT;
    assert!(
        (distance - expected).abs() < TOL,
        "{distance} vs {expected}"
    );
    // Forward-right at yaw 30: the facing turned 45 degrees clockwise, so yaw -15.
    let yaw = rad_to_deg((-x).patan2(-z));
    assert!((yaw - -15.0).abs() < 1e-6, "{yaw}");
}

#[test]
fn no_input_means_no_motion_and_kept_rotation() {
    let mut world = flat_world(0.0);
    let id = add_character(&mut world, [3.0, 0.0, 4.0]);
    world.entity_mut(id).unwrap().rotation_y_deg = 33.0;
    let mut sim = Sim::new(world).unwrap();
    sim.step(100);
    let e = sim.world().entity(id).unwrap();
    assert_eq!(e.position, [3.0, 0.0, 4.0]);
    assert_eq!(e.rotation_y_deg, 33.0);
    assert!(sim.body(id).unwrap().grounded);
}

#[test]
fn fall_closed_form_and_landing() {
    let h = 20.0;
    let mut world = flat_world(1.0);
    let id = add_character(&mut world, [0.0, 1.0 + h, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    let mut n: u64 = 0;
    loop {
        sim.step(1);
        let body = *sim.body(id).unwrap();
        if body.grounded {
            break;
        }
        n += 1;
        let nf = n as f64;
        let expected = 1.0 + h - GRAVITY_MPS2 * DT * DT * nf * (nf + 1.0) / 2.0;
        let y = pos(&sim, id)[1];
        assert!(
            (y - expected).abs() < TOL,
            "tick {n}: y {y} wanted {expected}"
        );
        assert!(n < 10_000, "never landed");
    }
    // Lands exactly on the ground and stays there.
    assert_eq!(pos(&sim, id)[1], 1.0);
    assert_eq!(sim.body(id).unwrap().velocity, [0.0, 0.0, 0.0]);
    sim.step(120);
    assert_eq!(pos(&sim, id)[1], 1.0);
    assert!(sim.body(id).unwrap().grounded);
}

#[test]
fn jump_apex_and_landing_and_held_jump() {
    let mut world = flat_world(2.0);
    let id = add_character(&mut world, [0.0, 2.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    settle(&mut sim);
    sim.set_input(
        id,
        Input {
            jump: true,
            ..Input::default()
        },
    )
    .unwrap();
    let v = Character::default().jump_speed_mps;
    let mut apex: f64 = 2.0;
    let mut left_ground = false;
    let mut landings = 0;
    for _ in 0..400 {
        sim.step(1);
        let y = pos(&sim, id)[1];
        apex = apex.max(y);
        let grounded = sim.body(id).unwrap().grounded;
        if !grounded {
            left_ground = true;
        } else if left_ground {
            landings += 1;
            assert_eq!(y, 2.0, "lands on the ground height");
            left_ground = false;
        }
    }
    let ideal = v * v / (2.0 * GRAVITY_MPS2);
    assert!(
        (apex - 2.0 - ideal).abs() <= v * DT,
        "apex {} vs {ideal}",
        apex - 2.0
    );
    assert!(
        landings >= 2,
        "a held jump jumps again ({landings} landings)"
    );

    // Released: grounded again afterwards and stays.
    sim.set_input(id, Input::default()).unwrap();
    sim.step(200);
    assert!(sim.body(id).unwrap().grounded);
    assert_eq!(pos(&sim, id)[1], 2.0);
}

#[test]
fn never_below_the_ground_with_random_inputs() {
    let mut world = noise_world(3, 40.0, 50.0);
    let id = add_character(&mut world, [0.0, 30.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    let hf = sim.heights().unwrap().clone();
    let [x0, z0] = hf.min();
    let [x1, z1] = hf.max();
    let mut rng = Rng::new(42);
    for tick in 0..20_000 {
        if tick % 30 == 0 {
            let input = Input {
                forward: rng.range(-1.0, 1.0),
                right: rng.range(-1.0, 1.0),
                yaw_deg: rng.range(-720.0, 720.0),
                run: rng.below(2) == 1,
                jump: rng.below(4) == 0,
            };
            sim.set_input(id, input).unwrap();
        }
        sim.step(1);
        let e = sim.world().entity(id).unwrap();
        let [x, y, z] = e.position;
        let body = sim.body(id).unwrap();
        assert!(
            x.is_finite() && y.is_finite() && z.is_finite(),
            "tick {tick}"
        );
        assert!(e.rotation_y_deg.is_finite());
        assert!(body.velocity.iter().all(|v| v.is_finite()));
        assert!(
            (x0..=x1).contains(&x) && (z0..=z1).contains(&z),
            "tick {tick}: {x} {z}"
        );
        let ground = hf.height_at(x, z).unwrap();
        assert!(
            y >= ground - TOL,
            "tick {tick}: y {y} below ground {ground}"
        );
    }
}

#[test]
fn the_centre_stays_inside_the_terrain() {
    let mut world = flat_world(0.0);
    let id = add_character(&mut world, [120.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(id, walk(270.0)).unwrap(); // east
    sim.step(600);
    let [x, y, _] = pos(&sim, id);
    assert_eq!(x, 128.0);
    assert_eq!(y, 0.0);
}

/// A point on a noise terrain where the ground rises between `lo` and `hi` metres per metre,
/// at the centroid of a triangle, with the uphill yaw.
fn slope_spot(sim: &Sim, lo: f64, hi: f64) -> ([f64; 3], f64) {
    let hf = sim.heights().unwrap();
    let c = hf.cell_m();
    let [x0, z0] = hf.min();
    for row in 10..hf.rows() - 10 {
        for col in 10..hf.cols() - 10 {
            let x = x0 + (col as f64 + 2.0 / 3.0) * c;
            let z = z0 + (row as f64 + 1.0 / 3.0) * c;
            let [nx, ny, nz] = hf.normal_at(x, z).unwrap();
            let grade = (nx * nx + nz * nz).sqrt() / ny;
            if grade > lo && grade < hi {
                // Uphill is against the normal's horizontal part.
                let (dx, dz) = (-nx, -nz);
                let yaw = rad_to_deg((-dx).patan2(-dz));
                return ([x, hf.height_at(x, z).unwrap(), z], yaw);
            }
        }
    }
    panic!("no slope between {lo} and {hi}");
}

fn slope_sim(max_slope_deg: f64) -> (Sim, EntityId, f64) {
    let world = noise_world(5, 120.0, 40.0);
    let probe = Sim::new(world.clone()).unwrap();
    let (spot, yaw) = slope_spot(&probe, 0.45, 0.7); // about 24 to 35 degrees
    let mut world = world;
    let id = add_character_with(
        &mut world,
        spot,
        Character {
            max_slope_deg,
            ..Character::default()
        },
    );
    let mut sim = Sim::new(world).unwrap();
    settle(&mut sim);
    assert_eq!(pos(&sim, id), spot, "settles where it was put");
    (sim, id, yaw)
}

#[test]
fn a_slope_steeper_than_the_limit_is_not_climbed() {
    let (mut sim, id, yaw) = slope_sim(15.0);
    let start = pos(&sim, id);
    sim.set_input(id, walk(yaw)).unwrap();
    sim.step(60);
    assert_eq!(pos(&sim, id), start);
}

#[test]
fn a_gentler_slope_is_climbed_following_the_ground() {
    let (mut sim, id, yaw) = slope_sim(60.0);
    let start = pos(&sim, id);
    sim.set_input(id, walk(yaw)).unwrap();
    let hf = sim.heights().unwrap().clone();
    for tick in 0..30 {
        sim.step(1);
        let [x, y, z] = pos(&sim, id);
        assert!(sim.body(id).unwrap().grounded, "tick {tick}");
        assert!((y - hf.height_at(x, z).unwrap()).abs() < TOL, "tick {tick}");
    }
    let end = pos(&sim, id);
    assert!(
        end[1] > start[1] + 0.1,
        "climbed from {} to {}",
        start[1],
        end[1]
    );
}

#[test]
fn downhill_is_always_allowed() {
    let (mut sim, id, yaw) = slope_sim(5.0);
    let start = pos(&sim, id);
    sim.set_input(id, walk(yaw + 180.0)).unwrap();
    sim.step(1);
    let end = pos(&sim, id);
    assert!(end[1] < start[1]);
}

#[test]
fn zero_input_on_a_slope_stays_put() {
    let (mut sim, id, _) = slope_sim(45.0);
    let start = pos(&sim, id);
    sim.step(600);
    assert_eq!(pos(&sim, id), start);
}
