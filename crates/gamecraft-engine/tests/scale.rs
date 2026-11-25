//! A scale per axis through the commands: `scale` is a number or `[x, y, z]`, the entities
//! below a rescaled instance take the scales in their own frames, a span stretches the length
//! of what it places and nothing else, and a uniform scale is written as it was.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, refuse};
use gamecraft_engine::Session;
use gamecraft_world::{Terrain, TerrainSource, World};
use serde_json::{Value, json};

/// A session of a flat terrain at height 0, 200 m square, without entities.
fn flat() -> Session {
    let mut w = World::new("Flat", 1);
    w.terrain = Some(Terrain {
        size_m: 200.0,
        cell_m: 2.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    Session::new(w)
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("{v} is not a number"))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn is_at(v: &Value, want: [f64; 3]) -> bool {
    v.as_array()
        .is_some_and(|a| a.len() == 3 && a.iter().zip(want).all(|(x, w)| close(num(x), w)))
}

fn get(s: &mut Session, id: u64) -> Value {
    s.run("entity.get", &json!({"id": id})).unwrap()
}

/// The id of the entity a ray down from 40 m hits at `x`, `z`; the terrain is `None`.
fn hit_down(s: &mut Session, x: f64, z: f64) -> Option<u64> {
    let hit = s
        .run(
            "query.raycast",
            &json!({"origin": [x, 40, z], "direction": [0, -1, 0]}),
        )
        .unwrap();
    (hit["kind"] == "entity").then(|| hit["entity"].as_u64().unwrap())
}

/// The height where a ray down from 40 m at `x`, `z` lands.
fn top_down(s: &mut Session, x: f64, z: f64) -> f64 {
    num(&s
        .run(
            "query.raycast",
            &json!({"origin": [x, 40, z], "direction": [0, -1, 0]}),
        )
        .unwrap()["point"][1])
}

/// The deck prefab: a board, two rails on it, two posts below it and a cross beam turned a
/// right angle under the board; 10 m long, 2 m wide, centred on the instance.
fn deck_prefab(s: &mut Session) {
    s.run(
        "prefab.add",
        &json!({
            "id": "deck",
            "name": "Deck",
            "children": [
                {"name": "Board", "components": [{"type": "shape", "kind": "box", "size": [2, 0.3, 10]}]},
                {"name": "Rail", "offset": [-0.95, 0.3, 0], "components": [{"type": "shape", "kind": "box", "size": [0.1, 0.9, 10]}]},
                {"name": "Rail", "offset": [0.95, 0.3, 0], "components": [{"type": "shape", "kind": "box", "size": [0.1, 0.9, 10]}]},
                {"name": "Post", "offset": [0, -2, 4.5], "components": [{"type": "shape", "kind": "cylinder", "size": [0.3, 2, 0.3]}]},
                {"name": "Post", "offset": [0, -2, -4.5], "components": [{"type": "shape", "kind": "cylinder", "size": [0.3, 2, 0.3]}]},
                // Turned a right angle: its 2 m lie along the deck's length.
                {"name": "Cross beam", "offset": [0, -0.2, 0], "rotation_y_deg": 90, "components": [{"type": "shape", "kind": "box", "size": [2, 0.2, 0.2]}]}
            ]
        }),
    )
    .unwrap();
}

fn children_of(out: &Value) -> Vec<u64> {
    out["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_u64().unwrap())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// the parameter

#[test]
fn scale_is_a_number_or_one_per_axis() {
    let mut s = flat();
    let per_axis = s
        .run(
            "entity.add",
            &json!({"name": "Deck", "position": [0, 0, 0], "scale": [1, 1, 4], "components": [{"type": "shape", "kind": "box", "size": [2, 0.5, 10]}]}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    assert_eq!(get(&mut s, per_axis)["scale"], json!([1.0, 1.0, 4.0]));
    // A number is a uniform scale; three equal numbers are the same scale, shown as one.
    let uniform = s
        .run(
            "entity.add",
            &json!({"name": "Crate", "position": [0, 0, 0], "scale": 2}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let thrice = s
        .run(
            "entity.add",
            &json!({"name": "Crate", "position": [0, 0, 0], "scale": [2.0, 2, 2.0]}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    assert_eq!(get(&mut s, uniform)["scale"], json!(2.0));
    assert_eq!(get(&mut s, thrice)["scale"], json!(2.0));
    let list = s.run("world.inspect", &json!({"limit": 10})).unwrap();
    assert_eq!(list["entities"][0]["scale"], json!([1.0, 1.0, 4.0]));
    // Given per axis, it is written per axis; as one number, as one number.
    let text = s.document().to_json().unwrap();
    let Value::Object(document) = serde_json::from_str(&text).unwrap() else {
        panic!("not an object");
    };
    let scales: Vec<Value> = document["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["scale"].clone())
        .collect();
    assert_eq!(scales, vec![json!([1.0, 1.0, 4.0]), json!(2.0), json!(2.0)]);
    assert_eq!(World::from_json(&text).unwrap(), *s.document());
    // The world's file of an example is as it was: one number.
    let e = example();
    let text = e.document().to_json().unwrap();
    assert!(
        text.contains("\"scale\": 1.0") && !text.contains("\"scale\": ["),
        "{text}"
    );
}

#[test]
fn bad_scales_name_the_parameter_or_the_axis() {
    let mut s = flat();
    s.run("entity.add", &json!({"name": "A", "position": [0, 0, 0]}))
        .unwrap();
    for (params, needles) in [
        (
            json!({"name": "B", "position": [0, 0, 0], "scale": [1, 0, 1]}),
            vec!["scale[1] is 0", "0.001"],
        ),
        (
            json!({"name": "B", "position": [0, 0, 0], "scale": [1, 1, 5000]}),
            vec!["scale[2] is 5000", "1000"],
        ),
        (
            json!({"name": "B", "position": [0, 0, 0], "scale": [1, 1]}),
            vec!["`scale` must be number | [x, y, z]"],
        ),
        (
            json!({"name": "B", "position": [0, 0, 0], "scale": [1, 2, 3, 4]}),
            vec!["`scale` must be number | [x, y, z]"],
        ),
        (
            json!({"name": "B", "position": [0, 0, 0], "scale": "2"}),
            vec!["`scale` must be number | [x, y, z]"],
        ),
        (
            json!({"name": "B", "position": [0, 0, 0], "scale": [1, "x", 1]}),
            vec!["`scale` must be number | [x, y, z]"],
        ),
        (
            json!({"name": "B", "position": [0, 0, 0], "scale": []}),
            vec!["`scale` must be number | [x, y, z]"],
        ),
    ] {
        refuse(&mut s, "entity.add", params, &needles);
    }
    refuse(
        &mut s,
        "entity.set",
        json!({"id": 1, "scale": [0.0001, 1, 1]}),
        &["scale[0] is 0.0001", "0.001"],
    );
    assert_eq!(get(&mut s, 1)["scale"], json!(1.0));
    assert_eq!(s.document().entity_count(), 1);
}

// ---------------------------------------------------------------------------------------------
// instances and their children

#[test]
fn an_instances_scales_are_its_childrens_in_their_own_frames() {
    let mut s = flat();
    deck_prefab(&mut s);
    let out = s
        .run(
            "prefab.spawn",
            &json!({"prefab": "deck", "position": [100, 0, 100], "scale": [1, 2, 10]}),
        )
        .unwrap();
    let id = out["id"].as_u64().unwrap();
    let c = children_of(&out);
    assert_eq!(c.len(), 6);
    assert_eq!(get(&mut s, id)["scale"], json!([1.0, 2.0, 10.0]));
    // The board: every axis. The rails: every axis. The posts: round, the middle for their
    // width and the y scale for their height.
    assert_eq!(get(&mut s, c[0])["scale"], json!([1.0, 2.0, 10.0]));
    assert_eq!(get(&mut s, c[1])["scale"], json!([1.0, 2.0, 10.0]));
    assert_eq!(get(&mut s, c[3])["scale"], json!([1.0, 2.0, 10.0]));
    assert!(is_at(&get(&mut s, c[3])["position"], [100.0, -4.0, 145.0]));
    // The cross beam is turned a right angle: the deck's length scale is its own x.
    assert_eq!(get(&mut s, c[5])["scale"], json!([10.0, 2.0, 1.0]));
    // What that means where it counts: a ray down finds the board 100 m long along z and
    // 2 m wide along x.
    assert_eq!(hit_down(&mut s, 100.0, 100.3), Some(c[0]));
    assert_eq!(hit_down(&mut s, 100.0, 149.7), Some(c[0]));
    assert_eq!(hit_down(&mut s, 100.0, 50.3), Some(c[0]));
    assert_eq!(hit_down(&mut s, 100.0, 150.3), None);
    assert_eq!(hit_down(&mut s, 100.7, 100.0), Some(c[0]), "still 2 m wide");
    assert_eq!(hit_down(&mut s, 101.1, 100.0), None);
    assert!(
        close(top_down(&mut s, 100.0, 100.0), 0.3 * 2.0),
        "twice as thick"
    );
    // Rescaled to one number: everything is the prefab again.
    s.run("entity.set", &json!({"id": id, "scale": 1})).unwrap();
    for c in &c {
        assert_eq!(get(&mut s, *c)["scale"], json!(1.0), "entity {c}");
    }
    assert!(is_at(&get(&mut s, c[3])["position"], [100.0, -2.0, 104.5]));
    // Rescaled per axis again, then moved and turned: relative places and scales stay.
    s.run("entity.set", &json!({"id": id, "scale": [1, 1, 4]}))
        .unwrap();
    s.run(
        "entity.set",
        &json!({"id": id, "position": [0, 1, 0], "rotation_y_deg": 90}),
    )
    .unwrap();
    assert_eq!(get(&mut s, c[0])["scale"], json!([1.0, 1.0, 4.0]));
    assert_eq!(get(&mut s, c[5])["scale"], json!([4.0, 1.0, 1.0]));
    assert!(
        is_at(&get(&mut s, c[3])["position"], [18.0, -1.0, 0.0]),
        "18 m behind, east"
    );
    // Unedited children are still derived data.
    let text = s.document().to_json().unwrap();
    // (The prefab's own `children` are its definition; the entities must say nothing.)
    let Value::Object(document) = serde_json::from_str(&text).unwrap() else {
        panic!("not an object");
    };
    for e in document["entities"].as_array().unwrap() {
        assert!(
            e.get("children").is_none() && e.get("child_edits").is_none(),
            "{e}"
        );
    }
    assert_eq!(World::from_json(&text).unwrap(), *s.document());
}

// ---------------------------------------------------------------------------------------------
// spans stretch the length only

#[test]
fn a_span_stretches_the_length_of_an_entity_and_nothing_else() {
    let mut s = flat();
    let out = s
        .run(
            "entity.add",
            &json!({"name": "Deck", "span": {"from": [0, 0], "to": [30, 0]}, "components": [{"type": "shape", "kind": "box", "size": [2, 0.5, 10]}]}),
        )
        .unwrap();
    let id = out["id"].as_u64().unwrap();
    assert_eq!(out["span"]["scale"], json!([1.0, 1.0, 3.0]));
    assert_eq!(get(&mut s, id)["scale"], json!([1.0, 1.0, 3.0]));
    // 30 m long, 2 m wide, 0.5 m thick: only the length followed the span.
    assert_eq!(hit_down(&mut s, 0.3, 0.0), Some(id));
    assert_eq!(hit_down(&mut s, 29.7, 0.0), Some(id));
    assert_eq!(hit_down(&mut s, -0.3, 0.0), None);
    assert_eq!(hit_down(&mut s, 15.0, 0.9), Some(id));
    assert_eq!(hit_down(&mut s, 15.0, 1.1), None);
    assert_eq!(top_down(&mut s, 15.0, 0.0), 0.5);
}

#[test]
fn a_span_stretches_a_deck_prefab_to_its_length_and_leaves_its_width_and_posts() {
    let mut s = flat();
    deck_prefab(&mut s);
    let out = s
        .run(
            "prefab.spawn",
            &json!({"prefab": "deck", "span": {"from": [0, 0], "to": [40, 0]}}),
        )
        .unwrap();
    let id = out["id"].as_u64().unwrap();
    let c = children_of(&out);
    assert_eq!(out["span"]["scale"], json!([1.0, 1.0, 4.0]));
    assert_eq!(get(&mut s, id)["scale"], json!([1.0, 1.0, 4.0]));
    // The board is 40 m long and 2 m wide, as thick as the prefab made it.
    assert_eq!(hit_down(&mut s, 0.3, 0.0), Some(c[0]));
    assert_eq!(hit_down(&mut s, 39.7, 0.0), Some(c[0]));
    assert_eq!(hit_down(&mut s, 20.0, 0.5), Some(c[0]));
    assert_eq!(hit_down(&mut s, 20.0, 1.1), None);
    assert!(close(top_down(&mut s, 20.0, 0.0), 0.3));
    // The rails stay at the edges: a hand's width in from each side.
    assert_eq!(hit_down(&mut s, 20.0, 0.96), Some(c[2]));
    assert_eq!(hit_down(&mut s, 20.0, -0.96), Some(c[1]));
    // The posts keep their 0.3 m width and 2 m height, 18 m from the centre (4.5 m x 4).
    assert_eq!(get(&mut s, c[3])["scale"], json!([1.0, 1.0, 4.0]));
    assert!(is_at(&get(&mut s, c[3])["position"], [2.0, -2.0, 0.0]));
    let side = s
        .run(
            "query.raycast",
            &json!({"origin": [10, -1, 0], "direction": [-1, 0, 0]}),
        )
        .unwrap();
    assert_eq!(side["entity"], json!(c[3]));
    assert!(close(num(&side["point"][0]), 2.15), "{}", side["point"]);
    // The cross beam lies along the deck: its own x took the length scale.
    assert_eq!(get(&mut s, c[5])["scale"], json!([4.0, 1.0, 1.0]));
}
