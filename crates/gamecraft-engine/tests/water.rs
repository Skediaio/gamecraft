//! Water through the commands: `water.add` and `water.set` keep the level as a world height
//! and the outline in the entity's frame; `query.ground` and `query.ground_height` say where
//! it is water and how deep; a walker wades while playing, and the game master can add water
//! ahead of one; hostile parameters change nothing.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{refuse, snapshot};
use gamecraft_engine::{Session, example_world};
use gamecraft_world::{Terrain, TerrainSource, World};
use serde_json::{Value, json};

/// A session of a flat terrain at height 0, 256 m square, 2 m cells, without entities.
fn flat() -> Session {
    let mut w = World::new("Flat", 1);
    w.terrain = Some(Terrain {
        size_m: 256.0,
        cell_m: 2.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    Session::new(w)
}

/// A pond `level` deep from x = 2 to x = 12, z from -20 to 20 (the west edge at x = 2).
fn pond(level: f64) -> Value {
    json!({"name": "Pond", "level_m": level, "outline": [[2, -20], [12, -20], [12, 20], [2, 20]]})
}

fn x_of(s: &mut Session, name: &str) -> f64 {
    let e = s.run("entity.get", &json!({"id": name})).unwrap();
    e["position"][0].as_f64().unwrap()
}

#[test]
fn water_add_and_set_keep_the_level_and_the_outline() {
    let mut s = flat();
    let added = s.run("water.add", &pond(0.4)).unwrap();
    assert_eq!(added["position"], json!([7.0, 0.4, 0.0]), "{added}");
    assert!(
        (added["area_m2"].as_f64().unwrap() - 400.0).abs() < 1e-9,
        "{added}"
    );
    assert_eq!(added["level_m"], 0.4);
    let e = s.run("entity.get", &json!({"id": "Pond"})).unwrap();
    let water = &e["components"][0];
    assert_eq!(water["type"], "water");
    assert!(
        water.get("level_m").is_none(),
        "0 above the entity: left out: {water}"
    );
    assert_eq!(
        water["outline"],
        json!([[-5.0, -20.0], [5.0, -20.0], [5.0, 20.0], [-5.0, 20.0]])
    );
    // Only the fields given change; the level moves the entity, the outline stays in its frame.
    let edited = s
        .run("water.set", &json!({"id": "Pond", "level_m": 0.6}))
        .unwrap();
    assert_eq!(edited["level_m"], 0.6);
    assert_eq!(edited["position"], json!([7.0, 0.6, 0.0]));
    let e = s.run("entity.get", &json!({"id": "Pond"})).unwrap();
    assert_eq!(e["components"][0]["outline"].as_array().unwrap().len(), 4);
    let edited = s
        .run(
            "water.set",
            &json!({"id": "Pond", "outline": [[7, -1], [9, -1], [9, 1], [7, 1]]}),
        )
        .unwrap();
    assert_eq!(edited["area_m2"], 4.0);
    assert_eq!(
        edited["position"],
        json!([7.0, 0.6, 0.0]),
        "the entity stays"
    );
    let e = s.run("entity.get", &json!({"id": "Pond"})).unwrap();
    assert_eq!(
        e["components"][0]["outline"],
        json!([[0.0, -1.0], [2.0, -1.0], [2.0, 1.0], [0.0, 1.0]])
    );
    // An entity without water gets some: everywhere with [], an outline otherwise.
    s.run(
        "entity.add",
        &json!({"name": "Rock", "position": [40, 2, 40], "components": [{"type": "shape", "kind": "box"}]}),
    )
    .unwrap();
    let e = s.run("entity.get", &json!({"id": "Rock"})).unwrap();
    let rock_id = e["id"].clone();
    let made = s
        .run("water.set", &json!({"id": "Rock", "outline": []}))
        .unwrap();
    assert_eq!(made["area_m2"], Value::Null, "{made}");
    assert_eq!(
        made["level_m"], 2.0,
        "the surface is where the entity stands"
    );
    assert_eq!(made["id"], rock_id);
    // The world is valid and round-trips through the file; every command is one undo step.
    let text = s.document().to_json().unwrap();
    let back = World::from_json(&text).unwrap();
    assert_eq!(&back, s.document());
    s.run(
        "entity.set",
        &json!({"id": "Rock", "position": [41, 3, 41]}),
    )
    .unwrap();
    assert!(s.can_undo());
    s.undo().unwrap();
    assert_eq!(x_of(&mut s, "Rock"), 40.0);
    // Removing the water: with the entity, or its component alone.
    s.run(
        "entity.remove_component",
        &json!({"id": "Rock", "type": "water"}),
    )
    .unwrap();
    let e = s.run("entity.get", &json!({"id": "Rock"})).unwrap();
    assert_eq!(e["components"].as_array().unwrap().len(), 1);
    s.run("entity.remove", &json!({"id": "Pond"})).unwrap();
    let e = s.run("world.inspect", &json!({"entities": true})).unwrap();
    assert!(
        e["entities"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| !e["components"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["type"] == "water")),
        "{e}"
    );
}

#[test]
fn bad_water_is_refused_by_name_and_changes_nothing() {
    let mut s = flat();
    s.run("water.add", &pond(0.4)).unwrap();
    s.run(
        "entity.add",
        &json!({"name": "Rock", "position": [40, 2, 40]}),
    )
    .unwrap();
    for (command, params, needles) in [
        (
            "water.add",
            json!({"outline": [[0, 0], [1, 1]]}),
            &["`outline` has 2 points"][..],
        ),
        (
            "water.add",
            json!({"level_m": 2e6}),
            &["`level_m` is 2000000"][..],
        ),
        (
            "water.add",
            json!({"level_m": "high"}),
            &["`level_m` must be number"][..],
        ),
        ("water.set", json!({}), &["missing `id`"][..]),
        (
            "water.add",
            json!({"level_m": 0, "outline": [[0, 0], [1, "x"], [2, 0], [0, 2]]}),
            &["`outline[1]` must be [x, z]"][..],
        ),
        (
            "water.add",
            json!({"level_m": 0, "outline": [[0, 0], [1, 0], [1, 0]]}),
            &["water.outline[2]", "0.01 m apart"][..],
        ),
        (
            "water.add",
            json!({"level_m": 0, "colour": "blue"}),
            &["unknown parameter"][..],
        ),
        (
            "water.set",
            json!({"id": "Nobody", "level_m": 1}),
            &["Nobody"][..],
        ),
        (
            "water.set",
            json!({"id": "Rock"}),
            &["entity", "has no water; give `outline`"][..],
        ),
        (
            "water.set",
            json!({"id": "Pond", "outline": [[0, 0], [1, 0], [1, 0], [0, 1]]}),
            &["water.outline[2]", "0.01 m apart"][..],
        ),
        (
            "entity.add",
            json!({"name": "Bad", "position": [0, 0, 0], "components": [{"type": "water", "level_m": 1e7}]}),
            &["water.level_m"][..],
        ),
        (
            "entity.add",
            json!({"name": "Bad", "position": [0, 0, 0], "components": [{"type": "water", "outline": [[0, 0], [1, 0], [1, 1], [0, 0]]}]}),
            &["water.outline[0]", "0.01 m apart"][..],
        ),
        (
            "entity.add",
            json!({"name": "Bad", "position": [0, 0, 0], "components": [{"type": "water", "depth": 1}]}),
            &["components[0]", "water", "unknown field"][..],
        ),
    ] {
        refuse(&mut s, command, params, needles);
    }
    let _ = snapshot(&s);
}

#[test]
fn the_ground_queries_say_where_it_is_water_and_how_deep() {
    let mut s = flat();
    let dry = s
        .run("query.ground", &json!({"points": [[0, 0], [7, 0]]}))
        .unwrap();
    for p in dry["points"].as_array().unwrap() {
        assert_eq!(p["surface"], "ground", "{p}");
        assert_eq!(p["water_level_m"], Value::Null);
        assert_eq!(p["water_depth_m"], Value::Null);
    }
    s.run("water.add", &pond(0.8)).unwrap();
    let wet = s
        .run(
            "query.ground",
            &json!({"points": [[0, 0], [7, 0], [7, 19.5], [13, 0]]}),
        )
        .unwrap();
    let points = wet["points"].as_array().unwrap();
    assert_eq!(points[0]["surface"], "ground");
    assert_eq!(points[1]["surface"], "water", "{wet}");
    assert_eq!(points[1]["water_depth_m"], 0.8);
    assert_eq!(points[1]["water_level_m"], 0.8);
    assert_eq!(points[1]["height_m"], 0.0, "the ground is still the ground");
    assert_eq!(points[2]["surface"], "water");
    assert_eq!(points[3]["surface"], "ground");
    let one = s
        .run("query.ground_height", &json!({"x": 7, "z": 0}))
        .unwrap();
    assert_eq!(one["surface"], "water");
    assert_eq!(one["water_depth_m"], 0.8);
    // The highest surface counts where areas overlap, and water everywhere covers all.
    s.run(
        "water.add",
        &json!({"name": "Pit", "level_m": 2.5, "outline": [[6, -1], [8, -1], [8, 1], [6, 1]]}),
    )
    .unwrap();
    let one = s
        .run("query.ground_height", &json!({"x": 7, "z": 0}))
        .unwrap();
    assert_eq!(one["water_depth_m"], 2.5);
    assert_eq!(one["water_level_m"], 2.5);
    s.run("water.set", &json!({"id": "Pond", "outline": []}))
        .unwrap();
    let all = s
        .run(
            "query.ground",
            &json!({"points": [[-100, -100], [100, 100]]}),
        )
        .unwrap();
    for p in all["points"].as_array().unwrap() {
        assert_eq!(p["surface"], "water", "{all}");
        assert_eq!(p["water_depth_m"], 0.8);
    }
    // Without ground there is no depth to give.
    let mut bare = Session::new(World::new("Bare", 1));
    bare.run(
        "entity.add",
        &json!({"name": "Sea", "position": [0, 0, 0], "components": [{"type": "water"}]}),
    )
    .unwrap();
    let e = bare
        .run("query.ground_height", &json!({"x": 0, "z": 0}))
        .unwrap();
    assert_eq!(e["surface"], Value::Null, "{e}");
    assert_eq!(e["water_depth_m"], Value::Null);
}

#[test]
fn a_walker_wades_while_playing_and_the_game_master_adds_water_ahead() {
    // Walker at x = -3 walks east. After 3 s the game master drops a 1 m deep pond ahead:
    // from then on the walker wades, and sim.state says how deep.
    let mut s = flat();
    s.run(
        "entity.add",
        &json!({"name": "Walker", "position": [-3, 0, 0], "components": [{"type": "character"}]}),
    )
    .unwrap();
    let mut reference = s.clone();
    for t in [&mut s, &mut reference] {
        t.run("sim.start", &json!({})).unwrap();
        t.run(
            "sim.input",
            &json!({"entity": "Walker", "forward": 1, "yaw_deg": -90}),
        )
        .unwrap();
        t.run("sim.step", &json!({"seconds": 3})).unwrap();
    }
    assert!((x_of(&mut s, "Walker") + 3.0 - 1.6 * 3.0).abs() < 1e-9);
    // The pond is placed on the running world: its edge 0.2 m ahead of the walker.
    s.run("water.add", &pond(1.0)).unwrap();
    assert_eq!(
        s.document().entity_count(),
        1,
        "the document is untouched while playing"
    );
    s.run("sim.step", &json!({"seconds": 7})).unwrap();
    reference.run("sim.step", &json!({"seconds": 7})).unwrap();
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["characters"][0]["water_depth_m"], 1.0, "{state}");
    let x = x_of(&mut s, "Walker");
    // 0.2 m more of dry land, then the rest at 0.7 of the speed.
    let expected = 2.0 + 0.7 * 1.6 * (7.0 - 0.2 / 1.6);
    assert!((x - expected).abs() < 0.01, "x {x}, expected {expected}");
    let dry = x_of(&mut reference, "Walker");
    assert!(
        dry > 12.0,
        "without the pond the walker is far past it: {dry}"
    );
    assert_ne!(
        state["hash"],
        reference.run("sim.state", &json!({})).unwrap()["hash"]
    );
    // Deepened under the walker's feet: it is in deep water now, so it may still move, at
    // the slowest share, and it gets out when it walks west.
    s.run("water.set", &json!({"id": "Pond", "level_m": 2.0}))
        .unwrap();
    s.run("sim.step", &json!({"seconds": 2})).unwrap();
    let deeper = x_of(&mut s, "Walker");
    assert!(
        (deeper - x - 0.4 * 1.6 * 2.0).abs() < 0.01,
        "x {deeper} from {x}"
    );
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["characters"][0]["water_depth_m"], 2.0, "{state}");
    s.run(
        "sim.input",
        &json!({"entity": "Walker", "forward": 1, "yaw_deg": 90}),
    )
    .unwrap();
    s.run("sim.step", &json!({"seconds": 20})).unwrap();
    assert!(x_of(&mut s, "Walker") < 2.0);
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(
        state["characters"][0]["water_depth_m"],
        Value::Null,
        "{state}"
    );
}

#[test]
fn a_world_without_water_keeps_its_text() {
    // A world without water keeps its file text: the component is written only where it is.
    let mut s = Session::new(example_world());
    let before = s.document().to_json().unwrap();
    assert!(!before.contains("water"));
    s.run("water.add", &json!({"name": "Sea"})).unwrap();
    let with = s.document().to_json().unwrap();
    assert!(with.contains(r#""type": "water""#), "{with}");
    s.undo().unwrap();
    assert_eq!(s.document().to_json().unwrap(), before);
}
