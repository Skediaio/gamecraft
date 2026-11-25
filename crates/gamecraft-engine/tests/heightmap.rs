//! `terrain.set` of kind `heightmap`: the document names the samples, the host hands them
//! over (`Session::set_heightmap`), and the ground is built from them.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{refuse, snapshot};
use gamecraft_engine::Session;
use gamecraft_world::{Heightmap, TerrainSource, World};
use serde_json::{Value, json};

const COLS: u32 = 3;
const ROWS: u32 = 2;

/// `height(c, r)` of the samples, metres: a plane, exact in binary.
fn sample(c: u32, r: u32) -> f64 {
    f64::from(c) * 0.5 + f64::from(r) * 0.25
}

fn map() -> Heightmap {
    Heightmap::from_samples(COLS, ROWS, sample).unwrap()
}

/// `x` of column `c` on a terrain of 256 m (the default of `terrain.set`).
fn x(c: u32) -> f64 {
    -128.0 + f64::from(c) * 128.0
}

fn z(r: u32) -> f64 {
    -128.0 + f64::from(r) * (256.0 / f64::from(ROWS - 1))
}

fn content() -> String {
    Heightmap::content_text(map().content_hash())
}

/// Names the heightmap in the document, without handing the samples over.
fn name(s: &mut Session) -> Value {
    s.run(
        "terrain.set",
        &json!({"kind": "heightmap", "cols": COLS, "rows": ROWS, "content": content()}),
    )
    .unwrap()
}

fn empty() -> Session {
    Session::new(World::new("Empty", 7))
}

fn ground(s: &mut Session, px: f64, pz: f64) -> Result<f64, String> {
    let height = s
        .run("query.ground_height", &json!({"x": px, "z": pz}))
        .map_err(|e| e.to_string())?["height_m"]
        .as_f64();
    height.ok_or_else(|| "no ground: outside the terrain".to_string())
}

#[test]
fn bad_parameters_name_the_parameter() {
    let mut s = empty();
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "heightmap", "cols": 3, "rows": 2}),
        &["content"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "heightmap", "cols": 3, "content": "0123456789abcdef"}),
        &["rows"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "heightmap", "cols": 3, "rows": 2, "content": "0123456789ABCDEF"}),
        &["content"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "heightmap", "cols": 3, "rows": 2, "content": "no", "heights": [1, 2]}),
        &["heights"],
    );
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "heightmap", "cols": 1, "rows": 2, "content": "0123456789abcdef"}),
        &["cols"],
    );
    // The old kinds refuse the new parameter.
    refuse(
        &mut s,
        "terrain.set",
        json!({"kind": "grid", "cols": 2, "rows": 2, "heights": [1, 2, 3, 4], "content": "x"}),
        &["content"],
    );
    assert!(!s.document().terrain.is_some());
}

#[test]
fn a_named_heightmap_waits_for_its_samples() {
    let mut s = empty();
    let r = name(&mut s);
    assert_eq!(
        r,
        json!({"cols": COLS, "rows": ROWS, "samples": "handed over by the host"})
    );
    let terrain = s.document().terrain.as_ref().unwrap();
    assert!(
        matches!(
            &terrain.source,
            TerrainSource::Heightmap { cols, rows, content: said, tiles }
                if *cols == COLS && *rows == ROWS && said == &content() && tiles.is_empty()
        ),
        "{:?}",
        terrain.source
    );
    assert!(s.current_heights().is_none());
    // Whatever needs the ground says why there is none.
    refuse(
        &mut s,
        "entity.add",
        json!({"name": "Crate", "at": [0, 0]}),
        &["samples", "terrain.import"],
    );
    // The layers and edits of the terrain before stay.
    s.run("terrain.set", &json!({"kind": "flat", "height_m": 4}))
        .unwrap();
    s.run(
        "terrain.layers",
        &json!({"layers": [{"name": "grass", "color": [0.2, 0.5, 0.1]}]}),
    )
    .unwrap();
    s.run(
        "terrain.sculpt",
        &json!({"op": "raise", "center": [0, 0], "radius_m": 10, "strength_m": 2}),
    )
    .unwrap();
    name(&mut s);
    let terrain = s.document().terrain.as_ref().unwrap();
    assert_eq!(terrain.edits.len(), 1);
    assert_eq!(terrain.layers.len(), 1, "the painted layers stay");
}

#[test]
fn the_samples_build_the_ground_and_the_document_does_not_move() {
    let mut s = empty();
    name(&mut s);
    let before = snapshot(&s);
    let text_before = s.document().to_json().unwrap();
    s.set_heightmap(map()).unwrap();
    // Samples are not part of the document: no revision, nothing to undo, the same history.
    let after = snapshot(&s);
    assert_eq!(
        (after.revision, after.view_revision),
        (before.revision, before.view_revision)
    );
    assert_eq!(
        (after.can_undo, after.can_redo, after.undo_depth),
        (before.can_undo, before.can_redo, before.undo_depth)
    );
    assert_eq!(
        after.current.to_json().unwrap(),
        before.current.to_json().unwrap()
    );
    assert_eq!(s.document().to_json().unwrap(), text_before);
    assert!(
        !text_before.contains("tiles"),
        "the text names the samples only"
    );
    assert!(text_before.contains(&content()), "{text_before}");
    // The ground is the samples, exactly, at their nodes and in between.
    for c in 0..COLS {
        for r in 0..ROWS {
            assert_eq!(
                ground(&mut s, x(c), z(r)).unwrap(),
                sample(c, r),
                "({c}, {r})"
            );
        }
    }
    let mid = ground(&mut s, (x(0) + x(1)) / 2.0, z(0)).unwrap();
    assert!((mid - 0.25).abs() < 1e-12, "{mid}");
    let e = ground(&mut s, 300.0, 0.0).unwrap_err();
    assert!(e.contains("outside"), "{e}");
    // The samples follow the document through the history.
    s.run("terrain.clear", &Value::Null).unwrap();
    assert!(s.current_heights().is_none());
    s.undo().unwrap();
    assert_eq!(ground(&mut s, x(2), z(1)).unwrap(), sample(2, 1));
    s.redo().unwrap();
    assert!(s.current_heights().is_none());
    s.undo().unwrap();
    // Handing the same samples over again changes nothing.
    s.set_heightmap(map()).unwrap();
    assert_eq!(ground(&mut s, x(1), z(0)).unwrap(), sample(1, 0));
    assert_eq!(s.document().to_json().unwrap(), text_before);
}

#[test]
fn a_running_world_gets_the_samples_too() {
    let mut s = empty();
    s.run("sim.start", &Value::Null).unwrap();
    name(&mut s);
    assert!(s.live().unwrap().heights().is_none());
    s.set_heightmap(map()).unwrap();
    {
        let live = s.live().unwrap();
        assert!(live.heights().is_some());
        assert_eq!(
            live.world()
                .terrain
                .as_ref()
                .unwrap()
                .heightmap()
                .unwrap()
                .tiles()
                .len(),
            1
        );
    }
    assert_eq!(ground(&mut s, x(1), z(1)).unwrap(), sample(1, 1));
    let tick = s.live().unwrap().tick();
    s.run("sim.step", &json!({"ticks": 5})).unwrap();
    assert_eq!(s.live().unwrap().tick(), tick + 5);
    assert_eq!(ground(&mut s, x(0), z(0)).unwrap(), sample(0, 0));
}

#[test]
fn the_samples_stay_out_of_the_state_hash_and_other_samples_are_refused() {
    let mut a = empty();
    let mut b = empty();
    name(&mut a);
    name(&mut b);
    a.set_heightmap(map()).unwrap();
    b.set_heightmap(map()).unwrap();
    assert_eq!(
        a.run("sim.state", &Value::Null).unwrap()["hash"],
        b.run("sim.state", &Value::Null).unwrap()["hash"]
    );
    // The samples do not enter the state hash: only what the document says does.
    let mut bare = empty();
    name(&mut bare);
    assert_eq!(
        bare.run("sim.state", &Value::Null).unwrap()["hash"],
        a.run("sim.state", &Value::Null).unwrap()["hash"]
    );
    // Samples of another content are refused, naming what the document says.
    let before = snapshot(&b);
    let e = b
        .set_heightmap(Heightmap::from_samples(COLS, ROWS, |_, _| 9.0).unwrap())
        .unwrap_err();
    assert!(e.to_string().contains("terrain.source.content"), "{e}");
    assert!(e.to_string().contains(&content()), "{e}");
    assert_eq!(snapshot(&b), before);
    let e = b
        .set_heightmap(Heightmap::from_samples(COLS + 1, ROWS, sample).unwrap())
        .unwrap_err();
    assert!(e.to_string().contains(&format!("{COLS} x {ROWS}")), "{e}");
}

#[test]
fn the_heightmap_round_trips_through_the_text_with_its_name_only() {
    let mut s = empty();
    name(&mut s);
    let text = s.document().to_json().unwrap();
    let mut back = World::from_json(&text).unwrap();
    let terrain = back.terrain.as_ref().unwrap();
    assert!(terrain.heightmap().unwrap().is_empty());
    assert!(
        terrain.build().is_err(),
        "without the samples there is no ground"
    );
    back.set_heightmap(map()).unwrap();
    let field = back.terrain.as_ref().unwrap().build().unwrap();
    assert_eq!(field.height_at(x(2), z(0)).unwrap(), sample(2, 0));
}

#[test]
fn a_kilometre_at_two_metres_reads_back_exactly_and_a_crater_rebuilds_its_tiles_only() {
    let mut s = empty();
    // 513 x 513 samples 2 m apart over 1024 m: every sample is a node of the field, whose
    // cells are 2 m. Heights exact in binary.
    let n = 513u32;
    let h = |c: u32, r: u32| ((c * 7 + r * 13) % 1000) as f64 / 8.0 - 40.0;
    let map = Heightmap::from_samples(n, n, h).unwrap();
    assert_eq!(map.tiles().len(), 16);
    s.run(
        "terrain.set",
        &json!({
            "kind": "heightmap",
            "cols": n,
            "rows": n,
            "content": Heightmap::content_text(map.content_hash()),
            "size_m": 1024,
            "cell_m": 2,
        }),
    )
    .unwrap();
    s.set_heightmap(map.clone()).unwrap();
    let at = |i: u32| -512.0 + f64::from(i) * 2.0;
    for (c, r) in [(0, 0), (1, 1), (256, 256), (512, 512), (511, 100), (300, 0)] {
        assert_eq!(ground(&mut s, at(c), at(r)).unwrap(), h(c, r), "({c}, {r})");
    }
    // Half-way between two samples: the plane of the field's triangle, the mean along the edge.
    let mid = ground(&mut s, at(200) + 1.0, at(300)).unwrap();
    assert_eq!(mid, (h(200, 300) + h(201, 300)) / 2.0);
    let before = s.run("sim.state", &Value::Null).unwrap()["hash"].clone();
    // A crater: only the field tiles it touches are built again.
    let r = s
        .run(
            "terrain.sculpt",
            &json!({"op": "raise", "center": [0, 0], "radius_m": 10, "strength_m": 2}),
        )
        .unwrap();
    assert_eq!(r["tiles_total"], 16, "{r}");
    let rebuilt = r["tiles_rebuilt"].as_u64().expect("a count");
    assert!(
        (1..=4).contains(&rebuilt),
        "a crater of 10 m touches one to four tiles: {r}"
    );
    assert!(ground(&mut s, 0.0, 0.0).unwrap() > h(256, 256));
    assert_eq!(ground(&mut s, at(0), at(0)).unwrap(), h(0, 0));
    // The same world run again hashes the same, crater or not.
    let mut again = empty();
    again
        .run(
            "terrain.set",
            &json!({
                "kind": "heightmap",
                "cols": n,
                "rows": n,
                "content": Heightmap::content_text(map.content_hash()),
                "size_m": 1024,
                "cell_m": 2,
            }),
        )
        .unwrap();
    again.set_heightmap(map.clone()).unwrap();
    assert_eq!(
        again.run("sim.state", &Value::Null).unwrap()["hash"],
        before
    );
    again
        .run(
            "terrain.sculpt",
            &json!({"op": "raise", "center": [0, 0], "radius_m": 10, "strength_m": 2}),
        )
        .unwrap();
    assert_eq!(
        again.run("sim.state", &Value::Null).unwrap()["hash"],
        s.run("sim.state", &Value::Null).unwrap()["hash"]
    );
    // Undo takes the crater off; the samples stay.
    s.undo().unwrap();
    assert_eq!(ground(&mut s, 0.0, 0.0).unwrap(), h(256, 256));
}
