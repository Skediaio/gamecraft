//! The two prototype games of `examples/`, through the built binary: each is rebuilt from its
//! `build.json` and must equal the committed world, its `playthrough.json` must win it with a
//! pinned state hash that a second run repeats, and a player who does nothing useful loses.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
use common::*;
use serde_json::{Value, json};
use std::path::PathBuf;

/// The directory of one example game.
fn game(name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().and_then(|p| p.parent()).unwrap();
    root.join("examples").join(name)
}

fn text(path: &std::path::Path) -> String {
    path.display().to_string()
}

/// Runs the binary and parses its one JSON answer; it must exit with code 0.
fn json_of(args: &[&str]) -> Value {
    let ran = run(args);
    assert_eq!(ran.code, 0, "{args:?}: {}{}", ran.out, ran.err);
    serde_json::from_str(&ran.out).unwrap_or_else(|e| panic!("{args:?}: not JSON: {e}"))
}

/// Builds the game from its script with the parameters its README gives and checks that the
/// result describes the same world as the committed file.
fn assert_build_matches(name: &str, new: &str) {
    let dir = game(name);
    let temp = Temp::new(name);
    let built = temp.path("world.gamecraft");
    let ran = run(&[
        "run",
        "--new",
        new,
        "--script",
        &text(&dir.join("build.json")),
        "--out",
        &text(&built),
    ]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
    let fresh = run(&["info", &text(&built)]);
    let committed = run(&["info", &text(&dir.join("world.gamecraft"))]);
    assert_eq!(fresh.code, 0, "{}", fresh.err);
    assert_eq!(committed.code, 0, "{}", committed.err);
    assert!(
        fresh.out == committed.out,
        "{name}: world.gamecraft is not what build.json builds; rebuild it with the command in the README"
    );
}

/// Plays the committed world with a script for `seconds`: the final state.
fn play(name: &str, script: Option<&str>, seconds: &str) -> Value {
    let dir = game(name);
    let world = text(&dir.join("world.gamecraft"));
    match script {
        Some(script) => json_of(&["play", &world, "--script", script, "--seconds", seconds]),
        None => json_of(&["play", &world, "--seconds", seconds]),
    }
}

fn status(state: &Value, id: &str) -> String {
    state["objectives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap_or_else(|| panic!("no objective {id}: {state}"))["status"]
        .as_str()
        .unwrap()
        .to_string()
}

fn player_is_down(state: &Value) -> bool {
    state["characters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Player")
        .unwrap()["health"]["down"]
        == json!(true)
}

#[test]
fn lantern_world_is_what_its_build_script_builds() {
    assert_build_matches("lantern", r#"{"name": "Lantern", "seed": 11}"#);
}

#[test]
fn skirmish_world_is_what_its_build_script_builds() {
    assert_build_matches("skirmish", r#"{"name": "Skirmish", "seed": 21}"#);
}

/// The state hash after the Lantern playthrough and 125 s in all (the mist has risen once, at
/// 120 s). A change means the simulation, the triggers or the world of the example changed.
///
/// Moved on 2026-11-19 from dbac71f254d2f7ab: `face` became a held facing (part of the state
/// hash), the playthrough lights each beacon in the tick it stops (one input instead of three),
/// and the world changed (trees kept clear of the start and the beacons with `exclude`,
/// messages with `{var:lit}`, the losing trigger conditioned on the objective).
///
/// Moved on 2026-11-19 from f2c1e0c554fd093b: the beacons became instances of a `beacon`
/// prefab, each with a child `Beacon ring` (a 3 m zone tagged `beacon-ring`). The world's text,
/// which the static part of the hash covers, now holds the prefab, the instances' references and
/// the five rings (which also shift the beacons' ids), and the player's passing through the rings
/// is zone membership in the hash. The beacons' own components, the player's path and the
/// objectives are unchanged.
const LANTERN_HASH: &str = "143a88eedd10f8c6";

#[test]
fn lantern_is_won_by_the_playthrough_and_replays_to_the_same_hash() {
    let script = text(&game("lantern").join("playthrough.json"));
    let first = play("lantern", Some(&script), "125");
    for id in [
        "light-all",
        "beacon-meadow",
        "beacon-pines",
        "beacon-ford",
        "beacon-hill",
        "beacon-ridge",
    ] {
        assert_eq!(status(&first, id), "done", "{id}: {first}");
    }
    assert_eq!(first["trigger_errors"], json!([]));
    assert_eq!(first["hash"], LANTERN_HASH, "golden hash changed");
    let second = play("lantern", Some(&script), "125");
    assert_eq!(first["hash"], second["hash"]);
}

#[test]
fn lantern_is_lost_by_standing_still() {
    let state = play("lantern", None, "305");
    assert_eq!(status(&state, "light-all"), "failed", "{state}");
    for beacon in ["meadow", "pines", "ford", "hill", "ridge"] {
        assert_eq!(status(&state, &format!("beacon-{beacon}")), "open");
    }
}

/// The state hash after the Skirmish playthrough and 60 s in all (the last enemy falls at
/// about 53 s). A change means the simulation, the triggers or the example changed.
///
/// Moved on 2026-11-19 from 66424923297bb357: `face` became a held facing (part of the state
/// hash), the playthrough aims with `face_entity` instead of computed yaws and pitches (the
/// shots leave a few ticks apart from before), and the world changed (trees kept clear of the
/// cover with `exclude`, one `enemy-down` message with `{target}` and `{var:enemies_left}`
/// instead of three count triggers, objective conditions on the end triggers).
///
/// Moved on 2026-11-19 from 18006793b8e61716: the four enemies became instances of an
/// `enemy-soldier` prefab (the patrols override its behaviour) and the two houses of a `house`
/// prefab (House 2 overrides its shape). The world's text, which the static part of the hash
/// covers, now holds the prefabs and the instances' references and overrides; every entity has
/// the same id and the same effective components as before, and the fight is the same.
const SKIRMISH_HASH: &str = "0221de111eb7dd5d";

#[test]
fn skirmish_is_won_by_the_playthrough_and_replays_to_the_same_hash() {
    let script = text(&game("skirmish").join("playthrough.json"));
    let first = play("skirmish", Some(&script), "60");
    assert_eq!(status(&first, "clear"), "done", "{first}");
    assert_eq!(status(&first, "alive"), "done", "{first}");
    assert!(!player_is_down(&first));
    assert_eq!(first["trigger_errors"], json!([]));
    assert_eq!(first["hash"], SKIRMISH_HASH, "golden hash changed");
    let second = play("skirmish", Some(&script), "60");
    assert_eq!(first["hash"], second["hash"]);
}

#[test]
fn skirmish_is_lost_by_walking_into_the_open_without_firing() {
    let temp = Temp::new("skirmish-lose");
    // Round the wall, then up the open road to the village, never firing.
    let script = temp.write(
        "lose.json",
        &json!([
            ["sim.input", {"entity": "Player", "face": [20, 45], "forward": 1, "run": true}],
            ["sim.step", {"seconds": 14}],
            ["sim.input", {"entity": "Player", "face": [20, -10]}],
            ["sim.step", {"seconds": 11}],
            ["sim.input", {"entity": "Player", "forward": 0}]
        ])
        .to_string(),
    );
    let state = play("skirmish", Some(&text(&script)), "120");
    assert_eq!(status(&state, "clear"), "failed", "{state}");
    assert_eq!(status(&state, "alive"), "failed", "{state}");
    assert!(player_is_down(&state));
}
