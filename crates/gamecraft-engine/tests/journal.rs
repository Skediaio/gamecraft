//! The journal: a directed session is recorded with its ticks, replays to the same hash at
//! every recorded entry, and a journal edited by hand diverges where it was edited.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, playing};
use gamecraft_engine::Session;
use gamecraft_engine::journal::Journal;
use serde_json::{Value, json};

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.run(command, &params)
        .unwrap_or_else(|e| panic!("{command} {params}: {e}"))
}

/// A directed session on the example world: inputs, a wall dropped in the scout's way, the
/// weather changed, an entity added, a shot fired, a message, a variable that fires the
/// `oak-found` trigger, queries in between. Returns the session, still playing.
fn directed() -> Session {
    let mut s = example();
    run(&mut s, "sim.start", json!({}));
    run(
        &mut s,
        "sim.input",
        json!({"entity": "Scout", "forward": 1, "face": [0, -60], "run": true}),
    );
    run(&mut s, "sim.step", json!({"ticks": 30}));
    run(&mut s, "world.inspect", json!({"limit": 5}));
    run(
        &mut s,
        "entity.add",
        json!({"name": "Wall", "position": [0, 0, -12], "components": [
            {"type": "shape", "kind": "box", "size": [8, 3, 1]}]}),
    );
    run(&mut s, "entity.snap_to_ground", json!({"id": "Wall"}));
    run(
        &mut s,
        "environment.set",
        json!({"rain": 0.4, "wind_speed_mps": 6}),
    );
    run(&mut s, "sim.step", json!({"seconds": 0.5}));
    run(
        &mut s,
        "sim.fire",
        json!({"entity": "Scout", "pitch_deg": 1}),
    );
    run(
        &mut s,
        "message.show",
        json!({"text": "Contact north", "seconds": 2}),
    );
    run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "marker", "at": [5, -5], "name": "Drop"}),
    );
    run(&mut s, "var.set", json!({"name": "score", "value": 3}));
    run(&mut s, "sim.state", json!({}));
    run(&mut s, "sim.step", json!({"ticks": 45}));
    run(
        &mut s,
        "sim.input",
        json!({"entity": "Scout", "face": [20, -30], "forward": 0.5}),
    );
    run(&mut s, "sim.step", json!({"ticks": 20}));
    s
}

#[test]
fn a_directed_session_records_its_commands_with_their_ticks() {
    let s = directed();
    let journal = s.journal().expect("a journal while playing");
    let commands: Vec<&str> = journal.entries.iter().map(|e| e.command.as_str()).collect();
    assert_eq!(
        commands,
        [
            "sim.input",
            "sim.step",
            "entity.add",
            "entity.snap_to_ground",
            "environment.set",
            "sim.step",
            "sim.fire",
            "message.show",
            "prefab.spawn",
            "var.set",
            "sim.step",
            "sim.input",
            "sim.step",
        ],
        "queries and the trigger's objective.set are not recorded"
    );
    let ticks: Vec<u64> = journal.entries.iter().map(|e| e.tick).collect();
    assert_eq!(ticks, [0, 0, 30, 30, 30, 30, 60, 60, 60, 60, 60, 105, 105]);
    let step = &journal.entries[5];
    assert_eq!(step.end_tick, Some(60));
    assert!(journal.entries.iter().all(|e| e.hash.is_some()));
    let end = journal.end.clone().unwrap();
    assert_eq!(end.tick, 125);
    assert_eq!(
        end.hash,
        run(&mut s.clone(), "sim.state", json!({}))["hash"]
    );
    // The trigger fired in the recorded session.
    let objective = s.current_world().objective("find-oak").unwrap();
    assert_eq!(objective.status, gamecraft_world::ObjectiveStatus::Done);
}

#[test]
fn a_replay_gives_the_same_hash_at_every_recorded_entry() {
    let s = directed();
    let journal = s.journal().unwrap();
    let replay = journal.replay(None).unwrap();
    assert_eq!(replay.divergence, None);
    assert_eq!(replay.hashes_checked, journal.entries.len() + 1);
    assert_eq!(replay.tick, 125);
    assert_eq!(
        Some(replay.hash.clone()),
        journal.end.clone().map(|e| e.hash)
    );
    assert_eq!(
        replay.session.current_world(),
        s.current_world(),
        "the replayed world is the recorded one"
    );
    // Through the file format too.
    let text = journal.to_json().unwrap();
    let back = Journal::from_json(&text).unwrap();
    assert_eq!(&back, journal);
    assert_eq!(back.replay(None).unwrap().divergence, None);
}

#[test]
fn a_journal_edited_by_hand_diverges_at_the_edited_tick() {
    let s = directed();
    let text = s.journal().unwrap().to_json().unwrap();
    // The second input, at tick 105: half speed becomes full speed.
    let edited = text.replacen(
        r#""face":[20,-30],"forward":0.5"#,
        r#""face":[20,-30],"forward":1.0"#,
        1,
    );
    assert_ne!(edited, text, "the input is in the file as written: {text}");
    let journal = Journal::from_json(&edited).unwrap();
    let replay = journal.replay(None).unwrap();
    let d = replay.divergence.expect("the edited journal diverges");
    assert_eq!(d.entry, Some(11));
    assert_eq!(d.tick, 105);
    assert!(d.message.contains("tick 105"), "{}", d.message);
    assert!(d.message.contains("sim.input"), "{}", d.message);
    assert_eq!(replay.hashes_checked, 11, "every entry before it matched");
}

#[test]
fn a_replay_can_stop_at_a_tick_to_bisect() {
    let s = directed();
    let journal = s.journal().unwrap();
    // Inside the second step (30 to 60): cut short at 45.
    let replay = journal.replay(Some(45)).unwrap();
    assert_eq!(replay.tick, 45);
    assert_eq!(replay.divergence, None);
    // At 60, before the commands recorded at tick 60.
    let replay = journal.replay(Some(60)).unwrap();
    assert_eq!(replay.tick, 60);
    assert!(replay.session.current_world().find("Drop").is_err());
    assert_eq!(replay.entries_run, 6);
    // Beyond the end: the whole journal.
    let replay = journal.replay(Some(10_000)).unwrap();
    assert_eq!(replay.tick, 125);
}

#[test]
fn the_journal_survives_sim_stop_until_the_next_start() {
    let mut s = directed();
    let entries = s.journal().unwrap().entries.len();
    run(&mut s, "sim.stop", json!({}));
    let got = run(&mut s, "journal.get", json!({}));
    assert_eq!(got["recording"], false);
    assert_eq!(got["entry_count"], entries);
    assert_eq!(got["final"]["tick"], 125);
    // Editing the document is not recorded.
    run(&mut s, "environment.set", json!({"mist": 0.2}));
    assert_eq!(s.journal().unwrap().entries.len(), entries);
    run(&mut s, "sim.start", json!({}));
    let got = run(&mut s, "journal.get", json!({}));
    assert_eq!(got["recording"], true);
    assert_eq!(got["entry_count"], 0);
    assert_eq!(got["final"]["tick"], 0);
    // The new journal starts from the edited document.
    assert_eq!(s.journal().unwrap().world.environment.mist, 0.2);
}

#[test]
fn journal_get_filters_by_tick_and_pages() {
    let mut s = directed();
    let got = run(
        &mut s,
        "journal.get",
        json!({"from_tick": 30, "to_tick": 60, "limit": 3}),
    );
    assert_eq!(got["matched"], 9);
    let entries = got["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0]["index"], 2);
    assert_eq!(entries[0]["command"], "entity.add");
    assert_eq!(entries[0]["tick"], 30);
    assert_eq!(entries[0]["hash"].as_str().unwrap().len(), 16);
    common::refuse(&mut s, "journal.get", json!({"limit": 0}), &["`limit`"]);
    common::refuse(
        &mut s,
        "journal.get",
        json!({"from_tick": 9, "to_tick": 2}),
        &["`from_tick`"],
    );
    // Before any simulation there is no journal, and that is no error.
    let got = run(&mut example(), "journal.get", json!({}));
    assert_eq!(got["recording"], false);
    assert_eq!(got["entries"], json!([]));
}

#[test]
fn a_failed_transaction_leaves_the_journal_as_it_was() {
    let mut s = playing();
    run(&mut s, "sim.step", json!({"ticks": 2}));
    let before = s.journal().unwrap().clone();
    let outcome = s.transaction(|s| {
        s.run("sim.step", &json!({"ticks": 3}))?;
        s.run("entity.remove", &json!({"id": "nobody"}))
    });
    assert!(outcome.is_err());
    assert_eq!(s.journal().unwrap(), &before);
    // A transaction that stops and starts again, then fails, puts the old journal back.
    let outcome = s.transaction(|s| {
        s.run("sim.stop", &json!({}))?;
        s.run("sim.start", &json!({}))?;
        s.run("entity.remove", &json!({"id": "nobody"}))
    });
    assert!(outcome.is_err());
    assert_eq!(s.journal().unwrap(), &before);
}

#[test]
fn a_journal_file_names_what_is_wrong() {
    let err = Journal::from_json(r#"{"gamecraft_journal": 2, "world": {}}"#).unwrap_err();
    assert!(err.contains("gamecraft_journal"), "{err}");
    let err = Journal::from_json(
        r#"{"gamecraft_journal": 1, "world": {"gamecraft": 1}, "entries": [{"tick": 0}]}"#,
    )
    .unwrap_err();
    assert!(err.contains("command"), "{err}");
    let err =
        Journal::from_json(r#"{"gamecraft_journal": 1, "world": {"gamecraft": 9}}"#).unwrap_err();
    assert!(err.contains("world"), "{err}");
    // A recorded command that no longer runs is a divergence, not a crash.
    let mut s = playing();
    run(&mut s, "sim.step", json!({"ticks": 1}));
    let mut journal = s.journal().unwrap().clone();
    journal.entries[0].command = "sim.input".into();
    journal.entries[0].params = json!({"entity": "Nobody"});
    let d = journal.replay(None).unwrap().divergence.unwrap();
    assert_eq!(d.entry, Some(0));
    assert!(d.message.contains("failed in the replay"), "{}", d.message);
}
