//! What adding entities costs: the work of one `entity.add` does not grow with the world,
//! and the result is the one the engine gave before the entities were shared between copies.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use gamecraft_engine::{Session, UNDO_LIMIT};
use gamecraft_world::{TerrainSource, World};
use serde_json::{Value, json};

/// The `entity.add` of box `i` of the golden script.
fn add_box(i: u64) -> Value {
    json!({
        "name": format!("b-{i}"),
        "position": [-500.0 + (i % 50) as f64 * 20.0, 45.0, -500.0 + (i / 50) as f64 * 20.0],
        "rotation_y_deg": (i * 37) % 360,
        "scale": 1.0 + (i % 5) as f64 / 10.0,
        "components": [{"type": "shape", "kind": "box", "size": [10.0, 3.0, 10.0],
                        "color": [0.7, 0.6, 0.5], "solid": !i.is_multiple_of(3)}]
    })
}

/// FNV-1a, 64 bits, of a text.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn flat_session() -> Session {
    let mut s = Session::new(World::new("Untitled", 0));
    s.run("world.set", &json!({"name": "golden", "seed": 5}))
        .unwrap();
    s.run(
        "terrain.set",
        &json!({"kind": "flat", "size_m": 1024, "cell_m": 4, "height_m": 45}),
    )
    .unwrap();
    s
}

/// The document and the state hash of 1 500 boxes, a move, a removal, an `at` and an undo
/// with its redo: the numbers are those `gamecraft-cli run` gave for the same script before
/// the world's entities were shared between copies (each command then copied and checked
/// the whole world).
#[test]
fn adding_many_entities_gives_the_document_and_hash_of_before() {
    let mut s = flat_session();
    for i in 0..1500 {
        s.run("entity.add", &add_box(i)).unwrap();
    }
    s.run(
        "entity.set",
        &json!({"id": 10, "position": [1.0, 45.0, 1.0]}),
    )
    .unwrap();
    s.run("entity.remove", &json!({"id": 20})).unwrap();
    s.run(
        "entity.add",
        &json!({"name": "on-ground", "at": [3.0, 4.0]}),
    )
    .unwrap();
    s.undo().unwrap();
    s.redo().unwrap();
    let text = s.document().to_json().unwrap();
    assert_eq!(text.len(), 708_690);
    assert_eq!(fnv1a(&text), 0xda10_70e8_f7c7_6860);
    let started = s.run("sim.start", &json!({})).unwrap();
    assert_eq!(started["hash"], json!("d6b1050da8bba7cf"));
}

/// The work oracle (AGENTS.md rule 18): every `entity.add` checks one entity, whether the
/// world holds ten entities or thousands; undo still takes back one add at a time.
#[test]
fn one_add_checks_one_entity_however_large_the_world() {
    let mut s = flat_session();
    let n = 3000;
    for i in 0..n {
        s.run("entity.add", &add_box(i)).unwrap();
        assert_eq!(s.checked_entities(), 1, "entity.add number {i}");
    }
    assert_eq!(s.document().entity_count(), n as usize);
    // A refused add changes nothing and leaves the next one cheap.
    let mut bad = add_box(n);
    bad["scale"] = json!(0);
    let e = s.run("entity.add", &bad).unwrap_err();
    assert!(e.to_string().starts_with("entity.add: "), "{e}");
    s.run("entity.add", &add_box(n)).unwrap();
    assert_eq!(s.checked_entities(), 1);
    // A move checks the moved entity.
    s.run(
        "entity.set",
        &json!({"id": 7, "position": [2.0, 45.0, 2.0]}),
    )
    .unwrap();
    assert_eq!(s.checked_entities(), 1);
    // A removal could orphan a child elsewhere, so it checks every entity.
    s.run("entity.remove", &json!({"id": 8})).unwrap();
    assert_eq!(s.checked_entities(), n as usize);
    // Each command is one undo step: back over the removal, the move and 96 adds.
    for _ in 0..UNDO_LIMIT - 2 {
        s.undo().unwrap();
    }
    assert_eq!(
        s.document().entity_count(),
        n as usize + 1 - (UNDO_LIMIT - 4)
    );
    // After an undo the document is one the session did not just check: the next command
    // checks it whole once, then only what changes again.
    s.run("entity.add", &add_box(n + 1)).unwrap();
    assert_eq!(s.checked_entities(), s.document().entity_count());
    s.run("entity.add", &add_box(n + 2)).unwrap();
    assert_eq!(s.checked_entities(), 1);
}

/// Playing, World commands change the running world the same way.
#[test]
fn adds_to_a_running_world_check_one_entity() {
    let mut s = flat_session();
    for i in 0..200 {
        s.run("entity.add", &add_box(i)).unwrap();
    }
    s.run("sim.start", &json!({})).unwrap();
    s.run("entity.add", &add_box(200)).unwrap();
    s.run("entity.add", &add_box(201)).unwrap();
    assert_eq!(s.checked_entities(), 1);
    assert_eq!(s.current_world().entity_count(), 202);
    assert_eq!(s.document().entity_count(), 200);
}

/// The node heights of a grid terrain are shared, not copied, by a command that leaves the
/// terrain alone: an `entity.add` on real ground costs what it costs on flat ground.
#[test]
fn adds_on_a_grid_terrain_copy_none_of_its_heights() {
    let mut s = Session::new(World::new("Untitled", 0));
    let heights: Vec<f64> = (0..257 * 257).map(|i| f64::from(i % 97) * 0.25).collect();
    s.run(
        "terrain.set",
        &json!({"kind": "grid", "size_m": 1024, "cell_m": 8, "cols": 257, "rows": 257,
                "heights": heights}),
    )
    .unwrap();
    let grid = |s: &Session| match &s.document().terrain.as_ref().unwrap().source {
        TerrainSource::Grid { heights, .. } => heights.as_ptr(),
        _ => panic!("not a grid"),
    };
    let before = grid(&s);
    for i in 0..50 {
        s.run("entity.add", &add_box(i)).unwrap();
        assert_eq!(s.checked_entities(), 1);
    }
    assert!(std::ptr::eq(before, grid(&s)), "the heights were copied");
    s.undo().unwrap();
    assert!(
        std::ptr::eq(before, grid(&s)),
        "the undo step holds its own copy"
    );
}
