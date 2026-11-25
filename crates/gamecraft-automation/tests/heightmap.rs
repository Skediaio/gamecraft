//! A project's heightmap on disk: `terrain.import` of an elevation file, the sample tiles
//! under `worlds/<world>/terrain/heightmap/`, and opening a project that names them.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

/// The names of the files in `folder`, sorted.
fn read_names(folder: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

use common::{TempDir, err, host_in, ok};
use serde_json::{Value, json};
use std::fs;

/// The side of a heightmap tile in cells; a tile holds one more sample per side.
const TILE_CELLS: usize = 128;
/// Bytes of a tile file: the header, the tile's place, and the samples (f32).
const TILE_BYTES: usize = 10 + 8 + (TILE_CELLS + 1) * (TILE_CELLS + 1) * 4;

/// A 16-bit PGM (P5, big-endian samples) of `cols` x `rows`: `raw(c, r)`.
fn pgm16(cols: usize, rows: usize, raw: impl Fn(usize, usize) -> u16) -> Vec<u8> {
    let mut out = format!("P5\n# made by a test\n{cols} {rows}\n65535\n").into_bytes();
    for r in 0..rows {
        for c in 0..cols {
            out.extend_from_slice(&raw(c, r).to_be_bytes());
        }
    }
    out
}

const N: usize = TILE_CELLS + 1;

/// The raw sample `(c, r)` of the test's heightmap.
fn raw(c: usize, r: usize) -> u16 {
    (c * 131 + r * 7) as u16
}

/// The same sample in metres: raw * 0.5 - 10.
fn metres(c: usize, r: usize) -> f64 {
    f64::from(raw(c, r)) * 0.5 - 10.0
}

/// `x` of column `c` on a terrain of 1024 m with `N` samples a side (8 m apart): a node of
/// the field, whose cells are 2 m.
fn at(c: usize) -> f64 {
    -512.0 + (c * 8) as f64
}

fn ground(h: &mut gamecraft_automation::Host, x: f64, z: f64) -> Value {
    ok(h, "query.ground_height", json!({"x": x, "z": z}))["height_m"].clone()
}

#[test]
fn a_pgm_becomes_the_terrain_and_the_project_keeps_its_samples() {
    let dir = TempDir::new("heightmap");
    fs::write(dir.join("dem.pgm"), pgm16(N, N, raw)).unwrap();
    let mut h = host_in(&dir);
    ok(&mut h, "file.new", json!({"name": "Map", "seed": 1}));
    let r = ok(
        &mut h,
        "terrain.import",
        json!({"path": "dem.pgm", "scale_m": 0.5, "offset_m": -10.0, "size_m": 1024}),
    );
    assert_eq!(
        (r["cols"].as_u64(), r["rows"].as_u64()),
        (Some(N as u64), Some(N as u64))
    );
    let content = r["content"].as_str().expect("the content hash").to_string();
    assert_eq!(content.len(), 16, "{content}");
    assert_eq!(r["min_height_m"], metres(0, 0));
    assert_eq!(r["max_height_m"], metres(N - 1, N - 1));
    assert_eq!(r["tiles"], 1);
    assert_eq!(r["stored"], "session", "no project is open yet: {r}");
    assert_eq!(ground(&mut h, at(100), at(3)), metres(100, 3));
    // The document names the heightmap; the samples are not in its text.
    let text = h.session().document().to_json().unwrap();
    assert!(
        text.contains("heightmap") && text.contains(&content),
        "{text}"
    );
    assert!(!text.contains("tiles"), "{text}");

    // A new project keeps the samples of the session beside its world.
    let made = ok(
        &mut h,
        "project.new",
        json!({"path": "map", "id": "map", "name": "Map", "world": "main"}),
    );
    assert_eq!(made["terrain"]["heightmap"]["tiles"], 1, "{made}");
    assert_eq!(made["terrain"]["heightmap"]["from"], "session", "{made}");
    let folder = dir.join("map/worlds/main/terrain/heightmap");
    let tile = folder.join("tile_0_0.bin");
    let good = fs::read(&tile).expect("the tile file");
    assert_eq!(good.len(), TILE_BYTES);
    let world_json = fs::read_to_string(dir.join("map/worlds/main/world.json")).unwrap();
    assert!(
        world_json.contains("heightmap") && world_json.contains(&content),
        "{world_json}"
    );

    // Opening the project reads the samples from the folder.
    let mut h = host_in(&dir);
    let opened = ok(&mut h, "project.open", json!({"path": "map"}));
    assert_eq!(opened["terrain"]["heightmap"]["tiles"], 1, "{opened}");
    assert_eq!(opened["terrain"]["heightmap"]["content"], content);
    assert_eq!(opened["terrain"]["heightmap"]["from"], "files");
    assert_eq!(opened["terrain"]["heightmap"]["error"], Value::Null);
    assert_eq!(opened["terrain"]["error"], Value::Null, "{opened}");
    assert_eq!(ground(&mut h, at(100), at(3)), metres(100, 3));
    assert_eq!(ground(&mut h, at(0), at(N - 1)), metres(0, N - 1));
    let files = fs::read_dir(&folder).unwrap().count();
    assert_eq!(files, 1, "opening a complete heightmap writes nothing");

    // A tile that is cut short, one of another content, a missing one: said, tile by tile,
    // and whatever needs the ground says why there is none.
    let problem = |h: &mut gamecraft_automation::Host| {
        let opened = ok(h, "project.open", json!({"path": "map"}));
        opened["terrain"]["heightmap"]["error"]
            .as_str()
            .expect("the problem")
            .to_string()
    };
    fs::write(&tile, &good[..good.len() - 1]).unwrap();
    let e = problem(&mut h);
    assert!(e.contains("bytes"), "{e}");
    let e = err(&mut h, "entity.add", json!({"name": "Crate", "at": [0, 0]}));
    assert!(e.contains("samples") && e.contains("terrain.import"), "{e}");
    let mut other = good.clone();
    other[TILE_BYTES - 4..].copy_from_slice(&12345.0f32.to_le_bytes());
    fs::write(&tile, &other).unwrap();
    let e = problem(&mut h);
    assert!(e.contains("terrain.source.content"), "{e}");
    fs::remove_file(&tile).unwrap();
    let e = problem(&mut h);
    assert!(e.contains("missing"), "{e}");
    // A link out of the pack is not followed.
    let outside = TempDir::new("heightmap-outside");
    fs::write(outside.join("tile_0_0.bin"), &good).unwrap();
    fs::remove_dir_all(&folder).unwrap();
    std::os::unix::fs::symlink(outside.path(), &folder).unwrap();
    let e = problem(&mut h);
    assert!(e.contains("missing"), "{e}");

    // A second import of another shape replaces the tiles: written, and the stale ones gone.
    fs::remove_file(&folder).unwrap();
    fs::write(dir.join("more.pgm"), pgm16(N + 5, N, raw)).unwrap();
    let r = ok(
        &mut h,
        "terrain.import",
        json!({"path": "more.pgm", "scale_m": 0.5, "offset_m": -10.0}),
    );
    assert_eq!(
        (r["cols"].as_u64(), r["rows"].as_u64()),
        (Some(N as u64 + 5), Some(N as u64))
    );
    assert_eq!(r["tiles"], 2);
    assert_eq!(r["stored"], "project");
    ok(&mut h, "project.save", json!({}));
    let mut names = read_names(&folder);
    names.sort();
    assert_eq!(names, ["tile_0_0.bin", "tile_1_0.bin"]);
    // And one of the first shape again, other content: one tile written, one removed.
    fs::write(dir.join("hill.pgm"), pgm16(N, N, |c, r| raw(c, r) + 1000)).unwrap();
    let r = ok(
        &mut h,
        "terrain.import",
        json!({"path": "hill.pgm", "scale_m": 0.5, "offset_m": -10.0}),
    );
    assert_eq!(r["tiles"], 1);
    assert_eq!(r["tiles_written"], 1, "{r}");
    assert_eq!(r["tiles_removed"], 1, "{r}");
    ok(&mut h, "project.save", json!({}));
    assert_eq!(read_names(&folder), ["tile_0_0.bin"]);
    let mut h2 = host_in(&dir);
    let opened = ok(&mut h2, "project.open", json!({"path": "map"}));
    assert_eq!(
        opened["terrain"]["heightmap"]["error"],
        Value::Null,
        "{opened}"
    );
    assert_eq!(ground(&mut h2, at(100), at(3)), metres(100, 3) + 500.0);
}

#[test]
fn text_and_raw_files_import_and_bad_files_are_errors() {
    let dir = TempDir::new("heightmap-formats");
    let mut h = host_in(&dir);
    ok(&mut h, "file.new", json!({"name": "Map", "seed": 1}));
    // Text: metres, one row per line.
    fs::write(dir.join("hill.txt"), "1 2.5\n-3 4\n").unwrap();
    let r = ok(
        &mut h,
        "terrain.import",
        json!({"path": "hill.txt", "size_m": 10}),
    );
    assert_eq!((r["cols"].as_u64(), r["rows"].as_u64()), (Some(2), Some(2)));
    assert_eq!(ground(&mut h, -5.0, -5.0), 1.0);
    assert_eq!(ground(&mut h, 5.0, -5.0), 2.5);
    // 16-bit integers, little-endian: height = raw * scale_m + offset_m.
    let mut bytes = Vec::new();
    for value in [10u16, 20, 30, 40] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(dir.join("raw.u16"), &bytes).unwrap();
    ok(
        &mut h,
        "terrain.import",
        json!({"path": "raw.u16", "cols": 2, "rows": 2, "scale_m": 0.1, "offset_m": 100.0}),
    );
    assert_eq!(ground(&mut h, -5.0, -5.0), 101.0);
    assert_eq!(ground(&mut h, 5.0, 5.0), 104.0);
    // 32-bit floats, little-endian.
    bytes.clear();
    for value in [1.5f32, 2.5, 3.5, 4.5] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(dir.join("raw.f32"), &bytes).unwrap();
    ok(
        &mut h,
        "terrain.import",
        json!({"path": "raw.f32", "cols": 2, "rows": 2}),
    );
    assert_eq!(ground(&mut h, -5.0, -5.0), 1.5);
    // An 8-bit PGM.
    fs::write(dir.join("eight.pgm"), b"P5\n2 2\n255\n\x01\x02\x03\x04").unwrap();
    let r = ok(
        &mut h,
        "terrain.import",
        json!({"path": "eight.pgm", "scale_m": 2.0}),
    );
    assert_eq!(r["format"], "pgm");
    assert_eq!(ground(&mut h, -5.0, -5.0), 2.0);

    // Bad files and bad parameters: an error that says what is wrong, and the ground stays.
    let mut bad = |params: Value, needle: &str| {
        let e = err(&mut h, "terrain.import", params.clone());
        assert!(e.contains(needle), "{params}: {e}");
    };
    bad(json!({"path": "missing.pgm"}), "cannot read");
    bad(json!({"path": "hill.txt", "scale_m": 2.0}), "scale_m");
    bad(json!({"path": "raw.u16"}), "cols");
    bad(json!({"path": "raw.f32", "cols": 3, "rows": 2}), "samples");
    bad(json!({"path": "raw.f32", "cols": 1, "rows": 4}), "cols");
    bad(json!({"path": "hill.txt", "format": "f32"}), "cols");
    bad(json!({"path": "hill.txt", "format": "pgm"}), "P5");
    bad(json!({"path": "dem"}), "format");
    bad(json!({"path": "../elsewhere.pgm"}), "read root");
    fs::write(dir.join("nan.txt"), "1 nan\n3 4\n").unwrap();
    bad(json!({"path": "nan.txt"}), "nan");
    fs::write(dir.join("inf.txt"), "1 2\n3 -inf\n").unwrap();
    bad(json!({"path": "inf.txt"}), "inf");
    fs::write(dir.join("ragged.txt"), "1 2\n3\n").unwrap();
    bad(json!({"path": "ragged.txt"}), "row");
    fs::write(dir.join("one.txt"), "1\n").unwrap();
    bad(json!({"path": "one.txt"}), "2");
    fs::write(dir.join("empty.txt"), "").unwrap();
    bad(json!({"path": "empty.txt"}), "no samples");
    fs::write(dir.join("big.pgm"), b"P5\n3000 3000\n255\n").unwrap();
    bad(json!({"path": "big.pgm"}), "2049");
    fs::write(dir.join("zero.pgm"), b"P5\n2 2\n0\n\x00\x00\x00\x00").unwrap();
    bad(json!({"path": "zero.pgm"}), "maxval");
    fs::write(dir.join("cut.pgm"), b"P5\n2 2\n255\n\x00").unwrap();
    bad(json!({"path": "cut.pgm"}), "samples");
    fs::write(dir.join("huge.txt"), "1000001 0\n0 0\n").unwrap();
    bad(json!({"path": "huge.txt"}), "1000001");
    bad(json!({"path": "hill.txt", "size_m": 100000.0}), "size_m");
    assert_eq!(
        ground(&mut h, -5.0, -5.0),
        2.0,
        "the last good import stays"
    );

    // The command's own example runs as it is listed.
    fs::create_dir_all(dir.join("dem")).unwrap();
    fs::write(
        dir.join("dem/valley.pgm"),
        pgm16(4, 3, |c, r| (c * 300 + r * 50) as u16),
    )
    .unwrap();
    let example = h
        .command_docs()
        .into_iter()
        .find(|d| d.id == "terrain.import")
        .expect("the command is listed")
        .example;
    let r = ok(&mut h, "terrain.import", example.clone());
    assert_eq!(
        (r["cols"].as_u64(), r["rows"].as_u64()),
        (Some(4), Some(3)),
        "{example}"
    );
    assert_eq!(r["format"], "pgm");
    assert_eq!(ground(&mut h, -5.0, -5.0), 20.0);
}
