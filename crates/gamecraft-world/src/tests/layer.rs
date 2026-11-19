//! The `layer` of an entity: kept by the file format, inherited by a prefab's children, and
//! checked so that it can name a layer file.

use crate::{Component, EntityDraft, EntityId, Prefab, PrefabChild, Spawn, World, WorldError};

#[test]
fn layer_round_trips_through_the_file_and_is_left_out_when_none() {
    let mut w = World::new("Layers", 1);
    let a = w
        .add_entity(EntityDraft {
            name: "In village".into(),
            layer: Some("village".into()),
            ..EntityDraft::default()
        })
        .unwrap();
    let b = w
        .add_entity(EntityDraft {
            name: "Default".into(),
            ..EntityDraft::default()
        })
        .unwrap();
    let text = w.to_json().unwrap();
    assert_eq!(text.matches("\"layer\"").count(), 1, "{text}");
    let back = World::from_json(&text).unwrap();
    assert_eq!(back, w);
    assert_eq!(back.entity(a).unwrap().layer.as_deref(), Some("village"));
    assert_eq!(back.entity(b).unwrap().layer, None);
}

#[test]
fn a_layer_must_be_a_plain_file_name() {
    for bad in [
        "",
        "../escape",
        "a/b",
        "with space",
        "x.json",
        "é",
        &"l".repeat(65),
    ] {
        let mut w = World::new("Layers", 1);
        let err = w
            .add_entity(EntityDraft {
                layer: Some(bad.to_string()),
                ..EntityDraft::default()
            })
            .unwrap_err();
        match &err {
            WorldError::Invalid { field, .. } => assert_eq!(field, "layer", "{bad:?}: {err}"),
            other => panic!("{bad:?}: {other}"),
        }
        assert!(err.to_string().contains("letters"), "{err}");
    }
    let mut w = World::new("Layers", 1);
    for good in ["village", "terrain-features", "more_enemies", "L2"] {
        w.add_entity(EntityDraft {
            layer: Some(good.to_string()),
            ..EntityDraft::default()
        })
        .unwrap();
    }
}

#[test]
fn a_bad_layer_in_a_file_is_refused_with_its_entity() {
    let text = r#"{"gamecraft": 1, "entities": [{"id": 1, "layer": "../../etc"}]}"#;
    let err = World::from_json(text).unwrap_err();
    assert!(
        err.to_string().contains("layer") && err.to_string().contains("entity 1"),
        "{err}"
    );
}

#[test]
fn a_spawned_prefab_and_its_children_take_the_spawn_layer() {
    let mut w = World::new("Layers", 1);
    w.put_prefab(Prefab {
        id: "hut".into(),
        name: "Hut".into(),
        components: vec![Component::Tags(crate::Tags {
            tags: vec!["hut".into()],
        })],
        children: vec![PrefabChild {
            name: "Door".into(),
            offset: [0.0, 0.0, -1.0],
            rotation_y_deg: 0.0,
            components: vec![],
        }],
    })
    .unwrap();
    let spawned = w
        .spawn_prefab(
            "hut",
            Spawn {
                layer: Some("village".into()),
                ..Spawn::default()
            },
        )
        .unwrap();
    assert_eq!(
        w.entity(spawned.id).unwrap().layer.as_deref(),
        Some("village")
    );
    for child in &spawned.children {
        assert_eq!(w.entity(*child).unwrap().layer.as_deref(), Some("village"));
    }
    let plain = w.spawn_prefab("hut", Spawn::default()).unwrap();
    assert_eq!(w.entity(plain.id).unwrap().layer, None);
    assert_eq!(w.entity(EntityId(3)).unwrap().layer, None);
}
