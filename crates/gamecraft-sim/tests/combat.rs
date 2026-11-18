//! Projectiles in a running world: wind drift, air density, shots up and into the ground,
//! hits, damage and `down`, perception and hearing, behaviours, determinism.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_shape, pos, settle};
use gamecraft_sim::{BrainState, Event, EventKind, Input, Sim};
use gamecraft_world::{
    Behaviour, BehaviourMode, Character, Component, EntityDraft, EntityId, Health, Launcher,
    ShapeKind, Tags, World,
};

/// A launcher without spread, firing nothing but what it is told.
fn rifle() -> Launcher {
    Launcher {
        muzzle_velocity_mps: 800.0,
        mass_kg: 0.0109,
        ballistic_coefficient: 200.0,
        dispersion_mrad: 0.0,
        rate_s: 0.5,
        magazine: 5,
        reload_s: 2.0,
        damage_per_joule: 0.05,
        muzzle_height_m: 1.5,
        max_flight_s: 60.0,
        sound_range_m: 400.0,
    }
}

/// A big flat world (1024 m) without wind.
fn range_world() -> World {
    let mut w = World::new("range", 11);
    w.terrain = Some(gamecraft_world::Terrain {
        size_m: 1024.0,
        cell_m: 4.0,
        source: gamecraft_world::TerrainSource::Flat { height_m: 0.0 },
    });
    w.environment.wind_speed_mps = 0.0;
    w
}

/// A launcher on a post (not a character) at `[x, 0, z]`.
fn add_gun(w: &mut World, at: [f64; 3], launcher: Launcher) -> EntityId {
    w.add_entity(EntityDraft {
        name: "gun".into(),
        position: at,
        components: vec![Component::Launcher(launcher)],
        ..EntityDraft::default()
    })
    .unwrap()
}

/// A wall 40 m wide and 30 m tall across the line of fire (north of the origin), `range_m`
/// away.
fn add_wall(w: &mut World, range_m: f64) -> EntityId {
    add_shape(
        w,
        ShapeKind::Box,
        [40.0, 30.0, 1.0],
        [0.0, 0.0, -range_m],
        0.0,
        true,
    )
}

fn hits(events: &[Event]) -> Vec<EventKind> {
    events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::Hit { .. }))
        .map(|e| e.kind.clone())
        .collect()
}

/// Steps until the first hit (at most `seconds`) and hands it back.
fn first_hit(sim: &mut Sim, seconds: f64) -> EventKind {
    let ticks = (seconds * 60.0) as u32;
    for _ in 0..ticks {
        sim.step(1);
        if let Some(hit) = hits(&sim.take_events()).into_iter().next() {
            return hit;
        }
    }
    panic!(
        "no hit within {seconds} s; projectiles: {:?}",
        sim.projectiles()
    );
}

fn hit_point(kind: &EventKind) -> [f64; 3] {
    match kind {
        EventKind::Hit { point, .. } => *point,
        other => panic!("not a hit: {other:?}"),
    }
}

/// Where a shot north from the origin hits a wall `range_m` away in `world`.
fn impact(mut world: World, range_m: f64, pitch_deg: f64) -> [f64; 3] {
    let gun = add_gun(&mut world, [0.0, 0.0, 0.0], rifle());
    add_wall(&mut world, range_m);
    let mut sim = Sim::new(world).unwrap();
    sim.fire(gun, Some(0.0), Some(pitch_deg)).unwrap();
    hit_point(&first_hit(&mut sim, 5.0))
}

#[test]
fn crosswind_drifts_downwind_more_at_longer_range() {
    let windy = |direction_deg: f64| {
        let mut w = range_world();
        w.environment.wind_speed_mps = 5.0;
        w.environment.wind_direction_deg = direction_deg;
        w
    };
    // Firing north, a wind from the west (from the left) drifts the shot east (right, +x).
    let near = impact(windy(270.0), 100.0, 0.0);
    let far = impact(windy(270.0), 300.0, 0.1);
    assert!(near[0] > 0.0, "{near:?}");
    assert!(far[0] > near[0] * 2.0, "near {near:?} far {far:?}");
    // No wind, no drift; a headwind (from the north) pushes nothing sideways.
    let calm = impact(range_world(), 300.0, 0.1);
    assert!(calm[0].abs() < 1e-9, "{calm:?}");
    let head = impact(windy(0.0), 300.0, 0.1);
    assert!(head[0].abs() < 1e-9, "{head:?}");
    // ... but it holds the shot back, so it drops more than in calm air.
    assert!(head[1] < calm[1], "head {head:?} calm {calm:?}");
}

#[test]
fn denser_air_drops_the_shot_more() {
    let with_air = |temperature_c: f64, sea_level_m: f64| {
        let mut w = range_world();
        w.environment.temperature_c = temperature_c;
        w.environment.sea_level_m = sea_level_m;
        w
    };
    let cold = impact(with_air(-20.0, 0.0), 300.0, 0.1);
    let thin = impact(with_air(40.0, 3000.0), 300.0, 0.1);
    assert!(cold[1] < thin[1], "cold {cold:?} thin {thin:?}");
}

#[test]
fn a_shot_straight_up_comes_down_on_the_terrain() {
    let mut w = range_world();
    let gun = add_gun(&mut w, [0.0, 0.0, 0.0], rifle());
    let mut sim = Sim::new(w).unwrap();
    sim.fire(gun, Some(0.0), Some(90.0)).unwrap();
    sim.step(60);
    let up = sim.projectiles()[0].clone();
    assert!(up.position[1] > 200.0, "{up:?}");
    let hit = first_hit(&mut sim, 59.0);
    let EventKind::Hit {
        target, shooter, ..
    } = hit
    else {
        panic!()
    };
    assert_eq!((target, shooter), (None, gun));
    assert!(sim.projectiles().is_empty());
}

#[test]
fn a_shot_into_the_ground_hits_the_terrain() {
    let mut w = range_world();
    let gun = add_gun(&mut w, [0.0, 0.0, 0.0], rifle());
    let mut sim = Sim::new(w).unwrap();
    sim.fire(gun, Some(0.0), Some(-30.0)).unwrap();
    let hit = first_hit(&mut sim, 1.0);
    let p = hit_point(&hit);
    assert!(
        p[1].abs() < 1e-6
            && (p[2] + 1.5 / gamecraft_math::Portable::ptan(30f64.to_radians())).abs() < 0.05
    );
    assert!(matches!(hit, EventKind::Hit { target: None, .. }));
}

#[test]
fn a_shot_ends_after_its_longest_flight() {
    let mut w = World::new("void", 1);
    w.environment.wind_speed_mps = 0.0;
    let gun = add_gun(
        &mut w,
        [0.0, 0.0, 0.0],
        Launcher {
            max_flight_s: 0.5,
            ..rifle()
        },
    );
    let mut sim = Sim::new(w).unwrap();
    sim.fire(gun, None, Some(10.0)).unwrap();
    sim.step(29);
    assert_eq!(sim.projectiles().len(), 1);
    sim.step(2);
    assert!(sim.projectiles().is_empty());
    assert!(hits(&sim.take_events()).is_empty());
}

fn soldier(w: &mut World, name: &str, at: [f64; 3], extra: Vec<Component>) -> EntityId {
    let mut components = vec![Component::Character(Character::default())];
    if !extra.iter().any(|c| c.kind() == "health") {
        components.push(Component::Health(Health::default()));
    }
    components.extend(extra);
    w.add_entity(EntityDraft {
        name: name.into(),
        position: at,
        components,
        ..EntityDraft::default()
    })
    .unwrap()
}

#[test]
fn a_hit_takes_health_by_the_energy_rule_and_downs_at_zero() {
    let mut w = range_world();
    let shooter = soldier(
        &mut w,
        "shooter",
        [0.0, 0.0, 0.0],
        vec![Component::Launcher(rifle())],
    );
    // Tough enough for several hits.
    let target = soldier(
        &mut w,
        "target",
        [0.0, 0.0, -50.0],
        vec![Component::Health(Health {
            max: 600.0,
            current: 600.0,
            armor: 0.0,
        })],
    );
    let mut sim = Sim::new(w).unwrap();
    settle(&mut sim);
    sim.take_events();
    sim.set_input(
        shooter,
        Input {
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            fire: true,
            ..Input::default()
        },
    )
    .unwrap();
    let hit = first_hit(&mut sim, 1.0);
    let EventKind::Hit {
        target: Some(t),
        energy_j,
        speed_mps,
        ..
    } = hit
    else {
        panic!("{hit:?}")
    };
    assert_eq!(t, target);
    assert!((energy_j - 0.5 * 0.0109 * speed_mps * speed_mps).abs() < 1e-6);
    let health = sim
        .world()
        .entity(target)
        .unwrap()
        .health()
        .unwrap()
        .clone();
    let damage = energy_j * 0.05;
    assert!(damage > 0.0 && (health.current - (600.0 - damage)).abs() < 1e-9);
    // Keep firing until it is down: one `down` event, and it stops taking input.
    let mut downs = Vec::new();
    for _ in 0..600 {
        sim.step(1);
        for e in sim.take_events() {
            if let EventKind::Down { who, target: t } = e.kind {
                downs.push((who, t));
            }
        }
    }
    assert_eq!(downs, vec![(shooter, target)]);
    assert!(sim.world().entity(target).unwrap().is_down());
    let before = pos(&sim, target);
    sim.set_input(
        target,
        Input {
            forward: 1.0,
            ..Input::default()
        },
    )
    .unwrap();
    sim.step(30);
    assert_eq!(pos(&sim, target), before, "a downed character stops");
}

#[test]
fn armor_takes_its_share_and_a_shape_stops_the_shot() {
    let mut w = range_world();
    let gun = add_gun(&mut w, [0.0, 0.0, 0.0], rifle());
    let wall = add_wall(&mut w, 20.0);
    let behind = soldier(&mut w, "behind", [0.0, 0.0, -50.0], vec![]);
    let mut sim = Sim::new(w).unwrap();
    sim.fire(gun, Some(0.0), Some(0.0)).unwrap();
    let hit = first_hit(&mut sim, 1.0);
    assert!(matches!(hit, EventKind::Hit { target: Some(t), .. } if t == wall));
    sim.step(60);
    let health = sim.world().entity(behind).unwrap().health().unwrap();
    assert_eq!(health.current, 100.0);

    let mut w = range_world();
    let gun = add_gun(&mut w, [0.0, 0.0, 0.0], rifle());
    let tank = w
        .add_entity(EntityDraft {
            name: "armored".into(),
            position: [0.0, 0.0, -30.0],
            components: vec![
                Component::Character(Character::default()),
                Component::Health(Health {
                    max: 100.0,
                    current: 100.0,
                    armor: 1e6,
                }),
            ],
            ..EntityDraft::default()
        })
        .unwrap();
    let mut sim = Sim::new(w).unwrap();
    settle(&mut sim);
    sim.fire(gun, Some(0.0), Some(0.0)).unwrap();
    let hit = first_hit(&mut sim, 1.0);
    assert!(matches!(hit, EventKind::Hit { target: Some(t), .. } if t == tank));
    assert_eq!(
        sim.world().entity(tank).unwrap().health().unwrap().current,
        100.0
    );
}

#[test]
fn magazine_rate_and_reload() {
    let mut w = range_world();
    let gun = add_gun(&mut w, [0.0, 0.0, 0.0], rifle());
    let mut sim = Sim::new(w).unwrap();
    sim.fire(gun, None, Some(45.0)).unwrap();
    let err = sim.fire(gun, None, Some(45.0)).unwrap_err().to_string();
    assert!(err.contains("ready in"), "{err}");
    for _ in 0..4 {
        sim.step(30);
        sim.fire(gun, None, Some(45.0)).unwrap();
    }
    sim.step(30);
    let err = sim.fire(gun, None, Some(45.0)).unwrap_err().to_string();
    assert!(err.contains("reload"), "{err}");
    sim.step(121);
    sim.fire(gun, None, Some(45.0)).unwrap();
    let state = sim.magazine(gun).unwrap();
    assert_eq!(state.left, 4);
}

#[test]
fn shots_are_heard_within_range() {
    let mut w = range_world();
    let gun = add_gun(&mut w, [0.0, 0.0, 0.0], rifle());
    let near = soldier(&mut w, "near", [0.0, 0.0, 300.0], vec![]);
    let _far = soldier(&mut w, "far", [0.0, 0.0, 450.0], vec![]);
    let mut sim = Sim::new(w).unwrap();
    settle(&mut sim);
    sim.take_events();
    sim.fire(gun, None, Some(45.0)).unwrap();
    sim.step(1);
    let heard: Vec<(EntityId, EntityId)> = sim
        .take_events()
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Heard { who, target, .. } => Some((who, target)),
            _ => None,
        })
        .collect();
    assert_eq!(heard, vec![(near, gun)]);
}

/// An observer with a behaviour facing north, a target `range_m` north of it.
fn sight_world(range_m: f64) -> (World, EntityId, EntityId) {
    let mut w = range_world();
    w.environment.time_of_day_h = 12.0;
    let observer = soldier(
        &mut w,
        "observer",
        [0.0, 0.0, 0.0],
        vec![Component::Behaviour(Behaviour {
            view_range_m: 400.0,
            ..Behaviour::default()
        })],
    );
    let target = soldier(&mut w, "target", [0.0, 0.0, -range_m], vec![]);
    (w, observer, target)
}

fn sees(w: World, a: EntityId, b: EntityId) -> bool {
    let mut sim = Sim::new(w).unwrap();
    settle(&mut sim);
    sim.sees(a, b)
}

#[test]
fn perception_by_light_mist_and_walls() {
    let (w, a, b) = sight_world(200.0);
    assert!(sees(w.clone(), a, b), "seen in daylight at 200 m");
    assert!(
        !sees(w.clone(), b, a),
        "not behind its back (it faces north, away)"
    );
    let mut misty = w.clone();
    misty.environment.mist = 1.0;
    assert!(!sees(misty, a, b), "not seen at 200 m in thick mist");
    let mut night = w.clone();
    night.environment.time_of_day_h = 0.0;
    assert!(!sees(night.clone(), a, b), "not seen at 200 m at night");
    let (mut near_night, a2, b2) = sight_world(30.0);
    near_night.environment = night.environment.clone();
    assert!(
        sees(near_night, a2, b2),
        "seen at night within the night range"
    );
    let mut walled = w;
    add_wall(&mut walled, 100.0);
    assert!(!sees(walled, a, b), "not seen behind a wall");
    let (w, a, b) = sight_world(450.0);
    assert!(!sees(w, a, b), "beyond the view range");
}

fn team(name: &str) -> Component {
    Component::Tags(Tags {
        tags: vec![format!("team:{name}")],
    })
}

/// A guard of team red facing north, a blue character 60 m north of it.
fn standoff() -> (World, EntityId, EntityId) {
    let mut w = range_world();
    w.environment.time_of_day_h = 12.0;
    let guard = soldier(
        &mut w,
        "guard",
        [0.0, 0.0, 0.0],
        vec![
            Component::Launcher(Launcher {
                dispersion_mrad: 1.0,
                ..rifle()
            }),
            Component::Behaviour(Behaviour {
                mode: BehaviourMode::Guard,
                team: "red".into(),
                reaction_s: 0.5,
                ..Behaviour::default()
            }),
        ],
    );
    let blue = soldier(&mut w, "blue", [0.0, 0.0, -60.0], vec![team("blue")]);
    (w, guard, blue)
}

#[test]
fn a_guard_sees_reacts_and_fires_at_an_enemy() {
    let (w, guard, blue) = standoff();
    let mut sim = Sim::new(w).unwrap();
    sim.step(20);
    assert!(matches!(
        sim.brain(guard).unwrap().state,
        BrainState::Alert { .. }
    ));
    assert!(
        hits(&sim.take_events()).is_empty(),
        "not before its reaction time"
    );
    let mut hit_blue = false;
    for _ in 0..240 {
        sim.step(1);
        for e in sim.take_events() {
            if let EventKind::Hit {
                target: Some(t), ..
            } = e.kind
            {
                hit_blue |= t == blue;
            }
        }
    }
    assert!(hit_blue, "the guard hit the enemy");
    assert!(sim.world().entity(blue).unwrap().health().unwrap().current < 100.0);
}

#[test]
fn a_patrol_walks_its_route_and_an_agent_takes_over_until_release() {
    let mut w = range_world();
    let walker = soldier(
        &mut w,
        "walker",
        [0.0, 0.0, 0.0],
        vec![Component::Behaviour(Behaviour {
            mode: BehaviourMode::Patrol,
            route: vec![[0.0, -10.0], [10.0, -10.0]],
            ..Behaviour::default()
        })],
    );
    let mut sim = Sim::new(w).unwrap();
    sim.step(60 * 8);
    let p = pos(&sim, walker);
    assert!(
        p[2] < -9.0 && p[0] > 1.0,
        "walked to the first point and on: {p:?}"
    );
    // An agent's input takes over: it stands still.
    sim.set_input(walker, Input::default()).unwrap();
    assert!(sim.is_taken_over(walker));
    let held = pos(&sim, walker);
    sim.step(60);
    assert_eq!(pos(&sim, walker), held);
    sim.release(walker).unwrap();
    assert!(!sim.is_taken_over(walker));
    sim.step(60);
    assert_ne!(pos(&sim, walker), held, "released, it patrols again");
}

#[test]
fn a_shot_heard_sends_a_guard_searching() {
    let mut w = range_world();
    let guard = soldier(
        &mut w,
        "guard",
        [0.0, 0.0, 0.0],
        vec![Component::Behaviour(Behaviour {
            mode: BehaviourMode::Guard,
            team: "red".into(),
            ..Behaviour::default()
        })],
    );
    // Behind the guard's back, out of sight.
    let gun = add_gun(&mut w, [0.0, 0.0, 100.0], rifle());
    let mut sim = Sim::new(w).unwrap();
    settle(&mut sim);
    sim.fire(gun, Some(180.0), Some(30.0)).unwrap();
    sim.step(1);
    assert!(matches!(
        sim.brain(guard).unwrap().state,
        BrainState::Search { .. }
    ));
    sim.step(120);
    assert!(pos(&sim, guard)[2] > 1.0, "it walks towards the sound");
}

/// A small fight with spread and behaviours on both sides.
fn melee_world() -> World {
    let (mut w, _guard, blue) = standoff();
    w.entity_mut(blue)
        .unwrap()
        .set_component(Component::Launcher(Launcher {
            dispersion_mrad: 2.0,
            ..rifle()
        }));
    w.entity_mut(blue)
        .unwrap()
        .set_component(Component::Behaviour(Behaviour {
            mode: BehaviourMode::Patrol,
            route: vec![[-20.0, -60.0], [20.0, -60.0]],
            team: "blue".into(),
            aim_error_mrad: 8.0,
            ..Behaviour::default()
        }));
    w
}

#[test]
fn same_seed_same_hash_every_tick_with_spread_and_behaviour() {
    let mut a = Sim::new(melee_world()).unwrap();
    let mut b = Sim::new(melee_world()).unwrap();
    let mut shots = 0;
    for tick in 0..900 {
        a.step(1);
        b.step(1);
        assert_eq!(a.state_hash(), b.state_hash(), "tick {tick}");
        shots += a
            .take_events()
            .iter()
            .filter(|e| matches!(e.kind, EventKind::Heard { .. } | EventKind::Hit { .. }))
            .count();
        b.take_events();
    }
    assert!(shots > 0, "the fight happened");
    let mut other = melee_world();
    other.seed = 12;
    let mut c = Sim::new(other).unwrap();
    c.step(900);
    assert_ne!(
        c.state_hash(),
        a.state_hash(),
        "another seed, another fight"
    );
}

const GOLDEN_COMBAT_HASH: u64 = 0x1031_3BAD_DBF1_4A9D;

#[test]
fn golden_hash_of_a_fight() {
    let mut sim = Sim::new(melee_world()).unwrap();
    sim.step(600);
    assert_eq!(
        sim.state_hash(),
        GOLDEN_COMBAT_HASH,
        "golden hash changed: {:#018x}",
        sim.state_hash()
    );
}

#[test]
fn hostile_fire_requests_are_errors() {
    let (w, guard, blue) = standoff();
    let mut sim = Sim::new(w).unwrap();
    assert!(sim.fire(blue, None, None).is_err(), "no launcher");
    assert!(sim.fire(EntityId(999), None, None).is_err());
    assert!(sim.fire(guard, Some(f64::NAN), None).is_err());
    assert!(sim.fire(guard, None, Some(f64::INFINITY)).is_err());
    assert!(sim.release(EntityId(999)).is_err());
    let bad = Input {
        pitch_deg: f64::NAN,
        ..Input::default()
    };
    assert!(sim.set_input(guard, bad).is_err());
    let steep = Input {
        pitch_deg: 120.0,
        ..Input::default()
    };
    sim.set_input(guard, steep).unwrap();
    assert_eq!(sim.input(guard).unwrap().pitch_deg, 89.0);
}

#[test]
fn a_world_change_keeps_the_projectiles_flying() {
    let mut w = range_world();
    let gun = add_gun(&mut w, [0.0, 0.0, 0.0], rifle());
    let mut sim = Sim::new(w).unwrap();
    sim.fire(gun, None, Some(45.0)).unwrap();
    sim.step(5);
    let mut changed = sim.world().clone();
    changed.remove_entity(gun);
    sim.replace_world(changed).unwrap();
    sim.step(5);
    assert_eq!(sim.projectiles().len(), 1);
}
