//! A world file finds its pack: `file.open` of a `.gamecraft` file inside a pack (or with
//! `pack`, or with the host's pack) takes the pack's assets, so the models' bounds reach the
//! session and solid models collide, the renderer is handed their files, and `asset.list` and
//! `asset.credits` list them; a pack that fails its licence gate is refused by name; a world
//! whose models no pack provides is said in one sentence; `project.export` says which files its
//! world needs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{TempDir, err, host_in, host_with_pictures, ok};
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

/// A flat yard with a solid crate at the origin and a character 6 m south of it.
fn yard(h: &mut Host) {
    ok(h, "file.new", json!({"name": "Yard", "seed": 3}));
    ok(
        h,
        "terrain.set",
        json!({"kind": "flat", "size_m": 64, "cell_m": 1}),
    );
    ok(
        h,
        "entity.add",
        json!({"name": "Crate", "position": [0, 0, 0],
               "components": [{"type": "model", "asset": "models/crate.glb", "solid": true}]}),
    );
    ok(
        h,
        "entity.add",
        json!({"name": "Walker", "at": [0, 6], "components": [{"type": "character"}]}),
    );
}

/// Saves the yard as `path` and returns the host it was made with.
fn saved_in(dir: &TempDir, path: &str) -> Host {
    let mut h = host_in(dir);
    yard(&mut h);
    ok(&mut h, "file.save", json!({"path": path}));
    h
}

/// Remembers which asset files it was handed.
struct Keeper {
    given: Rc<RefCell<Vec<String>>>,
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
            info: json!({"renderer": "fake", "problems": []}),
        })
    }

    fn provide_asset(&mut self, path: &str, _bytes: Vec<u8>) {
        self.given.borrow_mut().push(path.to_string());
    }
}

/// The lines of `problems` that mention `needle`.
fn problems_mentioning(answer: &Value, needle: &str) -> Vec<String> {
    answer["problems"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .filter(|p| p.contains(needle))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn z_of(h: &mut Host, name: &str) -> f64 {
    let e = ok(h, "entity.get", json!({"id": name}));
    e["position"][2].as_f64().unwrap()
}

#[test]
fn a_world_file_inside_a_pack_opens_with_the_pack_s_assets() {
    let dir = TempDir::new("world-file-pack");
    copy(&examples().join("assets-demo"), &dir.join("demo"));
    let h = saved_in(&dir, "demo/loose.gamecraft");
    // Saved, the world has no assets yet: it is the file that finds its pack when it opens.
    assert!(
        h.session()
            .document()
            .model_bounds("models/crate.glb")
            .is_none()
    );
    let given = Rc::new(RefCell::new(Vec::new()));
    let mut h = h.with_pictures(Box::new(Keeper {
        given: given.clone(),
    }));
    let opened = ok(&mut h, "file.open", json!({"path": "demo/loose.gamecraft"}));
    let pack = &opened["pack"];
    assert_eq!(pack["found"], "above", "{opened}");
    assert!(pack["path"].as_str().unwrap().ends_with("/demo"), "{pack}");
    assert_eq!(pack["id"], "assets-demo");
    assert_eq!(pack["packs"], json!(["assets-demo"]));
    assert_eq!(pack["assets"], 2);
    // The crate's bounds reached the session: a solid model collides from the first tick on.
    let bounds = h
        .session()
        .document()
        .model_bounds("models/crate.glb")
        .cloned()
        .expect("the crate's bounds");
    assert!((bounds.max[1] - 0.9).abs() < 1e-6);
    // The renderer is handed the crate's file (once).
    let picture = ok(&mut h, "view.render", json!({"width": 32, "height": 32}));
    ok(&mut h, "view.render", json!({"width": 32, "height": 32}));
    assert_eq!(*given.borrow(), vec!["models/crate.glb".to_string()]);
    assert!(
        problems_mentioning(&picture, "crate").is_empty(),
        "{picture}"
    );
    // The Walker walks north and stops at the crate's south face.
    ok(&mut h, "sim.start", json!({}));
    ok(
        &mut h,
        "sim.input",
        json!({"entity": "Walker", "forward": 1, "yaw_deg": 0}),
    );
    ok(&mut h, "sim.step", json!({"ticks": 600}));
    let walker = z_of(&mut h, "Walker");
    assert!((walker - 0.75).abs() < 1e-6, "Walker at z {walker}");
    // The pack's assets are listed and credited as a project's would be.
    let list = ok(&mut h, "asset.list", json!({}));
    let paths: Vec<&str> = list["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["models/crate.glb", "models/fence.glb"]);
    assert_eq!(list["assets"][0]["used_by"], 1);
    assert_eq!(list["missing"], json!([]));
    assert_eq!(
        ok(&mut h, "asset.credits", json!({})),
        json!({"text": "", "assets": []})
    );
    // A new world closes the pack again.
    ok(&mut h, "file.new", json!({"name": "Elsewhere"}));
    let e = err(&mut h, "asset.list", json!({}));
    assert!(e.contains("no project is open"), "{e}");
}

#[test]
fn a_pack_is_given_or_refused_for_a_world_file() {
    let dir = TempDir::new("world-file-given");
    copy(&examples().join("assets-demo"), &dir.join("demo"));
    fs::create_dir_all(dir.join("not-a-pack")).unwrap();
    let h = saved_in(&dir, "loose.gamecraft");
    fs::copy(
        dir.join("loose.gamecraft"),
        dir.join("demo/loose.gamecraft"),
    )
    .unwrap();
    let given = Rc::new(RefCell::new(Vec::new()));
    let mut h = h.with_pictures(Box::new(Keeper {
        given: given.clone(),
    }));
    // Outside every pack: no assets, and the renderer is handed nothing.
    let opened = ok(&mut h, "file.open", json!({"path": "loose.gamecraft"}));
    assert_eq!(opened["pack"], Value::Null, "{opened}");
    assert!(
        h.session()
            .document()
            .model_bounds("models/crate.glb")
            .is_none()
    );
    ok(&mut h, "view.render", json!({"width": 32, "height": 32}));
    assert!(given.borrow().is_empty());
    // Given: that pack's assets.
    let opened = ok(
        &mut h,
        "file.open",
        json!({"path": "loose.gamecraft", "pack": "demo"}),
    );
    assert_eq!(opened["pack"]["found"], "given", "{opened}");
    assert_eq!(opened["pack"]["id"], "assets-demo");
    assert!(
        h.session()
            .document()
            .model_bounds("models/crate.glb")
            .is_some()
    );
    ok(&mut h, "view.render", json!({"width": 32, "height": 32}));
    assert_eq!(*given.borrow(), vec!["models/crate.glb".to_string()]);
    // Given as null: none, even above a pack.
    let opened = ok(
        &mut h,
        "file.open",
        json!({"path": "demo/loose.gamecraft", "pack": null}),
    );
    assert_eq!(opened["pack"], Value::Null, "{opened}");
    assert!(
        h.session()
            .document()
            .model_bounds("models/crate.glb")
            .is_none()
    );
    // What is not a pack is refused, and the session stays as it was.
    let before = h.session().document().clone();
    for (pack, needle) in [
        ("nowhere", "does not exist"),
        ("loose.gamecraft", "not a project folder"),
        ("not-a-pack", "pack.json"),
    ] {
        let e = err(
            &mut h,
            "file.open",
            json!({"path": "loose.gamecraft", "pack": pack}),
        );
        assert!(e.contains(needle), "{pack}: {e}");
        assert_eq!(*h.session().document(), before);
    }
    let e = err(
        &mut h,
        "file.open",
        json!({"path": "loose.gamecraft", "pack": 7}),
    );
    assert!(e.contains("`pack`"), "{e}");
}

#[test]
fn the_host_s_pack_is_used_where_none_is_given() {
    let dir = TempDir::new("world-file-default");
    copy(&examples().join("assets-demo"), &dir.join("demo"));
    saved_in(&dir, "loose.gamecraft");
    fs::copy(
        dir.join("loose.gamecraft"),
        dir.join("demo/loose.gamecraft"),
    )
    .unwrap();
    // The host's pack stands where `file.open` names none, above a pack or not.
    let mut h = host_in(&dir).with_pack(dir.join("demo"));
    for path in ["loose.gamecraft", "demo/loose.gamecraft"] {
        let opened = ok(&mut h, "file.open", json!({"path": path}));
        assert_eq!(opened["pack"]["found"], "default", "{opened}");
        assert_eq!(opened["pack"]["id"], "assets-demo");
        assert!(
            h.session()
                .document()
                .model_bounds("models/crate.glb")
                .is_some()
        );
    }
    // Given, it is the given one that counts; given null, none.
    let opened = ok(
        &mut h,
        "file.open",
        json!({"path": "loose.gamecraft", "pack": "demo"}),
    );
    assert_eq!(opened["pack"]["found"], "given");
    let opened = ok(
        &mut h,
        "file.open",
        json!({"path": "loose.gamecraft", "pack": null}),
    );
    assert_eq!(opened["pack"], Value::Null);
    // A host's pack that is not one is refused when a world file would use it.
    let mut h = host_in(&dir).with_pack(dir.join("nowhere"));
    let e = err(&mut h, "file.open", json!({"path": "loose.gamecraft"}));
    assert!(e.contains("nowhere") && e.contains("does not exist"), "{e}");
}

#[test]
fn a_pack_s_dependencies_bring_their_assets() {
    let dir = TempDir::new("world-file-deps");
    copy(&examples().join("skirmish"), &dir.join("skirmish"));
    copy(
        &examples().join("skirmish-night-mod"),
        &dir.join("skirmish-night-mod"),
    );
    saved_in(&dir, "skirmish-night-mod/loose.gamecraft");
    let mut h = host_in(&dir);
    let opened = ok(
        &mut h,
        "file.open",
        json!({"path": "skirmish-night-mod/loose.gamecraft"}),
    );
    assert_eq!(opened["pack"]["id"], "skirmish-night", "{opened}");
    assert_eq!(
        opened["pack"]["packs"],
        json!(["skirmish", "skirmish-night"])
    );
    // The crate is the base pack's.
    let bounds = h
        .session()
        .document()
        .model_bounds("models/crate.glb")
        .cloned()
        .expect("the crate's bounds");
    assert!((bounds.max[1] - 0.9).abs() < 1e-6);
    let list = ok(&mut h, "asset.list", json!({}));
    assert_eq!(list["assets"][0]["path"], "models/crate.glb");
    assert_eq!(list["assets"][0]["pack"], "skirmish");
}

#[test]
fn a_broken_pack_above_a_world_file_is_refused_by_name() {
    let dir = TempDir::new("world-file-gate");
    copy(&examples().join("assets-demo"), &dir.join("demo"));
    let mut h = saved_in(&dir, "demo/loose.gamecraft");
    fs::remove_file(dir.join("demo/assets/models/fence.glb.license.json")).unwrap();
    let before = h.session().document().clone();
    let e = err(&mut h, "file.open", json!({"path": "demo/loose.gamecraft"}));
    assert!(
        e.contains("fence.glb")
            && e.contains("has no licence record")
            && e.contains("`pack: null`"),
        "{e}"
    );
    assert_eq!(
        *h.session().document(),
        before,
        "the session stays as it was"
    );
    // The world opens without its pack's assets when that is asked for.
    let opened = ok(
        &mut h,
        "file.open",
        json!({"path": "demo/loose.gamecraft", "pack": null}),
    );
    assert_eq!(opened["pack"], Value::Null);
}

#[test]
fn a_world_whose_models_no_pack_provides_is_said_in_one_sentence() {
    let dir = TempDir::new("world-file-said");
    copy(&examples().join("assets-demo"), &dir.join("demo"));
    let h = saved_in(&dir, "loose.gamecraft");
    let (mut h, _seen) = host_with_pictures(h);
    ok(&mut h, "file.open", json!({"path": "loose.gamecraft"}));
    ok(
        &mut h,
        "entity.add",
        json!({"name": "Ghost crate", "position": [3, 0, 0],
               "components": [{"type": "model", "asset": "models/ghost.glb"}]}),
    );
    let answer = ok(&mut h, "view.render", json!({"width": 32, "height": 32}));
    let said = problems_mentioning(&answer, "no pack provides");
    assert_eq!(said.len(), 1, "{answer}");
    let line = &said[0];
    assert!(
        line.contains("models/crate.glb")
            && line.contains("models/ghost.glb")
            && line.contains("--pack")
            && line.ends_with('.'),
        "{line}"
    );
    // With the pack, only what the pack lacks is said (the ghost is saved into the file first).
    ok(
        &mut h,
        "file.save",
        json!({"path": "loose.gamecraft", "overwrite": true}),
    );
    ok(
        &mut h,
        "file.open",
        json!({"path": "loose.gamecraft", "pack": "demo"}),
    );
    let answer = ok(&mut h, "view.render", json!({"width": 32, "height": 32}));
    let said = problems_mentioning(&answer, "models/");
    assert_eq!(said.len(), 1, "{answer}");
    assert!(
        said[0].contains("models/ghost.glb") && !said[0].contains("crate"),
        "{said:?}"
    );
    // A world without models says nothing about a pack.
    ok(&mut h, "file.new", json!({"name": "Bare"}));
    let answer = ok(&mut h, "view.render", json!({"width": 32, "height": 32}));
    assert!(problems_mentioning(&answer, "pack").is_empty(), "{answer}");
}

#[test]
fn project_export_says_which_files_its_world_needs() {
    let dir = TempDir::new("world-file-export");
    copy(&examples().join("assets-demo"), &dir.join("demo"));
    let mut h = host_in(&dir);
    ok(&mut h, "project.open", json!({"path": "demo"}));
    let exported = ok(
        &mut h,
        "project.export",
        json!({"path": "yard.gamecraft", "overwrite": true}),
    );
    let pack = &exported["pack"];
    assert!(
        pack["path"].as_str().unwrap().ends_with("/demo"),
        "{exported}"
    );
    assert_eq!(pack["id"], "assets-demo");
    assert_eq!(pack["packs"], json!(["assets-demo"]));
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
    // The exported file draws its models from inside the pack: export it there.
    let exported = ok(
        &mut h,
        "project.export",
        json!({"path": "demo/yard.gamecraft", "overwrite": true}),
    );
    assert!(exported["needs"].as_array().is_some(), "{exported}");
    let given = Rc::new(RefCell::new(Vec::new()));
    let mut h = h.with_pictures(Box::new(Keeper {
        given: given.clone(),
    }));
    let opened = ok(&mut h, "file.open", json!({"path": "demo/yard.gamecraft"}));
    assert_eq!(opened["pack"]["found"], "above");
    ok(&mut h, "view.render", json!({"width": 32, "height": 32}));
    let mut got = given.borrow().clone();
    got.sort();
    assert_eq!(got, ["models/crate.glb", "models/fence.glb"]);
}
