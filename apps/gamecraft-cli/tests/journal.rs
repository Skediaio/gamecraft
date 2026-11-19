//! `play --journal` and `replay`: both prototype playthroughs write journals that replay to
//! the same hash, a journal edited by hand diverges and `--check` names the tick, and
//! `--to-tick` stops early to bisect.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
use common::*;
use serde_json::Value;
use std::path::PathBuf;

fn game(name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().and_then(|p| p.parent()).unwrap();
    root.join("examples").join(name)
}

fn text(path: &std::path::Path) -> String {
    path.display().to_string()
}

fn one_json(ran: &Ran) -> Value {
    serde_json::from_str(ran.out.trim())
        .unwrap_or_else(|e| panic!("not one line of JSON: {e}: {}{}", ran.out, ran.err))
}

/// Plays a prototype with its playthrough and writes the journal: (state, journal path).
fn play_with_journal(temp: &Temp, name: &str, seconds: &str) -> (Value, PathBuf) {
    let dir = game(name);
    let journal = temp.path(&format!("{name}.journal.json"));
    let ran = run(&[
        "play",
        &text(&dir.join("world.gamecraft")),
        "--script",
        &text(&dir.join("playthrough.json")),
        "--seconds",
        seconds,
        "--journal",
        &text(&journal),
    ]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
    (one_json(&ran), journal)
}

#[test]
fn both_prototype_playthroughs_write_journals_that_replay() {
    let temp = Temp::new("journal-prototypes");
    for (name, seconds) in [("lantern", "125"), ("skirmish", "60")] {
        let (state, journal) = play_with_journal(&temp, name, seconds);
        let ran = run(&["replay", &text(&journal), "--check"]);
        assert_eq!(ran.code, 0, "{name}: {}{}", ran.out, ran.err);
        let replay = one_json(&ran);
        assert_eq!(replay["hash"], state["hash"], "{name}: {replay}");
        assert_eq!(replay["tick"], state["tick"], "{name}");
        assert_eq!(replay["matches"], true, "{name}");
        assert_eq!(replay["divergence"], Value::Null, "{name}");
        assert!(replay["hashes_checked"].as_u64().unwrap() > 10, "{name}");
    }
}

#[test]
fn a_journal_edited_by_hand_fails_the_check_at_the_edited_tick() {
    let temp = Temp::new("journal-edited");
    let (_, journal) = play_with_journal(&temp, "skirmish", "60");
    let original = std::fs::read_to_string(&journal).unwrap();
    // The third input of the playthrough, at tick 708 + 132 = 840.
    let edited = original.replacen(r#""face":[18,46]"#, r#""face":[18,47]"#, 1);
    assert_ne!(edited, original);
    let path = temp.write("edited.journal.json", &edited);
    let ran = run(&["replay", &text(&path), "--check"]);
    assert_eq!(ran.code, 1, "{}{}", ran.out, ran.err);
    assert!(ran.err.contains("tick 840"), "{}", ran.err);
    assert!(ran.err.contains("sim.input"), "{}", ran.err);
    // Without --check the answer says so and the exit code is 0.
    let ran = run(&["replay", &text(&path)]);
    assert_eq!(ran.code, 0, "{}", ran.err);
    let replay = one_json(&ran);
    assert_eq!(replay["matches"], false);
    assert_eq!(replay["divergence"]["tick"], 840);
}

#[test]
fn to_tick_stops_early_and_prints_the_state() {
    let temp = Temp::new("journal-bisect");
    let (_, journal) = play_with_journal(&temp, "lantern", "30");
    let ran = run(&["replay", &text(&journal), "--to-tick", "500", "--check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
    let state = one_json(&ran);
    assert_eq!(state["playing"], true);
    assert_eq!(state["tick"], 500);
    assert!(state["characters"].is_array());
    assert_eq!(state["replay"]["divergence"], Value::Null);
}

#[test]
fn replay_names_what_is_wrong() {
    let temp = Temp::new("journal-bad");
    let ran = run(&["replay"]);
    assert_eq!(ran.code, 2, "{}", ran.err);
    let ran = run(&["replay", "nothing-here.json"]);
    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("nothing-here.json"), "{}", ran.err);
    let bad = temp.write("bad.json", "{\"gamecraft_journal\": 7, \"world\": {}}");
    let ran = run(&["replay", &text(&bad)]);
    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("gamecraft_journal"), "{}", ran.err);
    let ran = run(&["replay", &text(&bad), "--to-tick", "soon"]);
    assert_eq!(ran.code, 2);
    assert!(ran.err.contains("--to-tick"), "{}", ran.err);
}
