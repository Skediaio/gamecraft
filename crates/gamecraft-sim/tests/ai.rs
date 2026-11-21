//! Skill, precision, gradual detection, alertness, stress and cover of behaviours: headless
//! scenarios with fixed seeds and closed-form or invariant oracles.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_shape, pos};
use gamecraft_math::wrap_deg_signed;
use gamecraft_sim::ai::{self, CoverPhase, FORGET_S, REACT_KNOWLEDGE};
use gamecraft_sim::cover::{CoverAsk, Threat, evaluate_world};
use gamecraft_sim::ray::line_of_sight;
use gamecraft_sim::{BrainState, DT, Sim, Trunks};
use gamecraft_world::{
    Alertness, Behaviour, BehaviourMode, Character, Component, EntityDraft, EntityId, Health,
    Launcher, ShapeKind, Tags, World,
};

/// The chest of a default character, as a share of its height.
const CHEST_M: f64 = 1.8 * 0.65;

/// A flat world (1024 m) at noon, without wind.
fn field() -> World {
    let mut w = World::new("field", 23);
    w.terrain = Some(gamecraft_world::Terrain {
        size_m: 1024.0,
        cell_m: 4.0,
        source: gamecraft_world::TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    w.environment.wind_speed_mps = 0.0;
    w.environment.time_of_day_h = 12.0;
    w
}

/// A launcher without spread.
fn rifle() -> Launcher {
    Launcher {
        dispersion_mrad: 0.0,
        ..Launcher::default()
    }
}

/// A character at `at` facing `yaw_deg` with `extra` components and health.
fn soldier(
    w: &mut World,
    name: &str,
    at: [f64; 3],
    yaw_deg: f64,
    extra: Vec<Component>,
) -> EntityId {
    let mut components = vec![
        Component::Character(Character::default()),
        Component::Health(Health::default()),
    ];
    components.extend(extra);
    w.add_entity(EntityDraft {
        name: name.into(),
        position: at,
        rotation_y_deg: yaw_deg,
        components,
        ..EntityDraft::default()
    })
    .unwrap()
}

fn blue() -> Component {
    Component::Tags(Tags {
        tags: vec!["team:blue".into()],
    })
}

/// A red guard at the origin facing north (no launcher, so it never fires) with `behaviour`,
/// and a blue character `range_m` north of it.
fn watcher(range_m: f64, behaviour: Behaviour) -> (World, EntityId, EntityId) {
    let mut w = field();
    let red = soldier(
        &mut w,
        "red",
        [0.0, 0.0, 0.0],
        0.0,
        vec![Component::Behaviour(Behaviour {
            mode: BehaviourMode::Guard,
            team: "red".into(),
            view_range_m: 400.0,
            ..behaviour
        })],
    );
    let blue = soldier(&mut w, "blue", [0.0, 0.0, -range_m], 0.0, vec![blue()]);
    (w, red, blue)
}

/// Spot time 8 s, held in combat (detection twice as fast), no reaction time.
fn spotter() -> Behaviour {
    Behaviour {
        spot_time_s: 8.0,
        alertness: Alertness::Combat,
        reaction_s: 0.0,
        ..Behaviour::default()
    }
}

fn knows(sim: &Sim, who: EntityId, of: EntityId) -> f64 {
    sim.brain(who).unwrap().knows(of)
}

#[test]
fn detection_builds_up_over_time_faster_at_short_range() {
    // Effective range 400 m by day: at 100 m the distance gain is 200 / 100 = 2, at 50 m it is
    // 4; in combat the spot time of 8 s counts as 4 s. So knowledge rises 0.5 per second at
    // 100 m and 1 per second at 50 m.
    for (range_m, per_s) in [(100.0, 0.5), (50.0, 1.0)] {
        let (w, red, blue) = watcher(range_m, spotter());
        let mut sim = Sim::new(w).unwrap();
        let mut last = 0.0;
        for tick in 1..=30u32 {
            sim.step(1);
            let k = knows(&sim, red, blue);
            let expected = per_s * f64::from(tick) * DT;
            assert!(
                (k - expected).abs() < 1e-9,
                "at {range_m} m after {tick} ticks: {k}, expected {expected}"
            );
            assert!(k > last, "knowledge rises while seen");
            last = k;
        }
    }
}

#[test]
fn it_reacts_and_fires_only_above_the_thresholds() {
    // 0.5 per second at 100 m: reacts from 0.25 (0.5 s), fires from 0.6 (1.2 s).
    let (w, red, blue) = watcher(100.0, spotter());
    let mut sim = Sim::new(w).unwrap();
    sim.step(25);
    assert!(knows(&sim, red, blue) < REACT_KNOWLEDGE);
    assert_eq!(sim.brain(red).unwrap().state, BrainState::Guard);
    sim.step(10);
    assert!(matches!(
        sim.brain(red).unwrap().state,
        BrainState::Alert { target, .. } if target == blue
    ));
    sim.step(30);
    assert!(matches!(
        sim.brain(red).unwrap().state,
        BrainState::Alert { .. }
    ));
    sim.step(15);
    assert!(matches!(
        sim.brain(red).unwrap().state,
        BrainState::Engage { target } if target == blue
    ));
}

#[test]
fn detection_decays_without_contact_and_keeps_the_last_known_place() {
    let (w, red, blue) = watcher(50.0, spotter());
    let mut sim = Sim::new(w).unwrap();
    sim.step(30);
    let k0 = knows(&sim, red, blue);
    assert!((k0 - 0.5).abs() < 1e-9, "{k0}");
    // Blue steps behind red's back: out of its field of view.
    let mut moved = sim.world().clone();
    moved.entity_mut(blue).unwrap().position = [0.0, 0.0, 50.0];
    sim.replace_world(moved).unwrap();
    sim.step(60);
    let brain = sim.brain(red).unwrap();
    let k = &brain.knowledge[&blue];
    assert!(!k.seen);
    let expected = k0 - 60.0 * DT / FORGET_S;
    assert!(
        (k.knowledge - expected).abs() < 1e-9,
        "{} expected {expected}",
        k.knowledge
    );
    assert_eq!(
        k.last_known,
        [0.0, CHEST_M, -50.0],
        "where it was last seen"
    );
}

#[test]
fn a_spot_time_of_zero_knows_at_first_sight_as_before() {
    let (w, red, blue) = watcher(100.0, Behaviour::default());
    let mut sim = Sim::new(w).unwrap();
    sim.step(1);
    assert_eq!(knows(&sim, red, blue), 1.0);
    assert!(matches!(
        sim.brain(red).unwrap().state,
        BrainState::Alert { .. }
    ));
}

#[test]
fn darkness_and_mist_slow_detection() {
    let (w, red, blue) = watcher(100.0, spotter());
    let mut misty = w.clone();
    // Visibility about 240 m: inside it, but the range halves the distance gain.
    misty.environment.mist = std::f64::consts::FRAC_1_SQRT_2;
    let clear_k = {
        let mut sim = Sim::new(w).unwrap();
        sim.step(20);
        knows(&sim, red, blue)
    };
    let misty_k = {
        let mut sim = Sim::new(misty).unwrap();
        sim.step(20);
        knows(&sim, red, blue)
    };
    assert!(misty_k > 0.0, "still seen in the mist");
    assert!(misty_k < clear_k, "slower in mist: {misty_k} vs {clear_k}");
}

#[test]
fn skill_scales_the_spot_time() {
    let (w, red, blue) = watcher(
        100.0,
        Behaviour {
            skill: 0.0,
            ..spotter()
        },
    );
    let mut sim = Sim::new(w).unwrap();
    sim.step(30);
    // Skill 0 doubles the spot time: 0.25 per second.
    assert!((knows(&sim, red, blue) - 0.125).abs() < 1e-9);
}

#[test]
fn precision_and_stress_scale_the_aim_error_in_closed_form() {
    let b = Behaviour {
        aim_error_mrad: 4.0,
        ..Behaviour::default()
    };
    assert_eq!(ai::aim_error_mrad(&b, 0.0, 0.0), 4.0);
    let half = Behaviour {
        precision: 0.5,
        ..b.clone()
    };
    assert!((ai::aim_error_mrad(&half, 0.0, 0.0) - 10.0).abs() < 1e-12);
    let none = Behaviour {
        precision: 0.0,
        ..b.clone()
    };
    assert!((ai::aim_error_mrad(&none, 0.0, 0.0) - 16.0).abs() < 1e-12);
    assert!((ai::aim_error_mrad(&b, 0.0, 0.5) - 8.0).abs() < 1e-12);
}

/// The yaw and pitch of red's first shot at blue 60 m north with `behaviour`.
fn first_shot(behaviour: Behaviour) -> (f64, f64) {
    let mut w = field();
    let red = soldier(
        &mut w,
        "red",
        [0.0, 0.0, 0.0],
        0.0,
        vec![
            Component::Launcher(rifle()),
            Component::Behaviour(Behaviour {
                team: "red".into(),
                reaction_s: 0.2,
                ..behaviour
            }),
        ],
    );
    soldier(&mut w, "blue", [0.0, 0.0, -60.0], 0.0, vec![blue()]);
    let mut sim = Sim::new(w).unwrap();
    for _ in 0..120 {
        sim.step(1);
        let input = sim.input(red).unwrap();
        if input.fire {
            return (input.yaw_deg, input.pitch_deg);
        }
    }
    panic!("red never fired");
}

#[test]
fn precision_scales_the_aim_error_of_the_shots() {
    let base = Behaviour {
        aim_error_mrad: 3.0,
        ..Behaviour::default()
    };
    let exact = first_shot(Behaviour {
        aim_error_mrad: 0.0,
        ..base.clone()
    });
    let sharp = first_shot(base.clone());
    let blunt = first_shot(Behaviour {
        precision: 0.0,
        ..base.clone()
    });
    let off = |s: (f64, f64)| (wrap_deg_signed(s.0 - exact.0), s.1 - exact.1);
    let (sy, sp) = off(sharp);
    let (by, bp) = off(blunt);
    assert!(sy.abs() + sp.abs() > 1e-6, "the error shows");
    // The same draw, a cone four times as wide.
    assert!((by - 4.0 * sy).abs() < 1e-9, "{by} vs 4 * {sy}");
    assert!((bp - 4.0 * sp).abs() < 1e-9, "{bp} vs 4 * {sp}");
}

#[test]
fn the_aim_settles_while_the_target_stays_in_view() {
    let b = Behaviour {
        aim_error_mrad: 8.0,
        aim_error_floor_mrad: 2.0,
        aim_settle_s: 2.0,
        ..Behaviour::default()
    };
    assert_eq!(ai::aim_error_mrad(&b, 0.0, 0.0), 8.0);
    assert!((ai::aim_error_mrad(&b, 1.0, 0.0) - 5.0).abs() < 1e-12);
    assert_eq!(ai::aim_error_mrad(&b, 2.0, 0.0), 2.0);
    assert_eq!(ai::aim_error_mrad(&b, 9.0, 0.0), 2.0);
    // In a running world: the settling time counts while the same enemy stays seen.
    let (w, red, blue) = watcher(60.0, b);
    let mut sim = Sim::new(w).unwrap();
    sim.step(150);
    let brain = sim.brain(red).unwrap();
    assert_eq!(brain.aim_target, Some(blue));
    assert!(
        (brain.aim_settled_s - 149.0 * DT).abs() < 1e-9,
        "{}",
        brain.aim_settled_s
    );
    let behaviour = sim.world().entity(red).unwrap().behaviour().unwrap();
    assert_eq!(
        ai::aim_error_mrad(behaviour, brain.aim_settled_s, brain.stress),
        2.0
    );
}

/// A wall 6 m wide (along x), 0.4 m thick and `height_m` tall at the origin; a red soldier
/// 1.2 m south of it facing south with cover search 10 m; a blue soldier 30 m south.
fn wall_scenario(height_m: f64) -> (World, EntityId, EntityId, EntityId) {
    let mut w = field();
    let wall = add_shape(
        &mut w,
        ShapeKind::Box,
        [6.0, height_m, 0.4],
        [0.0, 0.0, 0.0],
        0.0,
        true,
    );
    let red = soldier(
        &mut w,
        "red",
        [0.0, 0.0, 1.2],
        180.0,
        vec![
            Component::Launcher(rifle()),
            Component::Behaviour(Behaviour {
                mode: BehaviourMode::Guard,
                team: "red".into(),
                reaction_s: 0.5,
                cover_search_m: 10.0,
                ..Behaviour::default()
            }),
        ],
    );
    let blue = soldier(&mut w, "blue", [0.0, 0.0, 30.0], 0.0, vec![blue()]);
    (w, wall, red, blue)
}

fn chest_seen(sim: &Sim, from: EntityId, to: EntityId) -> bool {
    let [x, y, z] = pos(sim, from);
    let [tx, ty, tz] = pos(sim, to);
    let skip = |id| id == from || id == to;
    line_of_sight(
        sim.world(),
        sim.heights(),
        sim.trunks(),
        [x, y + CHEST_M, z],
        [tx, ty + CHEST_M, tz],
        skip,
    )
    .unwrap()
    .blocked_by
    .is_none()
}

#[test]
fn on_contact_it_runs_round_the_wall_to_its_far_side() {
    let (w, _, red, blue) = wall_scenario(2.5);
    let mut sim = Sim::new(w).unwrap();
    assert!(chest_seen(&sim, blue, red), "exposed at the start");
    // Within 6 seconds: behind the wall, out of blue's sight.
    let mut arrived = None;
    for tick in 1..=360u32 {
        sim.step(1);
        let p = pos(&sim, red);
        if p[2] < -0.2 && !chest_seen(&sim, blue, red) {
            arrived = Some(tick);
            break;
        }
    }
    let tick = arrived.expect("red reached the far side of the wall within 6 s");
    let brain = sim.brain(red).unwrap();
    assert_eq!(brain.alertness, Alertness::Combat);
    assert!(brain.cover.is_some(), "it chose a cover");
    // It stays down there (shorter than the time it hides before it peeks).
    for _ in 0..90 {
        sim.step(1);
        assert!(!chest_seen(&sim, blue, red), "stays in cover");
    }
    let brain = sim.brain(red).unwrap();
    assert!(matches!(
        brain.cover.unwrap().phase,
        CoverPhase::Hide { .. }
    ));
    assert!(tick > 1);
}

#[test]
fn behind_a_low_wall_it_fires_over_it() {
    let (w, _, red, blue) = wall_scenario(1.4);
    let mut sim = Sim::new(w).unwrap();
    sim.step(360);
    let p = pos(&sim, red);
    assert!(p[2] < -0.2, "behind the wall: {p:?}");
    assert!(!chest_seen(&sim, blue, red), "its chest is hidden");
    assert!(
        sim.magazine(red).unwrap().shots > 0,
        "it fired over the wall"
    );
    assert!(sim.world().entity(blue).unwrap().health().unwrap().current < 100.0);
}

#[test]
fn without_a_cover_search_it_stands_and_fights_as_before() {
    let (mut w, _, red, blue) = wall_scenario(2.5);
    let e = w.entity_mut(red).unwrap();
    let mut b = e.behaviour().unwrap().clone();
    b.cover_search_m = 0.0;
    e.set_component(Component::Behaviour(b));
    let mut sim = Sim::new(w).unwrap();
    // Until it has shot blue down (then it searches where blue fell).
    let mut fired = false;
    for _ in 0..120 {
        sim.step(1);
        if sim.world().entity(blue).unwrap().is_down() {
            break;
        }
        let p = pos(&sim, red);
        assert!((p[2] - 1.2).abs() < 1e-6, "it did not move: {p:?}");
        assert!(sim.brain(red).unwrap().cover.is_none());
        assert!(chest_seen(&sim, blue, red));
        fired |= sim.input(red).is_some_and(|i| i.fire);
    }
    assert!(fired, "it fought where it stood");
}

#[test]
fn the_cover_query_ranks_the_far_side_first() {
    let (w, wall, red, blue) = wall_scenario(2.5);
    let feet = w.entity(red).unwrap().position;
    let threats = [Threat {
        point: [0.0, CHEST_M, 30.0],
        entity: Some(blue),
    }];
    let ask = CoverAsk {
        me: Some(red),
        feet,
        height_m: 1.8,
        radius_m: 0.3,
        eye_height_m: 1.65,
        search_m: 10.0,
        threats: &threats,
        friends: &[],
    };
    let field = w.terrain.as_ref().unwrap().build().unwrap();
    let found = evaluate_world(&w, Some(&field), &Trunks::default(), &ask);
    assert!(found.len() > 10, "{} candidates", found.len());
    let best = found[0];
    assert_eq!(best.protection, 1.0);
    assert_eq!(best.reachable, Some(true));
    assert!(best.point[2] < -0.2, "the far side: {best:?}");
    assert!(best.via[0].is_some(), "round the wall's corners");
    // Scores fall.
    for pair in found.windows(2) {
        assert!(pair[0].score >= pair[1].score);
    }
    // Where it stands is exposed.
    let here = found
        .iter()
        .find(|c| c.source == gamecraft_sim::cover::CoverSource::Here)
        .unwrap();
    assert_eq!(here.protection, 0.0);
    let _ = wall;
}

#[test]
fn near_misses_stress_a_coward_but_not_a_brave_one() {
    for (courage, stressed) in [(0.0, true), (1.0, false)] {
        let mut w = field();
        let red = soldier(
            &mut w,
            "red",
            [0.0, 0.0, 0.0],
            0.0,
            vec![Component::Behaviour(Behaviour {
                team: "red".into(),
                courage,
                cover_search_m: 10.0,
                ..Behaviour::default()
            })],
        );
        let gun = w
            .add_entity(EntityDraft {
                name: "gun".into(),
                position: [-50.0, 0.0, 1.0],
                components: vec![Component::Launcher(Launcher {
                    muzzle_height_m: CHEST_M,
                    ..rifle()
                })],
                ..EntityDraft::default()
            })
            .unwrap();
        let mut sim = Sim::new(w).unwrap();
        sim.step(1);
        // Fired east, passing 1 m south of red's chest.
        sim.fire(gun, Some(-90.0), Some(0.0)).unwrap();
        sim.step(10);
        let stress = sim.brain(red).unwrap().stress;
        if stressed {
            // One near miss at 1 m: 0.35 * (1 - 1/3), less a few ticks of recovery; at most
            // two segments of the flight count.
            let one = ai::STRESS_PER_NEAR_MISS * (1.0 - 1.0 / 3.0);
            assert!(stress > one - 0.03 && stress < 2.0 * one, "{stress}");
            let b = sim.world().entity(red).unwrap().behaviour().unwrap();
            assert!(ai::aim_error_mrad(b, 0.0, stress) > b.aim_error_mrad);
        } else {
            assert_eq!(stress, 0.0);
        }
    }
}

#[test]
fn a_heard_shot_makes_it_aware_and_it_calms_down() {
    let mut w = field();
    let red = soldier(
        &mut w,
        "red",
        [0.0, 0.0, 0.0],
        0.0,
        vec![Component::Behaviour(Behaviour {
            team: "red".into(),
            calm_s: 2.0,
            search_s: 1.0,
            ..Behaviour::default()
        })],
    );
    let gun = w
        .add_entity(EntityDraft {
            name: "gun".into(),
            position: [0.0, 0.0, 40.0],
            components: vec![Component::Launcher(rifle())],
            ..EntityDraft::default()
        })
        .unwrap();
    let mut sim = Sim::new(w).unwrap();
    sim.step(1);
    sim.fire(gun, Some(0.0), Some(45.0)).unwrap();
    sim.step(1);
    assert_eq!(sim.brain(red).unwrap().alertness, Alertness::Aware);
    // Searches 1 s, then calms down 2 s later.
    sim.step(60 * 4);
    assert_eq!(sim.brain(red).unwrap().alertness, Alertness::Safe);
}

#[test]
fn the_same_cover_fight_gives_the_same_hashes() {
    let run = || {
        let (w, _, _, _) = wall_scenario(1.4);
        let mut sim = Sim::new(w).unwrap();
        let mut hashes = Vec::new();
        for _ in 0..12 {
            sim.step(30);
            hashes.push(sim.state_hash());
        }
        hashes
    };
    let a = run();
    assert_eq!(a, run());
    // The cover state is in the hash: another cover search radius is another history.
    let (mut w, _, red, _) = wall_scenario(1.4);
    let e = w.entity_mut(red).unwrap();
    let mut b = e.behaviour().unwrap().clone();
    b.cover_search_m = 9.0;
    e.set_component(Component::Behaviour(b));
    let mut sim = Sim::new(w).unwrap();
    sim.step(30);
    assert_ne!(sim.state_hash(), a[0]);
}

#[test]
fn a_changed_alertness_while_playing_takes_effect_and_holds() {
    let (w, red, _) = watcher(500.0, Behaviour::default());
    let mut sim = Sim::new(w).unwrap();
    sim.step(1);
    assert_eq!(sim.brain(red).unwrap().alertness, Alertness::Safe);
    let mut changed = sim.world().clone();
    let e = changed.entity_mut(red).unwrap();
    let mut b = e.behaviour().unwrap().clone();
    b.alertness = Alertness::Combat;
    b.calm_s = 0.5;
    e.set_component(Component::Behaviour(b));
    sim.replace_world(changed).unwrap();
    sim.step(120);
    assert_eq!(
        sim.brain(red).unwrap().alertness,
        Alertness::Combat,
        "never calmer than its behaviour's alertness"
    );
}
