//! The road commands: `road.add` and `road.set`, and roads in `query.raycast` and
//! `query.line_of_sight`.

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
use gamecraft_world::{EntityId, Road, RoadEnds};
use serde_json::json;

fn road(s: &Session, id: u64) -> Road {
    s.current_world()
        .entity(EntityId(id))
        .unwrap()
        .road()
        .unwrap()
        .clone()
}

fn ground(s: &Session, x: f64, z: f64) -> f64 {
    s.current_heights().unwrap().height_at(x, z).unwrap()
}

#[test]
fn a_road_is_added_at_its_first_point_with_its_shape() {
    let mut s = example();
    let r = s
        .run(
            "road.add",
            &json!({"name": "High Street", "points": [[-30, 20], [0, 18], [24, 36]], "width_m": 7, "ends": "flat"}),
        )
        .unwrap();
    assert_eq!(r["id"], json!(4));
    assert_eq!(r["tier"], json!(0));
    let length = (30.0f64 * 30.0 + 4.0).sqrt() + (24.0f64 * 24.0 + 18.0 * 18.0).sqrt();
    assert!(
        (r["length_m"].as_f64().unwrap() - length).abs() < 1e-9,
        "{r}"
    );
    let e = s.document().entity(EntityId(4)).unwrap();
    assert_eq!(e.name, "High Street");
    assert_eq!(e.position, [-30.0, ground(&s, -30.0, 20.0), 20.0]);
    let rd = e.road().unwrap();
    assert_eq!(rd.points, vec![[0.0, 0.0], [30.0, -2.0], [54.0, 16.0]]);
    assert_eq!(
        (rd.width_m, rd.ends, rd.lift_m),
        (7.0, RoadEnds::Flat, 0.03)
    );
    // One undo step.
    s.undo().unwrap();
    assert!(s.document().entity(EntityId(4)).is_none());
}

#[test]
fn crossing_roads_take_different_tiers_and_others_the_lowest() {
    let mut s = example();
    let add = |s: &mut Session, points: serde_json::Value| {
        s.run("road.add", &json!({"points": points})).unwrap()["tier"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(add(&mut s, json!([[-50, 0], [50, 0]])), 0);
    assert_eq!(add(&mut s, json!([[0, -50], [0, 50]])), 1);
    // Crosses both: the third tier.
    assert_eq!(add(&mut s, json!([[-40, -40], [40, 40]])), 2);
    // Meets none of them.
    assert_eq!(add(&mut s, json!([[-55, 30], [-30, 55]])), 0);
    // An explicit tier is kept, even where it is taken.
    let r = s
        .run(
            "road.add",
            &json!({"points": [[-50, 5], [50, 5]], "tier": 0}),
        )
        .unwrap();
    assert_eq!(r["tier"], json!(0));
    // retier_all sorts it out again, in id order.
    let r = s
        .run(
            "road.set",
            &json!({"id": 8, "lift_m": 0.04, "retier_all": true}),
        )
        .unwrap();
    assert_eq!(r["changed_tiers"], json!(1));
    assert_eq!(road(&s, 8).tier, 3);
}

#[test]
fn a_ray_onto_a_road_names_the_road() {
    let mut s = example();
    s.run(
        "road.add",
        &json!({"name": "Lane", "points": [[-40, -30], [40, -30]], "width_m": 5}),
    )
    .unwrap();
    let offset = road(&s, 4).surface_offset_m();
    let r = s
        .run(
            "query.raycast",
            &json!({"origin": [10, 80, -30.5], "direction": [0, -1, 0]}),
        )
        .unwrap();
    assert_eq!(r["kind"], json!("entity"));
    assert_eq!(r["entity"], json!(4));
    assert_eq!(r["name"], json!("Lane"));
    let y = r["point"][1].as_f64().unwrap();
    assert!((y - ground(&s, 10.0, -30.5) - offset).abs() < 1e-9, "{r}");
    let beside = s
        .run(
            "query.raycast",
            &json!({"origin": [10, 80, -36], "direction": [0, -1, 0]}),
        )
        .unwrap();
    assert_eq!(beside["kind"], json!("terrain"));
    // The ground under the asphalt is hidden by it; the asphalt itself can be seen.
    let g = ground(&s, 10.0, -30.0);
    let eye = [10.0, g + 30.0, 0.0];
    let r = s
        .run(
            "query.line_of_sight",
            &json!({"from": eye, "to": [10, g, -30]}),
        )
        .unwrap();
    assert_eq!(r["clear"], json!(false));
    assert_eq!(r["blocked_by"]["entity"], json!(4));
    let r = s
        .run(
            "query.line_of_sight",
            &json!({"from": eye, "to": [10, g + offset, -30]}),
        )
        .unwrap();
    assert_eq!(r["clear"], json!(true), "{r}");
}

#[test]
fn road_set_changes_only_what_it_is_given_and_keeps_the_entity_in_place() {
    let mut s = example();
    s.run("road.add", &json!({"points": [[0, 0], [20, 0]]}))
        .unwrap();
    s.run(
        "entity.set",
        &json!({"id": 4, "rotation_y_deg": 90, "scale": 2}),
    )
    .unwrap();
    let before = s.document().entity(EntityId(4)).unwrap().clone();
    let r = s
        .run(
            "road.set",
            &json!({"id": 4, "points": [[0, 0], [0, -30], [10, -40]], "widths_m": [4, 6, 8]}),
        )
        .unwrap();
    assert_eq!(r["tier"], json!(0));
    let e = s.document().entity(EntityId(4)).unwrap();
    assert_eq!(
        (e.position, e.rotation_y_deg, e.scale),
        (before.position, 90.0, gamecraft_world::Scale::uniform(2.0))
    );
    let line = e.road().unwrap().centre_line(e);
    for (got, want) in line.iter().zip([[0.0, 0.0], [0.0, -30.0], [10.0, -40.0]]) {
        assert!(
            (got[0] - want[0]).abs() < 1e-9 && (got[1] - want[1]).abs() < 1e-9,
            "{line:?}"
        );
    }
    assert_eq!(e.road().unwrap().widths_m, vec![4.0, 6.0, 8.0]);
    assert_eq!(e.road().unwrap().width_m, 6.0);
    s.run(
        "road.set",
        &json!({"id": 4, "widths_m": [], "color": [0.3, 0.25, 0.2]}),
    )
    .unwrap();
    let rd = road(&s, 4);
    assert!(rd.widths_m.is_empty());
    assert_eq!(rd.color, [0.3, 0.25, 0.2]);
    // An entity without a road gets one from its points.
    s.run(
        "road.set",
        &json!({"id": "Crate", "points": [[-6, 4], [12, 4]]}),
    )
    .unwrap();
    let crate_ = s.document().entity(EntityId(1)).unwrap();
    assert!(crate_.road().is_some() && crate_.shape().is_some());
}

#[test]
fn bad_road_parameters_are_refused_naming_the_parameter() {
    let mut s = example();
    refuse(&mut s, "road.add", json!({}), &["points"]);
    refuse(
        &mut s,
        "road.add",
        json!({"points": [[0, 0]]}),
        &["`points`", "2 to 4096"],
    );
    refuse(
        &mut s,
        "road.add",
        json!({"points": [[0, 0], ["a", 1]]}),
        &["`points[1]`"],
    );
    refuse(
        &mut s,
        "road.add",
        json!({"points": [[0, 0], [1e7, 1]]}),
        &["`points[1]`"],
    );
    refuse(
        &mut s,
        "road.add",
        json!({"points": [[0, 0], [5, 5], [5, 5.001]]}),
        &["`points[2]`", "0.01 m apart"],
    );
    let line = json!([[0, 0], [10, 0]]);
    for (extra, needle) in [
        (json!({"width_m": 0}), "width_m"),
        (json!({"width_m": 500}), "width_m"),
        (json!({"widths_m": [5]}), "widths_m"),
        (json!({"widths_m": [5, "wide"]}), "`widths_m[1]`"),
        (json!({"lift_m": 0}), "lift_m"),
        (json!({"tier": 8}), "`tier`"),
        (json!({"tier": "top"}), "`tier`"),
        (json!({"tier": -1}), "`tier`"),
        (json!({"color": [2, 0, 0]}), "color"),
        (json!({"ends": "square"}), "ends"),
        (json!({"retier_all": "yes"}), "retier_all"),
    ] {
        let mut params = extra.clone();
        params["points"] = line.clone();
        refuse(&mut s, "road.add", params, &[needle]);
    }
    refuse(
        &mut s,
        "road.set",
        json!({"id": "Oak", "width_m": 3}),
        &["has no road", "points"],
    );
    refuse(&mut s, "road.set", json!({"id": 99, "width_m": 3}), &["99"]);
}

#[test]
fn a_road_added_while_playing_is_in_the_running_world_and_seen_by_rays() {
    let mut s = playing();
    s.run("road.add", &json!({"points": [[-20, 12], [20, 12]]}))
        .unwrap();
    assert!(
        s.document().entity(EntityId(4)).is_none(),
        "the document waits"
    );
    let r = s
        .run(
            "query.raycast",
            &json!({"origin": [0, 80, 12], "direction": [0, -1, 0]}),
        )
        .unwrap();
    assert_eq!(r["entity"], json!(4));
}

#[test]
fn component_list_describes_the_road() {
    let s = example();
    let r = s.clone().run("component.list", &json!({})).unwrap();
    let road = r["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == json!("road"))
        .unwrap()
        .clone();
    let names: Vec<&str> = road["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "points", "width_m", "widths_m", "color", "lift_m", "tier", "ends"
        ]
    );
}
