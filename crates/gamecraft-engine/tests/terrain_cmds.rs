//! `terrain.sculpt`, `terrain.paint`, `terrain.layers`, `terrain.paint_rule`,
//! `terrain.remove_paint_rule`, `terrain.edits` and `terrain.remove_edit`.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, playing, refuse};
use gamecraft_engine::Session;
use gamecraft_world::{TerrainEdit, World};
use serde_json::{Value, json};

fn ground(s: &Session, x: f64, z: f64) -> f64 {
    s.current_heights().unwrap().height_at(x, z).unwrap()
}

fn edits(s: &Session) -> Vec<TerrainEdit> {
    s.current_world().terrain.as_ref().unwrap().edits.clone()
}

#[test]
fn a_raise_lifts_the_ground_and_undo_restores_the_field_exactly() {
    let mut s = example();
    let before = s.current_heights().unwrap().clone();
    let h0 = ground(&s, 20.0, 20.0);
    let r = s
        .run(
            "terrain.sculpt",
            &json!({"op": "raise", "center": [20, 20], "radius_m": 6, "strength_m": 2.5}),
        )
        .unwrap();
    // The example world has one edit already (a painted patch), so this is the second.
    assert_eq!(r["index"], 1);
    assert_eq!(r["edits"], 2);
    assert!(r["tiles_rebuilt"].as_u64().unwrap() >= 1);
    assert!((ground(&s, 20.0, 20.0) - (h0 + 2.5)).abs() < 1e-4);
    // The field is what a fresh build gives.
    let built = s.document().terrain.as_ref().unwrap().build().unwrap();
    assert!(*s.current_heights().unwrap() == built);
    s.undo().unwrap();
    assert!(*s.current_heights().unwrap() == before);
    s.redo().unwrap();
    assert!(*s.current_heights().unwrap() == built);
}

#[test]
fn flatten_and_ramp_take_their_heights_from_the_ground_when_left_out() {
    let mut s = example();
    let h = ground(&s, -10.0, 10.0);
    let r = s
        .run(
            "terrain.sculpt",
            &json!({"op": "flatten", "center": [-10, 10], "radius_m": 8}),
        )
        .unwrap();
    assert!((r["edit"]["height_m"].as_f64().unwrap() - h).abs() < 1e-9);
    assert!((ground(&s, -8.0, 10.0) - h).abs() < 1e-4);
    let (a, b) = (ground(&s, -40.0, -30.0), ground(&s, 40.0, -30.0));
    let r = s
        .run(
            "terrain.sculpt",
            &json!({"op": "ramp", "from": [-40, -30], "to": [40, -30], "width_m": 6}),
        )
        .unwrap();
    assert_eq!(r["edit"]["op"], "ramp");
    assert!((r["edit"]["from_height_m"].as_f64().unwrap() - a).abs() < 1e-9);
    assert!((ground(&s, 0.0, -30.0) - (a + b) / 2.0).abs() < 1e-4);
    // Given heights are kept.
    s.run(
        "terrain.sculpt",
        &json!({"op": "flatten", "center": [30, 30], "radius_m": 5, "height_m": 1.5, "falloff": 0}),
    )
    .unwrap();
    assert!((ground(&s, 30.0, 30.0) - 1.5).abs() < 1e-6);
    let list = s.run("terrain.edits", &json!({})).unwrap();
    let ops: Vec<&str> = list["edits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["op"].as_str().unwrap())
        .collect();
    assert_eq!(ops, ["paint", "flatten", "ramp", "flatten"]);
    assert_eq!(list["edits"][2]["index"], 2);
}

#[test]
fn sculpt_refuses_what_does_not_fit_its_op() {
    let mut s = example();
    refuse(
        &mut s,
        "terrain.sculpt",
        json!({"op": "raise", "center": [0, 0], "radius_m": 5}),
        &["strength_m", "raise"],
    );
    refuse(
        &mut s,
        "terrain.sculpt",
        json!({"op": "raise", "center": [0, 0], "radius_m": 5, "strength_m": 1, "width_m": 3}),
        &["width_m", "raise"],
    );
    refuse(
        &mut s,
        "terrain.sculpt",
        json!({"op": "dig", "center": [0, 0], "radius_m": 5}),
        &["op"],
    );
    refuse(
        &mut s,
        "terrain.sculpt",
        json!({"op": "lower", "center": [0, 0], "radius_m": -5, "strength_m": 1}),
        &["radius_m"],
    );
    refuse(
        &mut s,
        "terrain.sculpt",
        json!({"op": "smooth", "center": [0, 0], "radius_m": 5, "iterations": 50}),
        &["iterations", "1 to 8"],
    );
    // A flatten outside the terrain has no ground to take its height from.
    refuse(
        &mut s,
        "terrain.sculpt",
        json!({"op": "flatten", "center": [500, 0], "radius_m": 5}),
        &["outside the terrain", "height_m"],
    );
    s.run("terrain.clear", &json!({})).unwrap();
    refuse(
        &mut s,
        "terrain.sculpt",
        json!({"op": "raise", "center": [0, 0], "radius_m": 5, "strength_m": 1}),
        &["no terrain", "terrain.set"],
    );
}

#[test]
fn removing_an_edit_restores_the_ground_and_order_matters() {
    let mut s = example();
    let base = s.current_heights().unwrap().clone();
    s.run(
        "terrain.sculpt",
        &json!({"op": "raise", "center": [0, 20], "radius_m": 6, "strength_m": 2}),
    )
    .unwrap();
    s.run(
        "terrain.sculpt",
        &json!({"op": "flatten", "center": [0, 20], "radius_m": 10, "height_m": 0, "falloff": 0.2}),
    )
    .unwrap();
    // The flatten came last: the bump is gone.
    assert!(ground(&s, 0.0, 20.0).abs() < 1e-5);
    let r = s.run("terrain.remove_edit", &json!({"index": 1})).unwrap();
    assert_eq!(r["removed"]["op"], "raise");
    assert!(ground(&s, 0.0, 20.0).abs() < 1e-5);
    s.run("terrain.remove_edit", &json!({"index": 1})).unwrap();
    assert!(*s.current_heights().unwrap() == base);
    refuse(
        &mut s,
        "terrain.remove_edit",
        json!({"index": 7}),
        &["index", "7", "1 edit"],
    );
}

#[test]
fn layers_rules_and_paint() {
    let mut s = example();
    let r = s
        .run(
            "terrain.layers",
            &json!({"layers": [
                {"name": "grass", "color": [0.1, 0.2, 0.05]},
                {"name": "rock", "color": [0.3, 0.3, 0.3], "roughness": 0.8},
                {"name": "dirt", "color": [0.2, 0.15, 0.1]},
                {"name": "snow", "color": [0.9, 0.9, 0.9]}
            ]}),
        )
        .unwrap();
    assert_eq!(r["layers"].as_array().unwrap().len(), 4);
    s.run(
        "terrain.paint_rule",
        &json!({"layer": "snow", "height_min_m": 3}),
    )
    .unwrap();
    s.run(
        "terrain.paint",
        &json!({"layer": "dirt", "center": [-20, -20], "radius_m": 5}),
    )
    .unwrap();
    let field = s.current_heights().unwrap();
    assert_eq!(field.layer_count(), 4);
    assert!(
        field
            .weights()
            .iter()
            .all(|w| w.iter().map(|b| u32::from(*b)).sum::<u32>() == 255)
    );
    assert_eq!(field.weight_at(-20.0, -20.0).unwrap()[2], 255);
    // A layer still painted cannot go.
    refuse(
        &mut s,
        "terrain.layers",
        json!({"layers": [{"name": "grass", "color": [0.1, 0.2, 0.05]}]}),
        &["\"rock\"", "layers: grass"],
    );
    refuse(
        &mut s,
        "terrain.paint",
        json!({"layer": "lava", "center": [0, 0], "radius_m": 5}),
        &["lava", "grass, rock, dirt, snow"],
    );
    refuse(
        &mut s,
        "terrain.paint_rule",
        json!({"layer": "rock", "slope_min_deg": 50, "slope_max_deg": 40}),
        &["slope"],
    );
    let rules = s.run("terrain.edits", &json!({})).unwrap()["paint_rules"].clone();
    assert_eq!(rules.as_array().unwrap().len(), 2);
    s.run("terrain.remove_paint_rule", &json!({"index": 1}))
        .unwrap();
    let rules = s.run("terrain.edits", &json!({})).unwrap()["paint_rules"].clone();
    assert_eq!(rules, json!([{"layer": "rock", "slope_min_deg": 35.0}]));
}

#[test]
fn a_live_crater_moves_a_standing_character_with_the_ground() {
    let mut s = playing();
    s.run("sim.step", &json!({"ticks": 60})).unwrap();
    let y = |s: &Session| {
        s.current_world()
            .find("Scout")
            .map(|id| s.current_world().entity(id).unwrap().position[1])
            .unwrap()
    };
    let before = y(&s);
    assert!((before - ground(&s, 0.0, 0.0)).abs() < 1e-6);
    let r = s
        .run(
            "terrain.sculpt",
            &json!({"op": "lower", "center": [0, 0], "radius_m": 8, "strength_m": 3}),
        )
        .unwrap();
    assert!(r["tiles_rebuilt"].as_u64().unwrap() >= 1);
    // The document is untouched; the running world has the crater.
    assert_eq!(s.document().terrain.as_ref().unwrap().edits.len(), 1);
    assert_eq!(edits(&s).len(), 2);
    let floor = ground(&s, 0.0, 0.0);
    assert!((floor - (before - 3.0)).abs() < 1e-4);
    s.run("sim.step", &json!({"ticks": 120})).unwrap();
    assert!((y(&s) - floor).abs() < 1e-6, "{} vs {floor}", y(&s));
    // A raise lifts it back up.
    s.run(
        "terrain.sculpt",
        &json!({"op": "raise", "center": [0, 0], "radius_m": 8, "strength_m": 5}),
    )
    .unwrap();
    s.run("sim.step", &json!({"ticks": 60})).unwrap();
    assert!((y(&s) - ground(&s, 0.0, 0.0)).abs() < 1e-6);
    // The same run again ends on the same hash.
    let hash = s.run("sim.state", &json!({})).unwrap()["hash"].clone();
    let mut again = playing();
    again.run("sim.step", &json!({"ticks": 60})).unwrap();
    again
        .run(
            "terrain.sculpt",
            &json!({"op": "lower", "center": [0, 0], "radius_m": 8, "strength_m": 3}),
        )
        .unwrap();
    again.run("sim.step", &json!({"ticks": 120})).unwrap();
    again
        .run(
            "terrain.sculpt",
            &json!({"op": "raise", "center": [0, 0], "radius_m": 8, "strength_m": 5}),
        )
        .unwrap();
    again.run("sim.step", &json!({"ticks": 60})).unwrap();
    assert_eq!(again.run("sim.state", &json!({})).unwrap()["hash"], hash);
}

#[test]
fn queries_read_the_sculpted_ground() {
    let mut s = example();
    s.run(
        "terrain.sculpt",
        &json!({"op": "flatten", "center": [25, -25], "radius_m": 6, "height_m": 9, "falloff": 0}),
    )
    .unwrap();
    let g = s
        .run("query.ground_height", &json!({"x": 25, "z": -25}))
        .unwrap();
    assert!((g["height_m"].as_f64().unwrap() - 9.0).abs() < 1e-6);
    assert!(g["slope_deg"].as_f64().unwrap() < 1e-6);
    let hit = s
        .run(
            "query.raycast",
            &json!({"origin": [25, 50, -25], "direction": [0, -1, 0]}),
        )
        .unwrap();
    assert!(
        (hit["point"][1].as_f64().unwrap() - 9.0).abs() < 1e-6,
        "{hit}"
    );
}

#[test]
fn a_sculpted_world_round_trips_through_its_file() {
    let mut s = example();
    s.run(
        "terrain.sculpt",
        &json!({"op": "noise", "center": [0, 0], "radius_m": 20, "strength_m": 1}),
    )
    .unwrap();
    let text = s.document().to_json().unwrap();
    let back = World::from_json(&text).unwrap();
    assert_eq!(back, *s.document());
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["terrain"]["edits"][1]["op"], "noise");
    // The noise's seed defaults to the world's seed.
    assert_eq!(v["terrain"]["edits"][1]["seed"], 7);
}
