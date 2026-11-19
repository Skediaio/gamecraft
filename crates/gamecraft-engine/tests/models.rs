//! Models through the session: the component as entity.add takes it, the derived bounds a host
//! hands over, a running world that takes them, and the component list.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use gamecraft_engine::Session;
use gamecraft_world::{ModelBounds, World};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn bounds() -> BTreeMap<String, ModelBounds> {
    BTreeMap::from([(
        "models/crate.glb".to_string(),
        ModelBounds {
            min: [-0.5, 0.0, -0.5],
            max: [0.5, 1.0, 0.5],
        },
    )])
}

fn session() -> Session {
    let mut s = Session::new(World::new("m", 1));
    s.run(
        "entity.add",
        &json!({"name": "Crate", "position": [0, 0, -3],
                "components": [{"type": "model", "asset": "models/crate.glb", "solid": true}]}),
    )
    .unwrap();
    s
}

#[test]
fn entity_add_takes_a_model_and_a_bad_asset_names_the_field() {
    let mut s = session();
    let got = s.run("entity.get", &json!({"id": "Crate"})).unwrap();
    let text = got.to_string();
    assert!(
        text.contains("models/crate.glb") && text.contains("\"collider\":\"box\""),
        "{text}"
    );
    let e = s
        .run(
            "entity.add",
            &json!({"position": [0, 0, 0], "components": [{"type": "model", "asset": "/etc/passwd.glb"}]}),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("model.asset"), "{e}");
}

#[test]
fn the_host_s_bounds_reach_the_document_its_history_and_the_running_world() {
    let mut s = session();
    s.run(
        "entity.set",
        &json!({"id": "Crate", "position": [0, 0, -4]}),
    )
    .unwrap();
    let revision = s.revision();
    s.set_model_bounds(bounds());
    assert_eq!(
        s.revision(),
        revision,
        "derived data is no change of the document"
    );
    assert!(s.document().model_bounds("models/crate.glb").is_some());
    s.undo().unwrap();
    assert!(s.document().model_bounds("models/crate.glb").is_some());
    s.redo().unwrap();
    s.run("sim.start", &json!({})).unwrap();
    s.set_model_bounds(BTreeMap::new());
    assert!(s.current_world().model_bounds("models/crate.glb").is_none());
    s.set_model_bounds(bounds());
    assert!(s.current_world().model_bounds("models/crate.glb").is_some());
}

#[test]
fn the_component_list_documents_the_model() {
    let mut s = session();
    let list = s.run("component.list", &json!({})).unwrap();
    let model = list["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "model")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(model["example"]["asset"], "models/crate.glb");
}
