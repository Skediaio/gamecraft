//! The held facing of an input: a point or an entity the character keeps turning towards,
//! every tick, until a yaw or another facing replaces it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, flat_world, pos, settle};
use gamecraft_math::{Portable, rad_to_deg, wrap_deg};
use gamecraft_sim::behaviour::aim_pitch_deg;
use gamecraft_sim::{Face, Input, Sim, SimError};
use gamecraft_world::{Component, EntityDraft, EntityId, Launcher, World};

/// The yaw that faces from `from` to `to` (x, z).
fn bearing(from: [f64; 3], to: [f64; 2]) -> f64 {
    let (dx, dz) = (to[0] - from[0], to[1] - from[2]);
    wrap_deg(rad_to_deg((-dx).patan2(-dz)))
}

fn yaw(sim: &Sim, id: EntityId) -> f64 {
    sim.world().entity(id).unwrap().rotation_y_deg
}

/// Moves an entity by hand between two ticks, as a game master would.
fn move_to(sim: &mut Sim, id: EntityId, at: [f64; 3]) {
    let mut world = sim.world().clone();
    world.entity_mut(id).unwrap().position = at;
    sim.replace_world(world).unwrap();
}

fn close(a: f64, b: f64) -> bool {
    let d = (a - b).abs();
    d.min(360.0 - d) < 1e-9
}

#[test]
fn a_held_point_is_tracked_every_tick_while_strafing() {
    let mut w = flat_world(0.0);
    let hero = add_character(&mut w, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(w).unwrap();
    settle(&mut sim);
    let point = [0.0, -20.0];
    sim.set_input(
        hero,
        Input {
            right: 1.0,
            face: Some(Face::Point(point)),
            ..Input::default()
        },
    )
    .unwrap();
    // Facing the point at once, before any tick.
    assert!(close(sim.input(hero).unwrap().yaw_deg, 0.0));
    let mut yaws = Vec::new();
    for _ in 0..120 {
        sim.step(1);
        let p = pos(&sim, hero);
        // The facing was taken at the start of the tick, from where it stood then; it still
        // points at the point within the angle of one tick's step.
        yaws.push(yaw(&sim, hero));
        let off = (yaw(&sim, hero) - bearing(p, point)).abs();
        assert!(off.min(360.0 - off) < 1.0, "off by {off} deg at {p:?}");
    }
    // Strafing right around a held point turns the character: a circle, not a straight line.
    // 2 s at the walking speed of 1.6 m/s is an arc of 3.2 m on a radius of 20 m: 9.2 degrees.
    let turned = (yaws[119] - yaws[0]).abs();
    assert!(turned > 8.5 && turned < 9.5, "turned {turned} deg");
    let p = pos(&sim, hero);
    let r = (p[0] * p[0] + (p[2] + 20.0) * (p[2] + 20.0)).sqrt();
    assert!((r - 20.0).abs() < 1.0, "left the circle: r = {r}");
}

#[test]
fn a_held_entity_is_followed_and_a_yaw_replaces_it() {
    let mut w = flat_world(0.0);
    let hero = add_character(&mut w, [0.0, 0.0, 0.0]);
    let other = add_character(&mut w, [20.0, 0.0, 0.0]);
    let mut sim = Sim::new(w).unwrap();
    settle(&mut sim);
    sim.set_input(
        hero,
        Input {
            face: Some(Face::Entity {
                id: other,
                pitch: true,
            }),
            ..Input::default()
        },
    )
    .unwrap();
    sim.step(1);
    assert!(close(yaw(&sim, hero), 270.0), "{}", yaw(&sim, hero)); // east
    move_to(&mut sim, other, [0.0, 0.0, 20.0]);
    sim.step(1);
    assert!(close(yaw(&sim, hero), 180.0), "{}", yaw(&sim, hero)); // south
    // No launcher: the pitch is left alone.
    assert_eq!(sim.input(hero).unwrap().pitch_deg, 0.0);
    // A plain yaw (no facing) holds still while the other moves on.
    sim.set_input(
        hero,
        Input {
            yaw_deg: 90.0,
            ..Input::default()
        },
    )
    .unwrap();
    move_to(&mut sim, other, [-20.0, 0.0, 0.0]);
    sim.step(1);
    assert!(close(yaw(&sim, hero), 90.0));
    // A removed target: the last facing holds, nothing fails.
    sim.set_input(
        hero,
        Input {
            face: Some(Face::Entity {
                id: other,
                pitch: false,
            }),
            ..Input::default()
        },
    )
    .unwrap();
    sim.step(1);
    let mut world = sim.world().clone();
    world.remove_entity(other).unwrap();
    sim.replace_world(world).unwrap();
    let before = yaw(&sim, hero);
    sim.step(5);
    assert!(close(yaw(&sim, hero), before));
}

fn rifle() -> Launcher {
    Launcher {
        muzzle_velocity_mps: 300.0,
        dispersion_mrad: 0.0,
        ..Launcher::default()
    }
}

#[test]
fn a_held_entity_with_a_launcher_also_aims_the_pitch_at_the_chest() {
    let mut w = flat_world(0.0);
    let hero = w
        .add_entity(EntityDraft {
            name: "Shooter".into(),
            components: vec![
                Component::Character(Default::default()),
                Component::Launcher(rifle()),
            ],
            ..EntityDraft::default()
        })
        .unwrap();
    let target = add_character(&mut w, [0.0, 0.0, -150.0]);
    let mut sim = Sim::new(w).unwrap();
    settle(&mut sim);
    let face = |pitch| Input {
        face: Some(Face::Entity { id: target, pitch }),
        ..Input::default()
    };
    sim.set_input(hero, face(true)).unwrap();
    sim.step(1);
    let me = sim.world().entity(hero).unwrap();
    let eye = [
        me.position[0],
        me.position[1] + me.character().unwrap().eye_height_m,
        me.position[2],
    ];
    let chest = gamecraft_sim::aim::aim_point(sim.world().entity(target).unwrap());
    let want = aim_pitch_deg(eye, chest, 300.0);
    let got = sim.input(hero).unwrap().pitch_deg;
    assert!(want > 0.0 && (got - want).abs() < 1e-12, "{got} vs {want}");
    // With `pitch: false` the pitch stays as given.
    sim.set_input(
        hero,
        Input {
            pitch_deg: -3.0,
            ..face(false)
        },
    )
    .unwrap();
    sim.step(1);
    assert_eq!(sim.input(hero).unwrap().pitch_deg, -3.0);
}

/// A world with a hero and another character, playing, the hero facing as asked.
fn run_with(face: Option<Face>, yaw_deg: f64) -> u64 {
    let mut w: World = flat_world(0.0);
    let hero = add_character(&mut w, [0.0, 0.0, 0.0]);
    add_character(&mut w, [10.0, 0.0, -10.0]);
    let mut sim = Sim::new(w).unwrap();
    sim.set_input(
        hero,
        Input {
            forward: 1.0,
            yaw_deg,
            face,
            ..Input::default()
        },
    )
    .unwrap();
    sim.step(30);
    sim.state_hash()
}

#[test]
fn the_held_facing_is_part_of_the_state_hash_and_deterministic() {
    let point = run_with(Some(Face::Point([0.0, -50.0])), 0.0);
    assert_eq!(point, run_with(Some(Face::Point([0.0, -50.0])), 0.0));
    // The same motion (straight north) without a held facing hashes differently, and so does
    // another point on the same line.
    assert_ne!(point, run_with(None, 0.0));
    assert_ne!(point, run_with(Some(Face::Point([0.0, -60.0])), 0.0));
    let entity = run_with(
        Some(Face::Entity {
            id: EntityId(2),
            pitch: false,
        }),
        0.0,
    );
    assert_eq!(
        entity,
        run_with(
            Some(Face::Entity {
                id: EntityId(2),
                pitch: false
            }),
            0.0
        )
    );
    assert_ne!(
        entity,
        run_with(
            Some(Face::Entity {
                id: EntityId(2),
                pitch: true
            }),
            0.0
        )
    );
}

#[test]
fn a_bad_facing_is_refused() {
    let mut w = flat_world(0.0);
    let hero = add_character(&mut w, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(w).unwrap();
    let e = sim
        .set_input(
            hero,
            Input {
                face: Some(Face::Point([f64::NAN, 0.0])),
                ..Input::default()
            },
        )
        .unwrap_err();
    assert!(matches!(e, SimError::BadInput(_)), "{e}");
    assert!(e.to_string().contains("face"), "{e}");
    let e = sim
        .set_input(
            hero,
            Input {
                face: Some(Face::Entity {
                    id: EntityId(99),
                    pitch: false,
                }),
                ..Input::default()
            },
        )
        .unwrap_err();
    assert_eq!(e, SimError::NoEntity(EntityId(99)));
    assert!(sim.input(hero).is_none());
}
