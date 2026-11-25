//! A script builds a bridge with the ground query and a span: `gamecraft-cli run --script`.

// Test code: clippy.toml allows these inside #[test] functions only.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
use common::*;
use serde_json::{Value, json};

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("{v} is not a number"))
}

#[test]
fn a_script_queries_the_ground_and_spans_a_deck_over_a_valley() {
    let dir = Temp::new("ground-script");
    let script = dir.write(
        "bridge.json",
        r#"[
        ["terrain.set", {"kind": "noise", "size_m": 256, "cell_m": 2, "relief_m": 14, "feature_m": 160, "seed": 4}],
        ["terrain.sculpt", {"op": "flatten", "center": [-80, 0], "radius_m": 14, "height_m": 8}],
        ["terrain.sculpt", {"op": "flatten", "center": [80, 0], "radius_m": 14, "height_m": 8}],
        ["prefab.add", {"id": "deck", "name": "Deck", "children": [
            {"name": "Board", "components": [{"type": "shape", "kind": "box", "size": [2, 0.3, 10]}]},
            {"name": "Post", "offset": [0, -3, 4.5], "components": [{"type": "shape", "kind": "cylinder", "size": [0.3, 3, 0.3]}]},
            {"name": "Post", "offset": [0, -3, -4.5], "components": [{"type": "shape", "kind": "cylinder", "size": [0.3, 3, 0.3]}]}
        ]}],
        ["query.ground", {"points": [[-80, 0], [80, 0], [-40, 0], [0, 0], [40, 0], [500, 0]]}],
        ["prefab.spawn", {"prefab": "deck", "name": "Bridge", "span": {"from": [-80, 0], "to": [80, 0]}}],
        ["entity.add", {"name": "Abutment", "at": [-86, 3], "offset_m": 0.5, "components": [{"type": "shape", "kind": "box", "size": [2, 1, 2], "solid": true}]}]
    ]"#,
    );
    let ran = run_in(
        &dir.0,
        &[
            "run",
            "--new",
            "{}",
            "--script",
            script.to_str().unwrap(),
            "--out",
            "bridge.gamecraft",
        ],
    );
    assert_eq!(ran.code, 0, "{}", ran.err);
    let lines = ran.json_lines();
    assert_eq!(lines.len(), 7, "{lines:?}");
    // The ground at both banks is the pads' 8 m; the valley floor between is lower; a point
    // outside has none.
    let ground = &lines[4]["points"];
    let heights: Vec<Option<f64>> = (0..6).map(|i| ground[i]["height_m"].as_f64()).collect();
    assert_eq!(heights[0], Some(8.0));
    assert_eq!(heights[1], Some(8.0));
    assert!(heights[2].is_some_and(|h| h < 8.0), "{heights:?}");
    assert!(heights[3].is_some_and(|h| h < 8.0), "{heights:?}");
    assert!(heights[4].is_some_and(|h| h < 8.0), "{heights:?}");
    assert_eq!(heights[5], None);
    assert!(ground[3]["normal"][1].as_f64().unwrap() > 0.0);
    // 160 m between the ends, a 10 m deck: scale 16, level with the higher bank.
    let span = &lines[5]["span"];
    assert_eq!(lines[5]["position"], json!([0.0, 8.0, 0.0]));
    assert_eq!(span["length_m"], json!(10.0));
    assert_eq!(span["scale"], json!([1.0, 1.0, 16.0]), "the length only");
    assert_eq!(num(&span["ends"][0]["point"][1]), 8.0);
    assert_eq!(num(&span["ends"][1]["point"][1]), 8.0);
    assert_eq!(num(&span["ends"][0]["gap_m"]), 0.0);
    assert_eq!(num(&span["ends"][1]["gap_m"]), 0.0);
    // The abutment stands half a metre over the ground `at` named (checked against the saved
    // world below).

    // The saved world: the bridge is there, over the valley, with its posts under its ends.
    let world = dir.path("bridge.gamecraft");
    let mut server = Server::start("serve", &["--file", &world.display().to_string()], &dir.0);
    let bridge: Value = server.ok("entity.get", json!({"id": "Bridge"}));
    let children: Vec<u64> = bridge["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_u64().unwrap())
        .collect();
    assert_eq!(children.len(), 3);
    let post: Value = server.ok("entity.get", json!({"id": children[1]}));
    let p = post["position"].as_array().unwrap();
    assert_eq!(num(&p[0]), -72.0, "{post}");
    assert_eq!(num(&p[1]), 8.0 - 3.0, "{post}: the post keeps its 3 m");
    let below = server.ok(
        "query.raycast",
        json!({"origin": [0, 60, 0], "direction": [0, -1, 0]}),
    );
    assert_eq!(
        below["entity"],
        json!(children[0]),
        "the board over the valley"
    );
    assert!(
        (num(&below["point"][1]) - (8.0 + 0.3)).abs() < 1e-9,
        "{}",
        below["point"]
    );
    let valley = server.ok("query.ground_height", json!({"x": 0, "z": 0}));
    assert!(num(&valley["height_m"]) < 8.0);
    let beside = server.ok("query.ground_height", json!({"x": -86, "z": 3}));
    let abutment = num(&lines[6]["position"][1]);
    assert!(
        (abutment - (num(&beside["height_m"]) + 0.5)).abs() < 1e-9,
        "{abutment} vs {}",
        beside["height_m"]
    );
    let miss = server.ok(
        "query.raycast",
        json!({"origin": [100, 60, 0], "direction": [0, -1, 0]}),
    );
    assert_eq!(miss["kind"], "terrain", "nothing beyond `to`");
    server.finish();
}
