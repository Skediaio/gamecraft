//! Collision parts through the host: a pack's models are measured for their parts as for their
//! bounds, `asset.list` says what each model's collider is, and a solid model with the `mesh`
//! collider collides with its own geometry from a project, from a world file inside its pack
//! and after an import. The building is built in the test: a facade of three boxes around a
//! doorway, with three `*_collider` and `*-collider` nodes beside it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::models::glb;
use common::{TempDir, host_in, ok};
use gamecraft_automation::Host;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn examples() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

/// The 36 corners of a unit cube standing on y = 0, as triangles.
fn cube_corners() -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    for (axis, sign) in [
        (0, 0.0f32),
        (0, 1.0),
        (1, 0.0),
        (1, 1.0),
        (2, 0.0),
        (2, 1.0),
    ] {
        let (a, b) = match axis {
            0 => (1, 2),
            1 => (2, 0),
            _ => (0, 1),
        };
        let corner = |u: f32, v: f32| {
            let mut p = [0.0; 3];
            p[axis] = sign;
            p[a] = u;
            p[b] = v;
            p
        };
        let c00 = corner(0.0, 0.0);
        let c10 = corner(1.0, 0.0);
        let c11 = corner(1.0, 1.0);
        let c01 = corner(0.0, 1.0);
        out.extend([c00, c10, c11, c00, c11, c01]);
    }
    out
}

/// The GLB of a facade 8 m wide, 4 m high, 0.3 m thick with a doorway 2 m wide and 2.5 m
/// high: three meshes (unit cubes placed by their nodes), and the same three boxes again as
/// `*_collider` nodes (a facade with a hole in one mesh is more than a test needs to build).
fn facade() -> Vec<u8> {
    let cube = |name: &str, t: [f64; 3], s: [f64; 3]| json!({"name": name, "mesh": 0, "translation": t, "scale": s});
    let mut bin: Vec<u8> = Vec::new();
    for p in cube_corners() {
        for v in p {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    let count = (bin.len() / 12) as u64;
    let doc = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0, 1, 2, 3, 4, 5]}],
        "nodes": [
            cube("Left", [-4.0, 0.0, -0.15], [3.0, 4.0, 0.3]),
            cube("Right", [1.0, 0.0, -0.15], [3.0, 4.0, 0.3]),
            cube("Lintel", [-1.0, 2.5, -0.15], [2.0, 1.5, 0.3]),
            cube("left_collider", [-4.0, 0.0, -0.15], [3.0, 4.0, 0.3]),
            cube("right-collider", [1.0, 0.0, -0.15], [3.0, 4.0, 0.3]),
            cube("Lintel_COLLIDER", [-1.0, 2.5, -0.15], [2.0, 1.5, 0.3]),
        ],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": bin.len()}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3",
                       "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 1.0]}],
    });
    glb(&serde_json::to_vec(&doc).unwrap(), &bin)
}

/// The demo pack with the facade imported as `models/house.glb`, opened.
fn with_house(tag: &str) -> (TempDir, Host) {
    let dir = TempDir::new(tag);
    copy(&examples().join("assets-demo"), &dir.join("demo"));
    fs::create_dir_all(dir.join("downloads")).unwrap();
    fs::write(dir.join("downloads/house.glb"), facade()).unwrap();
    let mut h = host_in(&dir);
    ok(&mut h, "project.open", json!({"path": "demo"}));
    let imported = ok(
        &mut h,
        "asset.import",
        json!({"file": "downloads/house.glb", "to": "models/house.glb", "license": "CC0-1.0",
               "author": "The GameCraft project", "source_url": "https://example.org/house"}),
    );
    assert_eq!(
        imported["triangles"], 36,
        "the three visual meshes: the colliders are not counted"
    );
    (dir, h)
}

/// Where the house stands: away from the demo pack's own crates and characters, which sit
/// around the origin of its world.
const AT: [f64; 3] = [20.0, 0.0, -20.0];

fn house(h: &mut Host) {
    ok(
        h,
        "terrain.set",
        json!({"kind": "flat", "size_m": 64, "cell_m": 1}),
    );
    ok(
        h,
        "entity.add",
        json!({"name": "House", "position": AT,
               "components": [{"type": "model", "asset": "models/house.glb", "solid": true,
                               "collider": "mesh"}]}),
    );
}

fn z_of(h: &mut Host, name: &str) -> f64 {
    let e = ok(h, "entity.get", json!({"id": name}));
    e["position"][2].as_f64().unwrap()
}

/// Two walkers, one through the doorway and one at the wall, walking north for 5 s.
fn walkers(h: &mut Host) {
    for (name, x) in [("Through", 0.0), ("Wall", 2.5)] {
        ok(
            h,
            "entity.add",
            json!({"name": name, "position": [AT[0] + x, 0, AT[2] + 6.0], "components": [{"type": "character"}]}),
        );
    }
    ok(h, "sim.start", json!({}));
    for name in ["Through", "Wall"] {
        ok(
            h,
            "sim.input",
            json!({"entity": name, "forward": 1, "yaw_deg": 0}),
        );
    }
    ok(h, "sim.step", json!({"seconds": 5}));
}

#[test]
fn a_pack_s_models_are_measured_for_their_parts_and_listed() {
    let (_dir, mut h) = with_house("colliders-list");
    let list = ok(&mut h, "asset.list", json!({}));
    let assets = list["assets"].as_array().unwrap();
    let house_asset = assets
        .iter()
        .find(|a| a["path"] == "models/house.glb")
        .unwrap();
    let collider = &house_asset["collider"];
    assert_eq!(collider["source"], "colliders", "{house_asset}");
    assert_eq!(collider["parts"], 3);
    assert!(collider["note"].is_null(), "{collider}");
    let boxes = collider["boxes"].as_array().unwrap();
    assert!((boxes[2]["center"][1].as_f64().unwrap() - 3.25).abs() < 1e-6);
    assert!(
        (house_asset["bounds"]["min"][2].as_f64().unwrap() + 0.15).abs() < 1e-6
            && (house_asset["bounds"]["max"][0].as_f64().unwrap() - 4.0).abs() < 1e-6,
        "{}",
        house_asset["bounds"]
    );
    // The demo's one-mesh models: one box each, the bounding box.
    let crate_ = assets
        .iter()
        .find(|a| a["path"] == "models/crate.glb")
        .unwrap();
    assert_eq!(crate_["collider"]["source"], "meshes");
    assert_eq!(crate_["collider"]["parts"], 1);
    // The parts reached the session with the bounds.
    house(&mut h);
    let parts = h
        .session()
        .document()
        .model_parts("models/house.glb")
        .map(<[gamecraft_world::ModelBox]>::len);
    assert_eq!(parts, Some(3));
    walkers(&mut h);
    let (through, wall) = (z_of(&mut h, "Through"), z_of(&mut h, "Wall"));
    assert!(through < AT[2] - 1.0, "through the doorway: z {through}");
    assert!(
        (wall - (AT[2] + 0.45)).abs() < 1e-6,
        "stopped at the wall: z {wall}"
    );
}

#[test]
fn a_world_file_in_the_pack_gets_the_parts_and_the_box_collider_stays_a_wall() {
    let (dir, h) = with_house("colliders-file");
    let mut h = h;
    house(&mut h);
    ok(&mut h, "file.save", json!({"path": "demo/house.gamecraft"}));
    // Opened as a world file inside the pack: the parts come with the pack.
    let opened = ok(&mut h, "file.open", json!({"path": "demo/house.gamecraft"}));
    assert_eq!(opened["pack"]["found"], "above");
    assert_eq!(
        h.session()
            .document()
            .model_parts("models/house.glb")
            .map(<[gamecraft_world::ModelBox]>::len),
        Some(3)
    );
    walkers(&mut h);
    let (through, wall) = (z_of(&mut h, "Through"), z_of(&mut h, "Wall"));
    assert!(
        through < AT[2] - 1.0,
        "through the doorway: z {through} (wall at {wall})"
    );
    assert!(
        (wall - (AT[2] + 0.45)).abs() < 1e-6,
        "stopped at the wall: z {wall}"
    );
    let ray: Value = ok(
        &mut h,
        "query.raycast",
        json!({"origin": [AT[0], 1.0, AT[2] + 6.0], "direction": [0.0, 0.0, -1.0], "max_m": 12}),
    );
    assert_eq!(ray["hit"], false, "through the doorway: {ray}");
    // With the `box` collider the same house is a wall: a walker is stopped at the facade.
    ok(
        &mut h,
        "entity.set_component",
        json!({"id": "House", "component": {"type": "model", "asset": "models/house.glb",
                                             "solid": true, "collider": "box"}}),
    );
    ok(
        &mut h,
        "entity.add",
        json!({"name": "Again", "position": [AT[0], 0, AT[2] + 6.0], "components": [{"type": "character"}]}),
    );
    ok(
        &mut h,
        "sim.input",
        json!({"entity": "Again", "forward": 1, "yaw_deg": 0}),
    );
    ok(&mut h, "sim.step", json!({"seconds": 5}));
    assert!(
        (z_of(&mut h, "Again") - (AT[2] + 0.45)).abs() < 1e-6,
        "the bounding box stops at the facade"
    );
    let _ = dir;
}
