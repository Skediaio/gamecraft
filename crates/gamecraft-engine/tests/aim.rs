//! `query.aim`: the firing solution of a launcher against the air and wind of the current
//! world, and a shot fired with it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::refuse;
use gamecraft_engine::Session;
use gamecraft_math::{Portable, rad_to_deg};
use gamecraft_world::World;
use serde_json::{Value, json};

/// A flat range with a west wind: `Shooter` (a rifle without spread) at the origin, `Target`
/// 150 m north, `Post` (a launcher on a post) and `Rock` (a shape) beside them.
fn range(wind_mps: f64) -> Session {
    let mut s = Session::new(World::new("Range", 3));
    let mut run = |c: &str, p: Value| s.run(c, &p).unwrap();
    run(
        "terrain.set",
        json!({"kind": "flat", "size_m": 512, "cell_m": 4}),
    );
    run(
        "environment.set",
        json!({"wind_speed_mps": wind_mps, "wind_direction_deg": 270}),
    );
    run(
        "entity.add",
        json!({"name": "Shooter", "at": [0, 100], "components": [
            {"type": "character"},
            {"type": "launcher", "dispersion_mrad": 0, "muzzle_velocity_mps": 800,
             "ballistic_coefficient": 199.6}]}),
    );
    run(
        "entity.add",
        json!({"name": "Target", "at": [0, -50], "components": [
            {"type": "character"}, {"type": "health", "max": 100, "current": 100}]}),
    );
    run(
        "entity.add",
        json!({"name": "Mortar", "at": [5, 100], "components": [
            {"type": "launcher", "muzzle_velocity_mps": 120, "dispersion_mrad": 0}]}),
    );
    run(
        "entity.add",
        json!({"name": "Rock", "at": [20, 100], "components": [
            {"type": "shape", "kind": "box", "size": [1, 1, 1]}]}),
    );
    s
}

#[test]
fn the_solution_leads_into_the_wind_and_its_shot_hits() {
    let mut s = range(8.0);
    // It works while editing, on the document.
    let a = s
        .run("query.aim", &json!({"from": "Shooter", "to": "Target"}))
        .unwrap();
    assert_eq!(a["reachable"], json!(true), "{a}");
    let yaw = a["yaw_deg"].as_f64().unwrap();
    let pitch = a["pitch_deg"].as_f64().unwrap();
    assert!(yaw > 0.0 && yaw < 1.0, "{a}");
    // The chest is below the eyes: aimed down, but less than straight at it (the drop).
    let straight = rad_to_deg((1.8 * 0.65 - 1.65_f64).patan2(150.0));
    assert!(pitch > straight && pitch < 1.0, "{a} vs {straight}");
    for key in ["time_of_flight_s", "drop_m", "drift_m"] {
        assert!(a[key].as_f64().unwrap() > 0.0, "{key}: {a}");
    }
    assert!(a["miss_m"].as_f64().unwrap() <= 0.02, "{a}");
    assert_eq!(a["target"], json!([0.0, 0.0 + 1.8 * 0.65, -50.0]));
    // Fired as it says, it hits.
    s.run("sim.start", &json!({})).unwrap();
    s.run(
        "sim.fire",
        &json!({"entity": "Shooter", "yaw_deg": yaw, "pitch_deg": pitch}),
    )
    .unwrap();
    let r = s.run("sim.step", &json!({"seconds": 0.5})).unwrap();
    let hit = r["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event"] == "hit")
        .unwrap_or_else(|| panic!("no hit: {r}"))
        .clone();
    assert_eq!(hit["target"], json!(2), "{hit}");
}

#[test]
fn points_other_launchers_and_out_of_reach() {
    let mut s = range(0.0);
    // A point; the launcher of another entity, fired from `from`.
    let a = s
        .run(
            "query.aim",
            &json!({"from": "Shooter", "to": [0, 1.65, -300]}),
        )
        .unwrap();
    assert_eq!(a["reachable"], json!(true), "{a}");
    let slow = s
        .run(
            "query.aim",
            &json!({"from": "Shooter", "to": [0, 1.65, -300], "launcher": "Mortar"}),
        )
        .unwrap();
    assert_eq!(slow["reachable"], json!(true), "{slow}");
    assert!(
        slow["pitch_deg"].as_f64().unwrap() > a["pitch_deg"].as_f64().unwrap() * 5.0,
        "{slow} vs {a}"
    );
    // The mortar alone: from its post, but 3 km is beyond 120 m/s.
    let far = s
        .run("query.aim", &json!({"from": "Mortar", "to": [0, 0, -3000]}))
        .unwrap();
    assert_eq!(far["reachable"], json!(false), "{far}");
}

#[test]
fn bad_parameters_are_named() {
    let mut s = range(0.0);
    for (params, needles) in [
        (json!({"from": "Nobody", "to": [0, 0, 0]}), vec!["`from`"]),
        (
            json!({"from": "Rock", "to": [0, 0, 0]}),
            vec!["launcher", "Rock"],
        ),
        (json!({"from": "Target", "to": [0, 0, 0]}), vec!["launcher"]),
        (
            json!({"from": "Shooter", "to": [0, 0]}),
            vec!["`to`", "[x, y, z]"],
        ),
        (json!({"from": "Shooter", "to": true}), vec!["`to`"]),
        (
            json!({"from": "Shooter", "to": "Nobody"}),
            vec!["`to`", "Nobody"],
        ),
        (
            json!({"from": "Shooter", "to": "Shooter"}),
            vec!["`to`", "itself"],
        ),
        (
            json!({"from": "Shooter", "to": [0, 0, 0], "launcher": "Rock"}),
            vec!["`launcher`"],
        ),
        (json!({"from": "Shooter"}), vec!["`to`"]),
    ] {
        refuse(&mut s, "query.aim", params, &needles);
    }
}
