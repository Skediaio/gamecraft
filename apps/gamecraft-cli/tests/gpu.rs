//! Pictures: they need a GPU. Without an adapter these tests print one line and pass, unless
//! `GAMECRAFT_REQUIRE_GPU=1`, which turns a missing adapter into a failure.
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

/// Whether this machine can draw; prints why not, or fails when a GPU is required.
fn has_gpu(error: &str) -> bool {
    if error.contains("no GPU was found") {
        assert!(
            std::env::var("GAMECRAFT_REQUIRE_GPU").as_deref() != Ok("1"),
            "GAMECRAFT_REQUIRE_GPU=1 but there is no GPU: {error}"
        );
        println!("skipped: no GPU adapter on this machine");
        return false;
    }
    true
}

/// Decodes a PNG into (width, height, whether any two pixels differ).
fn decode(bytes: &[u8]) -> (u32, u32, bool) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    let data = &buf[..info.buffer_size()];
    let step = info.color_type.samples();
    let first = &data[..step];
    let varied = data.chunks_exact(step).any(|p| p != first);
    (info.width, info.height, varied)
}

#[test]
fn render_writes_a_png_of_the_asked_size() {
    let dir = Temp::new("render");
    let made = run_in(
        &dir.0,
        &[
            "run",
            "--cmd",
            "terrain.set",
            "--params",
            r#"{"kind":"noise","size_m":64,"cell_m":2,"relief_m":6,"feature_m":40,"seed":1}"#,
            "--cmd",
            "entity.add",
            "--params",
            r#"{"name":"Rock","at":[3,3],"components":[{"type":"shape","kind":"sphere","size":[2,2,2],"color":[0.5,0.5,0.5]}]}"#,
            "--out",
            "w.gamecraft",
        ],
    );
    assert_eq!(made.code, 0, "{}", made.err);
    let r = run_in(
        &dir.0,
        &[
            "render",
            "w.gamecraft",
            "--out",
            "w.png",
            "--size",
            "320x180",
        ],
    );
    if r.code != 0 && !has_gpu(&r.err) {
        return;
    }
    assert_eq!(r.code, 0, "{}", r.err);
    let info = &r.json_lines()[0];
    assert_eq!(info["renderer"], "gpu", "{info}");
    assert!(info["stats"]["triangles"].as_u64().unwrap() > 0);
    let (w, h, varied) = decode(&std::fs::read(dir.path("w.png")).unwrap());
    assert_eq!((w, h), (320, 180));
    assert!(varied, "the picture is one flat colour");

    // A named entity is followed; a missing one is an error that says so.
    let missing = run_in(
        &dir.0,
        &[
            "render",
            "w.gamecraft",
            "--out",
            "m.png",
            "--follow",
            "Nobody",
        ],
    );
    assert_eq!(missing.code, 1);
    let no_eyes = run_in(
        &dir.0,
        &[
            "render",
            "w.gamecraft",
            "--out",
            "m.png",
            "--follow",
            "Rock",
            "--view",
            "first",
        ],
    );
    assert_eq!(no_eyes.code, 1);
    assert!(no_eyes.err.contains("character"), "{}", no_eyes.err);
    let both = run_in(
        &dir.0,
        &[
            "render",
            "w.gamecraft",
            "--out",
            "m.png",
            "--follow",
            "Rock",
            "--camera",
            "{}",
        ],
    );
    assert_eq!(both.code, 2);
}

#[test]
fn view_render_over_serve_follows_an_entity() {
    let dir = Temp::new("follow");
    let mut s = Server::start("serve", &[], &dir.0);
    s.ok("file.new", json!({"name": "F"}));
    s.ok(
        "terrain.set",
        json!({"kind": "noise", "size_m": 64, "cell_m": 2, "relief_m": 3, "seed": 2}),
    );
    s.ok(
        "entity.add",
        json!({"name": "Scout", "at": [0, 0], "components": [{"type": "character"}]}),
    );
    let reply = s.reply(
        "view.render",
        json!({"width": 240, "height": 135, "follow": {"entity": "Scout"}}),
    );
    if reply["ok"] != true && !has_gpu(reply["error"].as_str().unwrap_or("")) {
        return;
    }
    assert_eq!(reply["ok"], true, "{reply}");
    let result = &reply["result"];
    assert_eq!(result["mime"], "image/png");
    assert_eq!(result["renderer"], "gpu");
    let b64 = result["base64"].as_str().unwrap();
    assert!(b64.starts_with("iVBORw0KGgo"), "not a PNG");
    // A second picture of an unchanged world reuses the scene.
    let again = s.ok("view.render", json!({"width": 240, "height": 135}));
    assert!(again["stats"]["draws"].as_u64().unwrap() > 0);
    assert!(s.finish());
}

#[test]
fn the_m0_scenario_ends_with_a_picture() {
    let out: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&out).unwrap();
    let dir = Temp::new("m0pic");
    let mut s = Server::start(
        "serve",
        &["--automation-write-root", out.to_str().unwrap()],
        &dir.0,
    );
    let scenario = m0_scenario(&mut s);
    let reply = s.reply(
        "view.render",
        json!({"path": "m0-scenario.png", "width": 960, "height": 540,
               "follow": {"entity": "Scout", "view": "third_person", "distance_m": 9}}),
    );
    if reply["ok"] != true && !has_gpu(reply["error"].as_str().unwrap_or("")) {
        return;
    }
    assert_eq!(reply["ok"], true, "{reply}");
    let (w, h, varied) = decode(&std::fs::read(out.join("m0-scenario.png")).unwrap());
    assert_eq!((w, h), (960, 540));
    assert!(varied);
    assert!(scenario.final_position[2] > scenario.wall_front_z);
    let _: &Value = &reply;
    assert!(s.finish());
}
