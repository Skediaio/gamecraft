//! A scale per axis: the file form (a number or `[x, y, z]`), validation, children that take
//! the instance's scales in the instance's frame, rescaling, the collider box and the cooked
//! form.

use crate::limits::{SCALE_MAX, SCALE_MIN};
use crate::{
    Component, EntityDraft, Model, ModelBounds, ModelCollider, Prefab, PrefabChild, Scale, Shape,
    ShapeKind, Spawn, World, WorldError,
};
use serde_json::{Value, json};

/// A world of one entity with the `"scale"` text `scale` in its file form.
fn one(scale: &str) -> Result<World, WorldError> {
    let text = format!(
        r#"{{"gamecraft": 1, "name": "W", "seed": 1, "next_entity_id": 2, "entities": [
            {{"id": 1, "name": "E", "position": [0.0, 0.0, 0.0], "scale": {scale},
              "components": [{{"type": "shape", "kind": "box", "size": [1.0, 2.0, 4.0]}}]}}
        ]}}"#
    );
    World::from_json(&text)
}

fn entity(world: &World) -> crate::Entity {
    world.entities().next().unwrap().clone()
}

/// Whether no entity of the file text says something of a child (an unedited child is derived
/// and is not written at all; a prefab's own `children` are its definition, not an edit).
fn no_child_is_written(text: &str) -> bool {
    let Ok(Value::Object(document)) = serde_json::from_str(text) else {
        return false;
    };
    document
        .get("entities")
        .and_then(Value::as_array)
        .is_some_and(|entities| {
            entities
                .iter()
                .all(|e| e.get("children").is_none() && e.get("child_edits").is_none())
        })
}

/// The entity `id` of `w`, by value.
fn entity_of(w: &World, id: crate::EntityId) -> crate::Entity {
    w.entity(id).unwrap().clone()
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

#[test]
fn a_number_is_a_uniform_scale_and_is_written_as_a_number() {
    let w = one("2.0").unwrap();
    assert_eq!(entity(&w).scale, Scale::uniform(2.0));
    assert!(entity(&w).scale.is_uniform());
    let text = w.to_json().unwrap();
    assert!(text.contains(r#""scale": 2.0"#), "{text}");
    // Three equal numbers are the same scale, and are written as one.
    let same = one("[2.0, 2.0, 2.0]").unwrap();
    assert_eq!(same, w, "an equal scale is the same scale");
    assert_eq!(same.to_json().unwrap(), text);
    // The example document of the format prints as it did.
    let example = World::from_json(super::EXAMPLE_COMPACT).unwrap();
    assert_eq!(example.to_json().unwrap(), super::EXAMPLE_PRETTY);
}

#[test]
fn a_scale_per_axis_round_trips_and_is_written_as_an_array() {
    let w = one("[2.0, 1.0, 3.0]").unwrap();
    assert_eq!(entity(&w).scale, Scale::per_axis(2.0, 1.0, 3.0));
    assert_eq!(entity(&w).scale.get(), [2.0, 1.0, 3.0]);
    assert!(!entity(&w).scale.is_uniform());
    let text = w.to_json().unwrap();
    let Value::Object(document) = serde_json::from_str(&text).unwrap() else {
        panic!("not an object");
    };
    let entities = document.get("entities").unwrap().as_array().unwrap();
    assert_eq!(entities.first().unwrap()["scale"], json!([2.0, 1.0, 3.0]));
    assert_eq!(World::from_json(&text).unwrap(), w);
}

#[test]
fn a_scale_cannot_be_nothing_or_out_of_range_on_any_axis() {
    for (text, needle) in [
        ("0.0", "scale[0] is 0"),
        ("-1.0", "scale[0] is -1"),
        ("1000.5", "scale[0] is 1000.5"),
        ("[0.0, 1.0, 1.0]", "scale[0] is 0"),
        ("[1.0, 1.0, -2.0]", "scale[2] is -2"),
        ("[1.0, 2000.0, 1.0]", "scale[1] is 2000"),
    ] {
        let e = one(text).unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains(needle), "{text}: {msg}");
        assert!(msg.contains(&format!("{SCALE_MIN}")), "{msg}");
        assert!(msg.contains(&format!("{SCALE_MAX}")), "{msg}");
    }
    for (text, needle) in [
        ("[1.0, 1.0]", "[x, y, z]"),
        ("[1.0, 1.0, 1.0, 1.0]", "[x, y, z]"),
        (r#""big""#, "scale"),
        ("[1.0, null, 1.0]", "scale"),
        ("null", "scale"),
        ("[1.0, 1.0, 1e400]", "scale"),
    ] {
        let e = one(text).unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains(needle), "{text}: {msg}");
    }
    // The limits themselves are allowed, on every axis.
    assert!(one(&format!("[{SCALE_MIN}, {SCALE_MAX}, 1.0]")).is_ok());
    // An entity built in code is checked the same way.
    let mut w = World::new("W", 1);
    let bad = w.add_entity(EntityDraft {
        scale: Scale::per_axis(1.0, 0.0, 1.0),
        ..EntityDraft::default()
    });
    assert!(bad.unwrap_err().to_string().contains("scale[1] is 0"));
    assert_eq!(w.entity_count(), 0);
}

#[test]
fn the_middle_and_the_largest_of_a_scale() {
    let s = Scale::per_axis(1.0, 1.0, 10.0);
    assert_eq!((s.middle(), s.largest()), (1.0, 10.0));
    let s = Scale::per_axis(10.0, 10.0, 1.0);
    assert_eq!((s.middle(), s.largest()), (10.0, 10.0));
    let s = Scale::per_axis(10.0, 1.0, 10.0);
    assert_eq!((s.middle(), s.largest()), (10.0, 10.0));
    let s = Scale::uniform(2.5);
    assert_eq!((s.middle(), s.largest()), (2.5, 2.5));
    assert_eq!(s.times([1.0, 2.0, 4.0]), [2.5, 5.0, 10.0]);
    assert_eq!(
        Scale::per_axis(2.0, 1.0, 4.0).times([1.0, 2.0, 0.5]),
        [2.0, 2.0, 2.0]
    );
    assert_eq!(Scale::ONE, Scale::uniform(1.0));
    assert_eq!(Scale::default(), Scale::ONE);
}

/// The prefab family of the children tests: `outer` has three children of its own and places
/// `inner`, turned.
fn prefabs() -> Vec<Prefab> {
    let shape = |size: [f64; 3]| {
        Component::Shape(Shape {
            visible: true,
            kind: ShapeKind::Box,
            size,
            ..Shape::default()
        })
    };
    vec![
        Prefab {
            id: "inner".into(),
            name: "Inner".into(),
            components: Vec::new(),
            children: vec![PrefabChild {
                name: "E".into(),
                offset: [3.0, 0.0, 0.0],
                rotation_y_deg: 0.0,
                prefab: None,
                components: vec![shape([1.0, 1.0, 1.0])],
            }],
        },
        Prefab {
            id: "outer".into(),
            name: "Outer".into(),
            components: Vec::new(),
            children: vec![
                PrefabChild {
                    name: "A".into(),
                    offset: [1.0, 0.0, 2.0],
                    rotation_y_deg: 0.0,
                    prefab: None,
                    components: vec![shape([1.0, 1.0, 1.0])],
                },
                PrefabChild {
                    name: "B".into(),
                    offset: [0.0, 1.0, 0.0],
                    rotation_y_deg: 90.0,
                    prefab: None,
                    components: vec![shape([2.0, 1.0, 4.0])],
                },
                PrefabChild {
                    name: "C".into(),
                    offset: [3.0, 0.0, 0.0],
                    rotation_y_deg: 45.0,
                    prefab: None,
                    components: vec![shape([1.0, 1.0, 1.0])],
                },
                PrefabChild {
                    name: "D".into(),
                    offset: [0.0, 0.0, 5.0],
                    rotation_y_deg: 90.0,
                    prefab: Some("inner".into()),
                    components: Vec::new(),
                },
            ],
        },
    ]
}

fn spawned() -> (World, [crate::EntityId; 5]) {
    let mut w = World::new("W", 1);
    w.prefabs = prefabs();
    let ids = w
        .spawn_prefab(
            "outer",
            Spawn {
                position: [100.0, 0.0, 100.0],
                scale: Scale::per_axis(1.0, 2.0, 10.0),
                ..Spawn::default()
            },
        )
        .unwrap();
    // Its four children in the prefab's order, then the nested instance's child, which gets
    // the next id.
    let [a, b, c, d] = [
        ids.children[0],
        ids.children[1],
        ids.children[2],
        ids.children[3],
    ];
    (w, [a, b, c, d, crate::EntityId(d.0 + 1)])
}

#[test]
fn children_take_the_instance_scales_in_the_instances_frame() {
    let (w, [a, b, c, d, e]) = spawned();
    // An unturned child: every offset and its own size by the axis.
    assert!(
        close(entity_of(&w, a).position, [101.0, 0.0, 120.0]),
        "{:?}",
        entity_of(&w, a).position
    );
    assert_eq!(entity_of(&w, a).scale, Scale::per_axis(1.0, 2.0, 10.0));
    // Turned a right angle: the instance's z scale is its x, and its x the instance's z.
    assert!(
        close(entity_of(&w, b).position, [100.0, 2.0, 100.0]),
        "{:?}",
        entity_of(&w, b).position
    );
    assert_eq!(entity_of(&w, b).scale, Scale::per_axis(10.0, 2.0, 1.0));
    // Turned by any other angle it keeps the scales along its own axes.
    assert!(
        close(entity_of(&w, c).position, [103.0, 0.0, 100.0]),
        "{:?}",
        entity_of(&w, c).position
    );
    assert_eq!(entity_of(&w, c).scale, Scale::per_axis(1.0, 2.0, 10.0));
    // A nested instance turned a right angle, and its own child inside it: the instance's
    // z scale runs along the child's x.
    let inner = entity_of(&w, d);
    assert!(
        close(inner.position, [100.0, 0.0, 150.0]),
        "{:?}",
        inner.position
    );
    assert_eq!(inner.scale, Scale::per_axis(10.0, 2.0, 1.0));
    assert!(
        close(entity_of(&w, e).position, [100.0, 0.0, 120.0]),
        "{:?}",
        entity_of(&w, e).position
    );
    assert_eq!(entity_of(&w, e).scale, Scale::per_axis(10.0, 2.0, 1.0));
    assert!(w.validate().is_ok());
    // Nothing of an unedited child is written, and the file gives the same world.
    let text = w.to_json().unwrap();
    assert!(no_child_is_written(&text), "{text}");
    assert_eq!(World::from_json(&text).unwrap(), w);
}

#[test]
fn rescaling_an_instance_rescales_its_children_per_axis() {
    let (mut w, [a, b, c, d, e]) = spawned();
    let (root, _) = {
        let first = w.entities().next().unwrap();
        (first.id, ())
    };
    w.set_scale(root, Scale::uniform(1.0)).unwrap();
    assert!(
        close(entity_of(&w, a).position, [101.0, 0.0, 102.0]),
        "{:?}",
        entity_of(&w, a).position
    );
    assert_eq!(entity_of(&w, a).scale, Scale::ONE);
    assert!(
        close(entity_of(&w, b).position, [100.0, 1.0, 100.0]),
        "{:?}",
        entity_of(&w, b).position
    );
    assert!(
        close(entity_of(&w, c).position, [103.0, 0.0, 100.0]),
        "{:?}",
        entity_of(&w, c).position
    );
    assert!(
        close(entity_of(&w, d).position, [100.0, 0.0, 105.0]),
        "{:?}",
        entity_of(&w, d).position
    );
    assert!(
        close(entity_of(&w, e).position, [100.0, 0.0, 102.0]),
        "{:?}",
        entity_of(&w, e).position
    );
    for id in [a, b, c, d, e] {
        assert_eq!(entity_of(&w, id).scale, Scale::ONE);
    }
    // Derived children stay derived: the file has no word of them.
    let text = w.to_json().unwrap();
    assert!(no_child_is_written(&text), "{text}");
    assert_eq!(World::from_json(&text).unwrap(), w);
    // Back again, by other axes.
    w.set_scale(root, Scale::per_axis(0.5, 3.0, 2.0)).unwrap();
    assert!(
        close(entity_of(&w, a).position, [100.5, 0.0, 104.0]),
        "{:?}",
        entity_of(&w, a).position
    );
    assert_eq!(entity_of(&w, a).scale, Scale::per_axis(0.5, 3.0, 2.0));
    assert_eq!(entity_of(&w, b).scale, Scale::per_axis(2.0, 3.0, 0.5));
    assert_eq!(entity_of(&w, e).scale, Scale::per_axis(2.0, 3.0, 0.5));
    // E lies 3 m along the nested instance's x, which runs along the outer instance's -z.
    assert!(
        close(entity_of(&w, e).position, [100.0, 0.0, 104.0]),
        "{:?}",
        entity_of(&w, e).position
    );
    // Moving and turning the instance keeps them relative, at their scales.
    w.place_entity(root, [0.0, 1.0, 0.0], 90.0).unwrap();
    assert_eq!(entity_of(&w, a).scale, Scale::per_axis(0.5, 3.0, 2.0));
    assert!(
        close(entity_of(&w, a).position, [4.0, 1.0, -0.5]),
        "{:?}",
        entity_of(&w, a).position
    );
    assert!(w.validate().is_ok());
}

#[test]
fn a_child_with_a_scale_of_its_own_keeps_it_when_the_instance_changes() {
    let (mut w, [a, b, ..]) = spawned();
    let root = w.entities().next().unwrap().id;
    // B is turned a right angle inside the instance: its own scale, twice the instance's on
    // every axis of its own frame.
    let own = Scale::uniform(2.0);
    {
        let derived = w.entity(b).unwrap().scale;
        w.entity_mut(b).unwrap().scale =
            Scale::per_axis(derived.x * 2.0, derived.y * 2.0, derived.z * 2.0);
    }
    w.set_scale(root, Scale::uniform(1.0)).unwrap();
    assert_eq!(
        w.entity(b).unwrap().scale,
        own,
        "twice a uniform scale is twice a uniform scale"
    );
    assert_eq!(w.entity(a).unwrap().scale, Scale::ONE);
    // The edited child is written as an edit; the others are not.
    let text = w.to_json().unwrap();
    assert!(
        text.contains("child_edits") && text.contains(r#""scale""#),
        "{text}"
    );
    assert_eq!(World::from_json(&text).unwrap(), w);
}

#[test]
fn a_models_collider_box_follows_the_scale_per_axis() {
    let mut w = World::new("W", 1);
    w.set_model_bounds(
        [(
            "models/crate.glb".to_string(),
            ModelBounds {
                min: [-1.0, 0.0, -2.0],
                max: [1.0, 1.0, 2.0],
            },
        )]
        .into_iter()
        .collect(),
    );
    let id = w
        .add_entity(EntityDraft {
            position: [10.0, 1.0, 5.0],
            scale: Scale::per_axis(2.0, 3.0, 0.5),
            components: vec![Component::Model(Model {
                collider: ModelCollider::Box,
                ..Model::of("models/crate.glb")
            })],
            ..EntityDraft::default()
        })
        .unwrap();
    let e = w.entity(id).unwrap();
    let b = w.model_box(e).unwrap();
    assert_eq!(b.center, [10.0, 1.0 + 1.5, 5.0]);
    assert_eq!(b.half, [2.0, 1.5, 1.0]);
    // The same with a uniform scale is the same box.
    w.set_scale(id, Scale::uniform(2.0)).unwrap();
    let b = w.model_box(w.entity(id).unwrap()).unwrap();
    assert_eq!(b.half, [2.0, 1.0, 4.0]);
}

#[test]
fn the_cooked_form_keeps_a_scale_per_axis() {
    let w = one("[2.0, 1.0, 3.0]").unwrap();
    let source = crate::cooked::source_hash(b"anything");
    let cooked = w.to_cooked(source).unwrap();
    let back = World::from_cooked(&cooked, source).unwrap();
    assert_eq!(back, w);
    assert_eq!(
        back.entities().next().unwrap().scale,
        Scale::per_axis(2.0, 1.0, 3.0)
    );
    // An instance with children, scaled per axis: its children are placed again on reading.
    let (w, _) = spawned();
    let cooked = w.to_cooked(source).unwrap();
    assert_eq!(World::from_cooked(&cooked, source).unwrap(), w);
}

#[test]
fn a_json_object_for_a_draft_scale_is_written_per_axis() {
    let mut w = World::new("W", 1);
    w.add_entity(EntityDraft {
        scale: Scale::per_axis(1.0, 1.0, 10.0),
        ..EntityDraft::default()
    })
    .unwrap();
    let text = w.to_json().unwrap();
    let Value::Object(document) = serde_json::from_str(&text).unwrap() else {
        panic!("not an object");
    };
    let entities = document.get("entities").unwrap().as_array().unwrap();
    assert_eq!(entities.first().unwrap()["scale"], json!([1.0, 1.0, 10.0]));
}
