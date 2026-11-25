//! Prefabs: an entity described once and placed many times, with overrides and children.

use crate::{
    Character, Component, EntityDraft, EntityId, Health, Prefab, PrefabChild, Shape, ShapeKind,
    Spawn, Tags, World, WorldError, Zone, ZoneShape,
};

fn soldier() -> Prefab {
    Prefab {
        id: "soldier".into(),
        name: "Soldier".into(),
        components: vec![
            Component::Character(Character::default()),
            Component::Health(Health::default()),
            Component::Tags(Tags {
                tags: vec!["team:red".into()],
            }),
        ],
        children: Vec::new(),
    }
}

fn beacon() -> Prefab {
    Prefab {
        id: "beacon".into(),
        name: "Beacon".into(),
        components: vec![Component::Tags(Tags {
            tags: vec!["beacon".into()],
        })],
        children: vec![
            PrefabChild {
                prefab: None,
                name: "Beacon post".into(),
                offset: [0.0, 0.0, -2.0],
                rotation_y_deg: 0.0,
                components: vec![Component::Shape(Shape {
                    kind: ShapeKind::Cylinder,
                    size: [0.2, 2.0, 0.2],
                    color: [0.5, 0.4, 0.3],
                    solid: true,
                })],
            },
            PrefabChild {
                prefab: None,
                name: "Beacon zone".into(),
                offset: [1.0, 0.0, 0.0],
                rotation_y_deg: 30.0,
                components: vec![Component::Zone(Zone {
                    shape: ZoneShape::Circle,
                    radius_m: 3.0,
                    size: [4.0, 4.0],
                    height_m: 3.0,
                })],
            },
        ],
    }
}

fn world() -> World {
    let mut w = World::new("Prefabs", 5);
    w.put_prefab(soldier()).unwrap();
    w.put_prefab(beacon()).unwrap();
    w
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

#[test]
fn overrides_replace_components_of_the_same_type() {
    let mut w = world();
    let wounded = Health {
        current: 40.0,
        ..Health::default()
    };
    let spawned = w
        .spawn_prefab(
            "soldier",
            Spawn {
                name: Some("Grunt".into()),
                position: [3.0, 0.0, 4.0],
                overrides: vec![Component::Health(wounded.clone())],
                ..Spawn::default()
            },
        )
        .unwrap();
    let e = w.entity(spawned.id).unwrap();
    assert_eq!(e.prefab.as_deref(), Some("soldier"));
    assert_eq!(e.name, "Grunt");
    assert_eq!(e.health(), Some(&wounded));
    assert!(e.character().is_some());
    assert!(e.has_tag("team:red"));
    // One component of each type, in the prefab's order.
    let kinds: Vec<&str> = e.components.iter().map(Component::kind).collect();
    assert_eq!(kinds, ["character", "health", "tags"]);
    assert_eq!(w.overrides_of(spawned.id), vec![Component::Health(wounded)]);
    // An override of a type the prefab lacks is added.
    let lit = w
        .spawn_prefab(
            "soldier",
            Spawn {
                overrides: vec![Component::Shape(Shape::default())],
                ..Spawn::default()
            },
        )
        .unwrap();
    let e = w.entity(lit.id).unwrap();
    assert_eq!(e.name, "Soldier", "the prefab's name when none is given");
    assert!(e.shape().is_some());
    assert!(e.health().is_some());
}

#[test]
fn children_are_placed_relative_to_the_parent_and_follow_it() {
    let mut w = world();
    let spawned = w
        .spawn_prefab(
            "beacon",
            Spawn {
                position: [10.0, 1.0, 0.0],
                rotation_y_deg: 90.0,
                ..Spawn::default()
            },
        )
        .unwrap();
    assert_eq!(spawned.children.len(), 2);
    let post = w.entity(spawned.children[0]).unwrap();
    assert_eq!(post.parent, Some(spawned.id));
    assert_eq!(post.name, "Beacon post");
    assert!(post.prefab.is_none());
    // Turned 90 degrees counter-clockwise, the local north (-Z) is the world's west (-X).
    assert!(
        close(post.position, [8.0, 1.0, 0.0]),
        "post at {:?}",
        post.position
    );
    let zone = w.entity(spawned.children[1]).unwrap();
    // The local east (+X) is the world's north (-Z) after a quarter turn.
    assert!(
        close(zone.position, [10.0, 1.0, -1.0]),
        "zone at {:?}",
        zone.position
    );
    assert!((zone.rotation_y_deg - 120.0).abs() < 1e-9);
    assert_eq!(w.children(spawned.id), spawned.children);

    // Moving the parent carries the children along.
    w.place_entity(spawned.id, [0.0, 0.0, 0.0], 0.0).unwrap();
    let post = w.entity(spawned.children[0]).unwrap();
    assert!(
        close(post.position, [0.0, 0.0, -2.0]),
        "post at {:?}",
        post.position
    );
    let zone = w.entity(spawned.children[1]).unwrap();
    assert!(close(zone.position, [1.0, 0.0, 0.0]));
    assert!((zone.rotation_y_deg - 30.0).abs() < 1e-9);

    // Removing the parent removes the children.
    let count = w.entity_count();
    assert!(w.remove_entity(spawned.id).is_some());
    assert_eq!(w.entity_count(), count - 3);
    assert!(w.entity(spawned.children[0]).is_none());
    w.validate().unwrap();
}

#[test]
fn a_placed_prefab_cannot_be_removed() {
    let mut w = world();
    let a = w.spawn_prefab("soldier", Spawn::default()).unwrap();
    let b = w.spawn_prefab("soldier", Spawn::default()).unwrap();
    let err = w.remove_prefab("soldier").unwrap_err();
    let text = err.to_string();
    assert!(text.contains("\"soldier\""), "{text}");
    assert!(text.contains(&format!("{} \"Soldier\"", a.id)), "{text}");
    assert!(text.contains(&format!("{} \"Soldier\"", b.id)), "{text}");
    w.remove_entity(a.id);
    w.remove_entity(b.id);
    assert_eq!(w.remove_prefab("soldier").unwrap().id, "soldier");
    assert!(matches!(
        w.remove_prefab("soldier"),
        Err(WorldError::PrefabNotFound { .. })
    ));
}

#[test]
fn round_trip_keeps_prefabs_overrides_and_children() {
    let mut w = world();
    let grunt = w
        .spawn_prefab(
            "soldier",
            Spawn {
                position: [1.0, 2.0, 3.0],
                overrides: vec![Component::Health(Health {
                    current: 10.0,
                    ..Health::default()
                })],
                ..Spawn::default()
            },
        )
        .unwrap();
    let plain = w.spawn_prefab("soldier", Spawn::default()).unwrap();
    let beacon = w.spawn_prefab("beacon", Spawn::default()).unwrap();
    let text = w.to_json().unwrap();
    let back = World::from_json(&text).unwrap();
    assert_eq!(back, w);
    assert_eq!(back.to_json().unwrap(), text, "the text is stable");

    // In the file an instance holds its prefab and overrides, not its components.
    let file: serde_json::Value = serde_json::from_str(&text).unwrap();
    let entities = file["entities"].as_array().unwrap();
    let find = |id: EntityId| {
        entities
            .iter()
            .find(|e| e["id"].as_u64() == Some(id.0))
            .unwrap()
    };
    let g = find(grunt.id);
    assert_eq!(g["prefab"], "soldier");
    assert!(g.get("components").is_none(), "{g}");
    assert_eq!(g["overrides"].as_array().unwrap().len(), 1);
    let p = find(plain.id);
    assert!(p.get("overrides").is_none(), "no overrides: {p}");
    // The children follow from the prefab: not written, placed again when read.
    assert_eq!(file["derived_children"], true);
    assert!(
        entities
            .iter()
            .all(|e| e["id"].as_u64() != Some(beacon.children[0].0)),
        "a child the prefab places is not written"
    );
    let child = back.entity(beacon.children[0]).unwrap();
    assert_eq!(child.parent, Some(beacon.id));
    assert_eq!(child.prefab_child, Some(0));
    assert_eq!(file["prefabs"].as_array().unwrap().len(), 2);
}

#[test]
fn a_document_with_prefabs_reads_with_effective_components() {
    let text = r#"{
      "gamecraft": 1,
      "prefabs": [
        {"id": "crate", "components": [
          {"type": "shape", "kind": "box", "size": [1, 1, 1]},
          {"type": "tags", "tags": ["loot"]}
        ]}
      ],
      "entities": [
        {"id": 1, "name": "Big crate", "prefab": "crate",
         "overrides": [{"type": "shape", "kind": "box", "size": [2, 2, 2]}]},
        {"id": 2, "prefab": "crate"}
      ]
    }"#;
    let w = World::from_json(text).unwrap();
    let big = w.entity(EntityId(1)).unwrap();
    assert_eq!(big.shape().unwrap().size, [2.0, 2.0, 2.0]);
    assert!(big.has_tag("loot"));
    let small = w.entity(EntityId(2)).unwrap();
    assert_eq!(small.shape().unwrap().size, [1.0, 1.0, 1.0]);
    assert_eq!(w.prefab("crate").unwrap().name, "");
}

#[test]
fn changing_a_prefab_changes_its_instances_but_keeps_their_overrides() {
    let mut w = world();
    let wounded = Component::Health(Health {
        current: 5.0,
        ..Health::default()
    });
    let a = w
        .spawn_prefab(
            "soldier",
            Spawn {
                overrides: vec![wounded.clone()],
                ..Spawn::default()
            },
        )
        .unwrap();
    let b = w.spawn_prefab("soldier", Spawn::default()).unwrap();
    let mut changed = soldier();
    changed.components = vec![
        Component::Character(Character {
            walk_speed_mps: 2.0,
            ..Character::default()
        }),
        Component::Health(Health {
            max: 200.0,
            current: 200.0,
            armor: 0.0,
        }),
        Component::Tags(Tags {
            tags: vec!["team:blue".into()],
        }),
    ];
    w.put_prefab(changed).unwrap();
    let ea = w.entity(a.id).unwrap();
    assert_eq!(ea.character().unwrap().walk_speed_mps, 2.0);
    assert!(ea.has_tag("team:blue"));
    assert_eq!(ea.components.get(1), Some(&wounded), "the override stays");
    let eb = w.entity(b.id).unwrap();
    assert_eq!(eb.health().unwrap().max, 200.0);
    w.validate().unwrap();
}

#[test]
fn validation_names_the_prefab_and_the_field() {
    let mut bad = soldier();
    bad.components.push(Component::Shape(Shape {
        size: [-1.0, 1.0, 1.0],
        ..Shape::default()
    }));
    let err = World::new("w", 0).put_prefab(bad).unwrap_err().to_string();
    assert!(err.contains("prefab \"soldier\""), "{err}");
    assert!(err.contains("shape.size[0]"), "{err}");

    let mut twice = soldier();
    twice.components.push(Component::Health(Health::default()));
    let err = World::new("w", 0)
        .put_prefab(twice)
        .unwrap_err()
        .to_string();
    assert!(err.contains("prefab \"soldier\""), "{err}");
    assert!(err.contains("health"), "{err}");

    let mut child = beacon();
    child.children[1].offset = [f64::NAN, 0.0, 0.0];
    let err = World::new("w", 0)
        .put_prefab(child)
        .unwrap_err()
        .to_string();
    assert!(err.contains("prefab \"beacon\""), "{err}");
    assert!(err.contains("children[1].offset[0]"), "{err}");

    let mut empty = soldier();
    empty.id = " ".into();
    let err = World::new("w", 0)
        .put_prefab(empty)
        .unwrap_err()
        .to_string();
    assert!(err.contains("id"), "{err}");

    let unknown = r#"{"gamecraft": 1, "entities": [{"id": 1, "prefab": "ghost"}]}"#;
    let err = World::from_json(unknown).unwrap_err().to_string();
    assert!(err.contains("entity 1"), "{err}");
    assert!(err.contains("\"ghost\""), "{err}");

    let both = r#"{"gamecraft": 1, "prefabs": [{"id": "p"}],
        "entities": [{"id": 1, "prefab": "p", "components": []}]}"#;
    let err = World::from_json(both).unwrap_err().to_string();
    assert!(err.contains("entity 1"), "{err}");
    assert!(err.contains("overrides"), "{err}");

    let dup = r#"{"gamecraft": 1, "prefabs": [{"id": "p"}, {"id": "p"}]}"#;
    let err = World::from_json(dup).unwrap_err().to_string();
    assert!(err.contains("prefab \"p\""), "{err}");

    let err = World::new("w", 0)
        .spawn_prefab("nobody", Spawn::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("\"nobody\""), "{err}");
}

#[test]
fn an_instance_keeps_every_component_type_of_its_prefab() {
    let mut w = world();
    let a = w.spawn_prefab("soldier", Spawn::default()).unwrap();
    w.entity_mut(a.id).unwrap().remove_component("health");
    let err = w.validate().unwrap_err().to_string();
    assert!(err.contains(&format!("entity {}", a.id)), "{err}");
    assert!(err.contains("health"), "{err}");
    assert!(err.contains("\"soldier\""), "{err}");
}

#[test]
fn parents_must_exist_and_not_loop() {
    let missing = r#"{"gamecraft": 1, "entities": [{"id": 1, "parent": 9}]}"#;
    let err = World::from_json(missing).unwrap_err().to_string();
    assert!(err.contains("entity 1"), "{err}");
    assert!(err.contains("parent"), "{err}");

    let looped = r#"{"gamecraft": 1, "entities": [
        {"id": 1, "parent": 2}, {"id": 2, "parent": 1}]}"#;
    let err = World::from_json(looped).unwrap_err().to_string();
    assert!(err.contains("parent"), "{err}");

    let own = r#"{"gamecraft": 1, "entities": [{"id": 1, "parent": 1}]}"#;
    assert!(World::from_json(own).is_err());
}

#[test]
fn a_world_without_prefabs_is_written_as_before() {
    let w = super::full_world();
    let text = w.to_json().unwrap();
    assert!(!text.contains("prefab"), "{text}");
    assert!(!text.contains("parent"), "{text}");
    assert!(!text.contains("overrides"), "{text}");
}

#[test]
fn remove_entities_takes_children_along_in_one_pass() {
    let mut w = World::new("camp", 3);
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(
            w.add_entity(EntityDraft {
                name: format!("Camp {i}"),
                position: [i as f64 * 10.0, 0.0, 0.0],
                components: vec![Component::Shape(Shape {
                    kind: ShapeKind::Box,
                    size: [2.0, 1.0, 2.0],
                    color: [0.5; 3],
                    solid: false,
                })],
                ..EntityDraft::default()
            })
            .unwrap(),
        );
        for c in 0..3 {
            let tent = w
                .add_entity(EntityDraft {
                    name: format!("Tent {c}"),
                    position: [i as f64 * 10.0 + c as f64, 0.0, 2.0],
                    ..EntityDraft::default()
                })
                .unwrap();
            w.entity_mut(tent).unwrap().parent = Some(ids[i as usize]);
        }
    }
    assert_eq!(w.entity_count(), 20);
    let first = ids[0];
    let last = ids[4];
    // Two camps by id, an id that is not there and one given twice: two camps and their six
    // children go.
    assert_eq!(w.remove_entities(&[first, first, last, EntityId(9999)]), 8);
    assert_eq!(w.entity_count(), 12);
    assert!(w.entity(first).is_none() && w.entity(last).is_none());
    // Every child of the first camp is gone, the others keep theirs.
    assert!(
        w.entities()
            .all(|e| e.parent.is_none_or(|p| w.entity(p).is_some()))
    );
    assert_eq!(w.children(ids[1]).len(), 3);
    // The ids are never handed out again.
    let fresh = w
        .add_entity(EntityDraft {
            name: "New".into(),
            ..EntityDraft::default()
        })
        .unwrap();
    assert!(fresh.0 > 20);
}
