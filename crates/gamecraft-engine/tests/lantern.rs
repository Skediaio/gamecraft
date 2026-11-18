//! The oracle of scenarios: a miniature of prototype P1, built and played with commands only.
//!
//! A small valley at night with mist. A player carrying a light walks to three beacons in
//! turn and lights them by interacting (inputs only: `face`, `forward`, `interact`). Each
//! beacon has a zone, an interactable, a light that is off and the tag `beacon`. Triggers turn
//! a beacon's light on, mark its objective done and count it in `lit`; when `lit` reaches 3
//! the main objective is done and a message is shown. At 120 s the mist rises, and the main
//! objective fails if it is still open.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use gamecraft_engine::Session;
use gamecraft_world::{ObjectiveStatus, VarValue, World};
use serde_json::{Value, json};

const BEACONS: [(&str, [f64; 2]); 3] = [
    ("Beacon 1", [0.0, -20.0]),
    ("Beacon 2", [20.0, -26.0]),
    ("Beacon 3", [8.0, -45.0]),
];

/// Close enough to a beacon's centre to light it, metres (its reach is 2 m from the eye).
const LIGHTING_DISTANCE_M: f64 = 1.2;

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.run(command, &params)
        .unwrap_or_else(|e| panic!("{command} {params}: {e}"))
}

/// The scenario, built as an agent would build it.
fn valley() -> Session {
    let mut s = Session::new(World::new("Lantern valley", 11));
    run(
        &mut s,
        "terrain.set",
        json!({"kind": "noise", "size_m": 128, "cell_m": 2, "relief_m": 3, "feature_m": 90, "seed": 11}),
    );
    run(
        &mut s,
        "environment.set",
        json!({"time_of_day_h": 22.5, "mist": 0.35, "cloud_cover": 0.7}),
    );
    run(
        &mut s,
        "entity.add",
        json!({"name": "Player", "at": [0, 0], "components": [
            {"type": "character"},
            {"type": "light", "color": [1.0, 0.8, 0.5], "intensity_cd": 60, "range_m": 8},
            {"type": "tags", "tags": ["player"]}]}),
    );
    run(&mut s, "var.set", json!({"name": "lit", "value": 0}));
    run(
        &mut s,
        "objective.add",
        json!({"id": "light-all", "title": "Light the three beacons before the mist rises"}),
    );
    for (i, (name, [x, z])) in BEACONS.iter().enumerate() {
        let n = i + 1;
        run(
            &mut s,
            "entity.add",
            json!({"name": name, "at": [x, z], "components": [
                {"type": "shape", "kind": "cylinder", "size": [0.6, 1.5, 0.6], "color": [0.4, 0.35, 0.3]},
                {"type": "interactable", "reach_m": 2, "prompt": "Light the beacon"},
                {"type": "light", "color": [1.0, 0.6, 0.2], "intensity_cd": 400, "range_m": 20, "on": false},
                {"type": "tags", "tags": ["beacon"]},
                {"type": "zone", "shape": "circle", "radius_m": 4}]}),
        );
        run(
            &mut s,
            "objective.add",
            json!({"id": format!("beacon-{n}"), "title": format!("Light {name}")}),
        );
        run(
            &mut s,
            "trigger.add",
            json!({"id": format!("light-{n}"),
                   "when": {"event": "interact", "target": name, "who": "tag:player"},
                   "do": [
                       {"command": "entity.set_component", "params": {"id": "$target", "component":
                           {"type": "light", "color": [1.0, 0.6, 0.2], "intensity_cd": 400, "range_m": 20, "on": true}}},
                       {"command": "objective.set", "params": {"id": format!("beacon-{n}"), "status": "done"}},
                       {"command": "var.set", "params": {"name": "lit", "add": 1}}]}),
        );
    }
    run(
        &mut s,
        "trigger.add",
        json!({"id": "all-lit", "when": {"event": "var", "name": "lit"},
               "if": {"var": "lit", "op": ">=", "value": 3},
               "do": [
                   {"command": "objective.set", "params": {"id": "light-all", "status": "done"}},
                   {"command": "message.show", "params": {"text": "The valley is safe tonight.", "seconds": 30}}]}),
    );
    run(
        &mut s,
        "trigger.add",
        json!({"id": "mist-rises", "when": {"event": "time", "at_s": 120},
               "do": [{"command": "environment.set", "params": {"mist": 0.95}}]}),
    );
    run(
        &mut s,
        "trigger.add",
        json!({"id": "too-late", "when": {"event": "time", "at_s": 120},
               "if": {"var": "lit", "op": "<", "value": 3},
               "do": [{"command": "objective.set", "params": {"id": "light-all", "status": "failed"}}]}),
    );
    s
}

/// What one run saw: the answer of every step, and the time each beacon was lit.
struct Run {
    session: Session,
    steps: Vec<Value>,
    lit_at_s: Vec<f64>,
    message_seen: bool,
}

fn player_position(step: &Value) -> [f64; 3] {
    let p = &step["characters"][0]["position"];
    [
        p[0].as_f64().unwrap(),
        p[1].as_f64().unwrap(),
        p[2].as_f64().unwrap(),
    ]
}

/// Plays until `end_s`. A diligent player walks to each beacon in turn and lights it; a
/// dawdling one stands still.
fn play(diligent: bool, end_s: f64) -> Run {
    let mut s = valley();
    run(&mut s, "sim.start", json!({}));
    let mut steps = Vec::new();
    let mut lit_at_s = Vec::new();
    let mut message_seen = false;
    let mut next = 0;
    let mut last = run(&mut s, "sim.step", json!({"ticks": 1}));
    steps.push(last.clone());
    while last["time_s"].as_f64().unwrap() < end_s {
        if diligent && let Some((name, [bx, bz])) = BEACONS.get(next) {
            let [x, _, z] = player_position(&last);
            let (dx, dz) = (bx - x, bz - z);
            let distance = (dx * dx + dz * dz).sqrt();
            if distance <= LIGHTING_DISTANCE_M {
                run(
                    &mut s,
                    "sim.input",
                    json!({"entity": "Player", "forward": 0, "face": [bx, bz], "interact": true}),
                );
                let pressed = run(&mut s, "sim.step", json!({"ticks": 1}));
                let fired = pressed["fired"].as_array().unwrap();
                assert!(
                    fired
                        .iter()
                        .any(|f| f["trigger"] == json!(format!("light-{}", next + 1))
                            && f["ok"] == json!(true)),
                    "{name}: {pressed}"
                );
                lit_at_s.push(pressed["time_s"].as_f64().unwrap());
                if next == 2 {
                    let state = run(&mut s, "sim.state", json!({}));
                    message_seen = state["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|m| m["text"] == json!("The valley is safe tonight."));
                }
                steps.push(pressed);
                run(
                    &mut s,
                    "sim.input",
                    json!({"entity": "Player", "interact": false}),
                );
                next += 1;
            } else {
                run(
                    &mut s,
                    "sim.input",
                    json!({"entity": "Player", "forward": 1, "run": true, "face": [bx, bz]}),
                );
            }
        } else if diligent {
            run(
                &mut s,
                "sim.input",
                json!({"entity": "Player", "forward": 0}),
            );
        }
        last = run(&mut s, "sim.step", json!({"ticks": 1}));
        steps.push(last.clone());
    }
    Run {
        session: s,
        steps,
        lit_at_s,
        message_seen,
    }
}

fn status(s: &Session, id: &str) -> ObjectiveStatus {
    s.current_world().objective(id).unwrap().status
}

fn fired_of<'a>(run: &'a Run, trigger: &str) -> Vec<&'a Value> {
    run.steps
        .iter()
        .flat_map(|step| step["fired"].as_array().unwrap())
        .filter(|f| f["trigger"] == json!(trigger))
        .collect()
}

/// The state hash at the end of the diligent run. It must be the same on every platform: a
/// change here means the simulation or the triggers' results changed. It moved on 2026-11-19
/// (from 00faed105492146f) when `face` became a held facing: the player keeps turning towards
/// the beacon every tick, and the held point is part of the state hash.
const GOLDEN_HASH: &str = "d4e0b83b3a0991b8";

#[test]
fn a_diligent_player_lights_every_beacon_before_the_mist() {
    let one = play(true, 125.0);
    let s = &one.session;
    assert_eq!(one.lit_at_s.len(), 3, "three beacons lit");
    assert!(
        one.lit_at_s.iter().all(|t| *t < 120.0),
        "{:?}",
        one.lit_at_s
    );
    for id in ["beacon-1", "beacon-2", "beacon-3", "light-all"] {
        assert_eq!(status(s, id), ObjectiveStatus::Done, "{id}");
    }
    assert_eq!(s.current_world().vars["lit"], VarValue::Number(3.0));
    assert!(one.message_seen, "the final message was shown");
    // Every beacon's light is on, in the running world only.
    for (name, _) in BEACONS {
        let beacon = s
            .current_world()
            .entities()
            .find(|e| e.name == name)
            .unwrap();
        assert!(beacon.light().unwrap().on, "{name}");
        let in_document = s.document().entities().find(|e| e.name == name).unwrap();
        assert!(!in_document.light().unwrap().on, "{name} in the document");
    }
    // The mist rose at 120 s; nothing failed.
    let mist = fired_of(&one, "mist-rises");
    assert_eq!(mist.len(), 1);
    assert_eq!(mist[0]["ok"], json!(true));
    assert_eq!(mist[0]["tick"], json!(7200));
    assert!(fired_of(&one, "too-late").is_empty());
    assert_eq!(s.current_world().environment.mist, 0.95);
    // The player passed through the beacons' zones on the way.
    let enters = one
        .steps
        .iter()
        .flat_map(|step| step["events"].as_array().unwrap())
        .filter(|e| e["event"] == json!("enter"))
        .count();
    assert!(enters >= 3, "{enters} enter events");

    // A second run gives the same hash after every step, and the same events and triggers.
    let two = play(true, 125.0);
    assert_eq!(one.steps.len(), two.steps.len());
    for (a, b) in one.steps.iter().zip(&two.steps) {
        assert_eq!(a["hash"], b["hash"], "tick {}", a["tick"]);
        assert_eq!(a["events"], b["events"], "tick {}", a["tick"]);
        assert_eq!(a["fired"], b["fired"], "tick {}", a["tick"]);
    }
    let last = one.steps.last().unwrap()["hash"].as_str().unwrap();
    assert_eq!(last, GOLDEN_HASH, "golden hash changed: {last}");
}

#[test]
fn a_dawdling_player_fails_at_120_seconds() {
    let run = play(false, 121.0);
    let s = &run.session;
    assert!(run.lit_at_s.is_empty());
    assert_eq!(status(s, "light-all"), ObjectiveStatus::Failed);
    for n in 1..=3 {
        assert_eq!(status(s, &format!("beacon-{n}")), ObjectiveStatus::Open);
    }
    let too_late = fired_of(&run, "too-late");
    assert_eq!(too_late.len(), 1);
    assert_eq!(too_late[0]["tick"], json!(7200));
    assert_eq!(s.current_world().environment.mist, 0.95);
}
