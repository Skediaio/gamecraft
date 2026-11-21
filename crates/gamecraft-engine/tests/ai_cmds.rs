//! The commands of soldiers' skill, knowledge and cover: `ai.set`, `ai.inspect` and
//! `query.cover`, on the document and on a running world.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, refuse};
use gamecraft_engine::Session;
use gamecraft_world::World;
use serde_json::{Value, json};

/// A flat field at noon: a wall 6 m wide and 2.5 m tall at the origin, a red soldier with a
/// behaviour 1.2 m south of it facing south, a blue soldier 30 m south.
fn field() -> Session {
    let mut s = Session::new(World::new("Field", 5));
    let mut run = |c: &str, p: Value| s.run(c, &p).unwrap();
    run(
        "terrain.set",
        json!({"kind": "flat", "size_m": 256, "cell_m": 4}),
    );
    run(
        "environment.set",
        json!({"wind_speed_mps": 0, "time_of_day_h": 12}),
    );
    run(
        "entity.add",
        json!({"name": "Wall", "at": [0, 0], "components": [
            {"type": "shape", "kind": "box", "size": [6, 2.5, 0.4], "solid": true}]}),
    );
    run(
        "entity.add",
        json!({"name": "Red", "at": [0, 1.2], "rotation_y_deg": 180, "components": [
            {"type": "character"}, {"type": "health"},
            {"type": "launcher", "dispersion_mrad": 0},
            {"type": "behaviour", "mode": "guard", "team": "red", "reaction_s": 0.5},
            {"type": "tags", "tags": ["squad"]}]}),
    );
    run(
        "entity.add",
        json!({"name": "Blue", "at": [0, 30], "components": [
            {"type": "character"}, {"type": "health"},
            {"type": "tags", "tags": ["team:blue"]}]}),
    );
    s
}

#[test]
fn ai_set_changes_the_numbers_on_the_document_and_keeps_the_rest() {
    let mut s = field();
    let r = s
        .run(
            "ai.set",
            &json!({"entity": "tag:squad", "skill": 0.8, "precision": 0.3, "spot_time_s": 2,
                    "aim_settle_s": 1.5, "aim_error_floor_mrad": 1, "courage": 0.4,
                    "cover_search_m": 12, "calm_s": 20, "state": "aware"}),
        )
        .unwrap();
    assert_eq!(r["set"][0]["name"], "Red");
    let b = s.document().find("Red").unwrap();
    let b = s.document().entity(b).unwrap().behaviour().unwrap().clone();
    assert_eq!(b.skill, 0.8);
    assert_eq!(b.precision, 0.3);
    assert_eq!(b.spot_time_s, 2.0);
    assert_eq!(b.aim_settle_s, 1.5);
    assert_eq!(b.aim_error_floor_mrad, 1.0);
    assert_eq!(b.courage, 0.4);
    assert_eq!(b.cover_search_m, 12.0);
    assert_eq!(b.calm_s, 20.0);
    assert_eq!(b.alertness.name(), "aware");
    assert_eq!(b.team, "red", "the rest is kept");
    assert_eq!(b.reaction_s, 0.5);
    // One undo step.
    s.undo().unwrap();
    let id = s.document().find("Red").unwrap();
    assert_eq!(
        s.document().entity(id).unwrap().behaviour().unwrap().skill,
        1.0
    );
}

#[test]
fn ai_set_works_on_the_running_world_and_inspect_shows_it() {
    let mut s = field();
    s.run("sim.start", &json!({})).unwrap();
    s.run(
        "ai.set",
        &json!({"entity": "Red", "cover_search_m": 10, "state": "combat"}),
    )
    .unwrap();
    let doc = s.document().find("Red").unwrap();
    assert_eq!(
        s.document()
            .entity(doc)
            .unwrap()
            .behaviour()
            .unwrap()
            .cover_search_m,
        0.0,
        "the document is untouched while playing"
    );
    let r = s.run("ai.inspect", &json!({"entity": "Red"})).unwrap();
    assert_eq!(r["playing"], true);
    assert_eq!(r["alertness"], "combat");
    assert_eq!(r["behaviour"]["cover_search_m"], 10.0);
    // On contact it takes cover behind the wall.
    s.run("sim.step", &json!({"seconds": 4})).unwrap();
    let r = s.run("ai.inspect", &json!({"entity": "Red"})).unwrap();
    assert_eq!(r["alertness"], "combat");
    let blue = s.current_world().find("Blue").unwrap();
    let k = r["knowledge"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["entity"] == blue.0)
        .expect("it knows Blue");
    assert!(k["knowledge"].as_f64().unwrap() >= 0.25);
    assert_eq!(k["name"], "Blue");
    assert!(!r["cover"].is_null(), "it chose a cover: {r}");
    assert!(r["cover"]["point"][2].as_f64().unwrap() < -0.2, "{r}");
    let red = s.current_world().find("Red").unwrap();
    let z = s.current_world().entity(red).unwrap().position[2];
    assert!(z < -0.2, "behind the wall: {z}");
    assert!(r["stress"].as_f64().unwrap() >= 0.0);
}

#[test]
fn ai_inspect_while_editing_and_without_a_behaviour() {
    let mut s = field();
    let r = s.run("ai.inspect", &json!({"entity": "Red"})).unwrap();
    assert_eq!(r["playing"], false);
    assert!(r["alertness"].is_null());
    assert_eq!(r["behaviour"]["skill"], 1.0);
    assert_eq!(r["effective"]["reaction_s"], 0.5);
    assert_eq!(r["team"], "red");
    let r = s.run("ai.inspect", &json!({"entity": "Blue"})).unwrap();
    assert!(r["behaviour"].is_null());
    assert_eq!(r["team"], "blue");
}

#[test]
fn query_cover_ranks_the_far_side_of_the_wall_first() {
    let mut s = field();
    let r = s
        .run("query.cover", &json!({"entity": "Red", "threat": "Blue"}))
        .unwrap();
    let best = &r["candidates"][0];
    assert_eq!(best["protection"], 1.0, "{r}");
    assert_eq!(best["reachable"], true);
    assert!(best["point"][2].as_f64().unwrap() < -0.2, "{r}");
    assert!(!best["via"].as_array().unwrap().is_empty());
    assert_eq!(r["radius_m"], 15.0);
    // From a bare position, against a point.
    let r = s
        .run(
            "query.cover",
            &json!({"position": [0, 0, 1.2], "threat": [0, 1.2, 30], "radius_m": 8, "limit": 3}),
        )
        .unwrap();
    assert_eq!(r["candidates"].as_array().unwrap().len(), 3);
    assert!(r["count"].as_u64().unwrap() >= 3);
    assert_eq!(r["candidates"][0]["protection"], 1.0);
}

#[test]
fn bad_parameters_name_the_parameter() {
    let mut s = field();
    refuse(
        &mut s,
        "ai.set",
        json!({"entity": "Red", "skill": 2}),
        &["`skill`", "from 0 to 1"],
    );
    refuse(
        &mut s,
        "ai.set",
        json!({"entity": "Red", "cover_search_m": 500}),
        &["`cover_search_m`", "100"],
    );
    refuse(
        &mut s,
        "ai.set",
        json!({"entity": "Red", "state": "panic"}),
        &["state"],
    );
    refuse(
        &mut s,
        "ai.set",
        json!({"entity": "Red"}),
        &["nothing to set"],
    );
    refuse(
        &mut s,
        "ai.set",
        json!({"entity": "Wall", "skill": 0.5}),
        &["`entity`", "not a character"],
    );
    refuse(
        &mut s,
        "ai.inspect",
        json!({"entity": "Nobody"}),
        &["`entity`"],
    );
    refuse(
        &mut s,
        "query.cover",
        json!({"entity": "Red"}),
        &["`threat`"],
    );
    refuse(
        &mut s,
        "query.cover",
        json!({"entity": "Red", "position": [0, 0, 0], "threat": [0, 1, 9]}),
        &["`entity`", "`position`"],
    );
    refuse(
        &mut s,
        "query.cover",
        json!({"entity": "Red", "threat": [0, 1, 9], "radius_m": 0}),
        &["`radius_m`"],
    );
    refuse(
        &mut s,
        "query.cover",
        json!({"entity": "Red", "threat": "Red"}),
        &["`threat`"],
    );
    refuse(
        &mut s,
        "query.cover",
        json!({"entity": "Red", "threat": [0, 1, 9], "limit": 0}),
        &["`limit`"],
    );
}

#[test]
fn the_example_world_takes_the_examples() {
    let mut s = example();
    s.run("ai.set", &json!({"entity": "Scout", "precision": 0.5}))
        .unwrap();
    let r = s.run("ai.inspect", &json!({"entity": "Scout"})).unwrap();
    assert_eq!(r["behaviour"]["precision"], 0.5);
    let r = s
        .run(
            "query.cover",
            &json!({"entity": "Scout", "threat": [0, 1.5, -30]}),
        )
        .unwrap();
    assert!(r["count"].as_u64().unwrap() > 0);
}
