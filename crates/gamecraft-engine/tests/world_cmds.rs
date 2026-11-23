//! `world.set`, `terrain.set`, `terrain.clear`, `environment.set`.

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
use gamecraft_world::{TerrainSource, World};
use serde_json::json;

#[test]
fn world_set_changes_only_the_fields_given() {
    let mut s = example();
    assert_eq!(
        s.run("world.set", &json!({"name": "Dale"})).unwrap(),
        json!(null)
    );
    assert_eq!((s.document().name.as_str(), s.document().seed), ("Dale", 7));
    s.run("world.set", &json!({"seed": 18_446_744_073_709_551_615u64}))
        .unwrap();
    assert_eq!(s.document().seed, u64::MAX);
    assert_eq!(s.document().name, "Dale");
}

#[test]
fn world_set_refuses_bad_values() {
    let mut s = example();
    refuse(
        &mut s,
        "world.set",
        json!({"name": "n".repeat(201)}),
        &["name", "at most 200"],
    );
    refuse(&mut s, "world.set", json!({"seed": -1}), &["`seed`"]);
    refuse(
        &mut s,
        "world.set",
        json!({"seed": 1.5}),
        &["`seed` must be integer"],
    );
    refuse(
        &mut s,
        "world.set",
        json!({"name": 3}),
        &["`name` must be string"],
    );
    refuse(
        &mut s,
        "world.set",
        json!({"nam": "x"}),
        &["did you mean `name`"],
    );
}

#[test]
fn terrain_set_noise_uses_the_defaults_and_the_world_seed() {
    let mut s = Session::new(World::new("W", 99));
    let r = s.run("terrain.set", &json!({})).unwrap();
    assert_eq!(
        (r["cols"].clone(), r["rows"].clone()),
        (json!(129), json!(129))
    );
    assert!(r["min_height_m"].as_f64().unwrap() < r["max_height_m"].as_f64().unwrap());
    let t = s.document().terrain.clone().unwrap();
    assert_eq!((t.size_m, t.cell_m), (256.0, 2.0));
    assert_eq!(
        t.source,
        TerrainSource::Noise {
            seed: 99,
            relief_m: 40.0,
            feature_m: 200.0,
            octaves: 5,
            roughness: 0.5
        }
    );
}

#[test]
fn terrain_set_grid_gives_the_node_heights() {
    // A 3x3 control grid over 16 m at 2 m cells: the field's nodes at the control nodes
    // (every second field node) carry the given heights.
    let heights: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
    let mut s = Session::new(World::new("Grid", 1));
    let params = json!({
        "kind": "grid", "size_m": 16, "cell_m": 2,
        "cols": 3, "rows": 3, "heights": heights,
    });
    let r = s.run("terrain.set", &params).unwrap();
    assert_eq!(
        r,
        json!({"cols": 9, "rows": 9, "min_height_m": 1.0, "max_height_m": 9.0})
    );
    let at = |s: &mut Session, x: f64, z: f64| {
        s.run("query.ground_height", &json!({"x": x, "z": z}))
            .unwrap()["height_m"]
            .as_f64()
            .unwrap()
    };
    for row in 0..3 {
        for col in 0..3 {
            let x = -8.0 + col as f64 * 8.0;
            let z = -8.0 + row as f64 * 8.0;
            let want = heights[row * 3 + col];
            assert!(
                (at(&mut s, x, z) - want).abs() < 1e-6,
                "node ({col}, {row})"
            );
        }
    }
    // The same call again builds the same field: the source is data, not randomness.
    let r2 = s.run("terrain.set", &params).unwrap();
    assert_eq!(r, r2);
    for row in 0..3 {
        for col in 0..3 {
            let x = -8.0 + col as f64 * 8.0;
            let z = -8.0 + row as f64 * 8.0;
            let want = heights[row * 3 + col];
            assert!(
                (at(&mut s, x, z) - want).abs() < 1e-6,
                "node ({col}, {row})"
            );
        }
    }
}

#[test]
fn terrain_set_grid_refuses_bad_values() {
    let mut s = Session::new(World::new("Grid", 1));
    let good: Vec<f64> = vec![0.0; 4];
    // Parameters of another kind are refused.
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "grid", "cols": 2, "rows": 2, "heights": good, "seed": 3}),
        &["`seed`", "grid"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "noise", "cols": 2, "rows": 2, "heights": good, "seed": 3}),
        &["`cols`", "noise"],
    );
    // A huge side, a wrong length and missing heights are errors, not panics.
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "grid", "cols": 1_000_000, "rows": 2, "heights": good}),
        &["terrain.source.cols"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "grid", "cols": 2, "rows": 2, "heights": vec![0.0; 5]}),
        &["terrain.source.heights"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "grid", "cols": 2, "rows": 2}),
        &["`heights` is required"],
    );
}

#[test]
fn terrain_set_flat_and_entities_stay() {
    let mut s = example();
    let before: Vec<[f64; 3]> = s.document().entities().map(|e| e.position).collect();
    let r = s
        .run(
            "terrain.set",
            &json!({"kind": "flat", "size_m": 32, "cell_m": 1, "height_m": -2.5}),
        )
        .unwrap();
    assert_eq!(
        r,
        json!({"cols": 33, "rows": 33, "min_height_m": -2.5, "max_height_m": -2.5})
    );
    let after: Vec<[f64; 3]> = s.document().entities().map(|e| e.position).collect();
    assert_eq!(before, after);
}

#[test]
fn terrain_set_refuses_parameters_of_the_other_kind_and_bad_sizes() {
    let mut s = example();
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "flat", "relief_m": 3}),
        &["`relief_m` belongs to kind \"noise\""],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"height_m": 3}),
        &["`height_m` belongs to kind \"flat\""],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"size_m": 16384, "cell_m": 0.25}),
        &["cell_m", "at most 2049"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"size_m": 3}),
        &["size_m", "8 to 16384"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"size_m": 100, "cell_m": 3}),
        &["cell_m", "whole multiple"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"octaves": 11}),
        &["octaves", "1 to 10"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"octaves": 5_000_000_000u64}),
        &["`octaves`"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"roughness": 2}),
        &["roughness"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"size_m": "big"}),
        &["`size_m` must be number"],
    );
}

#[test]
fn terrain_clear_removes_the_ground() {
    let mut s = example();
    assert_eq!(s.run("terrain.clear", &json!({})).unwrap(), json!(null));
    assert!(s.document().terrain.is_none() && s.current_heights().is_none());
    refuse(
        &mut s,
        "terrain.clear",
        json!({"all": true}),
        &["takes no parameters"],
    );
}

#[test]
fn environment_set_changes_only_the_fields_given_and_gives_the_sun() {
    let mut s = example();
    let r = s
        .run(
            "environment.set",
            &json!({"time_of_day_h": 12, "day_of_year": 80, "latitude_deg": 0, "rain": 0.7}),
        )
        .unwrap();
    let env = &s.document().environment;
    assert_eq!(
        (
            env.time_of_day_h,
            env.day_of_year,
            env.latitude_deg,
            env.rain
        ),
        (12.0, 80, 0.0, 0.7)
    );
    assert_eq!(env.wind_speed_mps, 3.0, "untouched fields keep their value");
    assert_eq!(r["environment"]["rain"], json!(0.7));
    assert!(r["sun"]["elevation_deg"].as_f64().unwrap() > 80.0, "{r}");
    assert!(r["sun"]["azimuth_deg"].is_number());
}

#[test]
fn environment_set_refuses_out_of_range_values() {
    let mut s = example();
    refuse(
        &mut s,
        "environment.set",
        json!({"time_of_day_h": 24}),
        &["time_of_day_h"],
    );
    refuse(
        &mut s,
        "environment.set",
        json!({"day_of_year": 0}),
        &["day_of_year"],
    );
    refuse(
        &mut s,
        "environment.set",
        json!({"day_of_year": -3}),
        &["`day_of_year`"],
    );
    refuse(
        &mut s,
        "environment.set",
        json!({"latitude_deg": 91}),
        &["latitude_deg"],
    );
    refuse(&mut s, "environment.set", json!({"mist": 1.5}), &["mist"]);
    refuse(
        &mut s,
        "environment.set",
        json!({"wind_speed_mps": -1}),
        &["wind_speed_mps"],
    );
    refuse(
        &mut s,
        "environment.set",
        json!({"cloud": 0.5}),
        &["did you mean `cloud_cover`"],
    );
}
