//! The scenarios natively: every one runs without error, records the checkpoints it promises,
//! and a second run gives the same hashes (the native half of `cargo xtask wasm-hash`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use gamecraft_wasmcheck::{Outcome, SCENARIOS, report, report_json, run_scenario};
use serde_json::Value;

fn outcome(name: &str) -> Outcome {
    let scenario = SCENARIOS
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no scenario {name}"));
    let outcome = run_scenario(scenario);
    assert_eq!(outcome.error, None, "{name}");
    outcome
}

fn labels(outcome: &Outcome) -> Vec<&str> {
    outcome.hashes.iter().map(|(l, _)| l.as_str()).collect()
}

#[test]
fn the_scenarios_are_the_ones_the_brief_names() {
    let names: Vec<&str> = SCENARIOS.iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        [
            "example-600-ticks",
            "lantern-playthrough",
            "skirmish-playthrough",
            "query-aim",
            "terrain-4km",
            "scatter",
        ]
    );
}

#[test]
fn the_example_play_records_a_hash_every_60_ticks() {
    let o = outcome("example-600-ticks");
    let expected: Vec<String> = (0..=10).map(|i| format!("tick {}", i * 60)).collect();
    assert_eq!(labels(&o), expected);
}

#[test]
fn the_lantern_playthrough_is_won_and_hashed_every_second() {
    let o = outcome("lantern-playthrough");
    // 125 s at 60 ticks per second, a hash every 60 ticks from tick 0, plus the outcome.
    assert_eq!(o.hashes.len(), 125 + 1 + 1, "{:?}", labels(&o));
    assert_eq!(o.hashes[1].0, "tick 60");
    assert_eq!(o.hashes[125].0, "tick 7500");
    let (label, won) = o.hashes.last().unwrap();
    assert_eq!(label, "objective light-all");
    assert_eq!(won, "done");
}

#[test]
fn the_skirmish_playthrough_is_won() {
    let o = outcome("skirmish-playthrough");
    assert_eq!(o.hashes.len(), 60 + 1 + 1, "{:?}", labels(&o));
    let (label, won) = o.hashes.last().unwrap();
    assert_eq!(label, "objective clear");
    assert_eq!(won, "done");
}

#[test]
fn every_aim_answer_is_hashed() {
    let o = outcome("query-aim");
    assert!(o.hashes.len() >= 6, "{:?}", labels(&o));
    assert!(o.hashes.iter().all(|(_, h)| h.len() == 16));
}

#[test]
fn the_4km_terrain_is_hashed_in_bands_of_rows() {
    let o = outcome("terrain-4km");
    // 2049 rows in bands of 128: 17 bands, the last one row; then the samples and the weights.
    let bands = o
        .hashes
        .iter()
        .filter(|(l, _)| l.starts_with("rows "))
        .count();
    assert_eq!(bands, 17, "{:?}", labels(&o));
    assert_eq!(o.hashes[0].0, "rows 0-127");
    assert!(labels(&o).contains(&"rows 2048-2048"));
    assert!(labels(&o).contains(&"weights"));
    assert!(labels(&o).contains(&"height_at and normal_at"));
}

#[test]
fn the_scatter_places_copies_and_hashes_the_document() {
    let o = outcome("scatter");
    assert!(labels(&o).contains(&"document"), "{:?}", labels(&o));
}

#[test]
fn two_runs_give_the_same_report_and_its_json_lists_every_scenario() {
    let first = report();
    let second = report();
    assert_eq!(first, second);
    let json: Value = serde_json::from_str(&report_json()).unwrap();
    let scenarios = json["scenarios"].as_array().unwrap();
    assert_eq!(scenarios.len(), SCENARIOS.len());
    for s in scenarios {
        assert_eq!(s["error"], Value::Null, "{s}");
        assert!(!s["hashes"].as_array().unwrap().is_empty(), "{s}");
    }
}
