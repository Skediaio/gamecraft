//! Magazines that keep their rounds, a launcher that does not reload by itself, a reload that
//! takes the weapon off the sights, and a shot that leaves from the muzzle of the held model.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, flat_world};
use gamecraft_sim::{DT, Input, Sim};
use gamecraft_world::{Component, EntityId, FireMode, HeldModel, Launcher, World};

/// A rifle without spread that fires every tenth of a second, 10 rounds a magazine, reloads in
/// one second.
fn rifle() -> Launcher {
    Launcher {
        dispersion_mrad: 0.0,
        rate_s: 0.1,
        magazine: 10,
        reload_s: 1.0,
        modes: vec![FireMode::Auto],
        muzzle_velocity_mps: 870.0,
        mass_kg: 0.004,
        ballistic_coefficient: 135.0,
        max_flight_s: 3.0,
        ..Launcher::default()
    }
}

fn armed(launcher: Launcher, held: Option<HeldModel>) -> (World, EntityId) {
    let mut w = flat_world(0.0);
    w.environment.wind_speed_mps = 0.0;
    let id = add_character(&mut w, [0.0, 0.0, 0.0]);
    let e = w.entity_mut(id).unwrap();
    e.set_component(Component::Launcher(launcher));
    if let Some(h) = held {
        e.set_component(Component::HeldModel(h));
    }
    (w, id)
}

fn hold(sim: &mut Sim, id: EntityId, input: Input, ticks: u32) {
    sim.set_input(id, input).unwrap();
    sim.step(ticks);
}

fn trigger() -> Input {
    Input {
        fire: true,
        ..Input::default()
    }
}

fn reload() -> Input {
    Input {
        reload: true,
        ..Input::default()
    }
}

fn magazine(sim: &Sim, id: EntityId) -> (u32, Option<Vec<u32>>, bool) {
    let m = sim.magazine(id).unwrap();
    (m.left, m.pouch_rounds(), m.reloading)
}

/// Fires `n` rounds with a pause that lets each shot's interval run out.
fn fire_rounds(sim: &mut Sim, id: EntityId, n: u32) {
    for _ in 0..n {
        hold(sim, id, trigger(), 1);
        hold(sim, id, Input::default(), 6);
    }
}

#[test]
fn a_reload_keeps_the_partly_used_magazine_and_loads_the_fullest() {
    let (w, id) = armed(
        Launcher {
            spare_magazines: Some(2),
            auto_reload: false,
            ..rifle()
        },
        None,
    );
    let mut sim = Sim::new(w).unwrap();
    assert_eq!(magazine(&sim, id), (10, Some(vec![10, 10]), false));
    // Four rounds fired: the six left go in the pouch, behind the full one.
    fire_rounds(&mut sim, id, 4);
    assert_eq!(magazine(&sim, id), (6, Some(vec![10, 10]), false));
    hold(&mut sim, id, reload(), 1);
    assert_eq!(magazine(&sim, id), (0, Some(vec![10, 6]), true));
    assert_eq!(sim.magazine(id).unwrap().spare, Some(2));
    assert_eq!(sim.magazine(id).unwrap().loading, 10);
    hold(&mut sim, id, Input::default(), 60);
    assert_eq!(magazine(&sim, id), (10, Some(vec![10, 6]), false));
    // Seven more: three left; the fullest (10) goes in, three go back.
    fire_rounds(&mut sim, id, 7);
    hold(&mut sim, id, reload(), 61);
    assert_eq!(magazine(&sim, id), (10, Some(vec![6, 3]), false));
    // A magazine with fewer rounds than the launcher holds is not worth swapping for.
    fire_rounds(&mut sim, id, 6);
    assert_eq!(sim.magazine(id).unwrap().left, 4);
    hold(&mut sim, id, Input::default(), 1);
    hold(&mut sim, id, reload(), 1);
    assert_eq!(magazine(&sim, id), (0, Some(vec![4, 3]), true), "6 beats 4");
    hold(&mut sim, id, Input::default(), 60);
    assert_eq!(magazine(&sim, id), (6, Some(vec![4, 3]), false));
    // Run it dry: the empty magazine is dropped, not kept.
    fire_rounds(&mut sim, id, 6);
    assert_eq!(magazine(&sim, id), (0, Some(vec![4, 3]), false));
    hold(&mut sim, id, reload(), 61);
    hold(&mut sim, id, Input::default(), 1);
    assert_eq!(magazine(&sim, id), (4, Some(vec![3]), false));
    fire_rounds(&mut sim, id, 4);
    hold(&mut sim, id, reload(), 61);
    hold(&mut sim, id, Input::default(), 1);
    fire_rounds(&mut sim, id, 3);
    assert_eq!(magazine(&sim, id), (0, Some(vec![]), false));
    assert_eq!(sim.magazine(id).unwrap().spare, Some(0));
    // Nothing left to load: a reload does nothing.
    hold(&mut sim, id, reload(), 120);
    assert_eq!(magazine(&sim, id), (0, Some(vec![]), false));
}

#[test]
fn a_pouch_magazine_with_fewer_rounds_is_not_swapped_in_and_unlimited_stays_as_before() {
    let (w, id) = armed(
        Launcher {
            spare_magazines: Some(1),
            ..rifle()
        },
        None,
    );
    let mut sim = Sim::new(w).unwrap();
    fire_rounds(&mut sim, id, 9);
    hold(&mut sim, id, reload(), 61);
    hold(&mut sim, id, Input::default(), 1);
    assert_eq!(magazine(&sim, id), (10, Some(vec![1]), false));
    fire_rounds(&mut sim, id, 2);
    // 8 in the launcher, 1 in the pouch: nothing to gain.
    hold(&mut sim, id, reload(), 61);
    assert_eq!(magazine(&sim, id), (8, Some(vec![1]), false));
    // Unlimited: no pouch, a reload fills up after its time and the old rounds are gone.
    let (w, id) = armed(rifle(), None);
    let mut sim = Sim::new(w).unwrap();
    fire_rounds(&mut sim, id, 4);
    hold(&mut sim, id, reload(), 1);
    assert_eq!(magazine(&sim, id), (6, None, true));
    assert_eq!(sim.magazine(id).unwrap().spare, None);
    hold(&mut sim, id, Input::default(), 61);
    assert_eq!(magazine(&sim, id), (10, None, false));
}

#[test]
fn a_pouch_of_used_magazines_is_hashed_and_replays_alike() {
    let run = |partial: bool| {
        let (w, id) = armed(
            Launcher {
                spare_magazines: Some(2),
                ..rifle()
            },
            None,
        );
        let mut sim = Sim::new(w).unwrap();
        let mut hashes = Vec::new();
        if partial {
            fire_rounds(&mut sim, id, 4);
        } else {
            hold(&mut sim, id, Input::default(), 28);
        }
        hold(&mut sim, id, reload(), 61);
        hold(&mut sim, id, Input::default(), 5);
        hashes.push(sim.state_hash());
        hashes
    };
    assert_eq!(run(true), run(true));
    assert_ne!(
        run(true),
        run(false),
        "a used magazine in the pouch is state"
    );
}

#[test]
fn without_auto_reload_an_empty_launcher_clicks_until_asked() {
    let (w, id) = armed(
        Launcher {
            magazine: 3,
            auto_reload: false,
            ..rifle()
        },
        None,
    );
    let mut sim = Sim::new(w).unwrap();
    hold(&mut sim, id, trigger(), 60);
    let m = sim.magazine(id).unwrap();
    assert_eq!((m.shots, m.left, m.reloading), (3, 0, false));
    // Held for more seconds: still nothing, and no reload by itself.
    hold(&mut sim, id, trigger(), 240);
    let m = sim.magazine(id).unwrap();
    assert_eq!((m.shots, m.left, m.reloading), (3, 0, false));
    hold(&mut sim, id, reload(), 1);
    assert!(sim.magazine(id).unwrap().reloading);
    hold(&mut sim, id, trigger(), 61);
    assert!(sim.magazine(id).unwrap().shots > 3, "it fires again");
    // With it on (the default), the same rifle reloads by itself.
    let (w, id) = armed(
        Launcher {
            magazine: 3,
            ..rifle()
        },
        None,
    );
    assert!(w.entity(id).unwrap().launcher().unwrap().auto_reload);
    let mut sim = Sim::new(w).unwrap();
    hold(&mut sim, id, trigger(), 60);
    let m = sim.magazine(id).unwrap();
    assert!(m.shots > 3 || m.reloading, "{m:?}");
}

#[test]
fn a_reload_takes_the_weapon_off_the_sights_and_nothing_fires_meanwhile() {
    let held = HeldModel {
        ads_offset_m: Some([0.0, -0.05, 0.25]),
        ads_time_s: 0.5,
        ..HeldModel::of("models/rifle.glb")
    };
    let (w, id) = armed(rifle(), Some(held));
    let mut sim = Sim::new(w).unwrap();
    let aim = Input {
        ads: true,
        ..Input::default()
    };
    let amount = |sim: &Sim| sim.handling(id).unwrap().ads_amount;
    // Up to the sights in ads_time_s (30 ticks): linear.
    hold(&mut sim, id, aim, 15);
    assert!((amount(&sim) - 0.5).abs() < 1e-9, "{}", amount(&sim));
    hold(&mut sim, id, aim, 25);
    assert_eq!(amount(&sim), 1.0);
    assert!(sim.handling(id).unwrap().ads);
    // Fire a round, then ask for a reload with the sights still held.
    hold(&mut sim, id, Input { fire: true, ..aim }, 1);
    hold(&mut sim, id, aim, 10);
    let asked = Input {
        reload: true,
        ..aim
    };
    hold(&mut sim, id, asked, 1);
    let h = *sim.handling(id).unwrap();
    assert!(!h.ads, "the input holds ads, the reload ignores it");
    assert!((h.ads_amount - (1.0 - DT / 0.5)).abs() < 1e-9);
    // Halfway through the lowering it is at half; the reload takes 60 ticks.
    hold(&mut sim, id, aim, 14);
    assert!((amount(&sim) - 0.5).abs() < 1e-9, "{}", amount(&sim));
    let shots = sim.magazine(id).unwrap().shots;
    hold(&mut sim, id, Input { fire: true, ..aim }, 20);
    assert_eq!(
        sim.magazine(id).unwrap().shots,
        shots,
        "no shot while reloading"
    );
    assert_eq!(amount(&sim), 0.0, "all the way down within ads_time_s");
    // The reload ends (60 ticks after it began); the sights come back up while ads is held.
    hold(&mut sim, id, aim, 40);
    assert!(!sim.magazine(id).unwrap().reloading);
    assert!(sim.handling(id).unwrap().ads);
    hold(&mut sim, id, aim, 10);
    assert!(amount(&sim) > 0.0 && amount(&sim) < 1.0);
    hold(&mut sim, id, aim, 30);
    assert_eq!(amount(&sim), 1.0);
}

#[test]
fn the_reload_progress_runs_from_zero_to_one() {
    let (w, id) = armed(rifle(), None);
    let mut sim = Sim::new(w).unwrap();
    fire_rounds(&mut sim, id, 2);
    assert_eq!(sim.magazine(id).unwrap().reload_progress(), 0.0);
    hold(&mut sim, id, reload(), 1);
    hold(&mut sim, id, Input::default(), 29);
    let p = sim.magazine(id).unwrap().reload_progress();
    assert!((p - 0.5).abs() < 0.03, "{p}");
    hold(&mut sim, id, Input::default(), 40);
    assert_eq!(sim.magazine(id).unwrap().reload_progress(), 0.0, "done");
    assert!(!sim.magazine(id).unwrap().reloading);
}

fn rifle_model() -> HeldModel {
    HeldModel {
        offset_m: [0.1, -0.2, 0.3],
        ads_offset_m: Some([0.0, -0.05, 0.25]),
        ads_time_s: 0.0,
        scale: 1.0,
        muzzle_m: Some([0.0, 0.01, 0.8]),
        ..HeldModel::of("models/rifle.glb")
    }
}

#[test]
fn a_shot_leaves_from_the_muzzle_at_the_hip_and_at_the_sights() {
    let (w, id) = armed(rifle(), Some(rifle_model()));
    let eye = w.entity(id).unwrap().character().unwrap().eye_height_m;
    let mut sim = Sim::new(w).unwrap();
    // At the hip, facing north: the offset plus the muzzle, 1.1 m forward.
    sim.fire(id, Some(0.0), Some(0.0)).unwrap();
    let p = sim.projectiles()[0].position;
    let want = [0.1, eye - 0.2 + 0.01, -1.1];
    assert!(
        p.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9),
        "{p:?}"
    );
    // At the sights: its offset (instant: ads_time_s is 0).
    let mut sim = Sim::new(armed(rifle(), Some(rifle_model())).0).unwrap();
    let aim = Input {
        ads: true,
        ..Input::default()
    };
    hold(&mut sim, id, aim, 2);
    assert_eq!(sim.handling(id).unwrap().ads_amount, 1.0);
    sim.fire(id, Some(0.0), Some(0.0)).unwrap();
    let p = sim.projectiles()[0].position;
    let want = [0.0, eye - 0.05 + 0.01, -1.05];
    assert!(
        p.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9),
        "{p:?}"
    );
    // Through the trigger, turned to the west (yaw 90): right is north.
    let mut sim = Sim::new(armed(rifle(), Some(rifle_model())).0).unwrap();
    let west = Input {
        fire: true,
        yaw_deg: 90.0,
        ..Input::default()
    };
    hold(&mut sim, id, west, 1);
    let p = sim.projectiles()[0].position;
    // It has flown one tick; the muzzle was 1.1 m west and 0.1 m north of the eye's foot, so
    // the projectile is further west and still within 1 cm of that line sideways.
    assert!(p[0] < -1.1, "{p:?}");
    assert!((p[2] + 0.1).abs() < 0.01, "{p:?}");
}

#[test]
fn a_character_without_a_muzzle_shoots_from_the_eye_as_before() {
    let plain = HeldModel {
        muzzle_m: None,
        ..rifle_model()
    };
    for held in [None, Some(plain)] {
        let (w, id) = armed(rifle(), held);
        let eye = w.entity(id).unwrap().character().unwrap().eye_height_m;
        let mut sim = Sim::new(w).unwrap();
        sim.fire(id, Some(0.0), Some(0.0)).unwrap();
        assert_eq!(sim.projectiles()[0].position, [0.0, eye, 0.0]);
    }
}

/// Where the projectile of a shot fired level at the sights crosses the plane `distance_m`
/// north of the eye: (x, y) relative to the eye.
fn crossing(sim: &mut Sim, eye_y: f64, distance_m: f64) -> (f64, f64) {
    let mut last = sim.projectiles()[0].position;
    for _ in 0..600 {
        sim.step(1);
        let Some(p) = sim.projectiles().first().map(|p| p.position) else {
            break;
        };
        if p[2] <= -distance_m {
            let k = (last[2] + distance_m) / (last[2] - p[2]);
            return (
                last[0] + k * (p[0] - last[0]),
                last[1] + k * (p[1] - last[1]) - eye_y,
            );
        }
        last = p;
    }
    panic!("the shot never reached {distance_m} m");
}

#[test]
fn a_shot_at_the_sights_crosses_the_line_of_sight_at_the_zeroing_distance() {
    for (zero, offset_right) in [(100.0, 0.0), (50.0, 0.0), (200.0, 0.0), (100.0, 0.03)] {
        let held = HeldModel {
            zero_m: Some(zero),
            ads_offset_m: Some([offset_right, -0.05, 0.25]),
            ..rifle_model()
        };
        let (w, id) = armed(rifle(), Some(held));
        let eye = w.entity(id).unwrap().character().unwrap().eye_height_m;
        let mut sim = Sim::new(w).unwrap();
        let aim = Input {
            ads: true,
            ..Input::default()
        };
        hold(&mut sim, id, aim, 2);
        sim.fire(id, Some(0.0), Some(0.0)).unwrap();
        let (x, y) = crossing(&mut sim, eye, zero);
        assert!(
            x.abs() < 0.01 && y.abs() < 0.01,
            "zero {zero} m, sights {offset_right} m right: off by ({x}, {y}) m"
        );
    }
}

#[test]
fn a_zeroed_shot_replays_to_the_same_hash_and_the_default_zero_is_a_hundred_metres() {
    let run = || {
        let (w, id) = armed(rifle(), Some(rifle_model()));
        let mut sim = Sim::new(w).unwrap();
        let aim = Input {
            ads: true,
            fire: true,
            pitch_deg: 2.0,
            ..Input::default()
        };
        hold(&mut sim, id, aim, 20);
        sim.state_hash()
    };
    assert_eq!(run(), run());
    // Left out, zero_m is 100: the same as saying so.
    let at = |zero: Option<f64>| {
        let held = HeldModel {
            zero_m: zero,
            ..rifle_model()
        };
        let (w, id) = armed(rifle(), Some(held));
        let eye = w.entity(id).unwrap().character().unwrap().eye_height_m;
        let mut sim = Sim::new(w).unwrap();
        hold(
            &mut sim,
            id,
            Input {
                ads: true,
                ..Input::default()
            },
            2,
        );
        sim.fire(id, Some(0.0), Some(0.0)).unwrap();
        crossing(&mut sim, eye, 100.0)
    };
    assert_eq!(at(None), at(Some(100.0)));
    assert!(at(Some(100.0)).1.abs() < 0.01);
    assert!(
        at(Some(300.0)).1 > 0.05,
        "zeroed farther, it flies high at 100 m"
    );
}

#[test]
fn a_running_shooter_fires_from_the_muzzle_of_this_tick_and_lends_its_speed() {
    let (w, id) = armed(rifle(), Some(rifle_model()));
    let eye_h = w.entity(id).unwrap().character().unwrap().eye_height_m;
    let mut sim = Sim::new(w).unwrap();
    let run = Input {
        forward: 1.0,
        run: true,
        ..Input::default()
    };
    hold(&mut sim, id, run, 30);
    let speed = {
        let v = sim.body(id).unwrap().velocity;
        (v[0] * v[0] + v[2] * v[2]).sqrt()
    };
    assert!(speed > 3.0, "it is running: {speed} m/s");
    // Between ticks the shot leaves from where the muzzle is now, not a tick ago.
    let at = sim.world().entity(id).unwrap().position;
    sim.fire(id, Some(0.0), Some(0.0)).unwrap();
    let p = sim.projectiles()[0].clone();
    let want = [at[0] + 0.1, at[1] + eye_h - 0.19, at[2] - 1.1];
    assert!(
        p.position
            .iter()
            .zip(want)
            .all(|(a, b)| (a - b).abs() < 1e-3),
        "{:?} vs {want:?}",
        p.position
    );
    // It carries the shooter's speed on top of the muzzle velocity (forward is -z).
    let along = -p.velocity[2];
    assert!(
        along > 870.0 * 0.999 + speed - 1.0,
        "{along} at {speed} m/s"
    );
    // Through the trigger: it starts at the muzzle of the tick it is fired in, after that
    // tick's step of the body (sideways, where nothing acts on it, it is exact).
    let fire = Input { fire: true, ..run };
    let mut sim = Sim::new(armed(rifle(), Some(rifle_model())).0).unwrap();
    hold(&mut sim, id, run, 30);
    hold(&mut sim, id, fire, 1);
    let at = sim.world().entity(id).unwrap().position;
    let p = sim.projectiles()[0].position;
    assert!((p[0] - (at[0] + 0.1)).abs() < 1e-3, "{p:?} from {at:?}");
    // A model without a muzzle lends no speed (as before).
    let plain = HeldModel {
        muzzle_m: None,
        ..rifle_model()
    };
    let mut sim = Sim::new(armed(rifle(), Some(plain)).0).unwrap();
    hold(&mut sim, id, run, 30);
    sim.fire(id, Some(0.0), Some(0.0)).unwrap();
    assert!((-sim.projectiles()[0].velocity[2] - 870.0).abs() < 1e-6);
}
