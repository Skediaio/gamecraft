//! A project's generators on disk: `worlds/<world>/generators.json`, the instances derived again
//! when it opens.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{TempDir, host_in, ok};
use serde_json::json;
use std::fs;

#[test]
fn generators_are_saved_as_recipes_and_derived_again_on_open() {
    let dir = TempDir::new("generators");
    let mut h = host_in(&dir);
    ok(&mut h, "file.new", json!({"name": "Woods", "seed": 3}));
    ok(
        &mut h,
        "terrain.set",
        json!({"kind": "noise", "size_m": 256, "cell_m": 2, "relief_m": 10}),
    );
    ok(
        &mut h,
        "project.new",
        json!({"path": "game", "id": "game", "name": "Game", "world": "valley"}),
    );
    let added = ok(
        &mut h,
        "generator.add",
        json!({
            "id": "spruces", "kind": "forest",
            "area": {"circle": {"center": [0, 0], "radius_m": 60}},
            "density_per_ha": 150, "min_distance_m": 3,
            "items": [{"weight": 1, "components": [{"type": "plant", "species": "picea-abies"}]}]
        }),
    );
    let placed = added["placed"].as_u64().unwrap();
    assert!(placed > 100, "{added}");
    let saved = ok(&mut h, "project.save", json!({}));
    assert_eq!(
        saved["written"],
        json!(["worlds/valley/generators.json"]),
        "{saved}"
    );
    let text = fs::read_to_string(dir.join("game/worlds/valley/generators.json")).unwrap();
    assert!(
        text.contains("\"spruces\"") && !text.contains("spruces#"),
        "{text}"
    );
    assert_eq!(text.lines().filter(|l| l.contains("\"kind\"")).count(), 1);
    // Opened again: the same instances, nothing to save.
    let before = h
        .session()
        .document()
        .generated()
        .set("spruces")
        .unwrap()
        .instances()
        .to_vec();
    ok(&mut h, "project.open", json!({"path": "game"}));
    let after = h
        .session()
        .document()
        .generated()
        .set("spruces")
        .unwrap()
        .instances()
        .to_vec();
    assert_eq!(after, before);
    let saved = ok(&mut h, "project.save", json!({}));
    assert_eq!(saved["unchanged"], json!(true), "{saved}");
}
