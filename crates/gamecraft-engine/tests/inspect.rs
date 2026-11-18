//! `world.inspect`, the plant library and the example world.

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
use gamecraft_engine::plants::{nearest_species, species_exists, species_list};
use gamecraft_engine::{EXAMPLE_SPECIES, Session, example_world};
use gamecraft_world::World;
use serde_json::json;

#[test]
fn inspect_describes_the_document_while_editing() {
    let mut s = example();
    s.run("world.set", &json!({"name": "Dale"})).unwrap();
    let r = s.run("world.inspect", &json!({})).unwrap();
    assert_eq!(r["name"], json!("Dale"));
    assert_eq!(r["seed"], json!(7));
    assert_eq!(r["revision"], json!(1));
    assert_eq!(r["playing"], json!(false));
    assert_eq!(r["tick"], json!(null));
    let t = &r["terrain"];
    assert_eq!(
        (t["size_m"].clone(), t["cell_m"].clone()),
        (json!(128.0), json!(2.0))
    );
    assert_eq!(t["source"]["kind"], json!("noise"));
    assert_eq!(
        (t["cols"].clone(), t["rows"].clone()),
        (json!(65), json!(65))
    );
    assert!(t["min_height_m"].as_f64().unwrap() <= t["max_height_m"].as_f64().unwrap());
    assert_eq!(r["environment"]["day_of_year"], json!(172));
    let sun = &r["sun"];
    assert_eq!(sun["direction"].as_array().unwrap().len(), 3);
    assert_eq!(sun["above_horizon"], json!(true));
    assert!(sun["azimuth_deg"].is_number() && sun["elevation_deg"].is_number());
    assert_eq!(r["entity_count"], json!(3));
    let e = r["entities"].as_array().unwrap();
    assert_eq!(e.len(), 3);
    assert_eq!(e[0]["name"], json!("Crate"));
    assert_eq!(e[0]["components"][0]["type"], json!("shape"));
    for key in [
        "id",
        "name",
        "position",
        "rotation_y_deg",
        "scale",
        "components",
    ] {
        assert!(e[2].get(key).is_some(), "{key}");
    }
    assert_eq!(r["truncated"], json!(false));
    assert_eq!(r["history"], json!({"can_undo": true, "can_redo": false}));
    assert_eq!(s.revision(), 1, "inspecting changes nothing");
}

#[test]
fn inspect_pages_through_the_entities() {
    let mut s = example();
    let r = s.run("world.inspect", &json!({"limit": 2})).unwrap();
    let ids: Vec<_> = r["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].clone())
        .collect();
    assert_eq!(
        (ids, r["truncated"].clone()),
        (vec![json!(1), json!(2)], json!(true))
    );
    let r = s
        .run("world.inspect", &json!({"limit": 2, "offset": 2}))
        .unwrap();
    assert_eq!(r["entities"][0]["id"], json!(3));
    assert_eq!(r["truncated"], json!(false));
    let r = s.run("world.inspect", &json!({"offset": 99})).unwrap();
    assert_eq!(r["entities"], json!([]));
    let r = s.run("world.inspect", &json!({"entities": false})).unwrap();
    assert!(r.get("entities").is_none());
    assert_eq!(r["entity_count"], json!(3));
    refuse(
        &mut s,
        "world.inspect",
        json!({"limit": 0}),
        &["`limit`", "1 to 1000"],
    );
    refuse(
        &mut s,
        "world.inspect",
        json!({"limit": 1001}),
        &["`limit`"],
    );
    refuse(
        &mut s,
        "world.inspect",
        json!({"offset": -1}),
        &["`offset`"],
    );
}

#[test]
fn inspect_describes_the_running_world_while_playing() {
    let mut s = playing();
    s.run("entity.remove", &json!({"id": "Oak"})).unwrap();
    s.run("sim.step", &json!({"ticks": 4})).unwrap();
    let r = s.run("world.inspect", &json!({})).unwrap();
    assert_eq!(
        (r["playing"].clone(), r["tick"].clone()),
        (json!(true), json!(4))
    );
    assert_eq!(r["entity_count"], json!(2));
    assert_eq!(s.document().entity_count(), 3);
}

#[test]
fn inspect_without_terrain() {
    let mut s = Session::new(World::new("Empty", 0));
    let r = s.run("world.inspect", &json!({})).unwrap();
    assert_eq!(r["terrain"], json!(null));
    assert_eq!(r["entities"], json!([]));
}

#[test]
fn the_plant_library() {
    let list = species_list();
    assert!(list.len() > 10);
    let oak = list.iter().find(|s| s.id == EXAMPLE_SPECIES).unwrap();
    assert_eq!(oak.category, "tree");
    assert!(oak.mature_height_m > 5.0 && oak.mature_width_m > 1.0 && !oak.evergreen);
    assert!(!oak.common_name.is_empty() && !oak.botanical_name.is_empty());
    assert!(list.iter().all(|s| {
        s.category
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '_')
    }));
    assert!(species_exists("quercus-robur") && !species_exists("Quercus-Robur"));
    assert_eq!(
        nearest_species("quercus-robr").as_deref(),
        Some("quercus-robur")
    );
    assert!(nearest_species(&"z".repeat(10_000)).is_some());
}

#[test]
fn the_example_world() {
    let w = example_world();
    w.validate().unwrap();
    let t = w.terrain.as_ref().unwrap();
    assert_eq!((t.size_m, t.cell_m), (128.0, 2.0));
    let names: Vec<&str> = w.entities().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Crate", "Oak", "Scout"]);
    let field = t.build().unwrap();
    let scout = w.entities().nth(2).unwrap();
    assert!(scout.character().is_some());
    assert_eq!(
        scout.position[1],
        field
            .height_at(scout.position[0], scout.position[2])
            .unwrap()
    );
    assert!(w.entities().next().unwrap().shape().unwrap().solid);
    assert!(species_exists(
        &w.entities().nth(1).unwrap().plant().unwrap().species
    ));
}
