//! AGENTS.md rule 12: no command panics, for any parameters and any session state.
//!
//! For every spec of the registry (so a new command is picked up by itself), the hunt runs
//! the command with hostile values for each parameter, with parameters missing and extra, and
//! with `params` that is not an object, in four session states. Every reply must be `Ok` or
//! `Err`; a panic reports the command, the state and the parameters.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use gamecraft_engine::{Session, example_world};
use gamecraft_world::World;
use serde_json::{Map, Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn nested(depth: usize) -> Value {
    let mut v = json!(0);
    for _ in 0..depth {
        v = Value::Array(vec![v]);
    }
    v
}

fn hostile_values() -> Vec<Value> {
    let long = "x".repeat(10_000);
    vec![
        Value::Null,
        json!(true),
        json!(false),
        json!(0),
        json!(-0.0),
        json!(1),
        json!(-1),
        json!(0.5),
        json!(1e-300),
        json!(1e300),
        json!(-1e300),
        json!(f64::MAX),
        json!(f64::MIN),
        json!(u64::MAX),
        json!(i64::MIN),
        json!(4_294_967_296u64),
        json!(36_001),
        json!(""),
        json!("NaN"),
        json!("inf"),
        json!("-Infinity"),
        json!("1"),
        json!("Scout"),
        json!("shape"),
        json!(long),
        json!([]),
        json!([0, 0]),
        json!([0, 0, 0]),
        json!([1e300, -1e300]),
        json!([1e300, 1e300, 1e300]),
        json!([-1e300, 0, 1e300]),
        json!(["a", "b", "c"]),
        json!([1, 2, 3, 4]),
        Value::Array(vec![json!(0); 10_000]),
        nested(100),
        json!({}),
        json!({"type": "shape"}),
        json!({"type": "shape", "kind": "sphere", "size": [1e300, 1e300, 1e300]}),
        json!({"type": "plant", "species": long}),
        json!({"type": "plant", "species": "quercus-robur", "growth": 1e-300}),
        json!({"type": "character", "height_m": 1e300, "radius_m": 1e300}),
        json!({"type": 5}),
        json!([{"type": "character"}, {"type": "character"}]),
        Value::Array(vec![json!({"type": "shape", "kind": "box"}); 1000]),
    ]
}

fn states() -> Vec<(&'static str, Session)> {
    let mut playing = Session::new(example_world());
    playing.run("sim.start", &json!({})).unwrap();
    let mut no_terrain = example_world();
    no_terrain.terrain = None;
    vec![
        ("empty world", Session::new(World::new("", 0))),
        ("example world", Session::new(example_world())),
        ("example world while playing", playing),
        ("world without terrain", Session::new(no_terrain)),
    ]
}

/// Runs one call on a clone of `base`; a panic fails the test, naming the call.
fn call(base: &Session, state: &str, command: &str, params: &Value, panics: &mut Vec<String>) {
    let mut s = base.clone();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let _ = s.run(command, params);
    }));
    if outcome.is_err() {
        let mut text = params.to_string();
        text.truncate(300);
        panics.push(format!("{command} in {state} with {text}"));
    }
}

#[test]
fn no_command_panics_on_hostile_parameters() {
    let values = hostile_values();
    let not_objects = [json!(1), json!("x"), json!([]), json!(true), json!([{}])];
    let mut panics = Vec::new();
    let mut calls = 0usize;
    for (state, base) in states() {
        let specs: Vec<_> = base.specs().into_iter().cloned().collect();
        for spec in specs {
            let example = spec.example.as_object().cloned().unwrap_or_default();
            let mut run = |params: &Value| {
                calls += 1;
                call(&base, state, &spec.name, params, &mut panics);
            };
            for p in &not_objects {
                run(p);
            }
            run(&Value::Null);
            run(&json!({}));
            run(&Value::Object(example.clone()));
            let mut extra = example.clone();
            extra.insert("zzz_extra".into(), json!(1));
            run(&Value::Object(extra));
            for param in &spec.params {
                let mut missing = example.clone();
                missing.remove(&param.name);
                run(&Value::Object(missing));
                for v in &values {
                    let mut with = example.clone();
                    with.insert(param.name.clone(), v.clone());
                    run(&Value::Object(with));
                    let mut alone = Map::new();
                    alone.insert(param.name.clone(), v.clone());
                    run(&Value::Object(alone));
                }
            }
        }
    }
    // Commands that do not exist, with hostile ids.
    let base = Session::new(World::new("", 0));
    for name in ["", "x", "entity.", ".", &"e".repeat(10_000), "entity.add\0"] {
        call(&base, "empty world", name, &json!({}), &mut panics);
    }
    assert!(calls > 5000, "only {calls} calls");
    assert!(
        panics.is_empty(),
        "{} panics:\n{}",
        panics.len(),
        panics.join("\n")
    );
}

#[test]
fn undo_redo_and_transactions_do_not_panic_in_any_state() {
    for (_, mut s) in states() {
        let _ = s.undo();
        let _ = s.redo();
        let _ = s.transaction(|s| {
            let _ = s.run("sim.start", &json!({}));
            let _ = s.undo();
            s.run("nope", &json!({}))
        });
        let _ = s.transaction(|s| s.transaction(|s| s.run("world.inspect", &json!({}))));
        s.replace_document(World::new("", 0));
        let _ = s.undo();
    }
}
