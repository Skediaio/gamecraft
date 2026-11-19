//! Assets through the host: the demo pack opens with its models measured, a solid crate stops a
//! character and a fence that is not solid lets one through, `asset.import` round trips into a
//! project, `asset.list` and `asset.credits`, the licence gate on open, the files a renderer is
//! handed, and hostile parameters.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{TempDir, err, host_in, ok};
use gamecraft_automation::{Host, Pictures, RenderRequest, Rendered};
use gamecraft_world::{HeightField, World};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;

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

/// The demo pack copied into a temporary folder and opened.
fn demo(tag: &str) -> (TempDir, Host) {
    let dir = TempDir::new(tag);
    copy(&examples().join("assets-demo"), &dir.join("demo"));
    let mut h = host_in(&dir);
    ok(&mut h, "project.open", json!({"path": "demo"}));
    (dir, h)
}

fn z_of(h: &mut Host, name: &str) -> f64 {
    let e = ok(h, "entity.get", json!({"id": name}));
    e["position"][2].as_f64().unwrap()
}

#[test]
fn the_demo_pack_s_models_are_listed_with_triangles_and_bounds() {
    let (_dir, mut h) = demo("assets-list");
    let list = ok(&mut h, "asset.list", json!({}));
    let assets = list["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 2, "{list}");
    let crate_ = &assets[0];
    assert_eq!(crate_["path"], "models/crate.glb");
    assert_eq!(crate_["license"], "CC0-1.0");
    assert_eq!(crate_["author"], "The GameCraft project");
    assert_eq!(crate_["triangles"], 12);
    assert_eq!(crate_["used_by"], 3);
    assert!((crate_["bounds"]["max"][1].as_f64().unwrap() - 0.9).abs() < 1e-6);
    assert_eq!(assets[1]["triangles"], 48);
    assert_eq!(list["missing"], json!([]));
    // CC0 only: nothing to credit.
    let credits = ok(&mut h, "asset.credits", json!({}));
    assert_eq!(credits, json!({"text": "", "assets": []}));
}

#[test]
fn a_solid_crate_stops_a_character_and_a_fence_does_not() {
    let (_dir, mut h) = demo("assets-walk");
    ok(&mut h, "sim.start", json!({}));
    for who in ["Walker", "Hiker"] {
        ok(
            &mut h,
            "sim.input",
            json!({"entity": who, "forward": 1, "yaw_deg": 0}),
        );
    }
    ok(&mut h, "sim.step", json!({"ticks": 600}));
    // The crate is 0.9 m wide on its base: its south face is at z = 0.45.
    let walker = z_of(&mut h, "Walker");
    assert!((walker - (0.45 + 0.3)).abs() < 1e-6, "Walker at z {walker}");
    // The fence at z = 0 is not solid: the hiker walked on, 10 s at 1.6 m/s from z = 6.
    let hiker = z_of(&mut h, "Hiker");
    assert!((hiker - (6.0 - 16.0)).abs() < 1e-6, "Hiker at z {hiker}");
}

#[test]
fn import_round_trips_into_a_project_and_credits_name_cc_by() {
    let dir = TempDir::new("assets-import");
    let mut h = host_in(&dir);
    ok(&mut h, "file.new", json!({"name": "Game"}));
    ok(
        &mut h,
        "project.new",
        json!({"path": "game", "id": "game", "name": "Game"}),
    );
    fs::create_dir_all(dir.join("downloads")).unwrap();
    fs::copy(
        examples().join("assets-demo/assets/models/fence.glb"),
        dir.join("downloads/fence.glb"),
    )
    .unwrap();
    let done = ok(
        &mut h,
        "asset.import",
        json!({"file": "downloads/fence.glb", "to": "props/fence.glb", "license": "CC-BY-4.0",
               "author": "Ada", "source_url": "https://example.org/fence", "title": "Fence"}),
    );
    assert_eq!(done["triangles"], 48);
    assert_eq!(
        done["files"],
        json!([
            "assets/props/fence.glb",
            "assets/props/fence.glb.license.json"
        ])
    );
    assert!(dir.join("game/assets/props/fence.glb").is_file());
    let record = fs::read_to_string(dir.join("game/assets/props/fence.glb.license.json")).unwrap();
    assert!(
        record.contains("\"CC-BY-4.0\"") && record.contains("Ada"),
        "{record}"
    );
    // Listed and credited now, and again after opening the project from its files.
    for round in 0..2 {
        let list = ok(&mut h, "asset.list", json!({}));
        assert_eq!(list["assets"][0]["path"], "props/fence.glb", "{round}");
        let credits = ok(&mut h, "asset.credits", json!({}));
        let text = credits["text"].as_str().unwrap();
        assert!(
            text.contains("\"Fence\" by Ada, CC BY 4.0") && text.contains("example.org/fence"),
            "{text}"
        );
        ok(&mut h, "project.open", json!({"path": "game"}));
    }
    // A solid model of it collides at once (its bounds reached the session).
    ok(
        &mut h,
        "entity.add",
        json!({"name": "Fence", "position": [0, 0, 0],
               "components": [{"type": "model", "asset": "props/fence.glb", "solid": true}]}),
    );
    assert!(
        h.session()
            .document()
            .model_bounds("props/fence.glb")
            .is_some()
    );
    // The same target again is refused, and so is a broken model, before anything is written.
    let e = err(
        &mut h,
        "asset.import",
        json!({"file": "downloads/fence.glb", "to": "props/fence.glb", "license": "CC0-1.0",
               "author": "Ada", "source_url": "x"}),
    );
    assert!(e.contains("exists already"), "{e}");
    fs::write(dir.join("downloads/broken.glb"), b"glTF\x02\0\0\0garbage").unwrap();
    let e = err(
        &mut h,
        "asset.import",
        json!({"file": "downloads/broken.glb", "to": "props/broken.glb", "license": "CC0-1.0",
               "author": "Ada", "source_url": "x"}),
    );
    assert!(e.contains("not a model that can be used"), "{e}");
    assert!(!dir.join("game/assets/props/broken.glb").exists());
    let e = err(
        &mut h,
        "asset.import",
        json!({"file": "downloads/fence.glb", "to": "props/f.glb", "license": "GPL-3.0",
               "author": "Ada", "source_url": "x"}),
    );
    assert!(e.contains("license") && e.contains("CC0-1.0"), "{e}");
}

#[test]
fn opening_a_pack_with_an_asset_without_a_record_is_refused() {
    let dir = TempDir::new("assets-gate");
    copy(&examples().join("assets-demo"), &dir.join("demo"));
    let record = dir.join("demo/assets/models/fence.glb.license.json");
    fs::remove_file(&record).unwrap();
    let mut h = host_in(&dir);
    let e = err(&mut h, "project.open", json!({"path": "demo"}));
    assert!(
        e.contains("fence.glb") && e.contains("has no licence record"),
        "{e}"
    );
    fs::write(
        &record,
        r#"{"license": "CC-BY-NC-4.0", "author": "x", "source_url": "y"}"#,
    )
    .unwrap();
    let e = err(&mut h, "project.open", json!({"path": "demo"}));
    assert!(
        e.contains("CC-BY-NC-4.0") && e.contains("not allowed"),
        "{e}"
    );
}

/// Remembers which asset files it was handed.
struct Keeper {
    given: Rc<RefCell<Vec<(String, usize)>>>,
}

impl Pictures for Keeper {
    fn render(
        &mut self,
        _world: &World,
        _heights: Option<&HeightField>,
        _view_revision: u64,
        _request: &RenderRequest,
    ) -> Result<Rendered, String> {
        Ok(Rendered {
            png: common::PNG_1X1.to_vec(),
            info: Value::Null,
        })
    }

    fn provide_asset(&mut self, path: &str, bytes: Vec<u8>) {
        self.given
            .borrow_mut()
            .push((path.to_string(), bytes.len()));
    }
}

#[test]
fn the_renderer_is_handed_each_model_s_files_once() {
    let (dir, h) = demo("assets-provide");
    let given = Rc::new(RefCell::new(Vec::new()));
    let mut h = h.with_pictures(Box::new(Keeper {
        given: given.clone(),
    }));
    ok(&mut h, "view.render", json!({"width": 32, "height": 32}));
    ok(&mut h, "view.render", json!({"width": 32, "height": 32}));
    let crate_len = fs::metadata(dir.join("demo/assets/models/crate.glb"))
        .unwrap()
        .len() as usize;
    let got = given.borrow().clone();
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got.contains(&("models/crate.glb".to_string(), crate_len)));
}

#[test]
fn asset_commands_take_hostile_parameters_without_panicking() {
    let (_dir, mut h) = demo("assets-hostile");
    let values = [
        json!(null),
        json!(""),
        json!("../../etc/passwd"),
        json!("/etc/passwd"),
        json!("a.glb"),
        json!("x".repeat(5000)),
        json!(-1),
        json!(1e308),
        json!([1, 2]),
        json!({"a": 1}),
        json!("CC0-1.0"),
        json!("\u{0}"),
    ];
    let keys = [
        "file",
        "to",
        "license",
        "author",
        "source_url",
        "title",
        "changes",
    ];
    for command in ["asset.import", "asset.list", "asset.credits"] {
        for key in keys {
            for v in &values {
                let mut params = json!({"file": "x.glb", "to": "m/x.glb", "license": "CC0-1.0",
                                         "author": "a", "source_url": "s"});
                params[key] = v.clone();
                let reply = common::execute(&mut h, command, params);
                let text = reply.to_string();
                assert!(!text.contains("internal error"), "{command} {key}: {text}");
            }
        }
    }
}
