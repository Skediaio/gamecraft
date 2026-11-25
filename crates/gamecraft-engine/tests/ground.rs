//! The ground while building: `query.ground` (the ground at many points at once, before any
//! simulation runs), `at` with an offset, and spans: `entity.add` and `prefab.spawn` with
//! `span`, which place a deck between two ground points with its ends on the ground.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, refuse, snapshot, undo_depth};
use gamecraft_engine::Session;
use gamecraft_math::Portable;
use gamecraft_world::{EntityId, Heightmap, ModelBounds, Scale, Terrain, TerrainSource, World};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// A session of a flat terrain at height `h`, `size` m square, 2 m cells, without entities.
fn flat(h: f64, size: f64) -> Session {
    let mut w = World::new("Flat", 1);
    w.terrain = Some(Terrain {
        size_m: size,
        cell_m: 2.0,
        source: TerrainSource::Flat { height_m: h },
        ..Default::default()
    });
    Session::new(w)
}

/// A noise terrain of 200 m with sculpt edits: a hill, a flattened pad, a ramp and a crater.
fn sculpted() -> Session {
    let mut w = World::new("Hills", 11);
    w.terrain = Some(Terrain {
        size_m: 200.0,
        cell_m: 2.0,
        source: TerrainSource::Noise {
            seed: 5,
            relief_m: 30.0,
            feature_m: 120.0,
            octaves: 4,
            roughness: 0.5,
        },
        ..Default::default()
    });
    let mut s = Session::new(w);
    for edit in [
        json!({"op": "raise", "center": [-40, 30], "radius_m": 25, "strength_m": 8}),
        json!({"op": "flatten", "center": [50, -50], "radius_m": 20, "height_m": 12}),
        json!({"op": "ramp", "from": [0, -80], "to": [0, 80], "width_m": 8}),
        json!({"op": "lower", "center": [60, 60], "radius_m": 15, "strength_m": 6}),
    ] {
        s.run("terrain.sculpt", &edit).unwrap();
    }
    s
}

/// The height of sample `(c, r)` of a heightmap of 129 x 129 samples on 256 m: rolling ground
/// in quarters of metres, with a bank 8 m wide along x = 0.
fn sample(c: u32, r: u32) -> f64 {
    let x = f64::from(c) - 64.0;
    let z = f64::from(r) - 64.0;
    let roll = ((x / 16.0).rem_euclid(2.0) - 1.0).abs() + ((z / 32.0).rem_euclid(2.0) - 1.0).abs();
    let bank = if x > -4.0 && x < 4.0 { 6.0 } else { 0.0 };
    roll * 2.0 + bank
}

/// A session whose terrain is that heightmap, with a flattened pad and a crater over it.
fn heightmapped() -> Session {
    let mut s = Session::new(World::new("Survey", 3));
    let cols = 129;
    let rows = 129;
    let map = Heightmap::from_samples(cols, rows, sample).unwrap();
    let content = Heightmap::content_text(map.content_hash());
    s.run(
        "terrain.set",
        &json!({"kind": "heightmap", "size_m": 256, "cell_m": 2, "cols": cols, "rows": rows, "content": content}),
    )
    .unwrap();
    s.set_heightmap(map).unwrap();
    s.run(
        "terrain.sculpt",
        &json!({"op": "flatten", "center": [-80, 80], "radius_m": 18, "height_m": 5}),
    )
    .unwrap();
    s.run(
        "terrain.sculpt",
        &json!({"op": "lower", "center": [80, -80], "radius_m": 12, "strength_m": 4}),
    )
    .unwrap();
    s
}

/// `query.ground` on `points`, as a list of its answers.
fn ground(s: &mut Session, points: Vec<[f64; 2]>) -> Vec<Value> {
    s.run("query.ground", &json!({ "points": points })).unwrap()["points"]
        .as_array()
        .unwrap()
        .clone()
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

fn position(s: &Session, id: u64) -> [f64; 3] {
    s.document().entity(EntityId(id)).unwrap().position
}

fn scale_of(s: &Session, id: u64) -> Scale {
    s.document().entity(EntityId(id)).unwrap().scale
}

/// A ray straight down from 40 m at `x`, `z`: what it hit.
fn down(s: &mut Session, x: f64, z: f64) -> Value {
    s.run(
        "query.raycast",
        &json!({"origin": [x, 40, z], "direction": [0, -1, 0]}),
    )
    .unwrap()
}

/// The id of the entity a ray down hits at `x`, `z`: the terrain is `None`.
fn hit_down(s: &mut Session, x: f64, z: f64) -> Option<u64> {
    let hit = down(s, x, z);
    (hit["kind"] == "entity").then(|| hit["entity"].as_u64().unwrap())
}

/// The deck prefab of the span tests: a board, two rails on it and two posts below it, 10 m
/// long, 2 m wide, centred on the instance.
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
                {"name": "Post", "offset": [0, -2, -4.5], "components": [{"type": "shape", "kind": "cylinder", "size": [0.3, 2, 0.3]}]}
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
// query.ground

#[test]
fn many_points_answer_as_ground_height_does_one_by_one() {
    let mut s = sculpted();
    let mut points: Vec<[f64; 2]> = Vec::new();
    for i in 0..15 {
        for j in 0..15 {
            points.push([-90.0 + f64::from(i) * 12.843, -97.0 + f64::from(j) * 13.75]);
        }
    }
    // The border, and two points outside.
    points.push([100.0, -100.0]);
    points.push([500.0, 0.0]);
    points.push([0.0, -400.5]);
    let answers = ground(&mut s, points.clone());
    assert_eq!(answers.len(), points.len());
    for (point, answer) in points.iter().zip(&answers) {
        let one = s
            .run(
                "query.ground_height",
                &json!({"x": point[0], "z": point[1]}),
            )
            .unwrap();
        assert_eq!(answer, &one, "{point:?}");
        let normal: Vec<f64> = answer["normal"]
            .as_array()
            .map(|n| n.iter().map(num).collect())
            .unwrap_or_default();
        if answer["height_m"].is_number() {
            assert!(close(normal.iter().map(|v| v * v).sum(), 1.0), "{normal:?}");
            assert!(normal[1] > 0.0, "the normal points up: {normal:?}");
            let slope = num(&answer["slope_deg"]);
            assert!((0.0..=90.0).contains(&slope), "{slope}");
        } else {
            // Outside the terrain: nothing, as for one point.
            assert!(answer["height_m"].is_null());
            assert!(answer["normal"].is_null());
            assert!(answer["slope_deg"].is_null());
        }
    }
    let inside = answers.iter().filter(|a| a["height_m"].is_number()).count();
    assert_eq!(
        inside,
        points.len() - 2,
        "only the two points outside have no ground"
    );
}

#[test]
fn a_world_without_ground_answers_null_for_every_point() {
    let mut s = Session::new(World::new("Empty", 1));
    let answers = ground(&mut s, vec![[0.0, 0.0], [5.0, 5.0]]);
    assert_eq!(answers.len(), 2);
    assert!(answers.iter().all(|a| a["height_m"].is_null()));
    assert!(s.document().terrain.is_none());
}

#[test]
fn bad_points_name_the_point() {
    let mut s = example();
    refuse(&mut s, "query.ground", json!({}), &["points"]);
    refuse(
        &mut s,
        "query.ground",
        json!({"points": 5}),
        &["`points` must be array"],
    );
    refuse(
        &mut s,
        "query.ground",
        json!({"points": [1, 2]}),
        &["`points[0]` must be [x, z]"],
    );
    refuse(
        &mut s,
        "query.ground",
        json!({"points": []}),
        &["`points` holds 0 points"],
    );
    refuse(
        &mut s,
        "query.ground",
        json!({"points": [[0, 0], [1, 2, 3]]}),
        &["`points[1]` must be [x, z]"],
    );
    refuse(
        &mut s,
        "query.ground",
        json!({"points": [[0, 0], ["1", 2]]}),
        &["`points[1]` must be [x, z]", "the string"],
    );
    refuse(
        &mut s,
        "query.ground",
        json!({"points": [[0, 0], 5]}),
        &["`points[1]` must be [x, z]"],
    );
    refuse(
        &mut s,
        "query.ground",
        json!({"points": [[0, 0]], "point": [1, 1]}),
        &["unknown parameter `point`", "did you mean `points`?"],
    );
    // A number too large for a double is not valid JSON; one too large for the world is.
    refuse(
        &mut s,
        "query.ground",
        json!({"points": [[1e300, 0]]}),
        &["`points[0]` must be [x, z]", "±1000000"],
    );
    let too_many: Vec<[u32; 2]> = (0..65_537).map(|i| [i, 0]).collect();
    refuse(
        &mut s,
        "query.ground",
        json!({"points": too_many}),
        &["`points` holds 65537 points", "1 to 65536"],
    );
}

#[test]
fn the_heights_are_the_ground_the_simulation_stands_on_over_a_recipe_terrain() {
    let mut s = sculpted();
    let mut points: Vec<[f64; 2]> = Vec::new();
    for i in 0..10 {
        for j in 0..8 {
            points.push([-80.0 + f64::from(i) * 17.0, -70.0 + f64::from(j) * 19.5]);
        }
    }
    let answers = ground(&mut s, points.clone());
    let field = s.document_heights().unwrap();
    for (point, answer) in points.iter().zip(&answers) {
        // The same field, to the bit.
        assert_eq!(
            num(&answer["height_m"]),
            field.height_at(point[0], point[1]).unwrap()
        );
    }
    // A character placed at each point settles on that ground.
    let ids: Vec<u64> = points
        .iter()
        .enumerate()
        .map(|(i, point)| {
            s.run(
                "entity.add",
                &json!({"name": format!("Walker {i}"), "at": point, "components": [{"type": "character"}]}),
            )
            .unwrap()["id"]
            .as_u64()
            .unwrap()
        })
        .collect();
    s.run("sim.start", &Value::Null).unwrap();
    s.run("sim.step", &json!({"ticks": 180})).unwrap();
    for ((id, point), answer) in ids.iter().zip(&points).zip(&answers) {
        let e = s.live().unwrap().world().entity(EntityId(*id)).unwrap();
        assert!(close(e.position[0], point[0]) && close(e.position[2], point[1]));
        assert!(
            (e.position[1] - num(&answer["height_m"])).abs() < 1e-3,
            "entity {id} stands at {} m, the queried ground is {}",
            e.position[1],
            answer["height_m"]
        );
    }
}

#[test]
fn the_heights_are_the_ground_the_simulation_stands_on_over_a_heightmap() {
    let mut s = heightmapped();
    let mut points: Vec<[f64; 2]> = Vec::new();
    for i in 0..10 {
        for j in 0..8 {
            points.push([-110.0 + f64::from(i) * 24.0, -112.0 + f64::from(j) * 31.0]);
        }
    }
    let answers = ground(&mut s, points.clone());
    // Sample (20, 40) lies at x = -128 + 40, z = -128 + 80, clear of both edits: the
    // sample's own height.
    let at_sample = ground(&mut s, vec![[-88.0, -48.0]])[0]["height_m"].clone();
    assert_eq!(num(&at_sample), sample(20, 40));
    let field = s.document_heights().unwrap();
    for (point, answer) in points.iter().zip(&answers) {
        assert_eq!(
            num(&answer["height_m"]),
            field.height_at(point[0], point[1]).unwrap()
        );
    }
    let ids: Vec<u64> = points
        .iter()
        .enumerate()
        .map(|(i, point)| {
            s.run(
                "entity.add",
                &json!({"name": format!("Walker {i}"), "at": point, "components": [{"type": "character"}]}),
            )
            .unwrap()["id"]
            .as_u64()
            .unwrap()
        })
        .collect();
    s.run("sim.start", &Value::Null).unwrap();
    s.run("sim.step", &json!({"ticks": 180})).unwrap();
    for (id, answer) in ids.iter().zip(&answers) {
        let e = s.live().unwrap().world().entity(EntityId(*id)).unwrap();
        assert!(
            (e.position[1] - num(&answer["height_m"])).abs() < 1e-3,
            "entity {id} stands at {} m, the ground is {}",
            e.position[1],
            answer["height_m"]
        );
    }
}

#[test]
fn the_ground_is_answered_while_playing_and_a_query_changes_nothing() {
    let mut s = sculpted();
    s.run("sim.start", &Value::Null).unwrap();
    let before = snapshot(&s);
    let answers = ground(&mut s, vec![[50.0, -50.0], [60.0, 60.0]]);
    assert!(
        close(num(&answers[0]["height_m"]), 12.0),
        "the flattened pad"
    );
    assert_eq!(snapshot(&s), before);
    // A running world takes a sculpt between ticks, and the query sees the new ground.
    s.run(
        "terrain.sculpt",
        &json!({"op": "raise", "center": [50, -50], "radius_m": 5, "strength_m": 3, "falloff": 0}),
    )
    .unwrap();
    let answers = ground(&mut s, vec![[50.0, -50.0]]);
    assert!(close(num(&answers[0]["height_m"]), 15.0));
}

// ---------------------------------------------------------------------------------------------
// at with an offset

#[test]
fn at_with_an_offset_stands_the_base_above_the_ground() {
    let mut s = flat(0.0, 200.0);
    let id = s
        .run(
            "entity.add",
            &json!({"name": "Lamp", "at": [3, 4], "offset_m": 2.5, "components": [{"type": "shape", "kind": "cylinder", "size": [0.2, 3, 0.2]}]}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    assert_eq!(position(&s, id), [3.0, 2.5, 4.0]);
    // Without an offset the base is on the ground, as before.
    let ground_id = s
        .run("entity.add", &json!({"name": "Crate", "at": [3, 4]}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    assert_eq!(position(&s, ground_id), [3.0, 0.0, 4.0]);
    // Moving it: the offset with `at`.
    s.run(
        "entity.set",
        &json!({"id": id, "at": [10, 10], "offset_m": -1}),
    )
    .unwrap();
    assert_eq!(position(&s, id), [10.0, -1.0, 10.0]);
    assert_eq!(
        position(&s, ground_id),
        [3.0, 0.0, 4.0],
        "the other one did not move"
    );
    // `at` alone puts it back on the ground.
    s.run("entity.set", &json!({"id": id, "at": [10, 10]}))
        .unwrap();
    assert_eq!(position(&s, id), [10.0, 0.0, 10.0]);
    // On ground that is not level, on the example world; an instance and its children.
    let mut e = example();
    let h = num(&e
        .run("query.ground_height", &json!({"x": 12, "z": -7}))
        .unwrap()["height_m"]);
    e.run(
        "prefab.add",
        &json!({"id": "lamp", "name": "Lamp", "children": [{"name": "Post", "components": [{"type": "shape", "kind": "cylinder", "size": [0.2, 4, 0.2]}]}]}),
    )
    .unwrap();
    let spawned = e
        .run(
            "prefab.spawn",
            &json!({"prefab": "lamp", "at": [12, -7], "offset_m": 0.4}),
        )
        .unwrap();
    let id = spawned["id"].as_u64().unwrap();
    let child = children_of(&spawned)[0];
    assert!(
        close(position(&e, id)[1], h + 0.4),
        "{} vs {h}",
        position(&e, id)[1]
    );
    assert!(close(position(&e, child)[1], h + 0.4));
}

#[test]
fn an_offset_needs_at() {
    let mut s = flat(0.0, 100.0);
    s.run("prefab.add", &json!({"id": "x", "name": "X"}))
        .unwrap();
    s.run("entity.add", &json!({"name": "A", "at": [0, 0]}))
        .unwrap();
    refuse(
        &mut s,
        "entity.add",
        json!({"name": "B", "position": [0, 1, 0], "offset_m": 1}),
        &["`offset_m` goes with `at`"],
    );
    refuse(
        &mut s,
        "entity.add",
        json!({"name": "B", "offset_m": 1}),
        &["`offset_m` goes with `at`"],
    );
    refuse(
        &mut s,
        "entity.set",
        json!({"id": 1, "offset_m": 1}),
        &["`offset_m` goes with `at`"],
    );
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "x", "position": [0, 1, 0], "offset_m": 1}),
        &["`offset_m` goes with `at`"],
    );
    refuse(
        &mut s,
        "entity.add",
        json!({"name": "B", "at": [0, 0], "offset_m": 1e300}),
        &["`offset_m` is 1e300"],
    );
}

// ---------------------------------------------------------------------------------------------
// spans

#[test]
fn a_span_lays_an_entity_between_two_ground_points() {
    let mut s = flat(0.0, 200.0);
    let out = s
        .run(
            "entity.add",
            &json!({"name": "Deck", "span": {"from": [0, 0], "to": [30, 0]}, "components": [{"type": "shape", "kind": "box", "size": [2, 0.5, 10]}]}),
        )
        .unwrap();
    let id = out["id"].as_u64().unwrap();
    assert!(
        is_at(&out["position"], [15.0, 0.0, 0.0]),
        "{}",
        out["position"]
    );
    assert_eq!(out["span"]["length_m"], json!(10.0));
    assert_eq!(out["span"]["scale"], json!([1.0, 1.0, 3.0]));
    let ends = out["span"]["ends"].as_array().unwrap();
    assert_eq!(ends.len(), 2);
    assert!(is_at(&ends[0]["point"], [0.0, 0.0, 0.0]));
    assert!(is_at(&ends[1]["point"], [30.0, 0.0, 0.0]));
    assert_eq!(num(&ends[0]["ground_m"]), 0.0);
    assert_eq!(num(&ends[1]["gap_m"]), 0.0);
    let e = s.document().entity(EntityId(id)).unwrap();
    assert_eq!(e.position, [15.0, 0.0, 0.0]);
    assert!(
        close(e.rotation_y_deg, 270.0),
        "it faces `to`: {}",
        e.rotation_y_deg
    );
    assert_eq!(e.scale, Scale::per_axis(1.0, 1.0, 3.0));
    // Only the length follows the span: 30 m long, 2 m wide, 0.5 m thick, its ends on the
    // ground and nothing beyond them.
    assert_eq!(hit_down(&mut s, 0.2, 0.0), Some(id));
    assert_eq!(hit_down(&mut s, 29.8, 0.9), Some(id));
    assert_eq!(hit_down(&mut s, -0.2, 0.0), None, "beyond `from`");
    assert_eq!(hit_down(&mut s, 30.2, 0.0), None, "beyond `to`");
    assert_eq!(hit_down(&mut s, 15.0, 1.1), None, "beside the deck");
    assert!(close(num(&down(&mut s, 15.0, 0.0)["point"][1]), 0.5));
    let ends = ground(&mut s, vec![[0.0, 0.0], [30.0, 0.0]]);
    assert_eq!(num(&ends[0]["height_m"]), 0.0);
    assert_eq!(num(&ends[1]["height_m"]), 0.0);
}

#[test]
fn a_span_takes_its_length_an_offset_and_its_own_direction() {
    let mut s = flat(0.0, 200.0);
    // `length_m` given: the entity is scaled so that length spans the ends.
    let out = s
        .run(
            "entity.add",
            &json!({"name": "Deck", "span": {"from": [0, 0], "to": [30, 0], "length_m": 5, "offset_m": 1}}),
        )
        .unwrap();
    assert!(is_at(&out["position"], [15.0, 1.0, 0.0]));
    assert_eq!(out["span"]["scale"], json!([1.0, 1.0, 6.0]));
    assert_eq!(num(&out["span"]["ends"][0]["gap_m"]), 1.0);
    assert_eq!(num(&out["span"]["ends"][1]["gap_m"]), 1.0);
    // South: the entity faces `to` (+Z), yaw 180.
    let out = s
        .run(
            "entity.add",
            &json!({"name": "Walkway", "span": {"from": [-40, -10], "to": [-40, 30]}, "components": [{"type": "shape", "kind": "cylinder", "size": [1, 0.2, 1], "color": [0.5, 0.5, 0.5]}]}),
        )
        .unwrap();
    let id = out["id"].as_u64().unwrap();
    assert!(is_at(&out["position"], [-40.0, 0.0, 10.0]));
    assert_eq!(
        out["span"]["length_m"],
        json!(1.0),
        "a cylinder's length is its diameter"
    );
    assert_eq!(out["span"]["scale"], json!([1.0, 1.0, 40.0]));
    let e = s.document().entity(EntityId(id)).unwrap();
    assert!(close(e.rotation_y_deg, 180.0), "{}", e.rotation_y_deg);
    // A diagonal span: 30 m along x and 40 m along z is 50 m.
    let out = s
        .run(
            "entity.add",
            &json!({"name": "Diagonal", "span": {"from": [0, 0], "to": [30, 40]}, "components": [{"type": "shape", "kind": "box", "size": [1, 1, 10]}]}),
        )
        .unwrap();
    assert!(is_at(&out["position"], [15.0, 0.0, 20.0]));
    assert_eq!(out["span"]["scale"], json!([1.0, 1.0, 5.0]));
    let id = out["id"].as_u64().unwrap();
    let e = s.document().entity(EntityId(id)).unwrap();
    // It faces (30, 40): yaw_forward = (-sin, -cos) is that direction, normalised.
    let yaw = e.rotation_y_deg * (std::f64::consts::PI / 180.0);
    assert!(
        close(-yaw.psin(), 0.6) && close(-yaw.pcos(), 0.8),
        "{:?}",
        e.rotation_y_deg
    );
}

#[test]
fn a_span_over_sloped_ground_stands_on_the_higher_end_and_says_the_gap() {
    let mut s = Session::new(World::new("Slope", 1));
    s.run(
        "terrain.set",
        // A plane rising from -10 m at the south edge to 10 m at the north edge.
        &json!({"kind": "grid", "size_m": 256, "cell_m": 2, "cols": 2, "rows": 2, "heights": [-10, -10, 10, 10]}),
    )
    .unwrap();
    let low = num(&s
        .run("query.ground_height", &json!({"x": 0, "z": -50}))
        .unwrap()["height_m"]);
    let high = num(&s
        .run("query.ground_height", &json!({"x": 0, "z": 50}))
        .unwrap()["height_m"]);
    assert!(close(low, -3.90625) && close(high, 3.90625), "{low} {high}");
    let out = s
        .run(
            "entity.add",
            &json!({"name": "Deck", "span": {"from": [0, -50], "to": [0, 50]}, "components": [{"type": "shape", "kind": "box", "size": [2, 0.5, 10]}]}),
        )
        .unwrap();
    assert!(
        is_at(&out["position"], [0.0, high, 0.0]),
        "{}",
        out["position"]
    );
    let ends = out["span"]["ends"].as_array().unwrap();
    assert!(is_at(&ends[0]["point"], [0.0, high, -50.0]));
    assert!(close(num(&ends[0]["ground_m"]), low));
    assert!(close(num(&ends[0]["gap_m"]), high - low), "{}", ends[0]);
    assert!(is_at(&ends[1]["point"], [0.0, high, 50.0]));
    assert!(close(num(&ends[1]["gap_m"]), 0.0), "{}", ends[1]);
    let id = out["id"].as_u64().unwrap();
    assert_eq!(hit_down(&mut s, 0.0, 50.0), Some(id));
    // Reversed ends: the same level.
    let out = s
        .run(
            "entity.add",
            &json!({"name": "Deck back", "span": {"from": [4, 50], "to": [4, -50]}, "components": [{"type": "shape", "kind": "box", "size": [2, 0.5, 10]}]}),
        )
        .unwrap();
    assert_eq!(num(&out["position"][1]), high);
}

#[test]
fn bad_spans_name_what_is_wrong() {
    let mut s = flat(0.0, 100.0);
    let span = |v: Value| json!({"name": "Deck", "span": v, "components": [{"type": "shape", "kind": "box", "size": [2, 0.5, 10]}]});
    refuse(
        &mut s,
        "entity.add",
        span(json!({"from": [0, 0], "to": [0, 0]})),
        &["the same point"],
    );
    refuse(
        &mut s,
        "entity.add",
        span(json!({"from": [0, 0], "to": [0, 1e-10]})),
        &["the same point"],
    );
    refuse(
        &mut s,
        "entity.add",
        span(json!({"from": [0, 0], "to": [0, 300]})),
        &["no ground at [0, 300]", "span.to"],
    );
    refuse(
        &mut s,
        "entity.add",
        span(json!({"from": [-300, 0], "to": [0, 0]})),
        &["no ground at [-300, 0]", "span.from"],
    );
    refuse(
        &mut s,
        "entity.add",
        span(json!({"from": [0, 0]})),
        &["span.to"],
    );
    refuse(
        &mut s,
        "entity.add",
        span(json!({"from": [0, 0], "to": [30, 0], "lenght_m": 10})),
        &["span.lenght_m", "did you mean `length_m`?"],
    );
    refuse(
        &mut s,
        "entity.add",
        json!({"name": "Deck", "span": {"from": [0, 0], "to": [30, 0]}}),
        &["`span` needs `length_m`", "no shape to measure"],
    );
    refuse(
        &mut s,
        "entity.add",
        span(json!({"from": [0, 0], "to": [30, 0], "length_m": 0})),
        &["span.length_m", "above 0"],
    );
    refuse(
        &mut s,
        "entity.add",
        span(json!({"from": [0, 0], "to": [30, 0], "length_m": -5})),
        &["span.length_m"],
    );
    refuse(
        &mut s,
        "entity.add",
        span(json!({"from": [-45, 0], "to": [45, 0], "length_m": 0.05})),
        &["scale it by 1800", "0.001 to 1000"],
    );
    refuse(
        &mut s,
        "entity.add",
        span(json!({"from": [0, 0], "to": [30, 0], "offset_m": 1e300})),
        &["`span.offset_m` is 1e300"],
    );
    // One way to place it, not two.
    let mut both = span(json!({"from": [0, 0], "to": [30, 0]}));
    both["position"] = json!([0, 0, 0]);
    refuse(
        &mut s,
        "entity.add",
        both,
        &["`span` places the entity", "`position`"],
    );
    let mut both = span(json!({"from": [0, 0], "to": [30, 0]}));
    both["at"] = json!([0, 0]);
    refuse(
        &mut s,
        "entity.add",
        both,
        &["`span` places the entity", "`at`"],
    );
    for key in ["scale", "rotation_y_deg"] {
        let mut both = span(json!({"from": [0, 0], "to": [30, 0]}));
        both[key] = json!(2);
        refuse(
            &mut s,
            "entity.add",
            both,
            &[
                "`span` sets the scale and the turn itself",
                &format!("`{key}`"),
            ],
        );
    }
    refuse(
        &mut s,
        "entity.add",
        json!({"name": "Deck", "span": [[0, 0], [30, 0]]}),
        &["`span` must be object"],
    );
    // No ground at all.
    let mut empty = Session::new(World::new("Empty", 1));
    refuse(
        &mut empty,
        "entity.add",
        span(json!({"from": [0, 0], "to": [30, 0]})),
        &["no ground at [0, 0]", "no terrain"],
    );
    // A prefab too.
    s.run("prefab.add", &json!({"id": "x", "name": "X"}))
        .unwrap();
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "x", "span": {"from": [0, 0], "to": [0, 0]}}),
        &["the same point"],
    );
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "x", "span": {"from": [0, 0], "to": [30, 0]}, "at": [0, 0]}),
        &["`span` places the instance", "`at`"],
    );
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "x", "span": {"from": [0, 0], "to": [30, 0]}, "scale": 2}),
        &["`span` sets the scale and the turn itself", "`scale`"],
    );
    assert_eq!(s.document().entity_count(), 0);
}

#[test]
fn a_prefab_span_scales_the_instance_and_its_children() {
    let mut s = flat(0.0, 200.0);
    deck_prefab(&mut s);
    let out = s
        .run(
            "prefab.spawn",
            &json!({"prefab": "deck", "span": {"from": [0, 0], "to": [40, 0]}}),
        )
        .unwrap();
    let id = out["id"].as_u64().unwrap();
    let children = children_of(&out);
    assert_eq!(children.len(), 5);
    assert!(is_at(&out["position"], [20.0, 0.0, 0.0]));
    assert_eq!(
        out["span"]["length_m"],
        json!(10.0),
        "measured from the board"
    );
    assert_eq!(out["span"]["scale"], json!([1.0, 1.0, 4.0]));
    assert!(is_at(&out["span"]["ends"][1]["point"], [40.0, 0.0, 0.0]));
    assert!(is_at(&json!(position(&s, id)), [20.0, 0.0, 0.0]));
    // The board stands on the instance, four times its length; the posts hang below its ends.
    assert!(is_at(&json!(position(&s, children[0])), [20.0, 0.0, 0.0]));
    assert!(is_at(&json!(position(&s, children[3])), [2.0, -2.0, 0.0]));
    assert!(is_at(&json!(position(&s, children[4])), [38.0, -2.0, 0.0]));
    for c in children.iter().chain(std::iter::once(&id)) {
        assert_eq!(
            scale_of(&s, *c),
            Scale::per_axis(1.0, 1.0, 4.0),
            "entity {c}"
        );
    }
    // Both ends of the deck are on the ground, and the deck reaches exactly to them.
    assert_eq!(hit_down(&mut s, 0.3, 0.0), Some(children[0]));
    assert!(close(num(&down(&mut s, 20.0, 0.0)["point"][1]), 0.3));
    assert_eq!(hit_down(&mut s, 39.7, 0.0), Some(children[0]));
    assert_eq!(hit_down(&mut s, -0.3, 0.0), None);
    assert_eq!(hit_down(&mut s, 40.3, 0.0), None);
    assert_eq!(
        hit_down(&mut s, 20.0, 0.5),
        Some(children[0]),
        "2 m wide, as the prefab made it"
    );
    // The rail on that side, a hand's width in from the edge.
    assert_eq!(hit_down(&mut s, 20.0, 0.96), Some(children[2]));
    assert_eq!(hit_down(&mut s, 20.0, 1.1), None);
    // The file keeps the instance and derives the children again: no edit of a child.
    assert_derived(&s, id, &children);
    let back = World::from_json(&s.document().to_json().unwrap()).unwrap();
    assert_eq!(back, *s.document());
}

/// The instance `id` is written as its prefab alone: no `children`, no `child_edits`, and
/// not one of its unedited children is in the file at all (they are placed again on reading).
fn assert_derived(s: &Session, id: u64, children: &[u64]) {
    let text = s.document().to_json().unwrap();
    let world: Value = serde_json::from_str(&text).unwrap();
    let written = |wanted: u64| {
        world["entities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"].as_u64() == Some(wanted))
            .cloned()
    };
    let instance = written(id).unwrap_or_else(|| panic!("the instance {id} is not in {text}"));
    assert!(
        instance.get("children").is_none() && instance.get("child_edits").is_none(),
        "{instance}"
    );
    for c in children {
        assert!(
            written(*c).is_none(),
            "entity {c} is derived and is written: {text}"
        );
    }
}

#[test]
fn a_span_centres_an_off_centre_prefab_on_its_ends() {
    let mut s = flat(0.0, 200.0);
    s.run(
        "prefab.add",
        &json!({
            "id": "ramp",
            "name": "Ramp",
            "children": [
                {"name": "Board", "offset": [0, 0, 5], "components": [{"type": "shape", "kind": "box", "size": [2, 0.3, 10]}]}
            ]
        }),
    )
    .unwrap();
    let out = s
        .run(
            "prefab.spawn",
            &json!({"prefab": "ramp", "span": {"from": [0, 0], "to": [10, 0]}}),
        )
        .unwrap();
    // The board runs from the instance's origin backwards 10 m; to lie between the ends the
    // instance stands past `to`.
    assert!(
        is_at(&out["position"], [10.0, 0.0, 0.0]),
        "{}",
        out["position"]
    );
    let board = children_of(&out)[0];
    assert!(is_at(&json!(position(&s, board)), [5.0, 0.0, 0.0]));
    assert_eq!(hit_down(&mut s, 0.3, 0.0), Some(board));
    assert_eq!(hit_down(&mut s, 9.7, 0.0), Some(board));
    assert_eq!(hit_down(&mut s, -0.3, 0.0), None);
    assert_eq!(hit_down(&mut s, 10.3, 0.0), None);
}

#[test]
fn a_prefab_span_measures_models_with_bounds() {
    let mut s = flat(0.0, 200.0);
    // A deck whose board is a model 12 m long along z, from z = -2 to 10: its centre lies
    // 4 m behind the model's origin.
    let mut bounds = BTreeMap::new();
    bounds.insert(
        "models/deck.glb".to_string(),
        ModelBounds {
            min: [-1.0, 0.0, -2.0],
            max: [1.0, 0.3, 10.0],
        },
    );
    s.set_model_bounds(bounds);
    s.run(
        "prefab.add",
        &json!({
            "id": "model-deck",
            "name": "Deck",
            "components": [{"type": "model", "asset": "models/deck.glb", "collider": "box"}]
        }),
    )
    .unwrap();
    let out = s
        .run(
            "prefab.spawn",
            &json!({"prefab": "model-deck", "span": {"from": [0, 0], "to": [60, 0]}}),
        )
        .unwrap();
    assert_eq!(out["span"]["length_m"], json!(12.0), "from z = -2 to 10");
    assert_eq!(out["span"]["scale"], json!([1.0, 1.0, 5.0]));
    // The board's centre is 4 m behind its origin: the instance stands 20 m past the midpoint.
    assert!(
        is_at(&out["position"], [50.0, 0.0, 0.0]),
        "{}",
        out["position"]
    );
    let id = out["id"].as_u64().unwrap();
    // The model's collider box now runs from x = 0 to 60: its ends are on the ground.
    assert_eq!(hit_down(&mut s, 0.3, 0.0), Some(id));
    assert_eq!(hit_down(&mut s, 59.7, 0.0), Some(id));
    assert_eq!(hit_down(&mut s, -0.3, 0.0), None);
    assert_eq!(hit_down(&mut s, 60.3, 0.0), None);
}

#[test]
fn a_prefab_span_measures_turned_and_nested_children() {
    let mut s = flat(0.0, 200.0);
    s.run(
        "prefab.add",
        &json!({
            "id": "beam",
            "name": "Beam",
            "children": [
                // Turned 90 degrees: 8 m along the parent's x, 0.4 along its z.
                {"name": "Cross", "offset": [0, -0.4, 0], "components": [{"type": "shape", "kind": "box", "size": [8, 0.4, 0.4]}]}
            ]
        }),
    )
    .unwrap();
    s.run(
        "prefab.add",
        &json!({
            "id": "long-deck",
            "name": "Long deck",
            "children": [
                {"name": "Board", "offset": [0, 0, -3], "components": [{"type": "shape", "kind": "box", "size": [2, 0.3, 10]}]},
                // A nested instance, turned 90 degrees: its 8 m along x lie along z.
                {"name": "Trestle", "offset": [0, 0, 5], "prefab": "beam", "rotation_y_deg": 90}
            ]
        }),
    )
    .unwrap();
    let out = s
        .run(
            "prefab.spawn",
            &json!({"prefab": "long-deck", "span": {"from": [0, 0], "to": [32, 0]}}),
        )
        .unwrap();
    // The board runs from z = -8 to 2; the turned trestle from z = 1 to 9: 17 m in all, its
    // centre 0.5 m behind the instance's origin.
    // (A right angle through sines leaves a hair: 17 m to a hair's breadth.)
    assert!(
        close(num(&out["span"]["length_m"]), 17.0),
        "{}",
        out["span"]
    );
    assert!(
        close(num(&out["span"]["scale"][2]), 32.0 / 17.0),
        "{}",
        out["span"]
    );
    assert!(
        is_at(&out["position"], [16.0 + 0.5 * 32.0 / 17.0, 0.0, 0.0]),
        "{}",
        out["position"]
    );
    // The trestle reaches `from` but lies under the ground; the board covers the rest.
    assert_eq!(
        hit_down(&mut s, 0.3, 0.0),
        None,
        "the trestle is below the ground"
    );
    assert_eq!(
        hit_down(&mut s, 13.0, 0.0),
        None,
        "the board starts at 13.2"
    );
    assert_eq!(
        hit_down(&mut s, 15.5, 0.0),
        Some(children_of(&out)[0]),
        "the board over the valley"
    );
    assert_eq!(
        hit_down(&mut s, 31.7, 0.0),
        Some(children_of(&out)[0]),
        "the board's far end is at `to`"
    );
}

#[test]
fn a_prefab_without_measurable_geometry_asks_for_its_length() {
    let mut s = flat(0.0, 200.0);
    s.run(
        "prefab.add",
        &json!({"id": "m", "name": "M", "components": [{"type": "model", "asset": "models/deck.glb"}]}),
    )
    .unwrap();
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "m", "span": {"from": [0, 0], "to": [30, 0]}}),
        &[
            "`span` needs `length_m`",
            "prefab \"m\"",
            "nothing to measure",
        ],
    );
    s.run(
        "prefab.add",
        &json!({"id": "light", "name": "L", "children": [{"name": "Bulb", "offset": [0, 3, 4], "components": [{"type": "light"}]}]}),
    )
    .unwrap();
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "light", "span": {"from": [0, 0], "to": [30, 0]}}),
        &["`span` needs `length_m`", "prefab \"light\""],
    );
    let out = s
        .run(
            "prefab.spawn",
            &json!({"prefab": "m", "span": {"from": [0, 0], "to": [30, 0], "length_m": 10}}),
        )
        .unwrap();
    assert_eq!(out["span"]["scale"], json!([1.0, 1.0, 3.0]));
}

#[test]
fn a_span_is_one_undo_step_and_works_on_a_running_world() {
    let mut s = flat(0.0, 200.0);
    deck_prefab(&mut s);
    assert_eq!(undo_depth(&s), 1);
    let out = s
        .run(
            "prefab.spawn",
            &json!({"prefab": "deck", "span": {"from": [0, 0], "to": [40, 0]}}),
        )
        .unwrap();
    assert_eq!(undo_depth(&s), 2);
    assert_eq!(s.document().entity_count(), 6);
    assert!(is_at(&out["position"], [20.0, 0.0, 0.0]));
    s.undo().unwrap();
    assert_eq!(
        s.document().entity_count(),
        0,
        "the instance and its children go together"
    );
    s.redo().unwrap();
    assert_eq!(s.document().entity_count(), 6);
    // While playing: the running world gets it, the document does not.
    s.run("sim.start", &Value::Null).unwrap();
    let out = s
        .run(
            "prefab.spawn",
            &json!({"prefab": "deck", "span": {"from": [0, 20], "to": [40, 20]}}),
        )
        .unwrap();
    assert_eq!(s.document().entity_count(), 6);
    assert_eq!(s.live().unwrap().world().entity_count(), 12);
    assert!(is_at(&out["position"], [20.0, 0.0, 20.0]));
    assert_eq!(num(&out["span"]["ends"][0]["gap_m"]), 0.0);
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "deck", "span": {"from": [0, 0], "to": [500, 0]}}),
        &["span.to", "no ground"],
    );
    assert_eq!(s.live().unwrap().world().entity_count(), 12);
}

#[test]
fn setting_the_scale_of_an_instance_scales_its_children_with_it() {
    let mut s = flat(0.0, 200.0);
    deck_prefab(&mut s);
    let out = s
        .run("prefab.spawn", &json!({"prefab": "deck", "at": [100, 100]}))
        .unwrap();
    let id = out["id"].as_u64().unwrap();
    let children = children_of(&out);
    assert!(is_at(
        &json!(position(&s, children[3])),
        [100.0, -2.0, 104.5]
    ));
    assert_eq!(scale_of(&s, children[3]), Scale::ONE);
    s.run("entity.set", &json!({"id": id, "scale": 2})).unwrap();
    assert!(is_at(
        &json!(position(&s, children[3])),
        [100.0, -4.0, 109.0]
    ));
    for c in &children {
        assert_eq!(scale_of(&s, *c), Scale::uniform(2.0), "entity {c}");
    }
    // Moving and turning the instance keeps them relative.
    s.run(
        "entity.set",
        &json!({"id": id, "position": [0, 1, 0], "rotation_y_deg": 90}),
    )
    .unwrap();
    let post = position(&s, children[3]);
    assert!(
        is_at(&json!(post), [9.0, -3.0, 0.0]),
        "9 m behind (east of) the instance, turned: {post:?}"
    );
    assert_eq!(scale_of(&s, children[3]), Scale::uniform(2.0));
    // The children are still derived data: the instance is written without a word of them.
    assert_derived(&s, id, &children);
    let text = s.document().to_json().unwrap();
    let back = World::from_json(&text).unwrap();
    assert_eq!(back, *s.document());
}

#[test]
fn an_instance_scale_of_one_leaves_children_where_they_were() {
    let mut s = example();
    s.run(
        "prefab.add",
        &json!({"id": "box", "name": "Box", "children": [{"name": "Inner", "offset": [1, 0.5, -2], "rotation_y_deg": 30, "components": [{"type": "shape", "kind": "box", "size": [1, 1, 1]}]}]}),
    )
    .unwrap();
    let out = s
        .run(
            "prefab.spawn",
            &json!({"prefab": "box", "position": [4, 2, -6], "rotation_y_deg": 17}),
        )
        .unwrap();
    let child = children_of(&out)[0];
    let e = s.document().entity(EntityId(child)).unwrap();
    // The offset turned by 17 degrees, as before scales reached children.
    let yaw = 17.0_f64.to_radians();
    assert!(
        is_at(
            &json!(e.position),
            [
                4.0 + yaw.pcos() - 2.0 * yaw.psin(),
                2.5,
                -6.0 - yaw.psin() - 2.0 * yaw.pcos()
            ]
        ),
        "{:?}",
        e.position
    );
    assert!(
        (e.rotation_y_deg - 47.0).abs() < 1e-9,
        "{}",
        e.rotation_y_deg
    );
    assert_eq!(e.scale, Scale::ONE);
}
