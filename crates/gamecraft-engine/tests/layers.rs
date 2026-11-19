//! The `layer` of an entity through the commands: `entity.add`, `entity.set`, `prefab.spawn`,
//! shown by `entity.get` and `world.inspect`.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, refuse};
use gamecraft_world::EntityId;
use serde_json::{Value, json};

#[test]
fn entity_add_keeps_the_layer_and_get_and_inspect_show_it() {
    let mut s = example();
    let r = s
        .run(
            "entity.add",
            &json!({"name": "Hut", "position": [0, 0, 0], "layer": "village"}),
        )
        .unwrap();
    let id = r["id"].as_u64().unwrap();
    assert_eq!(
        s.document().entity(EntityId(id)).unwrap().layer.as_deref(),
        Some("village")
    );
    let got = s.run("entity.get", &json!({"id": id})).unwrap();
    assert_eq!(got["layer"], json!("village"));
    let plain = s.run("entity.get", &json!({"id": 1})).unwrap();
    assert_eq!(plain["layer"], Value::Null, "the default layer is null");
    let inspect = s.run("world.inspect", &json!({})).unwrap();
    let listed = inspect["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == json!(id))
        .unwrap()
        .clone();
    assert_eq!(listed["layer"], json!("village"));
}

#[test]
fn entity_set_moves_an_entity_and_its_children_to_a_layer_and_back() {
    let mut s = example();
    s.run(
        "prefab.add",
        &json!({"id": "hut", "children": [{"name": "Door", "offset": [0, 0, -1]}]}),
    )
    .unwrap();
    let spawned = s
        .run(
            "prefab.spawn",
            &json!({"prefab": "hut", "position": [1, 0, 1], "layer": "village"}),
        )
        .unwrap();
    let root = spawned["id"].as_u64().unwrap();
    let child = spawned["children"][0].as_u64().unwrap();
    let layer_of = |s: &gamecraft_engine::Session, id: u64| {
        s.document().entity(EntityId(id)).unwrap().layer.clone()
    };
    assert_eq!(layer_of(&s, root).as_deref(), Some("village"));
    assert_eq!(layer_of(&s, child).as_deref(), Some("village"));
    s.run("entity.set", &json!({"id": root, "layer": "farm"}))
        .unwrap();
    assert_eq!(layer_of(&s, root).as_deref(), Some("farm"));
    assert_eq!(layer_of(&s, child).as_deref(), Some("farm"));
    s.run("entity.set", &json!({"id": root, "layer": ""}))
        .unwrap();
    assert_eq!(
        layer_of(&s, root),
        None,
        "an empty layer is the default one"
    );
    assert_eq!(layer_of(&s, child), None);
    s.undo().unwrap();
    assert_eq!(layer_of(&s, root).as_deref(), Some("farm"));
}

#[test]
fn a_layer_that_is_not_a_plain_name_is_refused_naming_the_parameter() {
    let mut s = example();
    refuse(
        &mut s,
        "entity.add",
        json!({"position": [0, 0, 0], "layer": "../outside"}),
        &["layer", "plain file name"],
    );
    refuse(
        &mut s,
        "entity.set",
        json!({"id": 1, "layer": "a/b"}),
        &["layer", "plain file name"],
    );
    s.run("prefab.add", &json!({"id": "hut"})).unwrap();
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "hut", "position": [0, 0, 0], "layer": "x.json"}),
        &["layer", "plain file name"],
    );
    refuse(
        &mut s,
        "entity.add",
        json!({"position": [0, 0, 0], "layer": 3}),
        &["layer"],
    );
}
