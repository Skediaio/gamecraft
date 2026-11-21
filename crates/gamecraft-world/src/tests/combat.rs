//! Combat data: the components `launcher`, `health` and `behaviour`, the air of the
//! environment (`temperature_c`, `sea_level_m`) and the trigger events `hit`, `down`, `heard`.

use super::EXAMPLE_PRETTY;
use crate::{
    Behaviour, BehaviourMode, Component, EntityDraft, EntityRef, Environment, Health, Launcher,
    Trigger, When, World,
};

fn armed_world() -> World {
    let mut w = World::new("Skirmish", 5);
    w.add_entity(EntityDraft {
        name: "Rifleman".into(),
        components: vec![
            Component::Launcher(Launcher {
                muzzle_velocity_mps: 790.0,
                mass_kg: 0.0113,
                ballistic_coefficient: 217.5,
                dispersion_mrad: 0.3,
                rate_s: 1.5,
                magazine: 5,
                reload_s: 3.0,
                damage_per_joule: 0.04,
                muzzle_height_m: 1.4,
                max_flight_s: 6.0,
                sound_range_m: 800.0,
                ..Launcher::default()
            }),
            Component::Health(Health {
                max: 120.0,
                current: 80.0,
                armor: 5.0,
            }),
            Component::Behaviour(Behaviour {
                mode: BehaviourMode::Patrol,
                route: vec![[0.0, 0.0], [10.0, -5.0]],
                team: "red".into(),
                ..Behaviour::default()
            }),
        ],
        ..EntityDraft::default()
    })
    .unwrap();
    w.triggers.push(Trigger {
        id: "down".into(),
        when: When::Down {
            who: None,
            target: Some(EntityRef("tag:enemy".into())),
        },
        condition: None,
        actions: vec![],
        once: false,
        enabled: true,
    });
    w.environment.temperature_c = -10.0;
    w.environment.sea_level_m = 1200.0;
    w
}

#[test]
fn combat_components_and_air_round_trip() {
    let w = armed_world();
    w.validate().unwrap();
    let text = w.to_json().unwrap();
    assert!(text.contains("\"temperature_c\": -10.0"), "{text}");
    assert!(text.contains("\"sea_level_m\": 1200.0"), "{text}");
    let back = World::from_json(&text).unwrap();
    assert_eq!(back, w);
    let e = back.entities().next().unwrap();
    assert_eq!(e.launcher().unwrap().magazine, 5);
    assert_eq!(e.health().unwrap().current, 80.0);
    assert_eq!(e.behaviour().unwrap().team, "red");
}

#[test]
fn defaults_of_the_new_parts() {
    let env = Environment::default();
    assert_eq!((env.temperature_c, env.sea_level_m), (15.0, 0.0));
    let c: Component = serde_json::from_str(r#"{"type": "launcher"}"#).unwrap();
    let Component::Launcher(l) = c else { panic!() };
    assert_eq!(l, Launcher::default());
    assert!(l.muzzle_velocity_mps > 0.0 && l.magazine >= 1);
    let c: Component = serde_json::from_str(r#"{"type": "health"}"#).unwrap();
    assert_eq!(
        c,
        Component::Health(Health {
            max: 100.0,
            current: 100.0,
            armor: 0.0
        })
    );
    let c: Component = serde_json::from_str(r#"{"type": "behaviour"}"#).unwrap();
    let Component::Behaviour(b) = c else { panic!() };
    assert_eq!(b.mode, BehaviourMode::Idle);
    assert!(b.route.is_empty());
    for kind in ["launcher", "health", "behaviour"] {
        assert!(Component::KINDS.contains(&kind), "{kind}");
    }
}

/// A document without the new fields prints back exactly as before: old worlds keep their
/// text (and so their state hashes).
#[test]
fn default_air_is_not_written() {
    let w = World::from_json(EXAMPLE_PRETTY).unwrap();
    let text = w.to_json().unwrap();
    assert!(!text.contains("temperature_c") && !text.contains("sea_level_m"));
    assert_eq!(text, EXAMPLE_PRETTY);
}

fn invalid(w: &World) -> String {
    w.validate().unwrap_err().to_string()
}

#[test]
fn bad_values_name_the_field() {
    let edit = |f: &dyn Fn(&mut World)| {
        let mut w = armed_world();
        f(&mut w);
        invalid(&w)
    };
    let set_launcher = |f: &dyn Fn(&mut Launcher)| {
        edit(&|w: &mut World| {
            let id = w.entities().next().unwrap().id;
            let e = w.entity_mut(id).unwrap();
            let mut l = e.launcher().unwrap().clone();
            f(&mut l);
            e.set_component(Component::Launcher(l));
        })
    };
    let msg = set_launcher(&|l| l.muzzle_velocity_mps = 0.0);
    assert!(msg.contains("launcher.muzzle_velocity_mps"), "{msg}");
    let msg = set_launcher(&|l| l.ballistic_coefficient = f64::NAN);
    assert!(msg.contains("launcher.ballistic_coefficient"), "{msg}");
    let msg = set_launcher(&|l| l.magazine = 0);
    assert!(msg.contains("launcher.magazine"), "{msg}");
    let msg = set_launcher(&|l| l.rate_s = -1.0);
    assert!(msg.contains("launcher.rate_s"), "{msg}");

    let set_health = |h: Health| {
        edit(&move |w: &mut World| {
            let id = w.entities().next().unwrap().id;
            w.entity_mut(id)
                .unwrap()
                .set_component(Component::Health(h.clone()));
        })
    };
    let msg = set_health(Health {
        max: 100.0,
        current: 150.0,
        armor: 0.0,
    });
    assert!(
        msg.contains("health.current") && msg.contains("100"),
        "{msg}"
    );
    let msg = set_health(Health {
        max: 0.0,
        current: 0.0,
        armor: 0.0,
    });
    assert!(msg.contains("health.max"), "{msg}");

    let set_behaviour = |b: Behaviour| {
        edit(&move |w: &mut World| {
            let id = w.entities().next().unwrap().id;
            w.entity_mut(id)
                .unwrap()
                .set_component(Component::Behaviour(b.clone()));
        })
    };
    let msg = set_behaviour(Behaviour {
        mode: BehaviourMode::Patrol,
        route: vec![],
        ..Behaviour::default()
    });
    assert!(msg.contains("behaviour.route"), "{msg}");
    let msg = set_behaviour(Behaviour {
        route: vec![[0.0, f64::INFINITY]],
        ..Behaviour::default()
    });
    assert!(msg.contains("behaviour.route[0]"), "{msg}");
    let msg = set_behaviour(Behaviour {
        fov_deg: 0.0,
        ..Behaviour::default()
    });
    assert!(msg.contains("behaviour.fov_deg"), "{msg}");

    let msg = edit(&|w| w.environment.temperature_c = 200.0);
    assert!(msg.contains("environment.temperature_c"), "{msg}");
    let msg = edit(&|w| w.environment.sea_level_m = f64::NAN);
    assert!(msg.contains("environment.sea_level_m"), "{msg}");
}

#[test]
fn combat_trigger_events_parse_and_print() {
    for (text, event) in [
        (r#"{"event": "hit", "who": "tag:player"}"#, "hit"),
        (r#"{"event": "down", "target": "tag:enemy"}"#, "down"),
        (
            r#"{"event": "heard", "who": "Guard", "target": "3"}"#,
            "heard",
        ),
    ] {
        let when: When = serde_json::from_str(text).unwrap();
        assert_eq!(when.event(), event);
        assert!(when.has_who() && when.has_target());
        let back: When = serde_json::from_str(&serde_json::to_string(&when).unwrap()).unwrap();
        assert_eq!(back, when);
    }
    let mut w = armed_world();
    w.triggers[0].when = When::Hit {
        who: Some(EntityRef("tag:".into())),
        target: None,
    };
    assert!(invalid(&w).contains("when.who"), "{}", invalid(&w));
}
