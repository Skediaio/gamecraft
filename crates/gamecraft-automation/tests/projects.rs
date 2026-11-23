//! The host's project commands on real folders: `project.new`, `project.open`,
//! `project.save`, `project.export`, `project.info`, `pack.add_dependency`, `project.query`.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{TempDir, err, host_in, ok};
use gamecraft_automation::Host;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Every file under `dir` (but the index) with its text, by relative path.
fn files(dir: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel.starts_with(".gamecraft-index") || rel.contains("/terrain/cache") {
                continue;
            }
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.insert(rel, fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// A host in `dir` holding a small game world with layers, saved as the project `game/`.
fn game(dir: &TempDir) -> Host {
    let mut h = host_in(dir);
    ok(&mut h, "file.new", json!({"name": "Game", "seed": 3}));
    ok(
        &mut h,
        "prefab.add",
        json!({"id": "soldier", "components": [{"type": "tags", "tags": ["enemy"]}]}),
    );
    for (name, x) in [("Hut", 0), ("Barn", 10)] {
        ok(
            &mut h,
            "entity.add",
            json!({"name": name, "position": [x, 0, 0], "layer": "village"}),
        );
    }
    ok(
        &mut h,
        "prefab.spawn",
        json!({"prefab": "soldier", "position": [5, 0, 5], "name": "Guard", "layer": "enemies"}),
    );
    ok(
        &mut h,
        "entity.add",
        json!({"name": "Sign", "position": [1, 0, 1]}),
    );
    let made = ok(
        &mut h,
        "project.new",
        json!({"path": "game", "id": "game", "name": "Game", "world": "village"}),
    );
    assert_eq!(made["id"], json!("game"));
    h
}

#[test]
fn project_new_writes_the_folder_and_the_project_is_open_and_clean() {
    let dir = TempDir::new("project-new");
    let mut h = game(&dir);
    let on_disk = files(&dir.join("game"));
    let paths: Vec<&str> = on_disk.keys().map(String::as_str).collect();
    assert_eq!(
        paths,
        vec![
            "pack.json",
            "prefabs/soldier.json",
            "worlds/village/layers/enemies.json",
            "worlds/village/layers/main.json",
            "worlds/village/layers/village.json",
            "worlds/village/world.json",
        ]
    );
    assert!(!h.is_dirty());
    let info = ok(&mut h, "project.info", json!({}));
    assert_eq!(info["root"], json!("game"));
    assert_eq!(info["changes"]["files"], json!([]));
    let saved = ok(&mut h, "project.save", json!({}));
    assert_eq!(saved["unchanged"], json!(true));
    assert_eq!(
        files(&dir.join("game")),
        on_disk,
        "an unchanged save writes nothing"
    );
    // The document carries the layers it got from the files.
    let sign = ok(&mut h, "entity.get", json!({"id": "Sign"}));
    assert_eq!(sign["layer"], json!("main"));
    // An existing, non-empty folder is refused.
    let e = err(
        &mut h,
        "project.new",
        json!({"path": "game", "id": "game", "name": "Again"}),
    );
    assert!(e.contains("not empty"), "{e}");
}

#[test]
fn an_edit_saves_one_line_and_reopening_gives_the_edited_world() {
    let dir = TempDir::new("project-edit");
    let mut h = game(&dir);
    let before = files(&dir.join("game"));
    ok(
        &mut h,
        "entity.set",
        json!({"id": "Barn", "position": [12.5, 0, 0]}),
    );
    let info = ok(&mut h, "project.info", json!({}));
    assert_eq!(info["changes"]["entities_changed"], json!(1));
    assert_eq!(
        info["changes"]["files"],
        json!(["worlds/village/layers/village.json"])
    );
    let saved = ok(&mut h, "project.save", json!({}));
    assert_eq!(
        saved["written"],
        json!(["worlds/village/layers/village.json"])
    );
    let after = files(&dir.join("game"));
    let changed: Vec<&String> = before.keys().filter(|k| before[*k] != after[*k]).collect();
    assert_eq!(changed, vec!["worlds/village/layers/village.json"]);
    let (a, b) = (
        &before["worlds/village/layers/village.json"],
        &after["worlds/village/layers/village.json"],
    );
    let diff: Vec<(&str, &str)> = a.lines().zip(b.lines()).filter(|(x, y)| x != y).collect();
    assert_eq!(diff.len(), 1, "{diff:?}");
    assert!(!h.is_dirty());
    let mut other = host_in(&dir);
    ok(&mut other, "project.open", json!({"path": "game"}));
    let barn = ok(&mut other, "entity.get", json!({"id": "Barn"}));
    assert_eq!(barn["position"], json!([12.5, 0.0, 0.0]));
}

#[test]
fn a_mod_over_the_game_takes_the_edits_of_game_entities_as_overrides() {
    let dir = TempDir::new("project-mod");
    drop(game(&dir));
    fs::create_dir_all(dir.join("night/worlds/village/layers")).unwrap();
    fs::write(
        dir.join("night/pack.json"),
        r#"{"gamecraft_pack": 1, "id": "night", "depends": [{"id": "game", "path": "../game"}]}"#,
    )
    .unwrap();
    fs::write(
        dir.join("night/worlds/village/world.json"),
        r#"{"gamecraft_world": 1, "environment": {"time_of_day_h": 23.0}}"#,
    )
    .unwrap();
    let game_before = files(&dir.join("game"));
    let mut h = host_in(&dir);
    let opened = ok(&mut h, "project.open", json!({"path": "night"}));
    assert_eq!(opened["packs"], json!(["game", "night"]));
    assert_eq!(opened["world"], json!("village"));
    let inspect = ok(&mut h, "world.inspect", json!({"entities": false}));
    assert_eq!(inspect["environment"]["time_of_day_h"], json!(23.0));
    ok(
        &mut h,
        "entity.set",
        json!({"id": "Hut", "position": [0, 0, 3]}),
    );
    ok(&mut h, "entity.remove", json!({"id": "Sign"}));
    let saved = ok(&mut h, "project.save", json!({}));
    assert_eq!(
        saved["written"],
        json!([
            "worlds/village/layers/main.json",
            "worlds/village/layers/village.json"
        ])
    );
    assert_eq!(
        files(&dir.join("game")),
        game_before,
        "the game is never edited"
    );
    let village = fs::read_to_string(dir.join("night/worlds/village/layers/village.json")).unwrap();
    assert!(
        village.contains("\"id\":\"hut\"") && !village.contains("\"id\":\"barn\""),
        "{village}"
    );
    let main = fs::read_to_string(dir.join("night/worlds/village/layers/main.json")).unwrap();
    assert!(main.contains(r#"{"id":"sign","removed":true}"#), "{main}");
    // Exported as one file, the world keeps its layers.
    let out = ok(&mut h, "project.export", json!({"path": "night.gamecraft"}));
    assert_eq!(out["entities"], json!(3));
    let text = fs::read_to_string(dir.join("night.gamecraft")).unwrap();
    assert!(text.contains("\"layer\": \"village\""));
    let e = err(&mut h, "project.export", json!({"path": "night.gamecraft"}));
    assert!(e.contains("overwrite"), "{e}");
}

#[test]
fn add_dependency_layers_a_pack_below_and_refuses_what_cannot_load() {
    let dir = TempDir::new("project-deps");
    drop(game(&dir));
    let mut h = host_in(&dir);
    ok(&mut h, "file.new", json!({"name": "Tools"}));
    ok(
        &mut h,
        "project.new",
        json!({"path": "tools", "id": "tools", "name": "Tools"}),
    );
    let manifest_before = fs::read_to_string(dir.join("tools/pack.json")).unwrap();
    let e = err(
        &mut h,
        "pack.add_dependency",
        json!({"id": "nature", "path": "../nature"}),
    );
    assert!(
        e.contains("`nature` is missing") && e.contains("tools -> nature"),
        "{e}"
    );
    assert_eq!(
        fs::read_to_string(dir.join("tools/pack.json")).unwrap(),
        manifest_before
    );
    let e = err(
        &mut h,
        "pack.add_dependency",
        json!({"id": "other", "path": "../game"}),
    );
    assert!(e.contains("`game`") && e.contains("other"), "{e}");
    ok(
        &mut h,
        "entity.add",
        json!({"name": "Unsaved", "position": [0, 0, 0]}),
    );
    let e = err(
        &mut h,
        "pack.add_dependency",
        json!({"id": "game", "path": "../game"}),
    );
    assert!(e.contains("unsaved"), "{e}");
    ok(&mut h, "edit.undo", json!({}));
    ok(&mut h, "project.save", json!({}));
    let added = ok(
        &mut h,
        "pack.add_dependency",
        json!({"id": "game", "path": "../game"}),
    );
    assert_eq!(added["packs"], json!(["game", "tools"]));
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("tools/pack.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["depends"],
        json!([{"id": "game", "path": "../game"}])
    );
    // A cycle: the game depending on the tools.
    let mut g = host_in(&dir);
    ok(&mut g, "project.open", json!({"path": "game"}));
    let e = err(
        &mut g,
        "pack.add_dependency",
        json!({"id": "tools", "path": "../tools"}),
    );
    assert!(e.contains("cycle: game -> tools -> game"), "{e}");
}

#[test]
fn hostile_folders_are_refused_and_the_session_stays() {
    let dir = TempDir::new("project-hostile");
    drop(game(&dir));
    let mut h = host_in(&dir);
    let before = ok(&mut h, "world.inspect", json!({"entities": false}));
    let e = err(&mut h, "project.open", json!({"path": "../somewhere"}));
    assert!(e.contains("`..` is not allowed"), "{e}");
    let e = err(&mut h, "project.open", json!({"path": "missing"}));
    assert!(e.contains("does not exist"), "{e}");
    fs::create_dir_all(dir.join("empty")).unwrap();
    let e = err(&mut h, "project.open", json!({"path": "empty"}));
    assert!(e.contains("no pack.json"), "{e}");
    // A layer that is a link out of the pack.
    let outside = TempDir::new("project-outside");
    fs::write(
        outside.join("evil.json"),
        r#"{"gamecraft_layer": 1, "entities": []}"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            outside.join("evil.json"),
            dir.join("game/worlds/village/layers/evil.json"),
        )
        .unwrap();
        let e = err(&mut h, "project.open", json!({"path": "game"}));
        assert!(e.contains("link that leads out of its pack"), "{e}");
        fs::remove_file(dir.join("game/worlds/village/layers/evil.json")).unwrap();
    }
    // Truncated JSON names the file.
    let layer = dir.join("game/worlds/village/layers/village.json");
    let text = fs::read_to_string(&layer).unwrap();
    fs::write(&layer, &text[..text.len() / 2]).unwrap();
    let e = err(&mut h, "project.open", json!({"path": "game"}));
    assert!(
        e.contains("village.json") && e.contains("not a valid layer file"),
        "{e}"
    );
    fs::write(&layer, &text).unwrap();
    // A dependency outside the read root.
    fs::create_dir_all(dir.join("far")).unwrap();
    fs::write(
        dir.join("far/pack.json"),
        format!(
            r#"{{"gamecraft_pack": 1, "id": "far", "depends": [{{"id": "x", "path": "{}"}}]}}"#,
            outside.path().display()
        ),
    )
    .unwrap();
    fs::create_dir_all(outside.join("x")).unwrap();
    fs::write(
        outside.join("x/pack.json"),
        r#"{"gamecraft_pack": 1, "id": "x", "worlds": ["w"]}"#,
    )
    .unwrap();
    let e = err(&mut h, "project.open", json!({"path": "far"}));
    assert!(e.contains("outside the read root"), "{e}");
    assert_eq!(
        ok(&mut h, "world.inspect", json!({"entities": false})),
        before
    );
    let e = err(&mut h, "project.info", json!({}));
    assert!(e.contains("no project is open"), "{e}");
}

#[test]
fn query_needs_an_index_and_file_open_closes_the_project() {
    let dir = TempDir::new("project-query");
    let mut h = game(&dir);
    let e = err(&mut h, "project.query", json!({"sql": "SELECT 1"}));
    assert!(e.contains("no project index"), "{e}");
    ok(&mut h, "file.save", json!({"path": "plain.gamecraft"}));
    ok(&mut h, "file.open", json!({"path": "plain.gamecraft"}));
    let e = err(&mut h, "project.save", json!({}));
    assert!(e.contains("no project is open"), "{e}");
}

#[test]
fn project_open_cannot_be_part_of_an_atomic_batch() {
    let dir = TempDir::new("project-atomic");
    drop(game(&dir));
    let mut h = host_in(&dir);
    let reply = common::ask(
        &mut h,
        json!({"id": 1, "method": "batch", "params": {"atomic": true, "steps": [
            {"command": "project.open", "params": {"path": "game"}}
        ]}}),
    );
    let text = reply.to_string();
    assert!(text.contains("cannot be part of an atomic batch"), "{text}");
}

#[test]
fn a_project_in_cells_on_disk_opens_in_part_and_switches_layout() {
    let dir = TempDir::new("cells");
    let mut h = host_in(&dir);
    ok(&mut h, "file.new", json!({}));
    for (name, x, z) in [("A", 10.0, 10.0), ("B", 100.0, 10.0), ("C", -10.0, 200.0)] {
        ok(
            &mut h,
            "entity.add",
            json!({"name": name, "position": [x, 0.0, z]}),
        );
    }
    let err_size = err(
        &mut h,
        "project.new",
        json!({"path": "p", "id": "p", "name": "P", "cell_size_m": 3}),
    );
    assert!(err_size.contains("cell_size_m"), "{err_size}");
    ok(
        &mut h,
        "project.new",
        json!({"path": "p", "id": "p", "name": "P", "cell_size_m": 64}),
    );
    let layer = dir.join("p/worlds/main/layers/main");
    let mut cells: Vec<String> = std::fs::read_dir(&layer)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    cells.sort();
    assert_eq!(cells, ["-1_3.json", "0_0.json", "1_0.json"]);

    let area = ok(
        &mut h,
        "project.cells",
        json!({"center": [0, 0], "radius_m": 70}),
    );
    assert_eq!(area["cell_size_m"], 64.0);
    assert_eq!(area["count"], 16);
    let e = err(&mut h, "project.cells", json!({"min": [0, 0]}));
    assert!(e.contains("center"), "{e}");

    let part = ok(
        &mut h,
        "project.open",
        json!({"path": "p", "cells": [[0, 0], [1, 0]]}),
    );
    assert_eq!(part["entities"], 2);
    assert_eq!(part["cells"], json!([[0, 0], [1, 0]]));
    let e = err(&mut h, "project.save", json!({}));
    assert!(e.contains("open it whole"), "{e}");
    let e = err(&mut h, "project.open", json!({"path": "p", "cells": [[0]]}));
    assert!(e.contains("cells[0]"), "{e}");

    let whole = ok(&mut h, "project.open", json!({"path": "p"}));
    assert_eq!(whole["entities"], 3);
    assert_eq!(whole["cell_size_m"], 64.0);
    let saved = ok(&mut h, "project.save", json!({"cell_size_m": 0}));
    assert_eq!(saved["removed"].as_array().unwrap().len(), 3, "{saved}");
    assert!(dir.join("p/worlds/main/layers/main.json").exists());
    assert_eq!(std::fs::read_dir(&layer).map_or(0, |d| d.count()), 0);
    let again = ok(&mut h, "project.open", json!({"path": "p"}));
    assert_eq!(again["entities"], 3);
    assert_eq!(again["cell_size_m"], Value::Null);
}
