//! The Skirmish's sculpted terrain through `gamecraft-cli serve`: its road and village square
//! are in the committed world, and a game master blows a crater into it while it is played.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
use common::*;
use serde_json::{Value, json};
use std::path::PathBuf;

fn skirmish() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .join("examples")
        .join("skirmish")
}

fn ground(server: &mut Server, x: f64, z: f64) -> f64 {
    server.ok("query.ground_height", json!({"x": x, "z": z}))["height_m"]
        .as_f64()
        .unwrap()
}

fn position(server: &mut Server, name: &str) -> [f64; 3] {
    let e: Value = server.ok("entity.get", json!({"id": name}));
    let p = e["position"].as_array().unwrap();
    [0, 1, 2].map(|k| p[k].as_f64().unwrap())
}

#[test]
fn the_skirmish_has_a_road_a_flat_square_and_rock_on_its_slopes() {
    let dir = Temp::new("terrain-skirmish");
    let world = skirmish().join("world.gamecraft");
    let mut server = Server::start("serve", &["--file", &world.display().to_string()], &dir.0);
    let recipe = server.ok("terrain.edits", json!({}));
    let ops: Vec<&str> = recipe["edits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["op"].as_str().unwrap())
        .collect();
    assert_eq!(&ops[..4], ["flatten", "ramp", "ramp", "smooth"], "{ops:?}");
    let layers: Vec<&str> = recipe["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert_eq!(layers, ["grass", "dirt", "rock", "road"]);
    // The village square is level where the houses stand.
    let square = ground(&mut server, 4.0, -54.0);
    for (x, z) in [(-1.0, -54.0), (9.0, -54.0), (0.0, -50.0), (8.0, -58.0)] {
        assert!((ground(&mut server, x, z) - square).abs() < 0.05, "{x} {z}");
    }
    // The road is a straight slope from the north edge to the square.
    let north = ground(&mut server, 4.0, -190.0);
    let half = ground(&mut server, 4.0, -122.0);
    assert!(
        (half - (north + square) / 2.0).abs() < 0.05,
        "{north} {half} {square}"
    );
    server.finish();
}

#[test]
fn a_live_crater_takes_a_standing_guard_down_with_the_ground() {
    let dir = Temp::new("terrain-crater");
    let world = skirmish().join("world.gamecraft");
    let mut server = Server::start("serve", &["--file", &world.display().to_string()], &dir.0);
    server.ok("sim.start", json!({}));
    server.ok("sim.step", json!({"ticks": 30}));
    let [x, y, z] = position(&mut server, "Village Guard");
    assert!((y - ground(&mut server, x, z)).abs() < 1e-6);
    let crater = server.ok(
        "terrain.sculpt",
        json!({"op": "lower", "center": [x, z], "radius_m": 6, "strength_m": 2.5}),
    );
    assert!(crater["tiles_rebuilt"].as_u64().unwrap() >= 1, "{crater}");
    let floor = ground(&mut server, x, z);
    assert!((floor - (y - 2.5)).abs() < 1e-3, "{floor} {y}");
    server.ok("sim.step", json!({"ticks": 90}));
    let [gx, gy, gz] = position(&mut server, "Village Guard");
    assert!((gy - ground(&mut server, gx, gz)).abs() < 1e-6, "{gy}");
    assert!(gy < y - 2.0, "{gy} {y}");
    server.finish();
}
