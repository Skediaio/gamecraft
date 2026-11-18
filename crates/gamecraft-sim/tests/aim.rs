//! Firing solutions (`gamecraft_sim::aim`): found by flying the simulation's own flight model,
//! checked against the closed-form vacuum solution, the wind, and a real shot.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::flat_world;
use gamecraft_math::{Portable, deg_to_rad, rad_to_deg};
use gamecraft_sim::aim::{AIM_TOLERANCE_M, aim_point, solve};
use gamecraft_sim::combat::muzzle;
use gamecraft_sim::{EventKind, GRAVITY_MPS2, Sim};
use gamecraft_world::limits::BALLISTIC_COEFFICIENT_MAX;
use gamecraft_world::{Character, Component, EntityDraft, EntityId, Health, Launcher, World};

/// A slow launcher with almost no drag (the largest ballistic coefficient).
fn mortar() -> Launcher {
    Launcher {
        muzzle_velocity_mps: 100.0,
        ballistic_coefficient: BALLISTIC_COEFFICIENT_MAX,
        dispersion_mrad: 0.0,
        max_flight_s: 60.0,
        ..Launcher::default()
    }
}

fn rifle() -> Launcher {
    Launcher {
        muzzle_velocity_mps: 800.0,
        mass_kg: 0.0109,
        ballistic_coefficient: 199.6,
        dispersion_mrad: 0.0,
        ..Launcher::default()
    }
}

#[test]
fn without_wind_or_drag_it_is_the_closed_form_solution() {
    let mut env = World::new("air", 1).environment;
    env.wind_speed_mps = 0.0;
    let mut world = World::new("air", 1);
    world.environment = env;
    let launcher = mortar();
    let from = [0.0, 1.5, 0.0];
    let v = launcher.muzzle_velocity_mps;
    for (x, dy) in [(300.0, 0.0), (500.0, 20.0), (150.0, -10.0)] {
        let target = [0.0, 1.5 + dy, -x];
        let s = solve(&world, None, from, &launcher, target);
        assert!(s.reachable, "{x} m: {s:?}");
        assert!(s.miss_m.unwrap() <= AIM_TOLERANCE_M, "{s:?}");
        // The low root of tan(theta) = (v^2 - sqrt(v^4 - g (g x^2 + 2 dy v^2))) / (g x).
        let g = GRAVITY_MPS2;
        let root = (v * v * v * v - g * (g * x * x + 2.0 * dy * v * v)).sqrt();
        let want = rad_to_deg(((v * v - root) / (g * x)).patan());
        assert!(
            (s.pitch_deg - want).abs() < 0.02,
            "{x} m: pitch {} vs {want}",
            s.pitch_deg
        );
        assert!(
            s.yaw_deg.abs() < 1e-6 || (s.yaw_deg - 360.0).abs() < 1e-6,
            "{s:?}"
        );
        let tof = x / (v * deg_to_rad(s.pitch_deg).pcos());
        assert!((s.time_of_flight_s - tof).abs() < 0.01, "{s:?} vs {tof}");
        // The drop below the line of departure is g t^2 / 2; no wind, no drift.
        let drop = 0.5 * g * tof * tof;
        assert!((s.drop_m - drop).abs() < 0.05, "{s:?} vs {drop}");
        assert!(s.drift_m.abs() < 1e-6, "{s:?}");
    }
    // Beyond the longest reach (v^2 / g, about 1019 m) there is no solution.
    let s = solve(&world, None, from, &launcher, [0.0, 1.5, -1500.0]);
    assert!(!s.reachable, "{s:?}");
}

#[test]
fn with_wind_the_yaw_leads_into_it() {
    let mut world = World::new("wind", 1);
    world.environment.wind_speed_mps = 10.0;
    world.environment.wind_direction_deg = 270.0; // from the west
    let from = [0.0, 1.5, 0.0];
    let target = [0.0, 1.5, -400.0];
    let s = solve(&world, None, from, &rifle(), target);
    assert!(s.reachable && s.miss_m.unwrap() <= AIM_TOLERANCE_M, "{s:?}");
    // Firing north with a wind from the left: aim left (west), counter-clockwise, so a small
    // positive yaw; the shot drifts right (east) of the line of departure.
    assert!(s.yaw_deg > 0.0 && s.yaw_deg < 2.0, "{s:?}");
    assert!(s.drift_m > 0.05, "{s:?}");
    assert!(s.drop_m > 0.5 && s.pitch_deg > 0.0, "{s:?}");
    // The mirror wind mirrors the yaw.
    world.environment.wind_direction_deg = 90.0;
    let m = solve(&world, None, from, &rifle(), target);
    assert!(
        (m.yaw_deg - (360.0 - s.yaw_deg)).abs() < 1e-3,
        "{m:?} vs {s:?}"
    );
    assert!((m.drift_m + s.drift_m).abs() < 1e-3, "{m:?} vs {s:?}");
}

/// A character entity with optional launcher and health at `at`.
fn add(w: &mut World, name: &str, at: [f64; 2], launcher: Option<Launcher>) -> EntityId {
    let y = w
        .terrain
        .as_ref()
        .unwrap()
        .build()
        .unwrap()
        .height_at(at[0], at[1])
        .unwrap();
    let mut components = vec![
        Component::Character(Character::default()),
        Component::Health(Health {
            max: 100.0,
            current: 100.0,
            armor: 0.0,
        }),
    ];
    components.extend(launcher.map(Component::Launcher));
    w.add_entity(EntityDraft {
        name: name.into(),
        position: [at[0], y, at[1]],
        components,
        ..EntityDraft::default()
    })
    .unwrap()
}

#[test]
fn the_solution_fired_without_dispersion_hits_its_target() {
    for (wind, range, side) in [(0.0, 120.0, 0.0), (12.0, 200.0, 40.0), (6.0, 90.0, -60.0)] {
        let mut w = flat_world(12.0);
        w.environment.wind_speed_mps = wind;
        w.environment.wind_direction_deg = 300.0;
        let shooter = add(&mut w, "Shooter", [0.0, 100.0], Some(rifle()));
        let target = add(&mut w, "Target", [side, 100.0 - range], None);
        let mut sim = Sim::new(w).unwrap();
        sim.step(1); // settle on the ground
        let world = sim.world();
        let me = world.entity(shooter).unwrap();
        let launcher = me.launcher().unwrap();
        let aim_at = aim_point(world.entity(target).unwrap());
        let s = solve(world, sim.heights(), muzzle(me, launcher), launcher, aim_at);
        assert!(s.reachable, "{s:?}");
        sim.fire(shooter, Some(s.yaw_deg), Some(s.pitch_deg))
            .unwrap();
        let mut hit = None;
        for _ in 0..120 {
            sim.step(1);
            hit = sim.take_events().into_iter().find_map(|e| match e.kind {
                EventKind::Hit { target, point, .. } => Some((target, point)),
                _ => None,
            });
            if hit.is_some() {
                break;
            }
        }
        let (hit_target, point) = hit.expect("no hit");
        assert_eq!(hit_target, Some(target), "wind {wind}: hit {point:?}");
        // It enters the capsule close to the chest.
        assert!(
            (point[1] - aim_at[1]).abs() < 0.1,
            "{point:?} vs {aim_at:?}"
        );
    }
}
