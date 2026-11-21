//! Tree trunks: derived collision columns of the plants. They block a walking character, stop a
//! ray (`query.raycast`), are listed by `query.trees`, follow the plants when they change, and
//! leave small plants alone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use gamecraft_engine::Session;
use gamecraft_world::{Character, Terrain, TerrainSource, World};
use serde_json::{Value, json};

fn flat() -> Session {
    let mut w = World::new("Grove", 1);
    w.terrain = Some(Terrain {
        size_m: 200.0,
        cell_m: 2.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    Session::new(w)
}

fn add_plant(s: &mut Session, species: &str, at: [f64; 3], seed: u64) -> u64 {
    s.run(
        "entity.add",
        &json!({"name": species, "position": at,
                "components": [{"type": "plant", "species": species, "seed": seed}]}),
    )
    .unwrap()["id"]
        .as_u64()
        .unwrap()
}

fn add_hero(s: &mut Session, at: [f64; 3]) {
    s.run(
        "entity.add",
        &json!({"name": "Hero", "position": at, "components": [{"type": "character"}]}),
    )
    .unwrap();
}

fn species_of(s: &mut Session, category: &str) -> String {
    let r = s
        .run("species.list", &json!({"category": category}))
        .unwrap();
    r["species"][0]["id"].as_str().unwrap().to_string()
}

fn trees(s: &mut Session, at: [f64; 3], radius: f64) -> Value {
    s.run("query.trees", &json!({"position": at, "radius_m": radius}))
        .unwrap()
}

#[test]
fn a_tree_has_a_trunk_and_small_plants_have_none() {
    let mut s = flat();
    let oak = add_plant(&mut s, "quercus-robur", [0.0, 0.0, -6.0], 1);
    let grass = species_of(&mut s, "grass");
    let shrub = species_of(&mut s, "shrub");
    add_plant(&mut s, &grass, [2.0, 0.0, -6.0], 1);
    add_plant(&mut s, &shrub, [-2.0, 0.0, -6.0], 1);
    let r = trees(&mut s, [0.0, 0.0, -6.0], 30.0);
    assert_eq!(r["count"], json!(1), "{r}");
    let t = &r["trees"][0];
    assert_eq!(t["entity"], json!(oak));
    assert_eq!(t["species"], json!("quercus-robur"));
    let (radius, height) = (
        t["radius_m"].as_f64().unwrap(),
        t["height_m"].as_f64().unwrap(),
    );
    assert!((0.08..=2.0).contains(&radius), "{radius}");
    assert!((2.0..=8.0).contains(&height), "{height}");
    assert_eq!(t["distance_m"], json!(0.0), "the point is inside the trunk");
}

#[test]
fn the_scale_of_the_entity_sizes_the_trunk() {
    let mut s = flat();
    let a = add_plant(&mut s, "quercus-robur", [0.0, 0.0, -6.0], 1);
    let before = trees(&mut s, [0.0, 0.0, -6.0], 5.0)["trees"][0]["radius_m"]
        .as_f64()
        .unwrap();
    s.run("entity.set", &json!({"id": a, "scale": 2.0}))
        .unwrap();
    let after = trees(&mut s, [0.0, 0.0, -6.0], 5.0)["trees"][0]["radius_m"]
        .as_f64()
        .unwrap();
    assert!((after - 2.0 * before).abs() < 1e-9, "{before} {after}");
}

#[test]
fn a_ray_hits_a_trunk_and_reports_what_it_is() {
    let mut s = flat();
    let oak = add_plant(&mut s, "quercus-robur", [0.0, 0.0, -6.0], 1);
    let radius = trees(&mut s, [0.0, 0.0, -6.0], 5.0)["trees"][0]["radius_m"]
        .as_f64()
        .unwrap();
    let r = s
        .run(
            "query.raycast",
            &json!({"origin": [0, 1.5, 0], "direction": [0, 0, -1]}),
        )
        .unwrap();
    assert_eq!(r["hit"], json!(true), "{r}");
    assert_eq!(r["kind"], json!("trunk"));
    assert_eq!(r["entity"], json!(oak));
    assert!((r["distance_m"].as_f64().unwrap() - (6.0 - radius)).abs() < 1e-9);
    // Over the crown of the trunk column nothing is hit.
    let over = s
        .run(
            "query.raycast",
            &json!({"origin": [0, 30, 0], "direction": [0, 0, -1]}),
        )
        .unwrap();
    assert_eq!(over["hit"], json!(false), "{over}");
    let sight = s
        .run(
            "query.line_of_sight",
            &json!({"from": [0, 1.5, 0], "to": [0, 1.5, -12]}),
        )
        .unwrap();
    assert_eq!(sight["clear"], json!(false));
    assert_eq!(sight["blocked_by"]["kind"], json!("trunk"));
}

#[test]
fn trunks_follow_the_plants_when_they_change() {
    let mut s = flat();
    let a = add_plant(&mut s, "quercus-robur", [0.0, 0.0, -6.0], 1);
    assert_eq!(trees(&mut s, [0.0, 0.0, 0.0], 50.0)["count"], json!(1));
    add_plant(&mut s, "quercus-robur", [10.0, 0.0, -6.0], 2);
    assert_eq!(trees(&mut s, [0.0, 0.0, 0.0], 50.0)["count"], json!(2));
    s.run("entity.remove", &json!({"id": a})).unwrap();
    let r = trees(&mut s, [0.0, 0.0, 0.0], 50.0);
    assert_eq!(r["count"], json!(1), "{r}");
    // And a running world keeps them while it changes.
    s.run("sim.start", &Value::Null).unwrap();
    assert_eq!(trees(&mut s, [0.0, 0.0, 0.0], 50.0)["count"], json!(1));
    add_plant(&mut s, "quercus-robur", [20.0, 0.0, -6.0], 3);
    assert_eq!(trees(&mut s, [0.0, 0.0, 0.0], 50.0)["count"], json!(2));
}

#[test]
fn a_walking_character_is_stopped_by_a_trunk_in_a_running_world() {
    let mut s = flat();
    add_plant(&mut s, "quercus-robur", [0.0, 0.0, -6.0], 1);
    add_hero(&mut s, [0.0, 0.0, 0.0]);
    let radius = trees(&mut s, [0.0, 0.0, -6.0], 5.0)["trees"][0]["radius_m"]
        .as_f64()
        .unwrap();
    s.run("sim.start", &Value::Null).unwrap();
    s.run(
        "sim.input",
        &json!({"entity": "Hero", "forward": 1, "yaw_deg": 0}),
    )
    .unwrap();
    let body = Character::default().radius_m;
    for _ in 0..30 {
        s.run("sim.step", &json!({"ticks": 20})).unwrap();
        let hero = s
            .current_world()
            .entities()
            .find(|e| e.name == "Hero")
            .unwrap()
            .position;
        let d = (hero[0] * hero[0] + (hero[2] + 6.0) * (hero[2] + 6.0)).sqrt();
        assert!(d >= radius + body - 1e-6, "inside the trunk: {hero:?}");
    }
}

#[test]
fn the_same_world_runs_to_the_same_hash_with_trees() {
    let run = || {
        let mut s = flat();
        for (i, x) in [-3.0, 0.0, 3.0].into_iter().enumerate() {
            add_plant(&mut s, "quercus-robur", [x, 0.0, -6.0], i as u64 + 1);
        }
        add_hero(&mut s, [0.5, 0.0, 0.0]);
        s.run("sim.start", &Value::Null).unwrap();
        s.run(
            "sim.input",
            &json!({"entity": "Hero", "forward": 1, "yaw_deg": 0}),
        )
        .unwrap();
        s.run("sim.step", &json!({"ticks": 300})).unwrap()["hash"].clone()
    };
    assert_eq!(run(), run());
}

#[test]
fn query_trees_names_the_bad_parameter() {
    let mut s = flat();
    let e = s
        .run(
            "query.trees",
            &json!({"position": [0, 0, 0], "radius_m": 0}),
        )
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("radius_m") && e.starts_with("query.trees"),
        "{e}"
    );
    let e = s
        .run("query.trees", &json!({"position": [0, 0, 0], "limit": 0}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("limit"), "{e}");
}

#[test]
fn every_tree_conifer_and_palm_species_gets_a_sane_trunk() {
    let mut s = flat();
    let mut missing = Vec::new();
    for category in ["tree", "conifer", "palm"] {
        let list = s
            .run(
                "species.list",
                &json!({"category": category, "limit": 1000}),
            )
            .unwrap();
        for sp in list["species"].as_array().unwrap() {
            let id = sp["id"].as_str().unwrap();
            let height = sp["mature_height_m"].as_f64().unwrap();
            let mut at = flat();
            add_plant(&mut at, id, [0.0, 0.0, 0.0], 1);
            let r = trees(&mut at, [0.0, 0.0, 0.0], 5.0);
            let Some(t) = r["trees"].get(0) else {
                // Small, shrubby and columnar species may be too thin to block.
                if height >= 8.0 {
                    missing.push(id.to_string());
                }
                continue;
            };
            let (radius, h) = (
                t["radius_m"].as_f64().unwrap(),
                t["height_m"].as_f64().unwrap(),
            );
            eprintln!("{id} ({height} m): radius {radius} m, column {h} m");
            assert!(radius <= height / 4.0 + 0.2, "{id}: radius {radius} m");
        }
    }
    assert!(missing.is_empty(), "no trunk for {missing:?}");
}
