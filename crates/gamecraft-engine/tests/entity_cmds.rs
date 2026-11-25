//! `entity.add`, `entity.set`, `entity.remove`, `entity.set_component`,
//! `entity.remove_component`, `entity.snap_to_ground`.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, refuse, snapshot};
use gamecraft_engine::Session;
use gamecraft_world::{Character, Component, EntityId, Plant, Shape, ShapeKind, World};
use serde_json::json;

fn ground(s: &Session, x: f64, z: f64) -> f64 {
    s.current_heights().unwrap().height_at(x, z).unwrap()
}

#[test]
fn add_with_position_and_components_with_defaults() {
    let mut s = example();
    let r = s
        .run(
            "entity.add",
            &json!({
                "name": "Tower",
                "position": [1, 2, 3],
                "rotation_y_deg": 30,
                "scale": 2,
                "components": [
                    {"type": "shape", "kind": "cylinder", "size": [2, 6, 2]},
                    {"type": "character", "run_speed_mps": 7}
                ]
            }),
        )
        .unwrap();
    assert_eq!(r, json!({"id": 4, "position": [1.0, 2.0, 3.0]}));
    let e = s.document().entity(EntityId(4)).unwrap();
    assert_eq!(
        (e.name.as_str(), e.rotation_y_deg, e.scale),
        ("Tower", 30.0, 2.0)
    );
    assert_eq!(
        e.components,
        vec![
            Component::Shape(Shape {
                kind: ShapeKind::Cylinder,
                size: [2.0, 6.0, 2.0],
                color: [0.8, 0.8, 0.8],
                solid: true
            }),
            Component::Character(Character {
                run_speed_mps: 7.0,
                ..Character::default()
            }),
        ]
    );
}

#[test]
fn add_at_puts_the_base_on_the_ground() {
    let mut s = example();
    let r = s.run("entity.add", &json!({"at": [10, 5]})).unwrap();
    let y = ground(&s, 10.0, 5.0);
    assert_eq!(r["position"], json!([10.0, y, 5.0]));
    assert_eq!(s.document().entity(EntityId(4)).unwrap().name, "");
}

#[test]
fn add_at_without_ground_is_an_error_that_says_why() {
    let mut s = example();
    refuse(
        &mut s,
        "entity.add",
        json!({"at": [100, 0]}),
        &["no ground at [100, 0]", "outside the terrain", "-64 to 64"],
    );
    let mut empty = Session::new(World::new("", 0));
    refuse(
        &mut empty,
        "entity.add",
        json!({"at": [0, 0]}),
        &["no terrain"],
    );
}

#[test]
fn add_refuses_bad_placements_and_values() {
    let mut s = example();
    refuse(
        &mut s,
        "entity.add",
        json!({"position": [0, 0, 0], "at": [0, 0]}),
        &["`position` or `at`, not both"],
    );
    refuse(
        &mut s,
        "entity.add",
        json!({"position": [0, 0, 0], "scale": 0}),
        &["scale"],
    );
    refuse(
        &mut s,
        "entity.add",
        json!({"position": [2e6, 0, 0]}),
        &["position[0]"],
    );
    refuse(
        &mut s,
        "entity.add",
        json!({"position": [0, 0, 0], "name": "x".repeat(300)}),
        &["name"],
    );
    refuse(
        &mut s,
        "entity.add",
        json!({"position": [0, 0, 0], "components": {}}),
        &["`components` must be array"],
    );
}

#[test]
fn add_refuses_wrong_components_by_type_and_field() {
    let mut s = example();
    let at = |c: serde_json::Value| json!({"position": [0, 0, 0], "components": [c]});
    refuse(
        &mut s,
        "entity.add",
        at(json!({"type": "shap", "kind": "box"})),
        &["components[0].type", "did you mean `shape`"],
    );
    refuse(
        &mut s,
        "entity.add",
        at(json!({"kind": "box"})),
        &["components[0]", "needs a `type`"],
    );
    refuse(
        &mut s,
        "entity.add",
        at(json!(3)),
        &["components[0]", "must be an object"],
    );
    refuse(
        &mut s,
        "entity.add",
        at(json!({"type": "shape", "kind": "box", "colr": [1, 0, 0]})),
        &["`components[0]` (shape)", "colr"],
    );
    refuse(
        &mut s,
        "entity.add",
        at(json!({"type": "shape", "kind": "box", "size": "big"})),
        &["`components[0]` (shape)", "field `size`"],
    );
    refuse(
        &mut s,
        "entity.add",
        at(json!({"type": "shape"})),
        &["`components[0]` (shape)", "kind"],
    );
    refuse(
        &mut s,
        "entity.add",
        at(json!({"type": "character", "height_m": -1})),
        &["character.height_m"],
    );
    refuse(
        &mut s,
        "entity.add",
        at(json!({"type": "plant", "species": "quercus-robr"})),
        &["components[0].species", "\"quercus-robur\""],
    );
    refuse(
        &mut s,
        "entity.add",
        json!({"position": [0, 0, 0], "components": [{"type": "character"}, {"type": "character"}]}),
        &["more than one `character`"],
    );
}

#[test]
fn set_changes_only_the_fields_given_by_id_or_name() {
    let mut s = example();
    let crate_before = s.document().entity(EntityId(1)).unwrap().clone();
    let r = s
        .run(
            "entity.set",
            &json!({"id": "crate", "rotation_y_deg": 45, "name": "Box"}),
        )
        .unwrap();
    assert_eq!(r, json!({"id": 1, "position": crate_before.position}));
    let e = s.document().entity(EntityId(1)).unwrap();
    assert_eq!(
        (e.name.as_str(), e.rotation_y_deg, e.scale),
        ("Box", 45.0, 1.0)
    );
    let r = s
        .run("entity.set", &json!({"id": 1, "at": [4, -6]}))
        .unwrap();
    assert_eq!(r["position"], json!([4.0, ground(&s, 4.0, -6.0), -6.0]));
    s.run(
        "entity.set",
        &json!({"id": 1, "position": [1, 1, 1], "scale": 3}),
    )
    .unwrap();
    let e = s.document().entity(EntityId(1)).unwrap();
    assert_eq!((e.position, e.scale), ([1.0, 1.0, 1.0], 3.0));
}

#[test]
fn set_refuses_unknown_entities_and_bad_values() {
    let mut s = example();
    refuse(
        &mut s,
        "entity.set",
        json!({"id": 99, "name": "x"}),
        &["no entity has the id"],
    );
    refuse(
        &mut s,
        "entity.set",
        json!({"id": "Nobody"}),
        &["\"Nobody\""],
    );
    refuse(
        &mut s,
        "entity.set",
        json!({"id": -1}),
        &["`id` must be id or name"],
    );
    refuse(
        &mut s,
        "entity.set",
        json!({"id": 1.5}),
        &["`id` must be id or name"],
    );
    refuse(
        &mut s,
        "entity.set",
        json!({"id": 1, "scale": 5000}),
        &["scale"],
    );
    refuse(
        &mut s,
        "entity.set",
        json!({"id": 1, "at": [500, 0]}),
        &["no ground"],
    );
}

#[test]
fn names_that_match_several_entities_are_refused_as_ambiguous() {
    let mut s = example();
    s.run(
        "entity.add",
        &json!({"name": "crate", "position": [0, 0, 0]}),
    )
    .unwrap();
    refuse(
        &mut s,
        "entity.remove",
        json!({"id": "CRATE"}),
        &["matches 2 entities"],
    );
    s.run("entity.remove", &json!({"id": 4})).unwrap();
}

#[test]
fn remove_removes_and_never_reuses_the_id() {
    let mut s = example();
    assert_eq!(
        s.run("entity.remove", &json!({"id": "Oak"})).unwrap(),
        json!(null)
    );
    assert!(s.document().entity(EntityId(2)).is_none());
    let r = s
        .run("entity.add", &json!({"position": [0, 0, 0]}))
        .unwrap();
    assert_eq!(r["id"], json!(4));
    refuse(
        &mut s,
        "entity.remove",
        json!({"id": 2}),
        &["no entity has the id"],
    );
}

#[test]
fn set_component_adds_or_replaces() {
    let mut s = example();
    s.run("entity.set_component", &json!({"id": "Crate", "component": {"type": "shape", "kind": "sphere", "size": [2, 2, 2]}})).unwrap();
    let e = s.document().entity(EntityId(1)).unwrap();
    assert_eq!(e.components.len(), 1);
    assert_eq!(e.shape().unwrap().kind, ShapeKind::Sphere);
    assert_eq!(
        e.shape().unwrap().color,
        [0.8, 0.8, 0.8],
        "fields left out take their defaults"
    );
    s.run(
        "entity.set_component",
        &json!({"id": 1, "component": {"type": "plant", "species": "quercus-robur"}}),
    )
    .unwrap();
    let e = s.document().entity(EntityId(1)).unwrap();
    assert_eq!(
        e.plant(),
        Some(&Plant {
            species: "quercus-robur".into(),
            seed: 0,
            growth: 1.0
        })
    );
    assert_eq!(e.components.len(), 2);
}

#[test]
fn set_component_refuses_bad_components() {
    let mut s = example();
    refuse(
        &mut s,
        "entity.set_component",
        json!({"id": 1}),
        &["missing `component`"],
    );
    refuse(
        &mut s,
        "entity.set_component",
        json!({"id": 1, "component": {"type": "shape", "kind": "sphere", "size": [1, 2, 3]}}),
        &["shape.size", "sphere"],
    );
    refuse(
        &mut s,
        "entity.set_component",
        json!({"id": 1, "component": {"type": "plant", "species": "nope", "growth": 0.5}}),
        &["component.species", "nearest"],
    );
    refuse(
        &mut s,
        "entity.set_component",
        json!({"id": 1, "component": {"type": "plant", "species": "quercus-robur", "growth": 0}}),
        &["plant.growth"],
    );
    refuse(
        &mut s,
        "entity.set_component",
        json!({"id": 1, "component": []}),
        &["`component` must be object"],
    );
}

#[test]
fn remove_component_removes_one_type() {
    let mut s = example();
    s.run(
        "entity.remove_component",
        &json!({"id": "Oak", "type": "plant"}),
    )
    .unwrap();
    assert!(
        s.document()
            .entity(EntityId(2))
            .unwrap()
            .components
            .is_empty()
    );
    refuse(
        &mut s,
        "entity.remove_component",
        json!({"id": "Oak", "type": "plant"}),
        &["has no `plant` component", "its components: none"],
    );
    refuse(
        &mut s,
        "entity.remove_component",
        json!({"id": 1, "type": "plant"}),
        &["has no `plant`", "its components: shape"],
    );
    refuse(
        &mut s,
        "entity.remove_component",
        json!({"id": 1, "type": "lamp"}),
        &["`type` must be one of \"shape\", \"plant\", \"character\", \"zone\""],
    );
}

#[test]
fn snap_to_ground_moves_one_or_all_and_counts_the_skipped() {
    let mut s = example();
    s.run("entity.set", &json!({"id": 1, "position": [-6, 30, 4]}))
        .unwrap();
    s.run(
        "entity.add",
        &json!({"name": "Far", "position": [500, 3, 0]}),
    )
    .unwrap();
    let r = s
        .run("entity.snap_to_ground", &json!({"id": "Crate"}))
        .unwrap();
    assert_eq!(r, json!({"moved": 1, "skipped": 0}));
    assert_eq!(
        s.document().entity(EntityId(1)).unwrap().position,
        [-6.0, ground(&s, -6.0, 4.0), 4.0]
    );
    let r = s.run("entity.snap_to_ground", &json!({})).unwrap();
    assert_eq!(r, json!({"moved": 3, "skipped": 1}));
    assert_eq!(
        s.document().entity(EntityId(4)).unwrap().position,
        [500.0, 3.0, 0.0]
    );
    let r = s
        .run("entity.snap_to_ground", &json!({"id": "Far"}))
        .unwrap();
    assert_eq!(r, json!({"moved": 0, "skipped": 1}));
    refuse(
        &mut s,
        "entity.snap_to_ground",
        json!({"id": "Nobody"}),
        &["Nobody"],
    );
}

#[test]
fn remove_many_refuses_an_id_that_is_not_there_like_a_name_that_is_not() {
    let mut s = example();
    let before = snapshot(&s);
    // An id no entity has is an error naming it, as a name no entity has is; nothing goes.
    refuse(
        &mut s,
        "entity.remove_many",
        json!({"ids": [1, 999_999]}),
        &["`ids`", "no entity has the id 999999"],
    );
    refuse(
        &mut s,
        "entity.remove_many",
        json!({"ids": [999_998, 999_999]}),
        &["`ids`: no entity has the ids 999998, 999999"],
    );
    // At most ten are named, and the rest counted.
    let absent: Vec<u64> = (2_000_000..2_000_013).collect();
    refuse(
        &mut s,
        "entity.remove_many",
        json!({"ids": absent}),
        &[
            "no entity has the ids 2000000, 2000001",
            "2000009 and 3 more",
        ],
    );
    assert_eq!(snapshot(&s), before);
    // A negative, a fractional number and 0 are not ids.
    for (item, got) in [
        (json!(-3), "the number -3"),
        (json!(1.5), "the number 1.5"),
        (json!(0), "the number 0"),
    ] {
        refuse(
            &mut s,
            "entity.remove_many",
            json!({"ids": [1, item]}),
            &[
                "`ids` must be a list of entities",
                "a whole number from 1",
                got,
            ],
        );
    }
    assert_eq!(snapshot(&s), before);
}

#[test]
fn remove_many_removes_a_set_with_its_children_once_checked() {
    let mut s = example();
    let before = s.document().entity_count();
    // A prefab of two children, placed twice: the instance and its children go together.
    s.run(
        "prefab.add",
        &json!({"id": "camp", "name": "Camp", "children": [
            {"name": "Tent", "offset": [2, 0, 0], "components": [{"type": "shape", "kind": "box", "size": [2, 1, 2]}]},
            {"name": "Fire", "offset": [0, 0, 2]}
        ]}),
    )
    .unwrap();
    let a = s
        .run("prefab.spawn", &json!({"prefab": "camp", "at": [20, 20]}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let b = s
        .run("prefab.spawn", &json!({"prefab": "camp", "at": [30, 30]}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    assert_eq!(s.document().entity_count(), before + 6);
    // The example world's Oak and Crate by name, both camps by id: eight go (each camp takes
    // its two children along).
    assert_eq!(
        s.run(
            "entity.remove_many",
            &json!({"ids": ["Oak", "Crate", a, b]})
        )
        .unwrap(),
        json!({"removed": 8})
    );
    assert_eq!(s.document().entity_count(), before + 6 - 8);
    assert!(s.document().entity(EntityId(a)).is_none());
    assert!(s.document().entity(EntityId(b)).is_none());
    assert!(s.document().entity(EntityId(2)).is_none(), "the Oak");
    // One undo step gives all eight back.
    s.undo().unwrap();
    assert_eq!(s.document().entity_count(), before + 6);
    assert!(s.document().entity(EntityId(a)).is_some());
    // Bad parameters name `ids`, and change nothing.
    refuse(
        &mut s,
        "entity.remove_many",
        json!({"ids": []}),
        &["`ids` holds 0 entities"],
    );
    refuse(
        &mut s,
        "entity.remove_many",
        json!({"ids": vec![1; 1_000_001]}),
        &["`ids` holds 1000001 entities"],
    );
    refuse(
        &mut s,
        "entity.remove_many",
        json!({"ids": [1, "Nobody"]}),
        &["`ids`", "Nobody"],
    );
    refuse(
        &mut s,
        "entity.remove_many",
        json!({"ids": [1, true]}),
        &["`ids` must be a list of entities", "a boolean"],
    );
    refuse(
        &mut s,
        "entity.remove_many",
        json!({"ids": "Oak"}),
        &["`ids` must be array"],
    );
}
