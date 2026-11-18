//! `file.new`, `file.open`, `file.save`: the document round-trips, and every path stays inside
//! its root.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use gamecraft_automation::{Access, Host};
use gamecraft_world::World;
use serde_json::json;

#[test]
fn save_then_open_gives_the_same_document() {
    let dir = TempDir::new("roundtrip");
    let mut h = host_in(&dir);
    ok(&mut h, "entity.add", json!({"name": "Rock", "at": [10, 5]}));
    ok(
        &mut h,
        "environment.set",
        json!({"time_of_day_h": 7.25, "rain": 0.3}),
    );
    let document = h.session().document().clone();
    assert!(h.is_dirty());
    assert_eq!(h.path(), None);

    let saved = ok(&mut h, "file.save", json!({"path": "valley.gamecraft"}));
    let bytes = std::fs::metadata(dir.join("valley.gamecraft"))
        .unwrap()
        .len();
    assert_eq!(saved["bytes"], bytes);
    assert!(
        saved["path"]
            .as_str()
            .unwrap()
            .ends_with("valley.gamecraft")
    );
    assert!(!h.is_dirty());
    assert!(h.path().unwrap().ends_with("valley.gamecraft"));
    // Nothing else is left in the folder (no temporary file).
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["valley.gamecraft"]);

    let new = ok(&mut h, "file.new", json!({"name": "Blank", "seed": 9}));
    assert_eq!(new, json!({"name": "Blank", "seed": 9}));
    assert_eq!(h.session().document().entity_count(), 0);
    assert_eq!(h.path(), None);
    assert!(!h.session().can_undo());

    let opened = ok(&mut h, "file.open", json!({"path": "valley.gamecraft"}));
    assert_eq!(opened["name"], "Example valley");
    assert_eq!(opened["entities"], 4);
    assert_eq!(opened["terrain"], true);
    assert!(
        opened["path"]
            .as_str()
            .unwrap()
            .ends_with("valley.gamecraft")
    );
    assert_eq!(h.session().document(), &document);
    assert!(!h.session().can_undo());
    assert!(!h.is_dirty());

    // Without a path, the open file.
    ok(&mut h, "world.set", json!({"name": "Renamed"}));
    ok(&mut h, "file.save", json!({}));
    let text = std::fs::read_to_string(dir.join("valley.gamecraft")).unwrap();
    assert_eq!(World::from_json(&text).unwrap().name, "Renamed");
    // An absolute path inside the root works too.
    let absolute = dir.join("copy.gamecraft");
    ok(
        &mut h,
        "file.save",
        json!({"path": absolute.to_str().unwrap()}),
    );
    assert!(absolute.exists());
}

#[test]
fn file_new_stops_the_simulation_and_forgets_the_file() {
    let dir = TempDir::new("new");
    let mut h = host_in(&dir);
    ok(&mut h, "file.save", json!({"path": "a.gamecraft"}));
    ok(&mut h, "sim.start", json!({}));
    let r = ok(&mut h, "file.new", json!({}));
    assert_eq!(r, json!({"name": "Untitled", "seed": 0}));
    assert!(!h.session().is_playing());
    assert_eq!(h.path(), None);
    let error = err(&mut h, "file.save", json!({}));
    assert!(error.contains("give `path`"), "{error}");
}

#[test]
fn saving_while_playing_writes_the_document() {
    let dir = TempDir::new("playing");
    let mut h = host_in(&dir);
    let document = h.session().document().clone();
    ok(&mut h, "sim.start", json!({}));
    ok(
        &mut h,
        "sim.input",
        json!({"entity": "Scout", "forward": 1}),
    );
    ok(&mut h, "sim.step", json!({"ticks": 60}));
    ok(
        &mut h,
        "entity.add",
        json!({"name": "Dropped", "position": [5, 0, 5]}),
    );
    assert_eq!(h.session().current_world().entity_count(), 4);
    ok(&mut h, "file.save", json!({"path": "play.gamecraft"}));
    let text = std::fs::read_to_string(dir.join("play.gamecraft")).unwrap();
    assert_eq!(World::from_json(&text).unwrap(), document);
    assert!(h.session().is_playing());
}

#[test]
fn without_a_root_files_are_refused_naming_the_flag() {
    let mut h = host();
    let error = err(&mut h, "file.save", json!({"path": "a.gamecraft"}));
    assert!(error.starts_with("file.save: "), "{error}");
    assert!(error.contains("--automation-write-root"), "{error}");
    let error = err(&mut h, "file.open", json!({"path": "a.gamecraft"}));
    assert!(error.contains("--automation-read-root"), "{error}");
    let list = h.command_docs();
    assert!(list.iter().any(|d| d.id == "file.open"));
}

#[test]
fn paths_outside_the_root_are_refused() {
    let dir = TempDir::new("root");
    let other = TempDir::new("other");
    std::fs::write(
        other.join("w.gamecraft"),
        World::new("x", 0).to_json().unwrap(),
    )
    .unwrap();
    let mut h = host_in(&dir);

    let outside = other.join("w.gamecraft");
    let error = err(
        &mut h,
        "file.save",
        json!({"path": outside.to_str().unwrap(), "overwrite": true}),
    );
    assert!(error.contains("outside the write root"), "{error}");
    let error = err(
        &mut h,
        "file.open",
        json!({"path": outside.to_str().unwrap()}),
    );
    assert!(error.contains("outside the read root"), "{error}");

    let error = err(&mut h, "file.save", json!({"path": "../x.gamecraft"}));
    assert!(error.contains("`..`"), "{error}");
    let error = err(&mut h, "file.open", json!({"path": "sub/../x.gamecraft"}));
    assert!(error.contains("`..`"), "{error}");

    let error = err(&mut h, "file.save", json!({"path": "missing/x.gamecraft"}));
    assert!(error.contains("does not exist"), "{error}");
    let error = err(&mut h, "file.save", json!({"path": ""}));
    assert!(error.contains("empty"), "{error}");
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn a_link_out_of_the_root_is_refused() {
    let dir = TempDir::new("links");
    let other = TempDir::new("target");
    std::fs::write(
        other.join("w.gamecraft"),
        World::new("x", 0).to_json().unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(other.path(), dir.join("out")).unwrap();
    std::os::unix::fs::symlink(other.join("w.gamecraft"), dir.join("link.gamecraft")).unwrap();
    let mut h = host_in(&dir);
    let error = err(&mut h, "file.save", json!({"path": "out/new.gamecraft"}));
    assert!(error.contains("outside the write root"), "{error}");
    assert!(!other.join("new.gamecraft").exists());
    let error = err(&mut h, "file.open", json!({"path": "out/w.gamecraft"}));
    assert!(error.contains("outside the read root"), "{error}");
    let error = err(&mut h, "file.open", json!({"path": "link.gamecraft"}));
    assert!(error.contains("outside the read root"), "{error}");
    let error = err(
        &mut h,
        "file.save",
        json!({"path": "link.gamecraft", "overwrite": true}),
    );
    assert!(error.contains("outside the write root"), "{error}");
}

#[test]
fn the_extension_must_be_gamecraft() {
    let dir = TempDir::new("ext");
    let mut h = host_in(&dir);
    for path in ["world.json", "world", "world.gamecraft.txt"] {
        let error = err(&mut h, "file.save", json!({"path": path}));
        assert!(error.contains(".gamecraft"), "{error}");
        let error = err(&mut h, "file.open", json!({"path": path}));
        assert!(error.contains(".gamecraft"), "{error}");
    }
}

#[test]
fn an_existing_other_file_needs_overwrite() {
    let dir = TempDir::new("overwrite");
    std::fs::write(dir.join("taken.gamecraft"), "old").unwrap();
    let mut h = host_in(&dir);
    let error = err(&mut h, "file.save", json!({"path": "taken.gamecraft"}));
    assert!(error.contains("overwrite"), "{error}");
    assert_eq!(
        std::fs::read_to_string(dir.join("taken.gamecraft")).unwrap(),
        "old"
    );
    ok(
        &mut h,
        "file.save",
        json!({"path": "taken.gamecraft", "overwrite": true}),
    );
    // The open file is its own: saving it again needs no overwrite.
    ok(&mut h, "file.save", json!({"path": "taken.gamecraft"}));
}

#[test]
fn a_bad_file_is_an_error_and_the_session_stays() {
    let dir = TempDir::new("bad");
    std::fs::write(dir.join("text.gamecraft"), "not json at all").unwrap();
    std::fs::write(dir.join("version.gamecraft"), r#"{"gamecraft": 99}"#).unwrap();
    std::fs::write(dir.join("binary.gamecraft"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
    std::fs::create_dir(dir.join("folder.gamecraft")).unwrap();
    let big = std::fs::File::create(dir.join("big.gamecraft")).unwrap();
    big.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(big);
    let mut h = host_in(&dir);
    ok(
        &mut h,
        "entity.add",
        json!({"name": "Kept", "position": [0, 0, 0]}),
    );
    let document = h.session().document().clone();
    let revision = h.session().revision();
    for (path, says) in [
        ("text.gamecraft", "text.gamecraft"),
        ("version.gamecraft", "version"),
        ("binary.gamecraft", "UTF-8"),
        ("folder.gamecraft", "folder.gamecraft"),
        ("big.gamecraft", "64 MB"),
        ("missing.gamecraft", "missing.gamecraft"),
    ] {
        let error = err(&mut h, "file.open", json!({"path": path}));
        assert!(error.starts_with("file.open: "), "{error}");
        assert!(error.contains(says), "{path}: {error}");
        assert_eq!(h.session().document(), &document);
        assert_eq!(h.session().revision(), revision);
        assert!(h.session().can_undo());
    }
}

#[test]
fn a_root_that_does_not_exist_is_an_error() {
    let mut h = Host::new(
        World::new("x", 0),
        Access {
            read_root: Some(std::env::temp_dir().join("gamecraft-automation-no-such-root-xyz")),
            write_root: None,
        },
    );
    let error = err(&mut h, "file.open", json!({"path": "a.gamecraft"}));
    assert!(error.contains("read root"), "{error}");
}
