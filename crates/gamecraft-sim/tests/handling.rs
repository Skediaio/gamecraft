//! A launcher in a character's hands: fire modes and the cyclic rate, reloads, recoil that
//! stacks and recovers, sway that is bounded and deterministic, aiming down the sights and
//! holding the breath.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, flat_world};
use gamecraft_math::Portable;
use gamecraft_sim::handling::{RECOVERY_TIME_CONSTANTS, SWAY_SMOOTHING_S, drift, sway_phases};
use gamecraft_sim::{Breath, DT, GRAVITY_MPS2, Input, Sim, SimError};
use gamecraft_world::{Component, EntityId, FireMode, Launcher, Recoil, Sway, World};

/// A rifle without spread: every mode, 600 rounds a minute (one shot every 6 ticks), `semi`
/// at most every 0.25 s (15 ticks), 100 rounds, no recoil and no sway unless a test adds them.
fn rifle() -> Launcher {
    Launcher {
        dispersion_mrad: 0.0,
        rate_s: 0.25,
        magazine: 100,
        modes: vec![
            FireMode::Safe,
            FireMode::Semi,
            FireMode::Auto,
            FireMode::Burst,
        ],
        cyclic_rpm: Some(600.0),
        max_flight_s: 0.5,
        ..Launcher::default()
    }
}

/// A flat world with one standing character holding `launcher`.
fn armed(launcher: Launcher, seed: u64) -> (World, EntityId) {
    let mut w = flat_world(0.0);
    w.seed = seed;
    w.environment.wind_speed_mps = 0.0;
    let id = add_character(&mut w, [0.0, 0.0, 0.0]);
    w.entity_mut(id)
        .unwrap()
        .set_component(Component::Launcher(launcher));
    (w, id)
}

fn trigger(fire: bool) -> Input {
    Input {
        fire,
        ..Input::default()
    }
}

fn shots(sim: &Sim, id: EntityId) -> u64 {
    sim.magazine(id).unwrap().shots
}

/// Holds `input` for `ticks` ticks.
fn hold(sim: &mut Sim, id: EntityId, input: Input, ticks: u32) {
    sim.set_input(id, input).unwrap();
    sim.step(ticks);
}

#[test]
fn safe_never_fires_semi_once_per_pull_auto_at_the_cyclic_rate_and_burst_its_count() {
    let (w, id) = armed(rifle(), 3);
    let mut sim = Sim::new(w).unwrap();
    assert_eq!(sim.magazine(id).unwrap().mode, FireMode::Safe);
    // Safe: held a second, nothing; sim.fire is refused too, naming the modes.
    hold(&mut sim, id, trigger(true), 60);
    assert_eq!(shots(&sim, id), 0);
    let err = sim.fire(id, None, None).unwrap_err().to_string();
    assert!(err.contains("safe") && err.contains("\"semi\""), "{err}");
    hold(&mut sim, id, trigger(false), 1);

    // Semi: held a second, one shot; a second pull, a second shot.
    sim.set_fire_mode(id, FireMode::Semi).unwrap();
    hold(&mut sim, id, trigger(true), 60);
    assert_eq!(shots(&sim, id), 1, "one shot per pull");
    hold(&mut sim, id, trigger(false), 1);
    hold(&mut sim, id, trigger(true), 60);
    assert_eq!(shots(&sim, id), 2);
    // Pulls faster than rate_s: a pull every other tick for a second fires every 15 ticks
    // at most (a pull held through the wait fires when ready).
    hold(&mut sim, id, trigger(false), 1);
    let before = shots(&sim, id);
    for _ in 0..30 {
        hold(&mut sim, id, trigger(true), 1);
        hold(&mut sim, id, trigger(false), 1);
    }
    let fast = shots(&sim, id) - before;
    assert!(
        (3..=5).contains(&fast),
        "{fast} shots in a second of tapping"
    );

    // Auto: held a second at 600 rounds a minute, ten shots (ticks 1, 7, ..., 55).
    sim.set_fire_mode(id, FireMode::Auto).unwrap();
    hold(&mut sim, id, trigger(false), 30);
    let before = shots(&sim, id);
    hold(&mut sim, id, trigger(true), 60);
    assert_eq!(shots(&sim, id) - before, 10, "auto at the cyclic rate");

    // Burst: three per pull, however long it is held.
    sim.set_fire_mode(id, FireMode::Burst).unwrap();
    hold(&mut sim, id, trigger(false), 30);
    let before = shots(&sim, id);
    hold(&mut sim, id, trigger(true), 60);
    assert_eq!(shots(&sim, id) - before, 3);
    hold(&mut sim, id, trigger(false), 1);
    hold(&mut sim, id, trigger(true), 60);
    assert_eq!(shots(&sim, id) - before, 6);
}

#[test]
fn the_selector_switches_once_per_press_in_the_launcher_s_order() {
    let (w, id) = armed(rifle(), 3);
    let mut sim = Sim::new(w).unwrap();
    let switch = Input {
        switch_mode: true,
        ..Input::default()
    };
    let mode = |sim: &Sim| sim.magazine(id).unwrap().mode;
    hold(&mut sim, id, switch, 10);
    assert_eq!(mode(&sim), FireMode::Semi, "held, it switches once");
    for want in [
        FireMode::Auto,
        FireMode::Burst,
        FireMode::Safe,
        FireMode::Semi,
    ] {
        hold(&mut sim, id, Input::default(), 1);
        hold(&mut sim, id, switch, 1);
        assert_eq!(mode(&sim), want);
    }
    let err = {
        let (w, id) = armed(
            Launcher {
                modes: vec![FireMode::Semi],
                ..rifle()
            },
            3,
        );
        let mut sim = Sim::new(w).unwrap();
        sim.set_fire_mode(id, FireMode::Auto).unwrap_err()
    };
    assert!(matches!(err, SimError::BadInput(_)), "{err:?}");
    assert!(err.to_string().contains("\"semi\""), "{err}");
}

#[test]
fn a_reload_takes_reload_s_and_one_from_empty_reload_empty_s() {
    let launcher = Launcher {
        magazine: 5,
        reload_s: 1.0,
        reload_empty_s: Some(2.0),
        modes: vec![FireMode::Auto],
        ..rifle()
    };
    let (w, id) = armed(launcher, 3);
    let mut sim = Sim::new(w).unwrap();
    // Five shots at ticks 1, 7, 13, 19, 25; the magazine is then empty and reloads for 2 s.
    hold(&mut sim, id, trigger(true), 25);
    let m = sim.magazine(id).unwrap().clone();
    assert_eq!((m.shots, m.left, m.reloading), (5, 0, true));
    assert!((m.ready_in_s - 2.0).abs() < 1e-9, "{m:?}");
    hold(&mut sim, id, trigger(true), 119);
    assert_eq!(shots(&sim, id), 5, "still reloading after 119 ticks");
    hold(&mut sim, id, trigger(true), 1);
    let m = sim.magazine(id).unwrap().clone();
    assert_eq!((m.shots, m.left, m.reloading), (6, 4, false), "{m:?}");
    // Asked for with rounds left: reload_s, once per press.
    let reload = Input {
        reload: true,
        ..Input::default()
    };
    hold(&mut sim, id, reload, 1);
    let m = sim.magazine(id).unwrap().clone();
    assert!(m.reloading && (m.ready_in_s - 1.0).abs() < 1e-9, "{m:?}");
    hold(&mut sim, id, reload, 59);
    assert_eq!(sim.magazine(id).unwrap().left, 4, "held, it reloads once");
    hold(&mut sim, id, reload, 1);
    let m = sim.magazine(id).unwrap().clone();
    assert_eq!((m.left, m.reloading), (5, false), "{m:?}");
    // Full: a reload asked for changes nothing.
    hold(&mut sim, id, Input::default(), 1);
    hold(&mut sim, id, reload, 1);
    assert!(!sim.magazine(id).unwrap().reloading);
}

#[test]
fn recoil_kicks_stacks_in_automatic_fire_and_recovers_in_closed_form() {
    let kick = 2.0;
    let recovery_s = 0.3;
    let launcher = Launcher {
        modes: vec![FireMode::Semi, FireMode::Auto],
        recoil: Recoil {
            kick_up_mrad: kick,
            kick_side_mrad: 0.5,
            recovery_s,
        },
        ..rifle()
    };
    let decay = (-DT * RECOVERY_TIME_CONSTANTS / recovery_s).pexp();
    let (w, id) = armed(launcher, 9);
    let mut sim = Sim::new(w).unwrap();
    // One shot in the first tick: the kick, then a decay of `decay` per tick.
    hold(&mut sim, id, trigger(true), 1);
    let up = |sim: &Sim| sim.handling(id).unwrap().recoil_mrad[1];
    assert_eq!(up(&sim), kick);
    let side = sim.handling(id).unwrap().recoil_mrad[0];
    assert!(side.abs() <= 0.5 && side != 0.0, "{side}");
    hold(&mut sim, id, trigger(false), 9);
    let want = kick * decay.ppowi(9);
    assert!((up(&sim) - want).abs() < 1e-12, "{} vs {want}", up(&sim));
    // After recovery_s about 5 % is left: three time constants.
    hold(&mut sim, id, trigger(false), 9);
    assert!((up(&sim) / kick - (-RECOVERY_TIME_CONSTANTS).pexp()).abs() < 1e-12);
    hold(&mut sim, id, trigger(false), 600);
    assert!(up(&sim) < 1e-9, "recovered: {}", up(&sim));

    // Automatic fire every 6 ticks stacks: after the third shot the climb is
    // kick * (1 + d^6 + d^12).
    sim.set_fire_mode(id, FireMode::Auto).unwrap();
    let left_before = up(&sim);
    hold(&mut sim, id, trigger(true), 13);
    let want = kick * (1.0 + decay.ppowi(6) + decay.ppowi(12)) + left_before * decay.ppowi(13);
    assert!((up(&sim) - want).abs() < 1e-12, "{} vs {want}", up(&sim));
    // The shots climb: each leaves higher than the one before (no spread, level aim).
    let climbs: Vec<f64> = sim
        .projectiles()
        .iter()
        .rev()
        .take(3)
        .map(|p| p.velocity[1])
        .collect();
    assert!(climbs[0] > climbs[1] && climbs[1] > climbs[2], "{climbs:?}");
    // And it settles back.
    hold(&mut sim, id, trigger(false), 600);
    assert!(up(&sim) < 1e-9);
}

/// A rifle that sways 2 mrad standing.
fn swaying() -> Launcher {
    Launcher {
        sway: Sway {
            standing_mrad: 2.0,
            ..Sway::default()
        },
        ..rifle()
    }
}

/// The sway's path over `ticks` ticks of `input`.
fn sway_path(seed: u64, input: Input, ticks: u32) -> Vec<[f64; 2]> {
    let (w, id) = armed(swaying(), seed);
    let mut sim = Sim::new(w).unwrap();
    sim.set_input(id, input).unwrap();
    (0..ticks)
        .map(|_| {
            sim.step(1);
            sim.handling(id).unwrap().sway_mrad
        })
        .collect()
}

#[test]
fn sway_is_smooth_bounded_and_the_same_for_the_same_seed() {
    let path = sway_path(5, Input::default(), 1200);
    assert_eq!(
        path,
        sway_path(5, Input::default(), 1200),
        "same seed, same path"
    );
    assert_ne!(
        path,
        sway_path(6, Input::default(), 1200),
        "another seed, another"
    );
    let mut widest: f64 = 0.0;
    for (a, b) in path.iter().zip(path.iter().skip(1)) {
        for axis in 0..2 {
            widest = widest.max(b[axis].abs());
            assert!(
                b[axis].abs() <= 2.0 + 1e-12,
                "bounded by the amplitude: {b:?}"
            );
            assert!((b[axis] - a[axis]).abs() < 0.12, "smooth: {a:?} {b:?}");
        }
    }
    assert!(widest > 1.0, "it does wander: {widest}");
    // The path is the closed form: the amplitude times the unit drift of the seeded phases.
    let (_, id) = armed(swaying(), 5);
    let phases = sway_phases(5, id);
    let t = 1200.0 * DT;
    let last = path[1199];
    let amplitude = 2.0 * (1.0 - (-t / SWAY_SMOOTHING_S).pexp());
    let want = amplitude * drift(t, 4.0, &phases[..3]);
    assert!((last[0] - want).abs() < 1e-9, "{last:?} vs {want}");
}

#[test]
fn aiming_down_the_sights_and_holding_the_breath_steady_the_sway_and_running_shakes_it() {
    let amplitude = |input: Input, ticks: u32| {
        let (w, id) = armed(swaying(), 5);
        let mut sim = Sim::new(w).unwrap();
        sim.set_input(id, input).unwrap();
        sim.step(ticks);
        *sim.handling(id).unwrap()
    };
    let hip = amplitude(Input::default(), 120);
    let ads = amplitude(
        Input {
            ads: true,
            ..Input::default()
        },
        120,
    );
    assert!((hip.sway_amplitude_mrad - 2.0).abs() < 1e-3, "{hip:?}");
    assert!(
        (ads.sway_amplitude_mrad - 1.0).abs() < 1e-3,
        "ads_sway_factor 0.5: {ads:?}"
    );
    // Holding the breath: a fifth of the sway for 4 s, then worse (1.5 times) for 3 s.
    let breath = Input {
        hold_breath: true,
        ..Input::default()
    };
    let held = amplitude(breath, 200);
    assert_eq!(held.breath, Breath::Holding);
    assert!((held.sway_amplitude_mrad - 0.4).abs() < 1e-3, "{held:?}");
    let spent = amplitude(breath, 240 + 120);
    assert_eq!(spent.breath, Breath::Recovering);
    assert!((spent.sway_amplitude_mrad - 3.0).abs() < 1e-3, "{spent:?}");
    // Running builds fatigue (full after 6 s) and moving doubles the sway: 2 * 2 * 2.5.
    let run = Input {
        forward: 1.0,
        run: true,
        ..Input::default()
    };
    let tired = amplitude(run, 480);
    assert_eq!(tired.fatigue, 1.0);
    assert!((tired.sway_amplitude_mrad - 10.0).abs() < 1e-3, "{tired:?}");
}

#[test]
fn shots_leave_along_the_aim_plus_recoil_plus_sway_with_less_spread_down_the_sights() {
    let launcher = Launcher {
        dispersion_mrad: 4.0,
        ads_dispersion_factor: 0.25,
        modes: vec![FireMode::Auto],
        magazine: 1000,
        ..swaying()
    };
    // The angle between a shot and the aim plus the handling's offset is within the spread.
    let spread = |ads: bool| {
        let (w, id) = armed(launcher.clone(), 21);
        let mut sim = Sim::new(w).unwrap();
        sim.set_input(
            id,
            Input {
                fire: true,
                ads,
                ..Input::default()
            },
        )
        .unwrap();
        let mut widest: f64 = 0.0;
        let mut farthest_sway: f64 = 0.0;
        for _ in 0..600 {
            let fired = shots(&sim, id);
            sim.step(1);
            if shots(&sim, id) == fired {
                continue;
            }
            // The shot took this tick's sway (this launcher has no recoil).
            let sway = sim.handling(id).unwrap().sway_mrad;
            let p = sim.projectiles().last().unwrap();
            // Taken back to the muzzle: the tick of flight it had added gravity.
            let [vx, vy, vz] = [
                p.velocity[0],
                p.velocity[1] + GRAVITY_MPS2 * p.age_s,
                p.velocity[2],
            ];
            let speed = (vx * vx + vy * vy + vz * vz).sqrt();
            // Aimed level and north: up is the pitch, left is -x.
            let up = (vy / speed).pasin() * 1e3;
            let left = (-vx / speed).pasin() * 1e3;
            let (dl, du) = (left - sway[0], up - sway[1]);
            widest = widest.max((dl * dl + du * du).sqrt());
            farthest_sway = farthest_sway.max(sway[0].abs().max(sway[1].abs()));
        }
        assert!(farthest_sway > 0.3, "the shots swayed: {farthest_sway}");
        widest
    };
    let hip = spread(false);
    let ads = spread(true);
    assert!(hip <= 4.0 + 0.02 && hip > 2.0, "hip spread {hip}");
    assert!(ads <= 1.0 + 0.02 && ads > 0.5, "ads spread {ads}");
}

#[test]
fn a_burst_with_recoil_and_sway_hashes_the_same_every_tick_of_two_runs() {
    let launcher = Launcher {
        dispersion_mrad: 0.5,
        burst_count: 3,
        recoil: Recoil {
            kick_up_mrad: 1.5,
            kick_side_mrad: 0.6,
            recovery_s: 0.25,
        },
        ..swaying()
    };
    let script = |tick: u32| Input {
        fire: tick % 40 < 20,
        ads: tick % 200 > 100,
        hold_breath: tick % 300 > 250,
        switch_mode: tick == 2 || tick == 4 || tick == 6,
        reload: tick == 500,
        forward: if tick % 120 < 60 { 1.0 } else { 0.0 },
        run: tick % 240 < 60,
        pitch_deg: 1.0,
        ..Input::default()
    };
    let run = |seed: u64| {
        let (w, id) = armed(launcher.clone(), seed);
        let mut sim = Sim::new(w).unwrap();
        let mut hashes = Vec::new();
        for tick in 0..900 {
            sim.set_input(id, script(tick)).unwrap();
            sim.step(1);
            hashes.push(sim.state_hash());
        }
        assert_eq!(sim.magazine(id).unwrap().mode, FireMode::Burst);
        assert!(
            shots(&sim, id) >= 30,
            "it fired bursts: {}",
            shots(&sim, id)
        );
        hashes
    };
    let a = run(31);
    assert_eq!(a, run(31), "same world, same inputs, same hash every tick");
    assert_ne!(a.last(), run(32).last(), "another seed, another history");
}

#[test]
fn a_reload_takes_a_spare_magazine_and_with_none_left_does_nothing() {
    let launcher = Launcher {
        magazine: 5,
        reload_s: 1.0,
        modes: vec![FireMode::Auto],
        spare_magazines: Some(2),
        ..rifle()
    };
    let (w, id) = armed(launcher, 3);
    let mut sim = Sim::new(w).unwrap();
    assert_eq!(sim.magazine(id).unwrap().spare, Some(2));
    // Five shots empty the magazine; the reload that follows uses the first spare one.
    hold(&mut sim, id, trigger(true), 25);
    let m = sim.magazine(id).unwrap().clone();
    assert_eq!((m.left, m.reloading, m.spare), (0, true, Some(1)), "{m:?}");
    // 60 ticks of reload (the trigger is let go), then a full magazine.
    hold(&mut sim, id, Input::default(), 60);
    let m = sim.magazine(id).unwrap().clone();
    assert_eq!((m.left, m.reloading, m.spare), (5, false, Some(1)), "{m:?}");
    // A partly used magazine goes back in the pouch, and the fullest one is loaded: the
    // pouch keeps the four rounds, the spare count stays.
    hold(&mut sim, id, trigger(true), 1);
    hold(&mut sim, id, Input::default(), 1);
    let reload = Input {
        reload: true,
        ..Input::default()
    };
    hold(&mut sim, id, reload, 1);
    let m = sim.magazine(id).unwrap().clone();
    assert_eq!(m.pouch_rounds(), Some(vec![4]), "{m:?}");
    assert_eq!((m.left, m.loading, m.spare), (0, 5, Some(1)), "{m:?}");
    hold(&mut sim, id, Input::default(), 61);
    let m = sim.magazine(id).unwrap().clone();
    assert_eq!(
        (m.left, m.reloading, m.pouch_rounds()),
        (5, false, Some(vec![4]))
    );
    // Fired dry, the reload that follows takes the four-round magazine: the pouch is empty.
    hold(&mut sim, id, trigger(true), 25);
    let m = sim.magazine(id).unwrap().clone();
    assert_eq!((m.left, m.reloading, m.spare), (0, true, Some(0)), "{m:?}");
    assert_eq!((m.loading, m.pouch_rounds()), (4, Some(vec![])), "{m:?}");
    hold(&mut sim, id, Input::default(), 61);
    assert_eq!(sim.magazine(id).unwrap().left, 4);
    // Out of magazines: firing it dry leaves it empty and a reload does nothing.
    hold(&mut sim, id, trigger(true), 25);
    let m = sim.magazine(id).unwrap().clone();
    assert_eq!((m.left, m.reloading, m.spare), (0, false, Some(0)), "{m:?}");
    hold(&mut sim, id, Input::default(), 1);
    hold(&mut sim, id, reload, 120);
    let m = sim.magazine(id).unwrap().clone();
    assert_eq!((m.left, m.reloading, m.spare), (0, false, Some(0)), "{m:?}");
    let before = shots(&sim, id);
    hold(&mut sim, id, trigger(true), 60);
    assert_eq!(shots(&sim, id), before, "an empty launcher does not fire");
}

#[test]
fn unlimited_spare_magazines_are_the_default_and_a_counted_one_is_hashed() {
    let run = |spare: Option<u32>| {
        let launcher = Launcher {
            magazine: 5,
            spare_magazines: spare,
            modes: vec![FireMode::Auto],
            ..rifle()
        };
        let (w, id) = armed(launcher, 3);
        let mut sim = Sim::new(w).unwrap();
        hold(&mut sim, id, trigger(true), 400);
        (sim.state_hash(), sim.magazine(id).unwrap().spare)
    };
    let (unlimited, spare) = run(None);
    assert_eq!(spare, None, "unlimited never counts down");
    assert_eq!(run(None).0, unlimited);
    let (counted, left) = run(Some(3));
    assert_eq!(left, Some(0));
    assert_ne!(counted, unlimited);
    assert_eq!(Launcher::default().spare_magazines, None);
}
