//! Assets through the host: the demo pack opens with its models measured, a solid crate stops a
//! character and a fence that is not solid lets one through, `asset.import` round trips into a
//! project, `asset.list` and `asset.credits`, the licence gate on open, the files a renderer is
//! handed, and hostile parameters. The texture store: models that share an image store it once.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::models::{
    embedded_image, glb_json_only, glb_with_image, gltf_with_files, gltf_with_image_in_buffer,
};
use common::{TempDir, err, host_in, ok};
use gamecraft_automation::{Host, Pictures, RenderRequest, Rendered};
use gamecraft_world::{HeightField, World};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeSet;
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
    // One level, drawn as far as its size asks (0.9 m on a side: about 364 m).
    assert_eq!(crate_["lod_source"], Value::Null);
    assert_eq!(crate_["lod_warnings"], json!([]));
    assert_eq!(
        crate_["lods"],
        json!([{"level": 0, "triangles": 12, "until_m": crate_["draw_distance_m"].clone()}])
    );
    let draw = crate_["draw_distance_m"].as_f64().unwrap();
    assert!((364.0..366.0).contains(&draw), "{draw}");
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
    // Its image goes into the pack's texture store, once, with the model's record.
    let stored = done["textures"].as_array().unwrap();
    assert_eq!(stored.len(), 1, "{done}");
    let texture = stored[0]["path"].as_str().unwrap();
    assert!(
        texture.starts_with("textures/") && stored[0]["stored"] == true,
        "{done}"
    );
    assert_eq!(
        done["files"],
        json!([
            format!("assets/{texture}"),
            format!("assets/{texture}.license.json"),
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
        "textures",
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

// ------------------------------------------------------------------------ the texture store

/// A new project `game` in a temporary folder, with a `downloads/` folder beside it.
fn project(tag: &str) -> (TempDir, Host) {
    let dir = TempDir::new(tag);
    let mut h = host_in(&dir);
    ok(&mut h, "file.new", json!({"name": "Game"}));
    ok(
        &mut h,
        "project.new",
        json!({"path": "game", "id": "game", "name": "Game"}),
    );
    fs::create_dir_all(dir.join("downloads")).unwrap();
    (dir, h)
}

/// The images of the two demo models: the crate's wood and the fence's.
fn demo_images() -> (Vec<u8>, Vec<u8>) {
    let models = examples().join("assets-demo/assets/models");
    let crate_ = fs::read(models.join("crate.glb")).unwrap();
    let fence = fs::read(models.join("fence.glb")).unwrap();
    (embedded_image(&crate_), embedded_image(&fence))
}

/// The files of the project's texture store (`assets/textures/`), without the records, sorted.
fn textures(dir: &TempDir) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(dir.join("game/assets/textures"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| !n.ends_with(".license.json"))
        .collect();
    out.sort();
    out
}

/// `asset.import` of `file` to `to` as CC0 by Ada.
fn import(h: &mut Host, file: &str, to: &str) -> Value {
    ok(
        h,
        "asset.import",
        json!({"file": file, "to": to, "license": "CC0-1.0", "author": "Ada",
               "source_url": "https://example.org/models"}),
    )
}

#[test]
fn two_models_with_the_same_image_share_one_texture_file() {
    let (dir, mut h) = project("assets-shared");
    let (wood, _) = demo_images();
    fs::write(dir.join("downloads/a.glb"), glb_with_image(&wood, "a")).unwrap();
    fs::write(dir.join("downloads/b.glb"), glb_with_image(&wood, "b")).unwrap();
    let a = import(&mut h, "downloads/a.glb", "models/a.glb");
    let b = import(&mut h, "downloads/b.glb", "models/b.glb");
    // One texture in the store, with one record: a stored it, b reused it.
    let store = textures(&dir);
    assert_eq!(store.len(), 1, "{store:?}");
    let name = store[0].clone();
    assert!(name.ends_with(".png"), "{name}");
    assert_eq!(
        fs::read(dir.path().join(format!("game/assets/textures/{name}"))).unwrap(),
        wood
    );
    assert!(
        dir.path()
            .join(format!("game/assets/textures/{name}.license.json"))
            .is_file()
    );
    let record = format!("assets/textures/{name}.license.json");
    let path = format!("textures/{name}");
    assert_eq!(
        a["textures"],
        json!([{"path": path, "bytes": wood.len(), "stored": true, "record": record}])
    );
    assert_eq!(
        a["files"],
        json!([
            format!("assets/{path}"),
            record,
            "assets/models/a.glb",
            "assets/models/a.glb.license.json"
        ])
    );
    assert_eq!(b["textures"][0]["path"], path);
    assert_eq!(b["textures"][0]["stored"], false, "{b}");
    assert_eq!(
        b["files"],
        json!(["assets/models/b.glb", "assets/models/b.glb.license.json"])
    );
    // The stored models no longer hold the image.
    for m in ["a", "b"] {
        let stored = fs::metadata(dir.path().join(format!("game/assets/models/{m}.glb")))
            .unwrap()
            .len();
        let source = glb_with_image(&wood, m).len() as u64;
        assert!(
            stored + wood.len() as u64 <= source,
            "{m}: {stored} vs {source}"
        );
    }
    // Listed once, used by both; before and after reopening the project from its files.
    for _ in 0..2 {
        let list = ok(&mut h, "asset.list", json!({}));
        assert_eq!(
            list["textures"],
            json!([{
                "path": path,
                "pack": "game",
                "bytes": wood.len(),
                "license": "CC0-1.0",
                "author": "Ada",
                "source_url": "https://example.org/models",
                "title": null,
                "changes": null,
                "used_by": ["models/a.glb", "models/b.glb"],
            }]),
            "{list}"
        );
        let assets = list["assets"].as_array().unwrap();
        assert_eq!(assets.len(), 2, "{list}");
        for (i, m) in ["a", "b"].iter().enumerate() {
            assert_eq!(assets[i]["path"], format!("models/{m}.glb"));
            assert_eq!(assets[i]["kind"], "model");
            assert_eq!(assets[i]["triangles"], 1);
            assert_eq!(assets[i]["files"][1], format!("assets/{path}"));
        }
        ok(&mut h, "project.open", json!({"path": "game"}));
    }
}

#[test]
fn a_model_with_a_new_image_adds_one_texture() {
    let (dir, mut h) = project("assets-new-texture");
    let (wood, plank) = demo_images();
    for (n, image) in [("a", &wood), ("b", &wood), ("c", &plank)] {
        fs::write(
            dir.path().join(format!("downloads/{n}.glb")),
            glb_with_image(image, n),
        )
        .unwrap();
    }
    import(&mut h, "downloads/a.glb", "models/a.glb");
    import(&mut h, "downloads/b.glb", "models/b.glb");
    let c = import(&mut h, "downloads/c.glb", "models/c.glb");
    assert_eq!(c["textures"][0]["stored"], true, "{c}");
    let store = textures(&dir);
    assert_eq!(store.len(), 2, "{store:?}");
    let held: BTreeSet<Vec<u8>> = store
        .iter()
        .map(|n| fs::read(dir.path().join(format!("game/assets/textures/{n}"))).unwrap())
        .collect();
    assert_eq!(held, BTreeSet::from([wood.clone(), plank.clone()]));
    // A model with an image the pack has again adds nothing.
    import(&mut h, "downloads/c.glb", "models/d.glb");
    assert_eq!(textures(&dir).len(), 2);
    let list = ok(&mut h, "asset.list", json!({}));
    let t = list["textures"].as_array().unwrap();
    assert_eq!(t.len(), 2, "{list}");
    assert!(
        t.iter()
            .any(|x| x["used_by"] == json!(["models/a.glb", "models/b.glb"]))
    );
    assert!(
        t.iter()
            .any(|x| x["used_by"] == json!(["models/c.glb", "models/d.glb"]))
    );
}

#[test]
fn a_shared_cc_by_texture_is_credited_once() {
    let (dir, mut h) = project("assets-texture-credit");
    let (wood, _) = demo_images();
    fs::write(dir.join("downloads/a.glb"), glb_with_image(&wood, "a")).unwrap();
    fs::write(dir.join("downloads/b.glb"), glb_with_image(&wood, "b")).unwrap();
    ok(
        &mut h,
        "asset.import",
        json!({"file": "downloads/a.glb", "to": "models/a.glb", "license": "CC0-1.0",
               "author": "Ada", "source_url": "https://example.org/a",
               "textures": {"license": "CC-BY-4.0", "author": "Poly Haven",
                            "source_url": "https://example.org/plaster", "title": "Plaster"}}),
    );
    // The texture a stored keeps its record; b's own (CC0, Bo) is not stamped on it.
    ok(
        &mut h,
        "asset.import",
        json!({"file": "downloads/b.glb", "to": "models/b.glb", "license": "CC0-1.0",
               "author": "Bo", "source_url": "https://example.org/b"}),
    );
    let name = textures(&dir)[0].clone();
    let record = fs::read_to_string(
        dir.path()
            .join(format!("game/assets/textures/{name}.license.json")),
    )
    .unwrap();
    assert!(
        record.contains("\"CC-BY-4.0\"")
            && record.contains("Poly Haven")
            && record.contains("Plaster"),
        "{record}"
    );
    for _ in 0..2 {
        let credits = ok(&mut h, "asset.credits", json!({}));
        assert_eq!(
            credits["assets"],
            json!([{"path": format!("textures/{name}"), "kind": "texture",
                    "title": "Plaster", "author": "Poly Haven", "license": "CC-BY-4.0",
                    "source_url": "https://example.org/plaster", "changes": null}]),
            "{credits}"
        );
        assert_eq!(
            credits["text"],
            "\"Plaster\" by Poly Haven, CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/), \
             from https://example.org/plaster.\n"
        );
        ok(&mut h, "project.open", json!({"path": "game"}));
    }
}

#[test]
fn a_gltf_with_its_image_beside_it_keeps_its_buffer_and_shares_its_image() {
    let (dir, mut h) = project("assets-gltf");
    let (wood, _) = demo_images();
    fs::create_dir_all(dir.join("downloads/tex")).unwrap();
    let (text, bin) = gltf_with_files("tex/wood.png", "house.bin", "house");
    fs::write(dir.join("downloads/house.gltf"), text).unwrap();
    fs::write(dir.join("downloads/house.bin"), bin).unwrap();
    fs::write(dir.join("downloads/tex/wood.png"), &wood).unwrap();
    let done = ok(
        &mut h,
        "asset.import",
        json!({"file": "downloads/house.gltf", "to": "models/houses/house.gltf",
               "license": "CC0-1.0", "author": "Ada", "source_url": "https://example.org/house"}),
    );
    let name = textures(&dir)[0].clone();
    assert_eq!(
        done["files"],
        json!([
            format!("assets/textures/{name}"),
            format!("assets/textures/{name}.license.json"),
            "assets/models/houses/house.bin",
            "assets/models/houses/house.gltf",
            "assets/models/houses/house.gltf.license.json"
        ]),
        "{done}"
    );
    let stored = fs::read_to_string(dir.join("game/assets/models/houses/house.gltf")).unwrap();
    assert!(
        stored.contains(&format!("\"uri\":\"../../textures/{name}\""))
            && stored.contains("\"uri\":\"house.bin\""),
        "{stored}"
    );
    assert!(dir.join("game/assets/models/houses/house.bin").is_file());
    assert!(!dir.join("game/assets/textures/wood.png").exists());
    ok(&mut h, "project.open", json!({"path": "game"}));
    let list = ok(&mut h, "asset.list", json!({}));
    assert_eq!(
        list["textures"][0]["used_by"],
        json!(["models/houses/house.gltf"])
    );
    assert_eq!(list["assets"][0]["triangles"], 1);
    assert_eq!(
        list["assets"][0]["files"],
        json!([
            "assets/models/houses/house.gltf",
            "assets/models/houses/house.bin",
            format!("assets/textures/{name}")
        ])
    );
}

#[test]
fn a_missing_texture_file_is_a_clear_error() {
    let (dir, mut h) = project("assets-missing-texture");
    let (wood, _) = demo_images();
    fs::write(dir.join("downloads/a.glb"), glb_with_image(&wood, "a")).unwrap();
    import(&mut h, "downloads/a.glb", "models/a.glb");
    let name = textures(&dir)[0].clone();
    fs::remove_file(dir.path().join(format!("game/assets/textures/{name}"))).unwrap();
    let e = err(&mut h, "project.open", json!({"path": "game"}));
    assert!(
        e.contains(&format!("textures/{name}"))
            && e.contains("models/a.glb")
            && e.contains("does not exist"),
        "{e}"
    );
    // A texture of the store without its record is refused like any asset.
    fs::write(
        dir.path().join(format!("game/assets/textures/{name}")),
        &wood,
    )
    .unwrap();
    fs::remove_file(
        dir.path()
            .join(format!("game/assets/textures/{name}.license.json")),
    )
    .unwrap();
    let e = err(&mut h, "project.open", json!({"path": "game"}));
    assert!(
        e.contains(&name) && e.contains("has no licence record"),
        "{e}"
    );
}

#[test]
fn an_embedded_model_still_loads_and_imports_into_the_store() {
    // The old way: the demo pack's models hold their images.
    let (_dir, mut h) = demo("assets-old-way");
    let list = ok(&mut h, "asset.list", json!({}));
    assert_eq!(list["textures"], json!([]), "{list}");
    assert_eq!(
        (
            list["assets"][0]["triangles"].clone(),
            list["assets"][1]["triangles"].clone()
        ),
        (json!(12), json!(48))
    );
    // Imported now, the crate's image goes to the store and the model measures the same.
    let (dir, mut h) = project("assets-old-import");
    let crate_ = fs::read(examples().join("assets-demo/assets/models/crate.glb")).unwrap();
    fs::write(dir.join("downloads/crate.glb"), &crate_).unwrap();
    let done = ok(
        &mut h,
        "asset.import",
        json!({"file": "downloads/crate.glb", "to": "models/crate.glb",
               "license": "CC0-1.0", "author": "The GameCraft project",
               "source_url": "https://example.org/crate"}),
    );
    assert_eq!(done["triangles"], 12);
    assert_eq!(done["textures"].as_array().unwrap().len(), 1);
    let bounds = done["bounds"].clone();
    ok(&mut h, "project.open", json!({"path": "game"}));
    let list = ok(&mut h, "asset.list", json!({}));
    assert_eq!(list["assets"][0]["bounds"], bounds);
    assert_eq!(list["assets"][0]["triangles"], 12);
    assert_eq!(list["textures"][0]["used_by"], json!(["models/crate.glb"]));
}

#[test]
fn the_renderer_is_handed_a_shared_texture_under_its_own_path() {
    let (dir, mut h) = project("assets-provide-texture");
    let (wood, _) = demo_images();
    for (i, m) in ["a", "b"].iter().enumerate() {
        fs::write(
            dir.path().join(format!("downloads/{m}.glb")),
            glb_with_image(&wood, m),
        )
        .unwrap();
        import(
            &mut h,
            &format!("downloads/{m}.glb"),
            &format!("models/{m}.glb"),
        );
        ok(
            &mut h,
            "entity.add",
            json!({"name": m.to_uppercase(), "position": [i * 2, 0, 0],
                   "components": [{"type": "model", "asset": format!("models/{m}.glb")}]}),
        );
    }
    let name = textures(&dir)[0].clone();
    let given = Rc::new(RefCell::new(Vec::new()));
    let mut h = h.with_pictures(Box::new(Keeper {
        given: given.clone(),
    }));
    ok(&mut h, "view.render", json!({"width": 32, "height": 32}));
    let got = given.borrow().clone();
    assert!(
        got.contains(&(format!("textures/{name}"), wood.len())),
        "{got:?}"
    );
    assert!(
        got.contains(&(
            "models/a.glb".to_string(),
            fs::metadata(dir.join("game/assets/models/a.glb"))
                .unwrap()
                .len() as usize
        )),
        "{got:?}"
    );
}

// ---------------------------------------------------------------- review round

#[test]
fn a_glb_without_a_binary_chunk_imports_and_the_project_reopens() {
    let (dir, mut h) = project("assets-glb-json-only");
    let (wood, _) = demo_images();
    fs::create_dir_all(dir.path().join("downloads/tex")).unwrap();
    let (model, bin) = glb_json_only("tex/wood.png", "house.bin", "house");
    fs::write(dir.path().join("downloads/house.glb"), model).unwrap();
    fs::write(dir.path().join("downloads/house.bin"), bin).unwrap();
    fs::write(dir.path().join("downloads/tex/wood.png"), &wood).unwrap();
    let done = ok(
        &mut h,
        "asset.import",
        json!({"file": "downloads/house.glb", "to": "models/house.glb", "license": "CC0-1.0",
               "author": "Ada", "source_url": "https://example.org/house"}),
    );
    let name = textures(&dir)[0].clone();
    assert_eq!(
        done["files"],
        json!([
            format!("assets/textures/{name}"),
            format!("assets/textures/{name}.license.json"),
            "assets/models/house.bin",
            "assets/models/house.glb",
            "assets/models/house.glb.license.json"
        ]),
        "{done}"
    );
    // Still a GLB (a JSON chunk only), which the host reads again on the next open.
    let stored = fs::read(dir.path().join("game/assets/models/house.glb")).unwrap();
    assert!(stored.starts_with(b"glTF"));
    ok(&mut h, "project.open", json!({"path": "game"}));
    let list = ok(&mut h, "asset.list", json!({}));
    assert_eq!(list["assets"][0]["triangles"], 1, "{list}");
    assert_eq!(list["textures"][0]["used_by"], json!(["models/house.glb"]));
}

#[test]
fn a_buffer_file_that_held_the_image_is_rewritten_without_it() {
    let (dir, mut h) = project("assets-bin-image");
    let (wood, _) = demo_images();
    // The model's one buffer file holds the geometry and the image.
    let (text, bin) = gltf_with_image_in_buffer(&wood, "house.bin", "house");
    fs::write(dir.path().join("downloads/house.gltf"), &text).unwrap();
    fs::write(dir.path().join("downloads/house.bin"), &bin).unwrap();
    let done = ok(
        &mut h,
        "asset.import",
        json!({"file": "downloads/house.gltf", "to": "models/house.gltf",
               "license": "CC0-1.0", "author": "Ada", "source_url": "https://example.org/house"}),
    );
    assert_eq!(done["triangles"], 1, "{done}");
    assert_eq!(done["files"].as_array().unwrap().len(), 5, "{done}");
    let kept = fs::read(dir.path().join("game/assets/models/house.bin")).unwrap();
    assert_eq!(kept.len(), 42, "the geometry alone: {} bytes", kept.len());
    let stored: Value = serde_json::from_str(
        &fs::read_to_string(dir.path().join("game/assets/models/house.gltf")).unwrap(),
    )
    .unwrap();
    assert_eq!(stored["buffers"][0]["byteLength"], 42);
    assert_eq!(
        stored["images"][0]["uri"],
        format!("../textures/{}", textures(&dir)[0])
    );
    ok(&mut h, "project.open", json!({"path": "game"}));
    let list = ok(&mut h, "asset.list", json!({}));
    assert_eq!(list["assets"][0]["triangles"], 1, "{list}");
    assert_eq!(list["assets"][0]["files"][1], "assets/models/house.bin");
}

#[test]
fn a_store_file_replaced_by_hand_is_refused() {
    let (dir, mut h) = project("assets-replaced");
    let (wood, plank) = demo_images();
    fs::write(
        dir.path().join("downloads/a.glb"),
        glb_with_image(&wood, "a"),
    )
    .unwrap();
    fs::write(
        dir.path().join("downloads/b.glb"),
        glb_with_image(&wood, "b"),
    )
    .unwrap();
    import(&mut h, "downloads/a.glb", "models/a.glb");
    let name = textures(&dir)[0].clone();
    // Someone replaced the texture with other bytes under the same name.
    fs::write(
        dir.path().join(format!("game/assets/textures/{name}")),
        &plank,
    )
    .unwrap();
    let e = err(
        &mut h,
        "asset.import",
        json!({"file": "downloads/b.glb", "to": "models/b.glb", "license": "CC0-1.0",
               "author": "Ada", "source_url": "https://example.org/models"}),
    );
    assert!(
        e.contains(&format!("textures/{name}")) && e.contains("other bytes"),
        "{e}"
    );
    assert!(
        !dir.path().join("game/assets/models/b.glb").exists(),
        "nothing is written"
    );
}

#[test]
fn a_buffer_outside_the_model_s_folder_is_refused_and_nothing_is_written() {
    let (dir, mut h) = project("assets-buffer-outside");
    let (wood, _) = demo_images();
    // The source model in its own folder, its buffer one folder up.
    fs::create_dir_all(dir.path().join("downloads/src/tex")).unwrap();
    fs::create_dir_all(dir.path().join("downloads/shared")).unwrap();
    let (text, bin) = gltf_with_files("tex/wood.png", "../shared/house.bin", "house");
    fs::write(dir.path().join("downloads/src/house.gltf"), &text).unwrap();
    fs::write(dir.path().join("downloads/shared/house.bin"), &bin).unwrap();
    fs::write(dir.path().join("downloads/src/tex/wood.png"), &wood).unwrap();
    let e = err(
        &mut h,
        "asset.import",
        json!({"file": "downloads/src/house.gltf", "to": "models/house.gltf",
               "license": "CC0-1.0", "author": "Ada", "source_url": "https://example.org/house"}),
    );
    assert!(
        e.contains("shared/house.bin") && e.contains("beside the model"),
        "{e}"
    );
    // Nothing was written: not the buffer, not the model, not the texture.
    assert!(!dir.path().join("game/assets/shared").exists());
    assert!(!dir.path().join("game/assets/models").exists());
    assert!(!dir.path().join("game/assets/textures").exists());
    let list = ok(&mut h, "asset.list", json!({}));
    assert_eq!(
        (
            list["assets"].as_array().unwrap().len(),
            list["textures"].as_array().unwrap().len()
        ),
        (0, 0)
    );
}

/// A `.gltf` of unit cubes with its buffer beside it: one mesh per count in `boxes` (its
/// cubes side by side), named `<object>_LOD<n>` by `names`, showing `roots`.
fn write_levels(dir: &Path, object: &str, boxes: &[usize], roots: &[usize], extras: Value) {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    for &count in boxes {
        let offset = bin.len();
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for b in 0..count {
            for face in 0..6 {
                for corner in 0..6 {
                    // A unit cube standing on y = 0: six corners per side.
                    let p = cube_corner(face, corner, b);
                    for k in 0..3 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                        bin.extend_from_slice(&p[k].to_le_bytes());
                    }
                }
            }
        }
        views.push(json!({"buffer": 0, "byteOffset": offset, "byteLength": bin.len() - offset}));
        accessors.push(json!({
            "bufferView": views.len() - 1,
            "componentType": 5126,
            "count": count * 36,
            "type": "VEC3",
            "min": lo,
            "max": hi
        }));
        meshes.push(json!({"primitives": [{"attributes": {"POSITION": accessors.len() - 1}}]}));
    }
    let nodes: Vec<Value> = boxes
        .iter()
        .enumerate()
        .map(|(level, _)| json!({"name": format!("{object}_LOD{level}"), "mesh": level}))
        .collect();
    let mut doc = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": roots}],
        "nodes": nodes,
        "meshes": meshes,
        "buffers": [{"uri": "thing.bin", "byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": accessors
    });
    if !extras.is_null() {
        doc["asset"]["extras"] = extras;
    }
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("thing.gltf"), serde_json::to_vec(&doc).unwrap()).unwrap();
    fs::write(dir.join("thing.bin"), &bin).unwrap();
    fs::write(
        dir.join("thing.gltf.license.json"),
        r#"{"license": "CC0-1.0", "author": "a", "source_url": "u"}"#,
    )
    .unwrap();
}

/// Corner `corner` (0 to 5) of side `face` of a unit cube at `x = b`, standing on y = 0.
fn cube_corner(face: usize, corner: usize, b: usize) -> [f32; 3] {
    let (axis, sign) = [
        (0, 0.0f32),
        (0, 1.0),
        (1, 0.0),
        (1, 1.0),
        (2, 0.0),
        (2, 1.0),
    ][face];
    let (a, c) = match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let (u, v) = [
        (0.0, 0.0),
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 0.0),
        (0.0, 1.0),
        (1.0, 1.0),
    ][corner];
    let mut p = [0.0; 3];
    p[axis] = sign;
    p[a] = u;
    p[c] = v;
    p[0] += b as f32;
    p
}

#[test]
fn a_model_with_levels_of_detail_lists_them_with_their_distances() {
    let dir = TempDir::new("assets-lods");
    copy(&examples().join("assets-demo"), &dir.join("demo"));
    let models = dir.join("demo/assets/models");
    write_levels(
        &models,
        "Thing",
        &[3, 2, 1],
        &[0, 1, 2],
        json!({"gamecraft": {"draw_distance_m": 900}}),
    );
    let mut h = host_in(&dir);
    ok(&mut h, "project.open", json!({"path": "demo"}));
    let list = ok(&mut h, "asset.list", json!({}));
    let thing = list["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["path"] == "models/thing.gltf")
        .cloned()
        .unwrap();
    assert_eq!(thing["triangles"], 36, "{thing}");
    assert_eq!(thing["lod_source"], "names");
    assert_eq!(thing["lod_warnings"], json!([]));
    let lods = thing["lods"].as_array().unwrap();
    assert_eq!(lods.len(), 3, "{thing}");
    assert_eq!(
        (
            lods[0]["level"].clone(),
            lods[0]["triangles"].clone(),
            lods[1]["triangles"].clone(),
            lods[2]["triangles"].clone()
        ),
        (json!(0), json!(36), json!(24), json!(12))
    );
    // The file says how far it is drawn; from its size (sqrt(11) m across) it changes level
    // at about 11.5 m and 46 m.
    let until: Vec<f64> = lods
        .iter()
        .map(|l| l["until_m"].as_f64().unwrap())
        .collect();
    assert!((11.4..11.6).contains(&until[0]), "{until:?}");
    assert!((45.8..46.1).contains(&until[1]), "{until:?}");
    assert_eq!(thing["draw_distance_m"], 900.0);
    assert_eq!(until[2], 900.0);
    // A broken set says so.
    write_levels(&models, "Thing", &[1, 3], &[0, 1], Value::Null);
    ok(&mut h, "project.open", json!({"path": "demo"}));
    let list = ok(&mut h, "asset.list", json!({}));
    let thing = &list["assets"].as_array().unwrap()[2];
    assert_eq!(thing["path"], "models/thing.gltf");
    assert_eq!(thing["lods"].as_array().unwrap().len(), 2);
    assert!(
        thing["lod_warnings"].as_array().unwrap().iter().any(|w| w
            .as_str()
            .unwrap()
            .contains("level 1 has more triangles (36) than level 0 (12)")),
        "{thing}"
    );
}
