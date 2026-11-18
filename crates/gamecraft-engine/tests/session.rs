//! The session: dispatch, parameter checks, undo and redo, transactions, replace_document.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, playing, refuse, snapshot, undo_depth};
use gamecraft_engine::{CommandError, Scope, Session, UNDO_LIMIT, example_world};
use gamecraft_world::World;
use serde_json::{Value, json};

#[test]
fn an_unknown_command_names_the_nearest() {
    let mut s = example();
    let e = s.run("entity.ad", &json!({})).unwrap_err();
    assert!(matches!(e, CommandError::Unknown(_)));
    assert_eq!(
        e.to_string(),
        "unknown command `entity.ad`; did you mean `entity.add`?"
    );
}

#[test]
fn specs_are_sorted_and_complete() {
    let s = example();
    let names: Vec<&str> = s.specs().iter().map(|c| c.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    for name in [
        "world.set",
        "terrain.set",
        "terrain.clear",
        "environment.set",
        "entity.add",
        "entity.set",
        "entity.remove",
        "entity.set_component",
        "entity.remove_component",
        "entity.snap_to_ground",
        "sim.start",
        "sim.stop",
        "sim.step",
        "sim.input",
        "sim.state",
        "world.inspect",
    ] {
        let spec = s.spec(name).unwrap_or_else(|| panic!("no spec {name}"));
        assert!(!spec.summary.is_empty() && !spec.label.is_empty());
        let expected = match name {
            "world.inspect" => Scope::Query,
            n if n.starts_with("sim.") => Scope::Play,
            _ => Scope::World,
        };
        assert_eq!(spec.scope, expected, "{name}");
        assert_eq!(
            spec.undoable,
            expected == Scope::World || name == "sim.stop"
        );
    }
    // Every command listed above exists with the right scope; the registry may hold more.
    assert!(names.len() >= 16);
}

#[test]
fn bad_parameters_name_the_command_and_the_parameter() {
    let mut s = example();
    refuse(
        &mut s,
        "entity.add",
        json!({"positon": [0, 0, 0]}),
        &[
            "entity.add: bad parameters: unknown parameter `positon`",
            "did you mean `position`",
            "name, position, at",
        ],
    );
    refuse(
        &mut s,
        "entity.add",
        json!({}),
        &["entity.add: bad parameters: missing `position` or `at`"],
    );
    refuse(&mut s, "entity.remove", json!({}), &["missing `id`"]);
    refuse(
        &mut s,
        "entity.add",
        json!({"position": [0, 0]}),
        &["`position` must be [x, y, z]"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "hills"}),
        &["`kind` must be one of \"flat\", \"noise\""],
    );
    refuse(&mut s, "world.set", json!([1, 2]), &["must be an object"]);
    refuse(&mut s, "world.set", json!("x"), &["must be an object"]);
    refuse(
        &mut s,
        "sim.state",
        json!({"x": 1}),
        &["takes no parameters"],
    );
}

#[test]
fn null_params_are_no_params() {
    let mut s = example();
    assert!(s.run("world.inspect", &Value::Null).is_ok());
    assert!(s.run("entity.snap_to_ground", &Value::Null).is_ok());
}

#[test]
fn undo_and_redo_restore_the_exact_document() {
    let mut s = example();
    let start = s.document().clone();
    s.run("entity.add", &json!({"name": "A", "position": [1, 2, 3]}))
        .unwrap();
    s.run("environment.set", &json!({"rain": 0.5})).unwrap();
    s.run("entity.remove", &json!({"id": "Crate"})).unwrap();
    let end = s.document().clone();
    assert_eq!(undo_depth(&s), 3);
    let rev = s.revision();
    while s.undo().is_ok() {}
    assert_eq!(*s.document(), start);
    assert!(s.can_redo() && !s.can_undo());
    while s.redo().is_ok() {}
    assert_eq!(*s.document(), end);
    assert_eq!(s.revision(), rev + 6);
    assert!(s.undo().is_ok());
    s.run("world.set", &json!({"name": "New"})).unwrap();
    assert!(!s.can_redo(), "a new change clears the redo stack");
}

#[test]
fn nothing_to_undo_or_redo_is_an_error() {
    let mut s = example();
    assert!(
        s.undo()
            .unwrap_err()
            .to_string()
            .contains("nothing to undo")
    );
    assert!(
        s.redo()
            .unwrap_err()
            .to_string()
            .contains("nothing to redo")
    );
}

#[test]
fn a_no_op_pushes_no_step() {
    let mut s = example();
    let rev = s.revision();
    s.run("world.set", &json!({"name": "Example valley"}))
        .unwrap();
    s.run("environment.set", &json!({})).unwrap();
    s.run("entity.set", &json!({"id": 1})).unwrap();
    assert!(!s.can_undo());
    assert_eq!(s.revision(), rev);
}

#[test]
fn the_undo_limit_holds() {
    let mut s = example();
    for i in 0..(UNDO_LIMIT + 30) {
        s.run("world.set", &json!({"seed": i + 100})).unwrap();
    }
    assert_eq!(undo_depth(&s), UNDO_LIMIT);
}

#[test]
fn replace_document_clears_the_history_and_stops_playing() {
    let mut s = example();
    s.run("world.set", &json!({"name": "X"})).unwrap();
    s.run("sim.start", &json!({})).unwrap();
    let (rev, view) = (s.revision(), s.view_revision());
    s.replace_document(World::new("Fresh", 1));
    assert!(!s.is_playing() && !s.can_undo() && !s.can_redo());
    assert_eq!(s.document().name, "Fresh");
    assert!(s.revision() > rev && s.view_revision() > view);
    assert!(s.current_heights().is_none());
}

#[test]
fn a_failed_transaction_rolls_everything_back() {
    let mut s = example();
    s.run("world.set", &json!({"name": "Before"})).unwrap();
    let before = snapshot(&s);
    let e = s
        .transaction(|s| {
            s.run("entity.add", &json!({"name": "A", "position": [0, 0, 0]}))?;
            s.run("environment.set", &json!({"mist": 0.4}))?;
            s.run("entity.remove", &json!({"id": "nobody"}))
        })
        .unwrap_err();
    assert!(e.to_string().starts_with("entity.remove: "), "{e}");
    assert_eq!(snapshot(&s), before);
}

#[test]
fn a_failed_transaction_restores_the_play_state() {
    let mut s = example();
    let before = snapshot(&s);
    let r: Result<(), CommandError> = s.transaction(|s| {
        s.run("sim.start", &json!({}))?;
        s.run("sim.step", &json!({"ticks": 10}))?;
        s.run("entity.add", &json!({"name": "B", "position": [1, 0, 1]}))?;
        s.run("sim.stop", &json!({"keep": true}))?;
        s.run("sim.stop", &json!({}))?;
        Ok(())
    });
    assert!(r.is_err());
    assert_eq!(snapshot(&s), before);

    let mut p = playing();
    p.run("sim.step", &json!({"ticks": 5})).unwrap();
    let before = snapshot(&p);
    let r: Result<(), CommandError> = p.transaction(|s| {
        s.run("sim.stop", &json!({}))?;
        s.run("nope", &json!({}))?;
        Ok(())
    });
    assert!(r.is_err());
    assert_eq!(snapshot(&p), before);
    assert_eq!(p.live().map(|l| l.tick()), Some(5));
}

#[test]
fn a_successful_transaction_is_one_undo_step() {
    let mut s = example();
    let start = s.document().clone();
    let id = s
        .transaction(|s| {
            let r = s.run("entity.add", &json!({"name": "A", "position": [0, 0, 0]}))?;
            s.run("environment.set", &json!({"mist": 0.4}))?;
            s.run("world.set", &json!({"name": "After"}))?;
            assert!(s.undo().is_err(), "undo is refused inside a transaction");
            Ok(r["id"].clone())
        })
        .unwrap();
    assert_eq!(id, json!(4));
    assert_eq!(undo_depth(&s), 1);
    s.undo().unwrap();
    assert_eq!(*s.document(), start);
    s.redo().unwrap();
    assert_eq!(s.document().name, "After");
}

#[test]
fn a_transaction_that_changes_nothing_pushes_no_step() {
    let mut s = example();
    s.transaction(|s| {
        s.run("world.set", &json!({"name": "X"}))?;
        s.run("world.set", &json!({"name": "Example valley"}))
    })
    .unwrap();
    assert!(!s.can_undo());
}

#[test]
fn nested_transactions_join_the_outer_one() {
    let mut s = example();
    let before = snapshot(&s);
    let r: Result<(), CommandError> = s.transaction(|s| {
        s.transaction(|s| s.run("world.set", &json!({"name": "Inner"})))?;
        s.run("nope", &json!({}))?;
        Ok(())
    });
    assert!(r.is_err());
    assert_eq!(snapshot(&s), before);
}

#[test]
fn the_height_field_is_cached_and_follows_the_terrain() {
    let mut s = example();
    let field = s.current_heights().unwrap().clone();
    assert_eq!(field.cols(), 65);
    s.run("entity.add", &json!({"position": [0, 0, 0]}))
        .unwrap();
    assert_eq!(*s.current_heights().unwrap(), field);
    s.run(
        "terrain.set",
        &json!({"kind": "flat", "size_m": 64, "cell_m": 4, "height_m": 3}),
    )
    .unwrap();
    let flat = s.current_heights().unwrap();
    assert_eq!((flat.cols(), flat.height_at(0.0, 0.0)), (17, Some(3.0)));
    s.undo().unwrap();
    assert_eq!(*s.current_heights().unwrap(), field);
    s.run("terrain.clear", &json!({})).unwrap();
    assert!(s.current_heights().is_none());
}

#[test]
fn new_session_of_an_empty_world() {
    let mut s = Session::new(World::new("", 0));
    assert!(s.current_heights().is_none() && !s.is_playing());
    assert_eq!(s.revision(), 0);
    s.run("world.inspect", &json!({})).unwrap();
    assert_eq!(*s.document(), World::new("", 0));
    let _ = example_world();
}
