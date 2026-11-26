//! A world file finds its pack from the command line: `render`, `run`, `info`, `play` and the
//! servers draw a world file's models from the pack above it (the nearest `pack.json`), from
//! `--pack DIR`, and say in one sentence what is missing without one. Pictures need a GPU;
//! without an adapter those parts print one line and pass, unless `GAMECRAFT_REQUIRE_GPU=1`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
use common::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A camera on the crate at the origin, scaled 3, as in the renderer's own test.
const CAMERA: &str = r#"{"position": [0, 2.2, 6], "target": [0, 1.2, 0], "fov_y_deg": 50}"#;

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples")
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// The demo pack in a temporary folder, with a world file that shows its crate (scaled 3)
/// inside the pack (`demo/loose.gamecraft`) and a copy outside every pack
/// (`loose.gamecraft`).
fn setup(tag: &str) -> Temp {
    let t = Temp::new(tag);
    copy(&examples().join("assets-demo"), &t.path("demo"));
    let made = run_in(
        &t.0,
        &[
            "run",
            "--new",
            r#"{"name": "Yard"}"#,
            "--cmd",
            "terrain.set",
            "--params",
            r#"{"kind": "flat", "size_m": 64, "cell_m": 1}"#,
            "--cmd",
            "entity.add",
            "--params",
            r#"{"name": "Crate", "position": [0, 0, 0], "components": [{"type": "model", "asset": "models/crate.glb", "scale": 3}]}"#,
            "--out",
            "demo/loose.gamecraft",
        ],
    );
    assert_eq!(made.code, 0, "{}", made.err);
    std::fs::copy(t.path("demo/loose.gamecraft"), t.path("loose.gamecraft")).unwrap();
    t
}

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

/// Decodes a PNG and returns the mean (r, g, b) of a square of `half` pixels around the
/// centre.
fn centre(png: &Path, half: usize) -> [f64; 3] {
    let bytes = std::fs::read(png).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type.samples(), 4, "RGBA pictures");
    let (w, h) = (info.width as usize, info.height as usize);
    let data = &buf[..info.buffer_size()];
    let mut sum = [0.0; 3];
    let mut n = 0.0;
    for y in h / 2 - half..h / 2 + half {
        for x in w / 2 - half..w / 2 + half {
            let i = (y * w + x) * 4;
            for (k, s) in sum.iter_mut().enumerate() {
                *s += f64::from(data[i + k]);
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

fn is_wood(c: [f64; 3]) -> bool {
    c[0] > 1.3 * c[2] && c[1] > 0.6 * c[2]
}

fn is_magenta(c: [f64; 3]) -> bool {
    c[0] > 1.5 * c[1] && c[2] > 1.5 * c[1]
}

/// The `problems` of a `render` answer.
fn problems(info: &Value) -> Vec<&str> {
    info["problems"]
        .as_array()
        .expect("problems in the answer")
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect()
}

/// The sentences that say a model has no pack.
fn no_pack(info: &Value) -> Vec<String> {
    problems(info)
        .into_iter()
        .filter(|p| p.contains("no pack provides"))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_world_file_draws_its_models_from_the_pack_above_it_or_from_pack() {
    let t = setup("world-file-pack");
    let draw = |file: &str, name: &str, more: &[&str]| {
        let mut args: Vec<&str> = vec![
            "render", file, "--out", name, "--size", "320x240", "--camera", CAMERA,
        ];
        args.extend_from_slice(more);
        run_in(&t.0, &args)
    };
    // Inside the pack: the crate is drawn, nothing is said of it.
    let r = draw("demo/loose.gamecraft", "a.png", &[]);
    if r.code != 0 && !has_gpu(&r.err) {
        return;
    }
    assert_eq!(r.code, 0, "{}", r.err);
    let info = &r.json_lines()[0];
    assert!(
        problems(info)
            .iter()
            .all(|p| !p.contains("crate") && !p.contains("magenta")),
        "{info}"
    );
    let models = info["stats"]["models"].as_array().unwrap();
    assert!(
        models
            .iter()
            .any(|m| m["asset"] == "models/crate.glb" && m["drawn"] == 1),
        "{info}"
    );
    let c = centre(&t.path("a.png"), 6);
    assert!(is_wood(c), "wood, not magenta: {c:?}");

    // Outside every pack: a magenta box, and one sentence says what is missing and what to do.
    let r = draw("loose.gamecraft", "b.png", &[]);
    assert_eq!(r.code, 0, "{}", r.err);
    let info = &r.json_lines()[0];
    let said = no_pack(info);
    assert_eq!(said.len(), 1, "{info}");
    assert!(
        said[0].contains("models/crate.glb") && said[0].contains("--pack"),
        "{}",
        said[0]
    );
    assert!(is_magenta(centre(&t.path("b.png"), 6)));

    // --pack gives the pack.
    let r = draw("loose.gamecraft", "c.png", &["--pack", "demo"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let info = &r.json_lines()[0];
    assert!(no_pack(info).is_empty(), "{info}");
    assert!(is_wood(centre(&t.path("c.png"), 6)));

    // --pack overrides the pack above the file: an empty pack leaves the crate without files.
    std::fs::create_dir_all(t.path("empty-pack")).unwrap();
    std::fs::write(
        t.path("empty-pack/pack.json"),
        r#"{"gamecraft_pack": 1, "id": "empty", "name": "Empty"}"#,
    )
    .unwrap();
    let r = draw("demo/loose.gamecraft", "d.png", &["--pack", "empty-pack"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let info = &r.json_lines()[0];
    let said = no_pack(info);
    assert_eq!(said.len(), 1, "{info}");
    assert!(said[0].contains("empty-pack"), "{}", said[0]);
    assert!(is_magenta(centre(&t.path("d.png"), 6)));

    // A pack that is not one is a mistake of the command line.
    let r = draw("loose.gamecraft", "e.png", &["--pack", "nowhere"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(
        r.err.contains("nowhere") && r.err.contains("does not exist"),
        "{}",
        r.err
    );
    // --pack goes with a world file, not with a project folder.
    let r = draw("demo", "f.png", &["--pack", "demo"]);
    assert_eq!(r.code, 2, "{}", r.out);
    assert!(r.err.contains("--pack"), "{}", r.err);
}

#[test]
fn run_info_and_the_servers_find_the_pack_of_a_world_file() {
    let t = setup("world-file-cli");
    // run: the pack's assets are listed for a world file inside it, and for --pack.
    for (file, more) in [
        ("demo/loose.gamecraft", vec![]),
        ("loose.gamecraft", vec!["--pack", "demo"]),
    ] {
        let mut args: Vec<&str> = vec!["run", file, "--cmd", "asset.list"];
        args.extend(more);
        let r = run_in(&t.0, &args);
        assert_eq!(r.code, 0, "{file}: {}{}", r.out, r.err);
        let list = &r.json_lines()[0];
        assert_eq!(list["assets"][0]["path"], "models/crate.glb", "{list}");
        assert_eq!(list["missing"], json!([]), "{list}");
    }
    // Without a pack, the world's model is the one missing.
    let r = run_in(&t.0, &["run", "loose.gamecraft", "--cmd", "asset.list"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let list = &r.json_lines()[0];
    assert_eq!(list["assets"], json!([]), "{list}");
    assert_eq!(list["missing"][0]["asset"], "models/crate.glb", "{list}");
    // info takes --pack too.
    let r = run_in(&t.0, &["info", "loose.gamecraft", "--pack", "demo"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.json_lines()[0]["name"], "Yard");
    // The servers start with the world file's pack, found above it or given; a world file
    // opened later finds the pack above it inside the read root, or the server's own --pack.
    for (file, more, outside, inside) in [
        ("demo/loose.gamecraft", vec![], Value::Null, json!("above")),
        (
            "loose.gamecraft",
            vec!["--pack", "demo"],
            json!("default"),
            json!("default"),
        ),
    ] {
        let root = t.0.display().to_string();
        let mut args: Vec<&str> = vec!["--file", file, "--automation-read-root", &root];
        args.extend(more);
        let mut s = Server::start("serve", &args, &t.0);
        let list = s.ok("asset.list", json!({}));
        assert_eq!(list["assets"][0]["path"], "models/crate.glb", "{list}");
        let opened = s.ok("file.open", json!({"path": "loose.gamecraft"}));
        assert_eq!(opened["pack"]["found"], outside, "{opened}");
        if outside.is_null() {
            assert_eq!(opened["pack"], Value::Null, "{opened}");
        }
        let opened = s.ok("file.open", json!({"path": "demo/loose.gamecraft"}));
        assert_eq!(opened["pack"]["found"], inside, "{opened}");
        // Given explicitly, that pack counts; given null, none.
        let opened = s.ok(
            "file.open",
            json!({"path": "loose.gamecraft", "pack": "demo"}),
        );
        assert_eq!(opened["pack"]["found"], "given", "{opened}");
        let opened = s.ok(
            "file.open",
            json!({"path": "demo/loose.gamecraft", "pack": null}),
        );
        assert_eq!(opened["pack"], Value::Null, "{opened}");
        assert!(s.finish());
    }
}

#[test]
fn project_export_says_what_the_exported_world_needs() {
    let t = Temp::new("world-file-export");
    copy(&examples().join("assets-demo"), &t.path("demo"));
    let out = t.path("yard.gamecraft");
    let r = run_in(
        &t.0,
        &[
            "project",
            "export",
            &t.path("demo").display().to_string(),
            "--out",
            &out.display().to_string(),
        ],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let exported: Value = serde_json::from_str(r.out.trim()).unwrap();
    assert!(
        exported["pack"]["path"]
            .as_str()
            .unwrap()
            .ends_with("/demo"),
        "{exported}"
    );
    assert_eq!(exported["pack"]["id"], "assets-demo");
    assert_eq!(
        exported["needs"],
        json!([
            "assets/models/crate.glb",
            "assets/models/crate.glb.license.json",
            "assets/models/fence.glb",
            "assets/models/fence.glb.license.json"
        ]),
        "{exported}"
    );
    // Rendered from inside the pack folder, the export draws its models.
    let inside = t.path("demo/yard.gamecraft");
    std::fs::copy(&out, &inside).unwrap();
    let r = run_in(
        &t.0,
        &["run", &inside.display().to_string(), "--cmd", "asset.list"],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let list = &r.json_lines()[0];
    assert_eq!(list["missing"], json!([]), "{list}");
}
