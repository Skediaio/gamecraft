//! The query commands: `entity.get`, `entity.find`, `query.ground_height`, `query.raycast`,
//! `query.line_of_sight`, `species.list` and `component.list`.

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
use gamecraft_math::{Portable, rad_to_deg};
use gamecraft_world::{Terrain, TerrainSource, World};
use serde_json::{Value, json};

/// A session of a flat terrain at height 0, 100 m square, 1 m cells, without entities.
fn flat() -> Session {
    let mut w = World::new("Flat", 1);
    w.terrain = Some(Terrain {
        size_m: 100.0,
        cell_m: 1.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    Session::new(w)
}

/// A session without terrain and without entities.
fn empty() -> Session {
    Session::new(World::new("Empty", 1))
}

/// Adds an entity with one shape and returns its id.
fn add_shape(s: &mut Session, name: &str, position: [f64; 3], rot: f64, shape: Value) -> u64 {
    let mut c = shape;
    c["type"] = json!("shape");
    s.run(
        "entity.add",
        &json!({"name": name, "position": position, "rotation_y_deg": rot, "components": [c]}),
    )
    .unwrap()["id"]
        .as_u64()
        .unwrap()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("{v} is not a number"))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn assert_vec(v: &Value, want: [f64; 3]) {
    let got: Vec<f64> = v.as_array().unwrap().iter().map(num).collect();
    assert!(
        got.iter().zip(want).all(|(g, w)| close(*g, w)),
        "{got:?} is not {want:?}"
    );
}

fn ray(s: &mut Session, origin: [f64; 3], direction: [f64; 3]) -> Value {
    s.run(
        "query.raycast",
        &json!({"origin": origin, "direction": direction}),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------------------------
// query.raycast

#[test]
fn a_ray_down_onto_a_box_top_hits_the_top_with_normal_up() {
    let mut s = flat();
    let id = add_shape(
        &mut s,
        "Block",
        [0.0, 0.0, 0.0],
        0.0,
        json!({"kind": "box", "size": [2, 2, 2]}),
    );
    let r = ray(&mut s, [0.3, 10.0, -0.2], [0.0, -1.0, 0.0]);
    assert_eq!(r["hit"], json!(true));
    assert_eq!(r["kind"], json!("entity"));
    assert_eq!(r["entity"], json!(id));
    assert_eq!(r["name"], json!("Block"));
    assert_vec(&r["point"], [0.3, 2.0, -0.2]);
    assert_vec(&r["normal"], [0.0, 1.0, 0.0]);
    assert!(close(num(&r["distance_m"]), 8.0));
}

#[test]
fn the_entity_scale_sizes_the_shape() {
    let mut s = flat();
    s.run(
        "entity.add",
        &json!({"position": [0, 0, 0], "scale": 3, "components": [{"type": "shape", "kind": "box", "size": [1, 1, 1]}]}),
    )
    .unwrap();
    let r = ray(&mut s, [0.0, 10.0, 0.0], [0.0, -2.0, 0.0]);
    assert!(close(num(&r["distance_m"]), 7.0), "{r}");
}

#[test]
fn a_horizontal_ray_hits_a_rotated_box_where_trigonometry_says() {
    // A 4 x 2 x 2 box turned 90 degrees spans x from -1 to 1 and z from -2 to 2.
    let mut s = flat();
    add_shape(
        &mut s,
        "Long",
        [0.0, 0.0, 0.0],
        90.0,
        json!({"kind": "box", "size": [4, 2, 2]}),
    );
    let r = ray(&mut s, [0.0, 1.0, 10.0], [0.0, 0.0, -1.0]);
    assert!(close(num(&r["distance_m"]), 8.0), "{r}");
    assert_vec(&r["normal"], [0.0, 0.0, 1.0]);
    let r = ray(&mut s, [10.0, 1.0, 0.0], [-1.0, 0.0, 0.0]);
    assert!(close(num(&r["distance_m"]), 9.0), "{r}");
    assert_vec(&r["normal"], [1.0, 0.0, 0.0]);

    // A 2 m cube turned 30 degrees: its face normal (cos 30, 0, -sin 30) meets the x axis at
    // x = 1 / cos 30.
    let mut s = flat();
    add_shape(
        &mut s,
        "Cube",
        [0.0, 0.0, 0.0],
        30.0,
        json!({"kind": "box", "size": [2, 2, 2]}),
    );
    let cos30 = 3f64.sqrt() / 2.0;
    let r = ray(&mut s, [10.0, 1.0, 0.0], [-1.0, 0.0, 0.0]);
    assert!(close(num(&r["distance_m"]), 10.0 - 1.0 / cos30), "{r}");
    assert_vec(&r["normal"], [cos30, 0.0, -0.5]);
    assert_vec(&r["point"], [1.0 / cos30, 1.0, 0.0]);
}

#[test]
fn a_ray_through_a_sphere_centre_hits_at_centre_distance_minus_radius() {
    let mut s = flat();
    add_shape(
        &mut s,
        "Ball",
        [0.0, 0.0, 0.0],
        0.0,
        json!({"kind": "sphere", "size": [2, 2, 2]}),
    );
    let r = ray(&mut s, [5.0, 1.0, 0.0], [-1.0, 0.0, 0.0]);
    assert!(close(num(&r["distance_m"]), 4.0), "{r}");
    assert_vec(&r["normal"], [1.0, 0.0, 0.0]);
    // Diagonal through the centre [0, 1, 0].
    let r = ray(&mut s, [3.0, 4.0, 0.0], [-1.0, -1.0, 0.0]);
    assert!(close(num(&r["distance_m"]), 18f64.sqrt() - 1.0), "{r}");
    let h = 0.5f64.sqrt();
    assert_vec(&r["normal"], [h, h, 0.0]);
}

#[test]
fn a_cylinder_is_hit_on_its_side_and_its_cap() {
    let mut s = flat();
    add_shape(
        &mut s,
        "Post",
        [0.0, 0.0, 0.0],
        0.0,
        json!({"kind": "cylinder", "size": [2, 4, 2]}),
    );
    let r = ray(&mut s, [5.0, 2.0, 0.0], [-1.0, 0.0, 0.0]);
    assert!(close(num(&r["distance_m"]), 4.0), "{r}");
    assert_vec(&r["normal"], [1.0, 0.0, 0.0]);
    let r = ray(&mut s, [0.5, 10.0, 0.0], [0.0, -1.0, 0.0]);
    assert!(close(num(&r["distance_m"]), 6.0), "{r}");
    assert_vec(&r["normal"], [0.0, 1.0, 0.0]);
    // Past the corner of the cap (radius 1): the ray misses the cylinder, hits the ground.
    let r = ray(&mut s, [1.2, 10.0, 0.0], [0.0, -1.0, 0.0]);
    assert_eq!(r["kind"], json!("terrain"));
}

#[test]
fn misses_and_max_m() {
    let mut s = flat();
    add_shape(
        &mut s,
        "Block",
        [0.0, 0.0, 0.0],
        0.0,
        json!({"kind": "box", "size": [2, 2, 2]}),
    );
    assert_eq!(
        ray(&mut s, [5.0, 1.0, 5.0], [1.0, 0.0, 0.0]),
        json!({"hit": false})
    );
    assert_eq!(
        ray(&mut s, [0.0, 5.0, 0.0], [0.0, 1.0, 0.0]),
        json!({"hit": false})
    );
    let r = s
        .run(
            "query.raycast",
            &json!({"origin": [0, 10, 0], "direction": [0, -1, 0], "max_m": 5}),
        )
        .unwrap();
    assert_eq!(r, json!({"hit": false}));
    let r = s
        .run(
            "query.raycast",
            &json!({"origin": [0, 10, 0], "direction": [0, -1, 0], "max_m": 8.5}),
        )
        .unwrap();
    assert_eq!(r["hit"], json!(true));
    // Without terrain nothing but shapes can be hit.
    assert_eq!(
        ray(&mut empty(), [0.0, 10.0, 0.0], [0.0, -1.0, 0.0]),
        json!({"hit": false})
    );
}

#[test]
fn the_nearest_of_several_shapes_wins_and_non_solid_ones_count() {
    let mut s = flat();
    let far = add_shape(
        &mut s,
        "Far",
        [-5.0, 0.0, 0.0],
        0.0,
        json!({"kind": "box", "size": [1, 2, 1]}),
    );
    let near = add_shape(
        &mut s,
        "Near",
        [2.0, 0.0, 0.0],
        0.0,
        json!({"kind": "sphere", "size": [2, 2, 2], "solid": false}),
    );
    let r = ray(&mut s, [10.0, 1.0, 0.0], [-1.0, 0.0, 0.0]);
    assert_eq!(r["entity"], json!(near));
    assert!(close(num(&r["distance_m"]), 7.0), "{r}");
    let r = ray(&mut s, [-10.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
    assert_eq!(r["entity"], json!(far));
    assert!(close(num(&r["distance_m"]), 4.5), "{r}");
}

#[test]
fn terrain_and_entities_are_ordered_by_distance() {
    // The ray from [0, 10, 0] towards [20, -10, 0] meets the flat ground at x = 10.
    let dir = [1.0, -1.0, 0.0];
    let mut s = flat();
    let r = ray(&mut s, [0.0, 10.0, 0.0], dir);
    assert_eq!(r["kind"], json!("terrain"));
    assert!(r.get("entity").is_none());
    assert_vec(&r["point"], [10.0, 0.0, 0.0]);
    assert_vec(&r["normal"], [0.0, 1.0, 0.0]);
    assert!(close(num(&r["distance_m"]), 200f64.sqrt()), "{r}");

    // A tall box beyond that point is not reached...
    add_shape(
        &mut s,
        "Behind",
        [15.0, -10.0, 0.0],
        0.0,
        json!({"kind": "box", "size": [2, 20, 2]}),
    );
    assert_eq!(ray(&mut s, [0.0, 10.0, 0.0], dir)["kind"], json!("terrain"));
    // ...a box before it is.
    let id = add_shape(
        &mut s,
        "Before",
        [5.0, 0.0, 0.0],
        0.0,
        json!({"kind": "box", "size": [2, 10, 2]}),
    );
    let r = ray(&mut s, [0.0, 10.0, 0.0], dir);
    assert_eq!(
        (r["kind"].clone(), r["entity"].clone()),
        (json!("entity"), json!(id))
    );
    assert!(close(num(&r["distance_m"]), 32f64.sqrt()), "{r}");
    assert_vec(&r["normal"], [-1.0, 0.0, 0.0]);
}

#[test]
fn a_ray_from_inside_a_shape_hits_at_distance_zero() {
    let mut s = flat();
    add_shape(
        &mut s,
        "Room",
        [0.0, 0.0, 0.0],
        0.0,
        json!({"kind": "box", "size": [4, 4, 4]}),
    );
    let r = ray(&mut s, [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
    assert_eq!(num(&r["distance_m"]), 0.0);
    assert_vec(&r["point"], [0.0, 1.0, 0.0]);
    assert_vec(&r["normal"], [-1.0, 0.0, 0.0]);
}

#[test]
fn raycast_on_the_noise_terrain_agrees_with_the_height_field() {
    let mut s = example();
    let r = ray(&mut s, [20.0, 50.0, 20.0], [0.0, -1.0, 0.0]);
    let field = s.current_heights().unwrap();
    let h = field.height_at(20.0, 20.0).unwrap();
    let n = field.normal_at(20.0, 20.0).unwrap();
    assert_eq!(r["kind"], json!("terrain"));
    assert!((num(&r["distance_m"]) - (50.0 - h)).abs() < 1e-6, "{r}");
    assert_vec(&r["normal"], n);
}

#[test]
fn raycast_refuses_bad_parameters_naming_them() {
    let mut s = flat();
    for d in [json!([0, 0, 0]), json!([0.0, -0.0, 0.0])] {
        refuse(
            &mut s,
            "query.raycast",
            json!({"origin": [0, 1, 0], "direction": d}),
            &["`direction`"],
        );
    }
    refuse(
        &mut s,
        "query.raycast",
        json!({"origin": [0, 1, 0], "direction": [1e300, 1e300, 1e300], "max_m": 0}),
        &["`max_m`", "0.01"],
    );
    refuse(
        &mut s,
        "query.raycast",
        json!({"origin": [0, 1, 0], "direction": [0, -1, 0], "max_m": 100001}),
        &["`max_m`", "100000"],
    );
    refuse(
        &mut s,
        "query.raycast",
        json!({"direction": [0, -1, 0]}),
        &["`origin`"],
    );
    // A huge but finite direction is fine.
    let r = ray(&mut s, [0.0, 1.0, 0.0], [0.0, -1e300, 0.0]);
    assert_eq!(r["kind"], json!("terrain"));
}

// ---------------------------------------------------------------------------------------------
// query.line_of_sight

#[test]
fn line_of_sight_clear_and_blocked() {
    let mut s = flat();
    let r = s
        .run(
            "query.line_of_sight",
            &json!({"from": [-10, 1, 0], "to": [10, 1, 0]}),
        )
        .unwrap();
    assert_eq!(r["clear"], json!(true));
    assert!(close(num(&r["distance_m"]), 20.0));
    assert!(r.get("blocked_by").is_none());

    // A point on the ground can be seen: the ground it lies on does not block it.
    let r = s
        .run(
            "query.line_of_sight",
            &json!({"from": [0, 5, 0], "to": [10, 0, 10]}),
        )
        .unwrap();
    assert_eq!(r["clear"], json!(true), "{r}");

    let id = add_shape(
        &mut s,
        "Wall",
        [0.0, 0.0, 0.0],
        0.0,
        json!({"kind": "box", "size": [2, 3, 6]}),
    );
    let r = s
        .run(
            "query.line_of_sight",
            &json!({"from": [-10, 1, 0], "to": [10, 1, 0]}),
        )
        .unwrap();
    assert_eq!(r["clear"], json!(false));
    let b = &r["blocked_by"];
    assert_eq!(b["kind"], json!("entity"));
    assert_eq!(b["entity"], json!(id));
    assert_eq!(b["name"], json!("Wall"));
    assert_vec(&b["point"], [-1.0, 1.0, 0.0]);
    assert!(close(num(&b["distance_m"]), 9.0));

    // Over the wall: clear. Under the ground: blocked by the terrain.
    let r = s
        .run(
            "query.line_of_sight",
            &json!({"from": [-10, 5, 0], "to": [10, 5, 0]}),
        )
        .unwrap();
    assert_eq!(r["clear"], json!(true));
    let r = s
        .run(
            "query.line_of_sight",
            &json!({"from": [-10, 5, 20], "to": [10, -5, 20]}),
        )
        .unwrap();
    assert_eq!(r["blocked_by"]["kind"], json!("terrain"));
    assert!(close(
        num(&r["blocked_by"]["distance_m"]),
        num(&r["distance_m"]) / 2.0
    ));

    // From a point to itself: clear, distance 0.
    let r = s
        .run(
            "query.line_of_sight",
            &json!({"from": [3, 1, 3], "to": [3, 1, 3]}),
        )
        .unwrap();
    assert_eq!(r, json!({"clear": true, "distance_m": 0.0}));
}

// ---------------------------------------------------------------------------------------------
// query.ground_height

#[test]
fn ground_height_on_flat_and_noise_terrain_and_without_ground() {
    let mut s = flat();
    let r = s
        .run("query.ground_height", &json!({"x": 3, "z": -4}))
        .unwrap();
    assert_eq!(r["height_m"], json!(0.0));
    assert_vec(&r["normal"], [0.0, 1.0, 0.0]);
    assert_eq!(num(&r["slope_deg"]), 0.0);
    assert_eq!((r["x"].clone(), r["z"].clone()), (json!(3.0), json!(-4.0)));

    let none = json!({"x": 500.0, "z": 0.0, "height_m": null, "normal": null, "slope_deg": null});
    assert_eq!(
        s.run("query.ground_height", &json!({"x": 500, "z": 0}))
            .unwrap(),
        none
    );
    assert_eq!(
        empty()
            .run("query.ground_height", &json!({"x": 500, "z": 0}))
            .unwrap(),
        none
    );

    let mut s = example();
    let r = s
        .run("query.ground_height", &json!({"x": 13.3, "z": -7.1}))
        .unwrap();
    let field = s.current_heights().unwrap();
    let n = field.normal_at(13.3, -7.1).unwrap();
    assert_eq!(num(&r["height_m"]), field.height_at(13.3, -7.1).unwrap());
    assert_vec(&r["normal"], n);
    let slope = rad_to_deg(n[1].pacos());
    assert!((num(&r["slope_deg"]) - slope).abs() < 1e-9, "{r}");
    assert!(num(&r["slope_deg"]) > 0.0);
    refuse(&mut s, "query.ground_height", json!({"x": 1}), &["`z`"]);
}

// ---------------------------------------------------------------------------------------------
// entity.get

#[test]
fn entity_get_while_editing() {
    let mut s = example();
    let r = s.run("entity.get", &json!({"id": "crate"})).unwrap();
    assert_eq!(r["id"], json!(1));
    assert_eq!(r["name"], json!("Crate"));
    assert_eq!(r["position"], json!([-6.0, 0.0, 4.0]));
    assert_eq!(r["rotation_y_deg"], json!(0.0));
    assert_eq!(r["scale"], json!(1.0));
    assert_eq!(r["components"][0]["type"], json!("shape"));
    let g = s.current_heights().unwrap().height_at(-6.0, 4.0).unwrap();
    assert_eq!(num(&r["ground_height_m"]), g);
    assert!(close(num(&r["height_above_ground_m"]), -g));
    assert_eq!(r["body"], json!(null));
    assert_eq!(r["input"], json!(null));

    // Without ground beneath: nulls.
    s.run("entity.set", &json!({"id": 1, "position": [900, 0, 0]}))
        .unwrap();
    let r = s.run("entity.get", &json!({"id": 1})).unwrap();
    assert_eq!(r["ground_height_m"], json!(null));
    assert_eq!(r["height_above_ground_m"], json!(null));

    refuse(&mut s, "entity.get", json!({"id": 99}), &["99"]);
    refuse(&mut s, "entity.get", json!({}), &["`id`"]);
}

#[test]
fn entity_get_while_playing_shows_body_and_input() {
    let mut s = playing();
    let r = s.run("entity.get", &json!({"id": "Scout"})).unwrap();
    assert_eq!(r["body"]["grounded"], json!(false));
    assert_eq!(r["body"]["velocity"], json!([0.0, 0.0, 0.0]));
    // No input record yet.
    assert_eq!(r["input"], json!(null));
    s.run("sim.input", &json!({"entity": "Scout", "forward": 1}))
        .unwrap();
    s.run("sim.step", &json!({"ticks": 30})).unwrap();
    let r = s.run("entity.get", &json!({"id": "Scout"})).unwrap();
    assert_eq!(r["body"]["grounded"], json!(true), "{r}");
    assert_eq!(r["input"]["forward"], json!(1.0));
    let v = &r["body"]["velocity"];
    assert!(num(&v[2]) < -1.0, "walking north: {r}");
    // The running world, not the document.
    let z = num(&r["position"][2]);
    assert!(z < -0.5, "{r}");
    assert!(close(num(&r["height_above_ground_m"]), 0.0), "{r}");
    // A non-character has neither.
    let r = s.run("entity.get", &json!({"id": "Crate"})).unwrap();
    assert_eq!(
        (r["body"].clone(), r["input"].clone()),
        (json!(null), json!(null))
    );
}

// ---------------------------------------------------------------------------------------------
// entity.find

/// Red box (1) at the origin, Blue box (2) at x 10, Oak (3) at (3, 4), red tree (4) at x -1,
/// Scout (5), a character, at (0, -2).
fn town() -> Session {
    let mut s = flat();
    let shape = json!([{"type": "shape", "kind": "box"}]);
    let plant = json!([{"type": "plant", "species": "quercus-robur"}]);
    for (name, at, components) in [
        ("Red box", [0, 0], &shape),
        ("Blue box", [10, 0], &shape),
        ("Oak", [3, 4], &plant),
        ("red tree", [-1, 0], &plant),
        ("Scout", [0, -2], &json!([{"type": "character"}])),
    ] {
        s.run(
            "entity.add",
            &json!({"name": name, "at": at, "components": components}),
        )
        .unwrap();
    }
    s
}

fn ids(r: &Value) -> Vec<u64> {
    r["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_u64().unwrap())
        .collect()
}

#[test]
fn entity_find_by_each_filter_and_combined() {
    let mut s = town();
    let all = s.run("entity.find", &json!({})).unwrap();
    assert_eq!(all["total"], json!(5));
    assert_eq!(ids(&all), [1, 2, 3, 4, 5]);
    assert_eq!(all["entities"][0]["name"], json!("Red box"));
    assert_eq!(all["entities"][0]["components"], json!(["shape"]));
    assert_eq!(all["entities"][0]["position"], json!([0.0, 0.0, 0.0]));
    assert!(all["entities"][0].get("distance_m").is_none());

    let r = s.run("entity.find", &json!({"name": "RED"})).unwrap();
    assert_eq!(ids(&r), [1, 4]);
    let r = s
        .run("entity.find", &json!({"component": "plant"}))
        .unwrap();
    assert_eq!(ids(&r), [3, 4]);
    let r = s
        .run("entity.find", &json!({"near": [0, 0], "radius_m": 5}))
        .unwrap();
    // By distance: Red box 0, red tree 1, Scout 2, Oak 5 (the radius is inclusive).
    assert_eq!(ids(&r), [1, 4, 5, 3]);
    let d: Vec<f64> = r["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| num(&e["distance_m"]))
        .collect();
    assert_eq!(d, [0.0, 1.0, 2.0, 5.0]);
    let r = s
        .run(
            "entity.find",
            &json!({"name": "red", "component": "plant", "near": [0, 0], "radius_m": 2}),
        )
        .unwrap();
    assert_eq!(ids(&r), [4]);
    assert_eq!(r["total"], json!(1));
    let r = s.run("entity.find", &json!({"name": "nobody"})).unwrap();
    assert_eq!(r, json!({"total": 0, "entities": []}));
}

#[test]
fn entity_find_orders_by_distance_then_id_and_limits() {
    let mut s = town();
    // Red box and Blue box are both 5 m from (5, 0): the lower id first.
    let r = s.run("entity.find", &json!({"near": [5, 0]})).unwrap();
    assert_eq!(ids(&r)[..2], [3, 1]);
    let r = s
        .run(
            "entity.find",
            &json!({"near": [5, 0], "component": "shape"}),
        )
        .unwrap();
    assert_eq!(ids(&r), [1, 2]);
    let r = s.run("entity.find", &json!({"limit": 2})).unwrap();
    assert_eq!(r["total"], json!(5));
    assert_eq!(ids(&r), [1, 2]);
    refuse(&mut s, "entity.find", json!({"radius_m": 3}), &["`near`"]);
    refuse(
        &mut s,
        "entity.find",
        json!({"limit": 0}),
        &["`limit`", "1000"],
    );
    refuse(&mut s, "entity.find", json!({"limit": 1001}), &["`limit`"]);
    refuse(
        &mut s,
        "entity.find",
        json!({"near": [0, 0], "radius_m": -1}),
        &["`radius_m`"],
    );
    refuse(
        &mut s,
        "entity.find",
        json!({"component": "tree"}),
        &["`component`"],
    );
}

#[test]
fn entity_find_reads_the_running_world() {
    let mut s = playing();
    s.run("entity.add", &json!({"name": "Live", "at": [1, 1]}))
        .unwrap();
    let r = s.run("entity.find", &json!({"name": "live"})).unwrap();
    assert_eq!(r["total"], json!(1));
}

// ---------------------------------------------------------------------------------------------
// species.list and component.list

#[test]
fn species_list_filters() {
    let mut s = example();
    let all = s.run("species.list", &json!({"limit": 1000})).unwrap();
    let total = all["total"].as_u64().unwrap();
    assert!(total > 10);
    assert_eq!(all["species"].as_array().unwrap().len() as u64, total);
    let oak = all["species"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == json!("quercus-robur"))
        .unwrap();
    for key in [
        "common_name",
        "botanical_name",
        "category",
        "mature_height_m",
        "mature_width_m",
        "evergreen",
    ] {
        assert!(oak.get(key).is_some(), "{key}");
    }

    for filter in ["QUERCUS-rob", "pedunculate OAK", "quercus robur"] {
        let r = s.run("species.list", &json!({"filter": filter})).unwrap();
        assert!(
            r["species"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["id"] == json!("quercus-robur")),
            "{filter}: {r}"
        );
    }
    let r = s.run("species.list", &json!({"category": "tree"})).unwrap();
    assert!(r["total"].as_u64().unwrap() >= 1);
    assert!(
        r["species"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["category"] == json!("tree"))
    );
    let r = s.run("species.list", &json!({"limit": 1})).unwrap();
    assert_eq!(r["total"].as_u64().unwrap(), total);
    assert_eq!(r["species"].as_array().unwrap().len(), 1);
    let r = s
        .run("species.list", &json!({"filter": "no such plant"}))
        .unwrap();
    assert_eq!(r, json!({"total": 0, "species": []}));
    refuse(
        &mut s,
        "species.list",
        json!({"category": "cactus"}),
        &["`category`"],
    );
    refuse(&mut s, "species.list", json!({"limit": 0}), &["`limit`"]);
}

#[test]
fn component_list_describes_every_component_and_its_examples_parse() {
    let mut s = example();
    let r = s.run("component.list", &json!({})).unwrap();
    let list = r["components"].as_array().unwrap();
    let types: Vec<&str> = list.iter().map(|c| c["type"].as_str().unwrap()).collect();
    assert_eq!(
        types,
        [
            "shape",
            "plant",
            "character",
            "zone",
            "interactable",
            "light",
            "tags",
            "launcher",
            "health",
            "behaviour"
        ]
    );
    for c in list {
        assert!(!c["summary"].as_str().unwrap().is_empty());
        for f in c["fields"].as_array().unwrap() {
            for key in ["name", "type", "unit", "default", "doc"] {
                assert!(f.get(key).is_some(), "{c}: {key}");
            }
            assert!(!f["doc"].as_str().unwrap().is_empty());
        }
        // The example is a component entity.add accepts.
        let example = c["example"].clone();
        assert_eq!(example["type"], c["type"]);
        s.run(
            "entity.add",
            &json!({"position": [0, 0, 0], "components": [example]}),
        )
        .unwrap_or_else(|e| panic!("{c}: {e}"));
    }
    let field = |kind: usize, name: &str| -> Value {
        list[kind]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == json!(name))
            .unwrap_or_else(|| panic!("{name}"))
            .clone()
    };
    assert_eq!(field(0, "size")["default"], json!([1.0, 1.0, 1.0]));
    assert_eq!(field(0, "solid")["default"], json!(true));
    assert_eq!(field(0, "kind")["default"], json!(null));
    assert_eq!(field(1, "growth")["default"], json!(1.0));
    assert_eq!(field(1, "seed")["default"], json!(0));
    assert_eq!(field(2, "height_m")["default"], json!(1.8));
    assert_eq!(field(2, "height_m")["unit"], json!("m"));
    assert_eq!(field(2, "max_slope_deg")["default"], json!(45.0));
}
