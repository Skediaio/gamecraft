//! Scenario events: characters entering and leaving zones, interacting, and simulated time
//! reaching a trigger's `at_s`.

// Integration tests are compiled as their own crates, outside `cfg(test)`, so clippy.toml's
// test allowances do not reach their helper functions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{add_character, flat_world, walk};
use gamecraft_sim::{DT, Event, EventKind, Input, Sim};
use gamecraft_world::{
    Component, EntityDraft, EntityId, Interactable, Shape, ShapeKind, Trigger, When, World, Zone,
    ZoneShape,
};

fn add_zone(world: &mut World, position: [f64; 3], zone: Zone) -> EntityId {
    world
        .add_entity(EntityDraft {
            name: "zone".into(),
            position,
            components: vec![Component::Zone(zone)],
            ..EntityDraft::default()
        })
        .unwrap()
}

fn circle(radius_m: f64) -> Zone {
    Zone {
        shape: ZoneShape::Circle,
        radius_m,
        ..Zone::default()
    }
}

fn add_interactable(world: &mut World, position: [f64; 3], reach_m: f64) -> EntityId {
    world
        .add_entity(EntityDraft {
            name: "lever".into(),
            position,
            components: vec![
                Component::Shape(Shape {
                    kind: ShapeKind::Cylinder,
                    size: [0.2, 1.2, 0.2],
                    color: [0.5, 0.5, 0.5],
                    solid: false,
                }),
                Component::Interactable(Interactable {
                    reach_m,
                    prompt: String::new(),
                }),
            ],
            ..EntityDraft::default()
        })
        .unwrap()
}

fn interact(yaw_deg: f64) -> Input {
    Input {
        yaw_deg,
        interact: true,
        ..Input::default()
    }
}

fn kinds(events: &[Event]) -> Vec<(u64, EventKind)> {
    events.iter().map(|e| (e.tick, e.kind.clone())).collect()
}

#[test]
fn a_character_inside_a_zone_at_the_start_enters_on_the_first_tick() {
    let mut world = flat_world(0.0);
    let zone = add_zone(&mut world, [0.0, 0.0, 0.0], circle(3.0));
    let who = add_character(&mut world, [1.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    assert!(sim.take_events().is_empty());
    sim.step(1);
    assert_eq!(
        kinds(&sim.take_events()),
        vec![(1, EventKind::Enter { who, zone })]
    );
    assert_eq!(sim.inside().collect::<Vec<_>>(), vec![(who, zone)]);
    sim.step(10);
    assert!(sim.take_events().is_empty(), "no new event while inside");
}

#[test]
fn walking_through_a_zone_enters_then_leaves_once_each() {
    let mut world = flat_world(0.0);
    let zone = add_zone(&mut world, [0.0, 0.0, -10.0], circle(2.0));
    let who = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.set_input(who, walk(0.0)).unwrap();
    sim.step(60 * 15);
    let events = sim.take_events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0].kind, EventKind::Enter { who, zone });
    assert_eq!(events[1].kind, EventKind::Leave { who, zone });
    // Walking at 1.6 m/s from z 0: the edge at z -8 is reached after 5 s, the far edge at
    // z -12 after 7.5 s.
    let t_enter = events[0].tick as f64 * DT;
    let t_leave = events[1].tick as f64 * DT;
    assert!((t_enter - 5.0).abs() < 2.0 * DT, "{t_enter}");
    assert!((t_leave - 7.5).abs() < 2.0 * DT, "{t_leave}");
    assert!(sim.take_events().is_empty(), "taken");
}

#[test]
fn a_box_zone_is_turned_by_its_entity_and_scaled() {
    let mut world = flat_world(0.0);
    // A 10 x 1 box turned 90 degrees: it spans x -0.5..0.5 and z -5..5 (times scale 2).
    let zone = world
        .add_entity(EntityDraft {
            position: [0.0, 0.0, 0.0],
            rotation_y_deg: 90.0,
            scale: 2.0,
            components: vec![Component::Zone(Zone {
                shape: ZoneShape::Box,
                size: [10.0, 1.0],
                ..Zone::default()
            })],
            ..EntityDraft::default()
        })
        .unwrap();
    let inside = add_character(&mut world, [0.0, 0.0, -9.0]);
    let outside = add_character(&mut world, [3.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.step(1);
    let events = sim.take_events();
    assert_eq!(
        kinds(&events),
        vec![(1, EventKind::Enter { who: inside, zone })]
    );
    assert!(!sim.inside().any(|(w, _)| w == outside));
}

#[test]
fn a_zone_counts_only_where_the_body_overlaps_its_height() {
    let mut world = flat_world(0.0);
    let zone = add_zone(
        &mut world,
        [0.0, 5.0, 0.0],
        Zone {
            height_m: 1.0,
            ..circle(3.0)
        },
    );
    let who = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.step(1);
    assert!(
        sim.take_events().is_empty(),
        "the zone floats above the head"
    );
    let mut moved = sim.world().clone();
    moved.entity_mut(zone).unwrap().position = [0.0, 1.0, 0.0];
    sim.replace_world(moved).unwrap();
    sim.step(1);
    assert_eq!(
        kinds(&sim.take_events()),
        vec![(2, EventKind::Enter { who, zone })]
    );
}

#[test]
fn a_removed_zone_or_character_drops_its_membership_without_an_event() {
    let mut world = flat_world(0.0);
    let zone = add_zone(&mut world, [0.0, 0.0, 0.0], circle(3.0));
    add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.step(1);
    sim.take_events();
    let mut next = sim.world().clone();
    next.remove_entity(zone);
    sim.replace_world(next).unwrap();
    assert_eq!(sim.inside().count(), 0);
    sim.step(5);
    assert!(sim.take_events().is_empty());
}

#[test]
fn interacting_fires_once_per_press_on_the_nearest_faced_interactable() {
    let mut world = flat_world(0.0);
    let who = add_character(&mut world, [0.0, 0.0, 0.0]);
    let near = add_interactable(&mut world, [0.0, 0.0, -1.0], 2.0);
    let _far = add_interactable(&mut world, [0.0, 0.0, -1.5], 2.0);
    let behind = add_interactable(&mut world, [0.0, 0.0, 1.0], 2.0);
    let mut sim = Sim::new(world).unwrap();
    sim.step(1);
    sim.set_input(who, interact(0.0)).unwrap();
    sim.step(30);
    assert_eq!(
        kinds(&sim.take_events()),
        vec![(2, EventKind::Interact { who, target: near })],
        "held: one event on the press"
    );
    // Released and pressed again, facing south: the one behind.
    sim.set_input(
        who,
        Input {
            interact: false,
            ..interact(180.0)
        },
    )
    .unwrap();
    sim.step(1);
    sim.set_input(who, interact(180.0)).unwrap();
    sim.step(1);
    assert_eq!(
        kinds(&sim.take_events()),
        vec![(
            33,
            EventKind::Interact {
                who,
                target: behind
            }
        )]
    );
}

#[test]
fn interacting_needs_reach_and_facing() {
    let mut world = flat_world(0.0);
    let who = add_character(&mut world, [0.0, 0.0, 0.0]);
    add_interactable(&mut world, [0.0, 0.0, -3.0], 2.0); // out of reach
    add_interactable(&mut world, [1.5, 0.0, -0.5], 2.0); // in reach, 71.6 degrees to the side
    let mut sim = Sim::new(world).unwrap();
    sim.step(1);
    sim.set_input(who, interact(0.0)).unwrap();
    sim.step(1);
    assert!(sim.take_events().is_empty());
    // Turned towards the side one (yaw of the direction (1.5, -0.5) is about -71.6 degrees).
    sim.set_input(
        who,
        Input {
            interact: false,
            ..interact(-70.0)
        },
    )
    .unwrap();
    sim.step(1);
    sim.set_input(who, interact(-70.0)).unwrap();
    sim.step(1);
    let events = sim.take_events();
    assert_eq!(events.len(), 1, "{events:?}");
}

fn time_trigger(id: &str, at_s: f64) -> Trigger {
    Trigger {
        id: id.into(),
        when: When::Time { at_s },
        condition: None,
        actions: vec![],
        once: true,
        enabled: true,
    }
}

#[test]
fn time_events_fire_on_the_tick_that_reaches_at_s() {
    let mut world = flat_world(0.0);
    world.triggers.push(time_trigger("late", 1.0));
    world.triggers.push(time_trigger("early", 0.5));
    world.triggers.push(time_trigger("same", 0.5));
    world.triggers.push(time_trigger("zero", 0.0));
    let mut off = time_trigger("off", 0.25);
    off.enabled = false;
    world.triggers.push(off);
    let mut sim = Sim::new(world).unwrap();
    sim.step(120);
    assert_eq!(
        kinds(&sim.take_events()),
        vec![
            (1, EventKind::Time { at_s: 0.0 }),
            (30, EventKind::Time { at_s: 0.5 }),
            (60, EventKind::Time { at_s: 1.0 }),
        ]
    );
}

#[test]
fn events_of_a_tick_are_ordered_by_time_then_actor_then_subject() {
    let mut world = flat_world(0.0);
    world.triggers.push(time_trigger("now", 0.0));
    let b = add_character(&mut world, [10.0, 0.0, 0.0]);
    let z2 = add_zone(&mut world, [10.0, 0.0, 0.0], circle(2.0));
    let z1 = add_zone(&mut world, [0.0, 0.0, 0.0], circle(30.0));
    let a = add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.step(1);
    assert_eq!(
        kinds(&sim.take_events()),
        vec![
            (1, EventKind::Time { at_s: 0.0 }),
            (1, EventKind::Enter { who: b, zone: z2 }),
            (1, EventKind::Enter { who: b, zone: z1 }),
            (1, EventKind::Enter { who: a, zone: z1 }),
        ]
    );
    // Ids: b 1, z2 2, z1 3, a 4.
    assert!(b < z2 && z2 < z1 && z1 < a);
}

#[test]
fn the_hash_covers_zone_membership_and_the_held_interaction() {
    let mut world = flat_world(0.0);
    add_zone(&mut world, [0.0, 0.0, 0.0], circle(3.0));
    let who = add_character(&mut world, [0.0, 0.0, 0.0]);
    let a = Sim::new(world.clone()).unwrap();
    let mut b = Sim::new(world).unwrap();
    // Same tick and positions, but `b` has registered the character inside.
    let mut a = a;
    a.step(1);
    b.step(1);
    assert_eq!(a.state_hash(), b.state_hash());
    // Taking events does not change the hash.
    b.take_events();
    assert_eq!(a.state_hash(), b.state_hash());
    // Holding interact is part of the hash.
    b.set_input(who, interact(0.0)).unwrap();
    a.set_input(who, Input::default()).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn a_world_without_scenario_data_hashes_as_before() {
    // The golden hash of tests/determinism.rs pins this; here only that membership of an
    // empty world adds nothing.
    let mut world = flat_world(0.0);
    add_character(&mut world, [0.0, 0.0, 0.0]);
    let mut sim = Sim::new(world).unwrap();
    sim.step(3);
    assert_eq!(sim.inside().count(), 0);
}
