//! A project's sculpted terrain on disk: `terrain/edits.json` and the binary tile cache.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{TempDir, host_in, ok};
use gamecraft_automation::Host;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

/// A host in `dir` with a 512 m terrain at 2 m cells (257 x 257 samples, 2 x 2 tiles) and two
/// edits, saved as the project `game/`.
fn game(dir: &TempDir) -> Host {
    let mut h = host_in(dir);
    ok(&mut h, "file.new", json!({"name": "Game", "seed": 3}));
    ok(
        &mut h,
        "terrain.set",
        json!({"kind": "noise", "size_m": 512, "cell_m": 2, "relief_m": 20}),
    );
    ok(
        &mut h,
        "terrain.sculpt",
        json!({"op": "raise", "center": [-100, -100], "radius_m": 10, "strength_m": 4}),
    );
    ok(
        &mut h,
        "terrain.sculpt",
        json!({"op": "flatten", "center": [100, 100], "radius_m": 12}),
    );
    ok(
        &mut h,
        "project.new",
        json!({"path": "game", "id": "game", "name": "Game", "world": "village"}),
    );
    h
}

fn cache(dir: &TempDir) -> PathBuf {
    dir.join("game/worlds/village/terrain/cache")
}

#[test]
fn opening_builds_the_cache_once_and_then_reads_it() {
    let dir = TempDir::new("terrain-cache");
    let mut h = game(&dir);
    let edits = fs::read_to_string(dir.join("game/worlds/village/terrain/edits.json")).unwrap();
    assert_eq!(edits.matches("\"op\"").count(), 2, "{edits}");
    let world = fs::read_to_string(dir.join("game/worlds/village/world.json")).unwrap();
    assert!(!world.contains("\"edits\""), "{world}");
    // project.new opened it: every tile built and written, with a .gitignore.
    assert!(cache(&dir).join(".gitignore").exists());
    for tile in [
        "tile_0_0.bin",
        "tile_1_0.bin",
        "tile_0_1.bin",
        "tile_1_1.bin",
    ] {
        assert!(cache(&dir).join(tile).exists(), "{tile}");
    }
    let first = h.session().document_heights().unwrap().clone();
    let opened = ok(&mut h, "project.open", json!({"path": "game"}));
    assert_eq!(opened["terrain"]["tiles"], 4);
    assert_eq!(opened["terrain"]["from_cache"], 4, "{opened}");
    assert_eq!(opened["terrain"]["built"], 0);
    assert!(*h.session().document_heights().unwrap() == first);
    // A crater: one tile changes on save, only that tile file is written again.
    let before: Vec<Vec<u8>> = [
        "tile_0_0.bin",
        "tile_1_0.bin",
        "tile_0_1.bin",
        "tile_1_1.bin",
    ]
    .iter()
    .map(|t| fs::read(cache(&dir).join(t)).unwrap())
    .collect();
    ok(
        &mut h,
        "terrain.sculpt",
        json!({"op": "lower", "center": [-128, 128], "radius_m": 6, "strength_m": 3}),
    );
    let saved = ok(&mut h, "project.save", json!({}));
    assert_eq!(
        saved["written"],
        json!(["worlds/village/terrain/edits.json"])
    );
    assert_eq!(saved["terrain_tiles_written"], 1, "{saved}");
    let after: Vec<Vec<u8>> = [
        "tile_0_0.bin",
        "tile_1_0.bin",
        "tile_0_1.bin",
        "tile_1_1.bin",
    ]
    .iter()
    .map(|t| fs::read(cache(&dir).join(t)).unwrap())
    .collect();
    let changed: Vec<usize> = (0..4).filter(|i| before[*i] != after[*i]).collect();
    assert_eq!(changed, vec![2]);
    // Reopened: all from the cache, and the field is a fresh build's.
    let crater = h.session().document_heights().unwrap().clone();
    let opened = ok(&mut h, "project.open", json!({"path": "game"}));
    assert_eq!(opened["terrain"]["from_cache"], 4, "{opened}");
    assert!(*h.session().document_heights().unwrap() == crater);
    let fresh = h
        .session()
        .document()
        .terrain
        .as_ref()
        .unwrap()
        .build()
        .unwrap();
    assert!(crater == fresh);
}

#[test]
fn a_missing_or_broken_tile_is_built_again() {
    let dir = TempDir::new("terrain-broken");
    let mut h = game(&dir);
    let fresh = h.session().document_heights().unwrap().clone();
    fs::remove_file(cache(&dir).join("tile_1_1.bin")).unwrap();
    fs::write(cache(&dir).join("tile_0_0.bin"), b"GCTILE1\nnot a tile").unwrap();
    let opened = ok(&mut h, "project.open", json!({"path": "game"}));
    assert_eq!(opened["terrain"]["built"], 2, "{opened}");
    assert_eq!(opened["terrain"]["written"], 2);
    assert!(*h.session().document_heights().unwrap() == fresh);
    // Deleting the whole cache loses nothing.
    fs::remove_dir_all(cache(&dir)).unwrap();
    let opened = ok(&mut h, "project.open", json!({"path": "game"}));
    assert_eq!(opened["terrain"]["built"], 4);
    assert!(*h.session().document_heights().unwrap() == fresh);
}

#[test]
fn a_read_only_project_builds_without_writing() {
    let dir = TempDir::new("terrain-readonly");
    drop(game(&dir));
    fs::remove_dir_all(cache(&dir)).unwrap();
    let mut h = Host::new(
        gamecraft_engine::example_world(),
        gamecraft_automation::Access {
            read_root: Some(dir.path().to_path_buf()),
            write_root: None,
        },
    );
    let opened = ok(&mut h, "project.open", json!({"path": "game"}));
    assert_eq!(opened["terrain"]["built"], 4);
    assert_eq!(opened["terrain"]["cache"], json!(null));
    assert!(!cache(&dir).exists());
}
