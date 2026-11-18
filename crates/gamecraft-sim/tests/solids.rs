//! Solid shapes: walls block, low boxes are steps, cylinders and spheres push.

// Integration tests are compiled as their own crates, outside `cfg(test)`, so clippy.toml's
// test allowances do not reach their helper functions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, add_shape, box_distance, flat_world, pos, walk};
use gamecraft_sim::Sim;
use gamecraft_world::{Character, ShapeKind};

const TOL: f64 = 1e-9;

fn radius() -> f64 {
    Character::default().radius_m
}

/// Walks a character north from `start` for `ticks`, checking `check` after every tick.
fn walk_north(
    world: gamecraft_world::World,
    start: [f64; 3],
    yaw: f64,
    ticks: u32,
    mut check: impl FnMut(u32, [f64; 3], &Sim),
) -> [f64; 3] {
    let mut world = world;
    let id = add_character(&mut world, start);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(id, walk(yaw)).unwrap();
    for tick in 0..ticks {
        sim.step(1);
        check(tick, pos(&sim, id), &sim);
    }
    pos(&sim, id)
}

#[test]
fn a_tall_box_blocks() {
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Box,
        [2.0, 3.0, 2.0],
        [0.0, 0.0, -3.0],
        0.0,
        true,
    );
    let end = walk_north(world, [0.0, 0.0, 0.0], 0.0, 600, |tick, p, _| {
        let d = box_distance([p[0], p[2]], [0.0, -3.0], [1.0, 1.0], 0.0);
        assert!(d >= radius() - TOL, "tick {tick}: distance {d}");
    });
    assert!((end[2] - (-2.0 + radius())).abs() < TOL, "{end:?}");
    assert!(end[0].abs() < TOL);
}

#[test]
fn a_rotated_box_blocks() {
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Box,
        [2.0, 3.0, 1.0],
        [0.0, 0.0, -3.0],
        30.0,
        true,
    );
    let end = walk_north(world, [0.2, 0.0, 0.0], 0.0, 600, |tick, p, _| {
        let d = box_distance([p[0], p[2]], [0.0, -3.0], [1.0, 0.5], 30.0);
        assert!(d >= radius() - TOL, "tick {tick}: distance {d}");
    });
    // It slid along the slanted face instead of passing through.
    let d = box_distance([end[0], end[2]], [0.0, -3.0], [1.0, 0.5], 30.0);
    assert!(d >= radius() - TOL);
}

#[test]
fn walking_past_a_corner_slides_around_it() {
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Box,
        [2.0, 3.0, 2.0],
        [0.0, 0.0, -3.0],
        0.0,
        true,
    );
    let end = walk_north(world, [1.1, 0.0, 0.0], 0.0, 600, |tick, p, _| {
        let d = box_distance([p[0], p[2]], [0.0, -3.0], [1.0, 1.0], 0.0);
        assert!(d >= radius() - TOL, "tick {tick}: distance {d}");
    });
    assert!(end[2] < -4.5, "got past the box: {end:?}");
    assert!(end[0] >= 1.0 + radius() - TOL, "{end:?}");
}

#[test]
fn a_non_solid_box_does_not_block() {
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Box,
        [2.0, 3.0, 2.0],
        [0.0, 0.0, -3.0],
        0.0,
        false,
    );
    let end = walk_north(world, [0.0, 0.0, 0.0], 0.0, 600, |_, _, _| {});
    assert!((end[2] - -16.0).abs() < TOL);
    assert_eq!(end[1], 0.0);
}

#[test]
fn a_low_box_is_a_step() {
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Box,
        [2.0, 0.25, 2.0],
        [0.0, 0.0, -3.0],
        0.0,
        true,
    );
    let mut on_top = 0;
    let end = walk_north(world, [0.0, 0.0, 0.0], 0.0, 600, |_, p, sim| {
        if (-4.0..=-2.0).contains(&p[2]) {
            assert_eq!(p[1], 0.25, "stands on the top at z {}", p[2]);
            on_top += 1;
        }
        assert!(p[1] >= 0.0);
        let _ = sim;
    });
    assert!(on_top > 0);
    // Stepped back down to the terrain on the far side.
    assert_eq!(end[1], 0.0);
    assert!((end[2] - -16.0).abs() < TOL);
}

#[test]
fn walking_off_a_tall_box_falls_to_the_terrain() {
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Box,
        [2.0, 3.0, 2.0],
        [0.0, 0.0, 0.0],
        0.0,
        true,
    );
    let mut fell = false;
    let end = walk_north(world, [0.0, 3.0, 0.0], 0.0, 600, |tick, p, sim| {
        let body = sim.bodies().next().unwrap().1;
        if !body.grounded && tick > 2 {
            fell = true;
        }
        if p[1] < 3.0 - 0.35 {
            let d = box_distance([p[0], p[2]], [0.0, 0.0], [1.0, 1.0], 0.0);
            assert!(d >= radius() - TOL, "tick {tick}: inside the box falling");
        }
    });
    assert!(fell);
    assert_eq!(end[1], 0.0);
}

#[test]
fn standing_on_a_tall_box_top() {
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Box,
        [2.0, 3.0, 2.0],
        [0.0, 0.0, 0.0],
        0.0,
        true,
    );
    let end = walk_north(world, [0.0, 3.0, 0.0], 0.0, 0, |_, _, _| {});
    assert_eq!(end, [0.0, 3.0, 0.0]);
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Box,
        [2.0, 3.0, 2.0],
        [0.0, 0.0, 0.0],
        0.0,
        true,
    );
    let id = add_character(&mut world, [0.0, 3.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.step(60);
    assert_eq!(pos(&sim, id), [0.0, 3.0, 0.0]);
    assert!(sim.body(id).unwrap().grounded);
}

#[test]
fn a_cylinder_blocks() {
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Cylinder,
        [2.0, 3.0, 2.0],
        [0.0, 0.0, -3.0],
        0.0,
        true,
    );
    let end = walk_north(world, [0.0, 0.0, 0.0], 0.0, 600, |tick, p, _| {
        let d = (p[0] * p[0] + (p[2] + 3.0) * (p[2] + 3.0)).sqrt();
        assert!(d >= 1.0 + radius() - TOL, "tick {tick}: {d}");
    });
    assert!((end[2] - (-2.0 + radius())).abs() < TOL, "{end:?}");
}

#[test]
fn walking_past_a_cylinder_slides_around_it() {
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Cylinder,
        [2.0, 3.0, 2.0],
        [0.0, 0.0, -3.0],
        0.0,
        true,
    );
    let end = walk_north(world, [0.5, 0.0, 0.0], 0.0, 600, |tick, p, _| {
        let d = (p[0] * p[0] + (p[2] + 3.0) * (p[2] + 3.0)).sqrt();
        assert!(d >= 1.0 + radius() - TOL, "tick {tick}: {d}");
    });
    assert!(end[2] < -4.5, "{end:?}");
}

#[test]
fn a_sphere_blocks_with_its_widest_circle() {
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Sphere,
        [2.0, 2.0, 2.0],
        [0.0, 0.0, -3.0],
        0.0,
        true,
    );
    let end = walk_north(world, [0.0, 0.0, 0.0], 0.0, 600, |tick, p, _| {
        let d = (p[0] * p[0] + (p[2] + 3.0) * (p[2] + 3.0)).sqrt();
        assert!(d >= 1.0 + radius() - TOL, "tick {tick}: {d}");
    });
    assert!((end[2] - (-2.0 + radius())).abs() < TOL, "{end:?}");
}

#[test]
fn a_raised_sphere_blocks_with_the_circle_at_head_height() {
    // Sphere of radius 1 from y 1.5 to 3.5; the character spans 0 to 1.8, so the overlap is
    // 1.5 to 1.8 and the widest circle in it is at 1.8, 0.7 below the centre.
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Sphere,
        [2.0, 2.0, 2.0],
        [0.0, 1.5, -3.0],
        0.0,
        true,
    );
    let r = (1.0_f64 - 0.7 * 0.7).sqrt();
    let end = walk_north(world, [0.0, 0.0, 0.0], 0.0, 600, |_, _, _| {});
    assert!((end[2] - (-3.0 + r + radius())).abs() < 1e-6, "{end:?}");
}

#[test]
fn a_shape_above_the_head_does_not_block() {
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Box,
        [2.0, 1.0, 2.0],
        [0.0, 2.0, -3.0],
        0.0,
        true,
    );
    let end = walk_north(world, [0.0, 0.0, 0.0], 0.0, 600, |_, _, _| {});
    assert!((end[2] - -16.0).abs() < TOL);
}

#[test]
fn a_scaled_box_blocks_at_its_scaled_size() {
    let mut world = flat_world(0.0);
    let id = add_shape(
        &mut world,
        ShapeKind::Box,
        [1.0, 3.0, 1.0],
        [0.0, 0.0, -3.0],
        0.0,
        true,
    );
    world.entity_mut(id).unwrap().scale = 2.0;
    let end = walk_north(world, [0.0, 0.0, 0.0], 0.0, 600, |_, _, _| {});
    assert!((end[2] - (-2.0 + radius())).abs() < TOL, "{end:?}");
}

#[test]
fn a_character_inside_a_wall_is_pushed_out_finitely() {
    // Centre exactly on the centre of a cylinder: pushed along a fixed axis.
    let mut world = flat_world(0.0);
    add_shape(
        &mut world,
        ShapeKind::Cylinder,
        [1.0, 3.0, 1.0],
        [5.0, 0.0, 5.0],
        0.0,
        true,
    );
    add_shape(
        &mut world,
        ShapeKind::Box,
        [1.0, 3.0, 1.0],
        [-5.0, 0.0, -5.0],
        0.0,
        true,
    );
    let a = add_character(&mut world, [5.0, 0.0, 5.0]);
    let b = add_character(&mut world, [-5.0, 0.0, -5.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.step(2);
    for id in [a, b] {
        let p = pos(&sim, id);
        assert!(p.iter().all(|v| v.is_finite()));
    }
    let pa = pos(&sim, a);
    let da = ((pa[0] - 5.0) * (pa[0] - 5.0) + (pa[2] - 5.0) * (pa[2] - 5.0)).sqrt();
    assert!(da >= 0.5 + radius() - TOL, "{pa:?}");
    let pb = pos(&sim, b);
    let db = box_distance([pb[0], pb[2]], [-5.0, -5.0], [0.5, 0.5], 0.0);
    assert!(db >= radius() - TOL, "{pb:?}");
}
