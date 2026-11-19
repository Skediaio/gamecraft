//! `journal.save`: the journal of a directed session is written inside the write root and
//! replays to the same hash.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use gamecraft_engine::journal::Journal;
use serde_json::json;

#[test]
fn a_saved_journal_replays_to_the_same_hash() {
    let dir = TempDir::new("journal");
    let mut h = host_in(&dir);
    assert!(err(&mut h, "journal.save", json!({"path": "early.json"})).contains("sim.start"));
    ok(&mut h, "sim.start", json!({}));
    ok(
        &mut h,
        "sim.input",
        json!({"entity": "Scout", "forward": 1, "face": [0, -40]}),
    );
    ok(&mut h, "sim.step", json!({"ticks": 20}));
    ok(&mut h, "environment.set", json!({"mist": 0.5}));
    ok(&mut h, "sim.step", json!({"ticks": 20}));
    let state = ok(&mut h, "sim.state", json!({}));
    let saved = ok(&mut h, "journal.save", json!({"path": "run.journal.json"}));
    assert_eq!(saved["entries"], 4);
    assert_eq!(saved["final"]["tick"], 40);
    assert_eq!(saved["final"]["hash"], state["hash"]);
    let text = std::fs::read_to_string(dir.join("run.journal.json")).unwrap();
    assert_eq!(saved["bytes"], text.len());
    let journal = Journal::from_json(&text).unwrap();
    let replay = journal.replay(None).unwrap();
    assert_eq!(replay.divergence, None);
    assert_eq!(replay.hash, state["hash"]);

    // After sim.stop the journal can still be saved; an existing file needs overwrite.
    ok(&mut h, "sim.stop", json!({}));
    let e = err(&mut h, "journal.save", json!({"path": "run.journal.json"}));
    assert!(e.starts_with("journal.save: "), "{e}");
    assert!(e.contains("overwrite"), "{e}");
    ok(
        &mut h,
        "journal.save",
        json!({"path": "run.journal.json", "overwrite": true}),
    );
    let e = err(&mut h, "journal.save", json!({"path": "run.txt"}));
    assert!(e.contains(".json"), "{e}");
    let e = err(&mut h, "journal.save", json!({"path": "../out.json"}));
    assert!(e.contains(".."), "{e}");
}
