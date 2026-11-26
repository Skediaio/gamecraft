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

use common::{example, playing, playing_with_mounts, refuse, snapshot, undo_depth};
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
        json!({"entity": 3, "forward": 1.0, "right": 0.0, "yaw_deg": 30.0, "run": false, "jump": false, "interact": false, "pitch_deg": 0.0, "fire": false, "face": null, "face_entity": null, "face_pitch": false})
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
        json!({"entity": 3, "forward": 1.0 / len, "right": -0.5 / len, "yaw_deg": 30.0, "run": true, "jump": false, "interact": false, "pitch_deg": 0.0, "fire": false, "face": null, "face_entity": null, "face_pitch": false})
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
/// leaves the document untouched (rule 6). The attachment commands' examples name a model
/// with measured sockets and a mount on it (`common::mounts`).
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
        let mut s = if name.starts_with("attachment.") {
            playing_with_mounts()
        } else {
            playing()
        };
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

/// A flat field: `Player` at the origin, `Other` 20 m east, `Shooter` (with a rifle) 20 m west.
fn field() -> Session {
    let mut s = Session::new(World::new("Field", 4));
    let mut run = |c: &str, p: Value| s.run(c, &p).unwrap();
    run(
        "terrain.set",
        json!({"kind": "flat", "size_m": 256, "cell_m": 4}),
    );
    run(
        "entity.add",
        json!({"name": "Player", "at": [0, 0], "components": [{"type": "character"}]}),
    );
    run(
        "entity.add",
        json!({"name": "Other", "at": [20, 0], "components": [{"type": "character"}]}),
    );
    run(
        "entity.add",
        json!({"name": "Shooter", "at": [-20, 0], "components": [
            {"type": "character"}, {"type": "launcher", "muzzle_velocity_mps": 300}]}),
    );
    run("sim.start", json!({}));
    s
}

fn yaw_of(s: &Session, name: &str) -> f64 {
    let w = s.current_world();
    w.entity(w.find(name).unwrap()).unwrap().rotation_y_deg
}

fn assert_yaw(s: &Session, name: &str, want: f64) {
    let got = yaw_of(s, name);
    let off = (got - want).abs();
    assert!(
        off.min(360.0 - off) < 1e-6,
        "{name}: yaw {got}, wanted {want}"
    );
}

#[test]
fn face_is_held_every_tick_until_a_yaw_or_null_replaces_it() {
    let mut s = field();
    let run = |s: &mut Session, c: &str, p: Value| s.run(c, &p).unwrap();
    let r = run(
        &mut s,
        "sim.input",
        json!({"entity": "Player", "face": [0, -10]}),
    );
    assert_eq!(r["face"], json!([0.0, -10.0]));
    assert_eq!(r["yaw_deg"], json!(0.0));
    // The game master moves the player: the next tick turns it to the point again.
    run(&mut s, "entity.set", json!({"id": "Player", "at": [10, 0]}));
    run(&mut s, "sim.step", json!({"ticks": 1}));
    assert_yaw(&s, "Player", 45.0); // north-west
    // An entity: followed as it moves.
    let r = run(
        &mut s,
        "sim.input",
        json!({"entity": "Player", "face_entity": "Other"}),
    );
    assert_eq!(
        (r["face"].clone(), r["face_entity"].clone()),
        (json!(null), json!(2))
    );
    run(&mut s, "entity.set", json!({"id": "Other", "at": [10, 30]}));
    run(&mut s, "sim.step", json!({"ticks": 1}));
    assert_yaw(&s, "Player", 180.0); // south
    // A later input without a facing keeps it.
    run(
        &mut s,
        "sim.input",
        json!({"entity": "Player", "run": true}),
    );
    run(&mut s, "entity.set", json!({"id": "Other", "at": [-10, 0]}));
    run(&mut s, "sim.step", json!({"ticks": 1}));
    assert_yaw(&s, "Player", 90.0); // west
    // `yaw_deg` replaces it.
    let r = run(
        &mut s,
        "sim.input",
        json!({"entity": "Player", "yaw_deg": 270}),
    );
    assert_eq!(r["face_entity"], json!(null));
    run(&mut s, "entity.set", json!({"id": "Other", "at": [10, 30]}));
    run(&mut s, "sim.step", json!({"ticks": 1}));
    assert_yaw(&s, "Player", 270.0);
    // `face: null` clears a point and keeps the facing it had.
    run(
        &mut s,
        "sim.input",
        json!({"entity": "Player", "face": [10, 40]}),
    );
    run(&mut s, "sim.step", json!({"ticks": 1}));
    let r = run(
        &mut s,
        "sim.input",
        json!({"entity": "Player", "face": null}),
    );
    assert_eq!(r["face"], json!(null));
    let held = yaw_of(&s, "Player");
    run(
        &mut s,
        "entity.set",
        json!({"id": "Player", "at": [-30, 0]}),
    );
    run(&mut s, "sim.step", json!({"ticks": 1}));
    assert_yaw(&s, "Player", held);
    // `face_entity: null` clears an entity.
    run(
        &mut s,
        "sim.input",
        json!({"entity": "Player", "face_entity": "Other"}),
    );
    let r = run(
        &mut s,
        "sim.input",
        json!({"entity": "Player", "face_entity": null}),
    );
    assert_eq!(r["face_entity"], json!(null));
    let state = s.run("sim.state", &json!({})).unwrap();
    assert!(
        state["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["face_entity"].is_null())
    );
}

#[test]
fn face_entity_aims_the_pitch_of_a_launcher_until_pitch_deg_is_given() {
    let mut s = field();
    let r = s
        .run(
            "sim.input",
            &json!({"entity": "Shooter", "face_entity": "Other"}),
        )
        .unwrap();
    assert_eq!(r["face_pitch"], json!(true));
    // Other is 40 m east, its chest below the shooter's eyes: aimed a little down, plus drop.
    s.run("sim.step", &json!({"ticks": 1})).unwrap();
    let state = s.run("sim.state", &json!({})).unwrap();
    let input = state["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["entity"] == json!(3))
        .unwrap()
        .clone();
    let pitch = input["pitch_deg"].as_f64().unwrap();
    assert!(pitch != 0.0 && pitch.abs() < 2.0, "{input}");
    assert_yaw(&s, "Shooter", 270.0);
    // A pitch given by hand takes the pitch back; the yaw still follows.
    let r = s
        .run("sim.input", &json!({"entity": "Shooter", "pitch_deg": 5}))
        .unwrap();
    assert_eq!(
        (r["face_pitch"].clone(), r["face_entity"].clone()),
        (json!(false), json!(2))
    );
    s.run("entity.set", &json!({"id": "Other", "at": [-20, -30]}))
        .unwrap();
    s.run("sim.step", &json!({"ticks": 1})).unwrap();
    assert_yaw(&s, "Shooter", 0.0);
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["inputs"][0]["entity"], json!(3), "{state}");
    assert_eq!(state["inputs"][0]["pitch_deg"], json!(5.0), "{state}");
    // `face_entity` with `pitch_deg` in the same call: the yaw is held, the pitch is the one given.
    let r = s
        .run(
            "sim.input",
            &json!({"entity": "Shooter", "face_entity": "Other", "pitch_deg": 1}),
        )
        .unwrap();
    assert_eq!(r["face_pitch"], json!(false));
    // A character without a launcher holds no pitch.
    let r = s
        .run(
            "sim.input",
            &json!({"entity": "Player", "face_entity": "Other"}),
        )
        .unwrap();
    assert_eq!(r["face_pitch"], json!(false));
}

#[test]
fn bad_facings_are_refused_naming_the_parameter() {
    let mut s = field();
    for (params, needles) in [
        (
            json!({"entity": "Player", "face": [0, -5], "face_entity": "Other"}),
            vec!["not both"],
        ),
        (
            json!({"entity": "Player", "yaw_deg": 3, "face_entity": "Other"}),
            vec!["not both"],
        ),
        (
            json!({"entity": "Player", "face_entity": "Player"}),
            vec!["`face_entity`", "itself"],
        ),
        (
            json!({"entity": "Player", "face_entity": "Nobody"}),
            vec!["`face_entity`", "Nobody"],
        ),
    ] {
        refuse(&mut s, "sim.input", params, &needles);
    }
}
