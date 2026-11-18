//! Playing: `sim.*`, and World commands on a running world (AGENTS.md rule 6).
//!
//! The simulation may still be the placeholder of the interface contract here: these tests
//! never assert motion.

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
use gamecraft_engine::{Scope, Session};
use gamecraft_sim::MAX_STEP_TICKS;
use gamecraft_world::{EntityId, World};
use serde_json::{Value, json};

fn is_hash(v: &Value) -> bool {
    v.as_str().is_some_and(|h| {
        h.len() == 16
            && h.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    })
}

#[test]
fn start_gives_tick_hash_and_characters() {
    let mut s = example();
    let view = s.view_revision();
    let r = s.run("sim.start", &json!({})).unwrap();
    assert_eq!(r["tick"], json!(0));
    assert!(is_hash(&r["hash"]), "{r}");
    assert_eq!(r["characters"], json!([3]));
    assert!(s.is_playing() && s.view_revision() > view);
    assert_eq!(s.live().unwrap().world(), s.document());
    refuse(&mut s, "sim.start", json!({}), &["already running"]);
}

#[test]
fn play_commands_that_need_a_simulation_refuse_without_one() {
    let mut s = example();
    refuse(
        &mut s,
        "sim.stop",
        json!({}),
        &["no simulation is running", "sim.start"],
    );
    refuse(&mut s, "sim.step", json!({}), &["no simulation is running"]);
    refuse(
        &mut s,
        "sim.input",
        json!({"entity": "Scout", "forward": 1}),
        &["no simulation is running"],
    );
    assert_eq!(
        s.run("sim.state", &json!({})).unwrap(),
        json!({"playing": false})
    );
}

#[test]
fn step_by_ticks_and_by_seconds() {
    let mut s = playing();
    let r = s.run("sim.step", &json!({})).unwrap();
    assert_eq!(r["tick"], json!(1));
    let r = s.run("sim.step", &json!({"ticks": 59})).unwrap();
    assert_eq!(r["tick"], json!(60));
    assert_eq!(r["time_s"], json!(1.0));
    let r = s.run("sim.step", &json!({"seconds": 0.5})).unwrap();
    assert_eq!(r["tick"], json!(90));
    let r = s.run("sim.step", &json!({"seconds": 0.009})).unwrap();
    assert_eq!(r["tick"], json!(91), "0.54 ticks round to 1");
    assert!(is_hash(&r["hash"]));
    let c = &r["characters"][0];
    assert_eq!(c["id"], json!(3));
    assert_eq!(c["name"], json!("Scout"));
    for key in ["position", "rotation_y_deg", "velocity", "grounded"] {
        assert!(c.get(key).is_some(), "{key} in {c}");
    }
    assert!(s.document().entity(EntityId(3)).is_some());
}

#[test]
fn step_holds_the_tick_limit() {
    let mut s = playing();
    let max = MAX_STEP_TICKS;
    s.run("sim.step", &json!({"ticks": max})).unwrap();
    refuse(
        &mut s,
        "sim.step",
        json!({"ticks": max + 1}),
        &["`ticks`", &format!("at most {max} ticks")],
    );
    refuse(
        &mut s,
        "sim.step",
        json!({"seconds": 601}),
        &["`seconds`", "600"],
    );
    refuse(&mut s, "sim.step", json!({"seconds": -1}), &["`seconds`"]);
    refuse(&mut s, "sim.step", json!({"ticks": -1}), &["`ticks`"]);
    refuse(
        &mut s,
        "sim.step",
        json!({"ticks": 1, "seconds": 1}),
        &["not both"],
    );
}

#[test]
fn view_revision_grows_with_steps_and_revision_does_not() {
    let mut s = playing();
    let (rev, view) = (s.revision(), s.view_revision());
    s.run("sim.step", &json!({"ticks": 3})).unwrap();
    assert!(s.view_revision() > view);
    assert_eq!(s.revision(), rev);
}

#[test]
fn input_merges_fields_and_starts_from_rest_and_the_current_rotation() {
    let mut s = example();
    s.run("entity.set", &json!({"id": "Scout", "rotation_y_deg": 30}))
        .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    let r = s
        .run("sim.input", &json!({"entity": "scout", "forward": 1}))
        .unwrap();
    assert_eq!(
        r,
        json!({"entity": 3, "forward": 1.0, "right": 0.0, "yaw_deg": 30.0, "run": false, "jump": false, "interact": false})
    );
    let r = s
        .run(
            "sim.input",
            &json!({"entity": 3, "run": true, "right": -0.5}),
        )
        .unwrap();
    // The simulation clamps (forward, right) to the unit disc: (1, -0.5) has length sqrt(1.25).
    let len = 1.25_f64.sqrt();
    assert_eq!(
        r,
        json!({"entity": 3, "forward": 1.0 / len, "right": -0.5 / len, "yaw_deg": 30.0, "run": true, "jump": false, "interact": false})
    );
    let r = s
        .run(
            "sim.input",
            &json!({"entity": 3, "yaw_deg": 90, "jump": true}),
        )
        .unwrap();
    assert_eq!(r["yaw_deg"], json!(90.0));
    assert_eq!(r["forward"], json!(1.0 / len));
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["inputs"], json!([r]));
}

#[test]
fn face_turns_towards_the_four_compass_points() {
    let mut s = playing();
    let pos = s
        .live()
        .unwrap()
        .world()
        .entity(EntityId(3))
        .unwrap()
        .position;
    let (x, z) = (pos[0], pos[2]);
    for (point, yaw) in [
        ([x, z - 10.0], 0.0),
        ([x - 10.0, z], 90.0),
        ([x, z + 10.0], 180.0),
        ([x + 10.0, z], 270.0),
    ] {
        let r = s
            .run("sim.input", &json!({"entity": "Scout", "face": point}))
            .unwrap();
        let got = r["yaw_deg"].as_f64().unwrap();
        assert!(
            (got - yaw).abs() < 1e-9,
            "face {point:?}: yaw {got}, wanted {yaw}"
        );
    }
    refuse(
        &mut s,
        "sim.input",
        json!({"entity": "Scout", "face": [x, z]}),
        &["`face`", "where the character stands"],
    );
    refuse(
        &mut s,
        "sim.input",
        json!({"entity": "Scout", "face": [0, 1], "yaw_deg": 3}),
        &["not both"],
    );
}

#[test]
fn input_refuses_entities_that_are_no_character() {
    let mut s = playing();
    refuse(
        &mut s,
        "sim.input",
        json!({"entity": "Crate", "forward": 1}),
        &["no `character` component"],
    );
    refuse(
        &mut s,
        "sim.input",
        json!({"entity": 77}),
        &["no entity has the id or name \"77\""],
    );
    refuse(
        &mut s,
        "sim.input",
        json!({"forward": 1}),
        &["missing `entity`"],
    );
}

#[test]
fn state_while_playing() {
    let mut s = playing();
    s.run("sim.step", &json!({"ticks": 2})).unwrap();
    let r = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(r["playing"], json!(true));
    assert_eq!(r["tick"], json!(2));
    assert!(is_hash(&r["hash"]));
    assert_eq!(r["characters"].as_array().unwrap().len(), 1);
    assert_eq!(r["inputs"], json!([]));
}

#[test]
fn stop_gives_the_document_back_unchanged() {
    let mut s = example();
    let doc = s.document().clone();
    let rev = s.revision();
    s.run("sim.start", &json!({})).unwrap();
    s.run("sim.step", &json!({"ticks": 30})).unwrap();
    s.run("entity.add", &json!({"name": "Spawned", "at": [3, 3]}))
        .unwrap();
    let view = s.view_revision();
    let r = s.run("sim.stop", &json!({})).unwrap();
    assert_eq!(r, json!({"tick": 30, "kept": false}));
    assert_eq!(*s.document(), doc);
    assert_eq!(s.revision(), rev);
    assert!(s.view_revision() > view);
    assert!(!s.can_undo());
}

#[test]
fn stop_with_keep_makes_the_running_world_the_document_in_one_undo_step() {
    let mut s = example();
    let doc = s.document().clone();
    s.run("sim.start", &json!({})).unwrap();
    s.run("entity.add", &json!({"name": "Spawned", "at": [3, 3]}))
        .unwrap();
    s.run("environment.set", &json!({"rain": 1})).unwrap();
    let running = s.current_world().clone();
    let (rev, view) = (s.revision(), s.view_revision());
    let r = s.run("sim.stop", &json!({"keep": true})).unwrap();
    assert_eq!(r["kept"], json!(true));
    assert_eq!(*s.document(), running);
    assert!(s.revision() > rev && s.view_revision() > view);
    assert_eq!(undo_depth(&s), 1);
    s.undo().unwrap();
    assert_eq!(*s.document(), doc);
}

#[test]
fn keep_of_an_unchanged_world_pushes_no_step() {
    let mut s = playing();
    s.run("sim.stop", &json!({"keep": true})).unwrap();
    assert!(!s.can_undo());
}

#[test]
fn undo_and_redo_are_refused_while_playing_with_the_reason() {
    let mut s = example();
    s.run("world.set", &json!({"name": "Edited"})).unwrap();
    s.undo().unwrap();
    s.run("sim.start", &json!({})).unwrap();
    let before = snapshot(&s);
    let e = s.undo().unwrap_err().to_string();
    assert!(
        e.starts_with("undo: ") && e.contains("simulation runs") && e.contains("sim.stop"),
        "{e}"
    );
    let e = s.redo().unwrap_err().to_string();
    assert!(e.starts_with("redo: "), "{e}");
    assert_eq!(snapshot(&s), before);
}

/// Every World command, run with its example while playing, changes the running world and
/// leaves the document untouched (rule 6).
#[test]
fn every_world_command_acts_on_the_running_world() {
    let base = example();
    let specs: Vec<(String, Value)> = base
        .specs()
        .iter()
        .filter(|s| s.scope == Scope::World)
        .map(|s| (s.name.clone(), s.example.clone()))
        .collect();
    assert!(specs.len() >= 10);
    for (name, example) in specs {
        let mut s = playing();
        let doc = s.document().clone();
        let running = s.live().unwrap().world().clone();
        let (rev, view) = (s.revision(), s.view_revision());
        s.run(&name, &example)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(*s.document(), doc, "{name} changed the document");
        assert_ne!(
            *s.live().unwrap().world(),
            running,
            "{name} did not change the running world"
        );
        assert_eq!(s.current_world(), s.live().unwrap().world());
        assert_eq!(s.revision(), rev, "{name}");
        assert!(s.view_revision() > view, "{name}");
        assert!(!s.can_undo(), "{name}: a running world has no history");
    }
}

#[test]
fn a_failed_world_command_while_playing_changes_nothing() {
    let mut s = playing();
    refuse(
        &mut s,
        "entity.add",
        json!({"at": [999, 0]}),
        &["no ground"],
    );
    refuse(
        &mut s,
        "entity.remove",
        json!({"id": "Nobody"}),
        &["Nobody"],
    );
}

#[test]
fn terrain_commands_while_playing_change_the_running_heights() {
    let mut s = playing();
    s.run(
        "terrain.set",
        &json!({"kind": "flat", "size_m": 16, "cell_m": 1}),
    )
    .unwrap();
    assert_eq!(s.current_heights().unwrap().cols(), 17);
    s.run("sim.stop", &json!({})).unwrap();
    assert_eq!(s.current_heights().unwrap().cols(), 65);
}

#[test]
fn start_on_a_world_without_characters() {
    let mut s = Session::new(World::new("", 0));
    let r = s.run("sim.start", &json!({})).unwrap();
    assert_eq!(r["characters"], json!([]));
    assert_eq!(
        s.run("sim.step", &json!({"ticks": 0})).unwrap()["tick"],
        json!(0)
    );
}
