//! The commands of fighting: `sim.fire`, `sim.release`, `sim.input`'s `fire` and `pitch_deg`,
//! what `sim.step`, `sim.state` and `entity.get` show, the air of `environment.set`, and the
//! trigger events `hit`, `down` and `heard`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, playing, refuse};
use gamecraft_engine::Session;
use gamecraft_world::{ObjectiveStatus, World};
use serde_json::{Value, json};

/// A flat range: a shooter (team blue, a rifle without spread) at the origin facing north and a
/// target (tag `enemy`, 150 health) 40 m north.
fn range() -> Session {
    let mut s = Session::new(World::new("Range", 3));
    let mut run = |c: &str, p: Value| s.run(c, &p).unwrap();
    run(
        "terrain.set",
        json!({"kind": "flat", "size_m": 256, "cell_m": 4}),
    );
    run("environment.set", json!({"wind_speed_mps": 0}));
    run(
        "entity.add",
        json!({"name": "Shooter", "at": [0, 0], "components": [
            {"type": "character"},
            {"type": "launcher", "dispersion_mrad": 0, "rate_s": 0.5, "damage_per_joule": 0.03},
            {"type": "tags", "tags": ["team:blue"]}]}),
    );
    run(
        "entity.add",
        json!({"name": "Target", "at": [0, -40], "components": [
            {"type": "character"}, {"type": "health", "max": 150, "current": 150},
            {"type": "tags", "tags": ["enemy"]}]}),
    );
    s
}

fn events(r: &Value, name: &str) -> Vec<Value> {
    r["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"] == name)
        .cloned()
        .collect()
}

#[test]
fn fire_flies_hits_and_shows_in_state() {
    let mut s = range();
    s.run("sim.start", &json!({})).unwrap();
    let r = s
        .run("sim.fire", &json!({"entity": "Shooter", "yaw_deg": 0}))
        .unwrap();
    assert_eq!(r["projectile"], 1);
    assert_eq!(r["magazine"]["left"], 9);
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["projectiles"].as_array().unwrap().len(), 1);
    assert_eq!(state["projectiles"][0]["shooter"], 1);
    let r = s.run("sim.step", &json!({"ticks": 10})).unwrap();
    let hits = events(&r, "hit");
    assert_eq!(hits.len(), 1, "{r}");
    assert_eq!(
        (hits[0]["shooter"].clone(), hits[0]["target"].clone()),
        (json!(1), json!(2))
    );
    assert!(hits[0]["energy_j"].as_f64().unwrap() > 1000.0);
    assert!(r["projectiles"].as_array().unwrap().is_empty());
    let target = &r["characters"][1];
    assert!(target["health"]["current"].as_f64().unwrap() < 150.0);
    let got = s.run("entity.get", &json!({"id": "Target"})).unwrap();
    assert_eq!(got["health"], target["health"]);
    let shooter = s.run("entity.get", &json!({"id": "Shooter"})).unwrap();
    assert_eq!(shooter["magazine"]["shots"], 1);
}

#[test]
fn input_fire_and_pitch_and_a_down_trigger_completes_an_objective() {
    let mut s = range();
    s.run(
        "objective.add",
        &json!({"id": "clear", "title": "Down the target"}),
    )
    .unwrap();
    s.run(
        "trigger.add",
        &json!({"id": "downed", "when": {"event": "down", "target": "tag:enemy", "who": "Shooter"},
                "do": [{"command": "objective.set", "params": {"id": "clear", "status": "done"}},
                       {"command": "message.show", "params": {"text": "Down", "to": "$who"}}]}),
    )
    .unwrap();
    s.run(
        "trigger.add",
        &json!({"id": "struck", "when": {"event": "hit", "target": "Target"}, "once": false,
                "do": [{"command": "var.set", "params": {"name": "hits", "add": 1}}]}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    let r = s
        .run(
            "sim.input",
            &json!({"entity": "Shooter", "face": [0, -40], "pitch_deg": 0.1, "fire": true}),
        )
        .unwrap();
    assert_eq!(
        (r["pitch_deg"].clone(), r["fire"].clone()),
        (json!(0.1), json!(true))
    );
    let r = s.run("sim.step", &json!({"seconds": 3})).unwrap();
    let downs = events(&r, "down");
    assert_eq!(downs.len(), 1, "{r}");
    assert_eq!(
        (downs[0]["who"].clone(), downs[0]["target"].clone()),
        (json!(1), json!(2))
    );
    let world = s.current_world();
    assert_eq!(
        world.objective("clear").unwrap().status,
        ObjectiveStatus::Done
    );
    assert!(matches!(
        world.vars["hits"],
        gamecraft_world::VarValue::Number(n) if n >= 1.0
    ));
    assert_eq!(r["characters"][1]["health"]["down"], true);
}

#[test]
fn heard_triggers_name_the_listener_and_the_shooter() {
    let mut s = range();
    s.run(
        "trigger.add",
        &json!({"id": "ears", "when": {"event": "heard", "who": "Target", "target": "Shooter"},
                "do": [{"command": "var.set", "params": {"name": "alarm", "value": true}}]}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    s.run(
        "sim.fire",
        &json!({"entity": "Shooter", "yaw_deg": 180, "pitch_deg": 10}),
    )
    .unwrap();
    let r = s.run("sim.step", &json!({})).unwrap();
    let heard = events(&r, "heard");
    assert_eq!(heard.len(), 1, "{r}");
    assert_eq!(
        (heard[0]["who"].clone(), heard[0]["target"].clone()),
        (json!(2), json!(1))
    );
    assert_eq!(
        s.current_world().vars["alarm"],
        gamecraft_world::VarValue::Bool(true)
    );
}

#[test]
fn release_hands_a_character_back_to_its_behaviour() {
    let mut s = range();
    s.run(
        "entity.set_component",
        &json!({"id": "Target", "component": {"type": "behaviour", "mode": "patrol",
                "route": [[10, -40], [-10, -40]], "team": "red"}}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    let r = s.run("sim.step", &json!({})).unwrap();
    assert_eq!(r["characters"][1]["behaviour"]["taken_over"], false);
    s.run("sim.input", &json!({"entity": "Target", "forward": 0}))
        .unwrap();
    let got = s.run("entity.get", &json!({"id": "Target"})).unwrap();
    assert_eq!(got["behaviour"]["taken_over"], true);
    let r = s.run("sim.release", &json!({"entity": "Target"})).unwrap();
    assert_eq!(r["released"], true);
    assert_eq!(r["behaviour"]["taken_over"], false);
    let r = s.run("sim.release", &json!({"entity": "Target"})).unwrap();
    assert_eq!(r["released"], false);
}

#[test]
fn environment_set_takes_the_air() {
    let mut s = example();
    let r = s
        .run(
            "environment.set",
            &json!({"temperature_c": -5, "sea_level_m": 900}),
        )
        .unwrap();
    assert_eq!(r["environment"]["temperature_c"], -5.0);
    assert_eq!(r["environment"]["sea_level_m"], 900.0);
    let r = s.run("world.inspect", &json!({})).unwrap();
    assert_eq!(r["environment"]["temperature_c"], -5.0);
    refuse(
        &mut s,
        "environment.set",
        json!({"temperature_c": 100}),
        &["environment.temperature_c"],
    );
}

#[test]
fn bad_parameters_name_the_parameter() {
    let mut s = playing();
    refuse(
        &mut s,
        "sim.input",
        json!({"entity": "Scout", "pitch_deg": 95}),
        &["`pitch_deg`", "89"],
    );
    refuse(
        &mut s,
        "sim.fire",
        json!({"entity": "Scout", "pitch_deg": -91}),
        &["`pitch_deg`", "90"],
    );
    refuse(&mut s, "sim.fire", json!({"entity": "Oak"}), &["launcher"]);
    refuse(&mut s, "sim.fire", json!({}), &["entity"]);
    refuse(
        &mut s,
        "sim.release",
        json!({"entity": "Nobody"}),
        &["Nobody"],
    );
    refuse(
        &mut s,
        "sim.release",
        json!({"entity": "Oak"}),
        &["character"],
    );
    s.run("sim.fire", &json!({"entity": "Scout"})).unwrap();
    refuse(
        &mut s,
        "sim.fire",
        json!({"entity": "Scout"}),
        &["ready in"],
    );
    let mut idle = example();
    refuse(
        &mut idle,
        "sim.fire",
        json!({"entity": "Scout"}),
        &["sim.start"],
    );
    refuse(
        &mut idle,
        "sim.release",
        json!({"entity": "Scout"}),
        &["sim.start"],
    );
}

#[test]
fn a_trigger_with_who_for_a_terrain_hit_needs_no_target() {
    let mut s = range();
    s.run(
        "trigger.add",
        &json!({"id": "dirt", "when": {"event": "hit", "who": "tag:team:blue"}, "once": false,
                "do": [{"command": "var.set", "params": {"name": "shots", "add": 1}}]}),
    )
    .unwrap();
    s.run(
        "trigger.add",
        &json!({"id": "never", "when": {"event": "hit", "target": "tag:enemy"},
                "do": [{"command": "var.set", "params": {"name": "struck", "value": true}}]}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    s.run(
        "sim.fire",
        &json!({"entity": "Shooter", "yaw_deg": 90, "pitch_deg": -20}),
    )
    .unwrap();
    s.run("sim.step", &json!({"ticks": 30})).unwrap();
    let vars = &s.current_world().vars;
    assert_eq!(vars["shots"], gamecraft_world::VarValue::Number(1.0));
    assert!(!vars.contains_key("struck"));
}
