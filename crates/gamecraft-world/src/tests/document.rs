use super::{EXAMPLE_COMPACT, EXAMPLE_PRETTY, example_world, full_world};
use crate::{
    Character, Component, EntityDraft, EntityId, Plant, Shape, ShapeKind, World, WorldError,
};

#[test]
fn golden_document_parses_to_the_expected_world_and_prints_back_identically() {
    let parsed = World::from_json(EXAMPLE_PRETTY).unwrap();
    assert_eq!(parsed, example_world());
    assert_eq!(parsed.to_json().unwrap(), EXAMPLE_PRETTY);
    let compact = World::from_json(EXAMPLE_COMPACT).unwrap();
    assert_eq!(compact, example_world());
}

#[test]
fn to_json_is_stable_and_round_trips() {
    let w = full_world();
    let a = w.to_json().unwrap();
    let b = w.to_json().unwrap();
    assert_eq!(a, b);
    let back = World::from_json(&a).unwrap();
    assert_eq!(back, w);
    assert_eq!(back.to_json().unwrap(), a);
}

#[test]
fn round_trip_keeps_awkward_floats_bit_for_bit() {
    let mut w = World::new("Floats", u64::MAX);
    w.environment.latitude_deg = 0.1 + 0.2;
    w.environment.time_of_day_h = 1.0 / 3.0;
    w.add_entity(EntityDraft {
        position: [1e-7, -123_456.789_012_345_6, 999_999.999_999_9],
        rotation_y_deg: -0.0,
        components: vec![Component::Shape(Shape {
            visible: true,
            kind: ShapeKind::Box,
            size: [0.001, 9999.999, 1.0 / 7.0],
            color: [0.1, 0.2, 1.0 / 3.0],
            solid: false,
        })],
        ..EntityDraft::default()
    })
    .unwrap();
    let back = World::from_json(&w.to_json().unwrap()).unwrap();
    assert_eq!(back, w);
    let e = back.entity(EntityId(1)).unwrap();
    assert_eq!(
        e.position[1].to_bits(),
        (-123_456.789_012_345_6_f64).to_bits()
    );
    assert_eq!(e.rotation_y_deg.to_bits(), (-0.0_f64).to_bits());
    assert_eq!(back.seed, u64::MAX);
}

#[test]
fn entities_are_written_in_id_order() {
    let mut w = World::new("Order", 1);
    for name in ["c", "a", "b"] {
        w.add_entity(EntityDraft {
            name: name.into(),
            ..EntityDraft::default()
        })
        .unwrap();
    }
    w.remove_entity(EntityId(2));
    let json = w.to_json().unwrap();
    let c = json.find("\"c\"").unwrap();
    let b = json.find("\"b\"").unwrap();
    assert!(c < b);
    // Entities listed out of order in a file are still read and written sorted.
    let shuffled = r#"{"gamecraft": 1, "next_entity_id": 9,
        "entities": [{"id": 8, "name": "late"}, {"id": 3, "name": "early"}]}"#;
    let w = World::from_json(shuffled).unwrap();
    let ids: Vec<u64> = w.entities().map(|e| e.id.0).collect();
    assert_eq!(ids, vec![3, 8]);
}

#[test]
fn optional_fields_take_their_defaults() {
    let w = World::from_json(
        r#"{"gamecraft": 1, "entities": [
            {"id": 1, "components": [{"type": "character"}, {"type": "shape", "kind": "box"}]},
            {"id": 2, "components": [{"type": "plant", "species": "betula-pendula"}]}]}"#,
    )
    .unwrap();
    assert_eq!(w.name, "");
    assert_eq!(w.seed, 0);
    assert_eq!(w.terrain, None);
    assert_eq!(w.environment, crate::Environment::default());
    assert_eq!(w.next_entity_id(), EntityId(3));
    let e = w.entity(EntityId(1)).unwrap();
    assert_eq!(e.scale, crate::Scale::ONE);
    assert_eq!(e.position, [0.0; 3]);
    assert_eq!(e.character(), Some(&Character::default()));
    let s = e.shape().unwrap();
    assert_eq!(s.size, [1.0; 3]);
    assert!(s.solid);
    let p = w.entity(EntityId(2)).unwrap().plant().unwrap();
    assert_eq!(p.growth, 1.0);
    assert_eq!(p.seed, 0);
    // Terrain explicitly null is no terrain.
    let w = World::from_json(r#"{"gamecraft": 1, "terrain": null}"#).unwrap();
    assert_eq!(w.terrain, None);
}

fn err(text: &str) -> String {
    match World::from_json(text) {
        Ok(_) => panic!("accepted: {text}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn unknown_fields_are_rejected_everywhere() {
    let cases = [
        (r#"{"gamecraft": 1, "colour": 3}"#, "colour"),
        (r#"{"gamecraft": 1, "environment": {"fog": 1}}"#, "fog"),
        (
            r#"{"gamecraft": 1, "terrain": {"size_m": 64, "cell_m": 2, "source": {"kind": "flat"}, "x": 1}}"#,
            "`x`",
        ),
        (
            r#"{"gamecraft": 1, "terrain": {"size_m": 64, "cell_m": 2, "source": {"kind": "flat", "hight_m": 1}}}"#,
            "hight_m",
        ),
        (
            r#"{"gamecraft": 1, "entities": [{"id": 1, "colour": 1}]}"#,
            "colour",
        ),
        (
            r#"{"gamecraft": 1, "entities": [{"id": 1, "components": [{"type": "shape", "kind": "box", "mass": 3}]}]}"#,
            "mass",
        ),
        (
            r#"{"gamecraft": 1, "entities": [{"id": 1, "components": [{"type": "character", "speed": 3}]}]}"#,
            "speed",
        ),
        (
            r#"{"gamecraft": 1, "entities": [{"id": 1, "components": [{"type": "lamp"}]}]}"#,
            "lamp",
        ),
        (
            r#"{"gamecraft": 1, "terrain": {"size_m": 64, "cell_m": 2, "source": {"kind": "ridged"}}}"#,
            "ridged",
        ),
    ];
    for (text, needle) in cases {
        let e = err(text);
        assert!(e.contains(needle), "{text}\n -> {e}");
    }
}

#[test]
fn wrong_or_missing_version_is_rejected() {
    for (text, needle) in [
        (r#"{"gamecraft": 2}"#, "is 2"),
        (r#"{"gamecraft": 2, "new_thing": true}"#, "is 2"),
        (r#"{"gamecraft": "1"}"#, "is \"1\""),
        (r#"{"name": "x"}"#, "missing"),
        (r#"{"gamecraft": 0}"#, "is 0"),
    ] {
        let e = err(text);
        assert!(
            e.contains("gamecraft") && e.contains(needle),
            "{text} -> {e}"
        );
        assert!(e.contains("version 1"), "{e}");
    }
}

#[test]
fn duplicate_ids_and_ids_at_or_above_next_are_rejected() {
    let e = err(r#"{"gamecraft": 1, "entities": [{"id": 4}, {"id": 4}]}"#);
    assert!(e.contains("id 4") && e.contains("more than one"), "{e}");
    let e = err(r#"{"gamecraft": 1, "next_entity_id": 4, "entities": [{"id": 4}]}"#);
    assert!(
        e.contains("entity 4") && e.contains("next_entity_id"),
        "{e}"
    );
    let e = err(r#"{"gamecraft": 1, "next_entity_id": 2, "entities": [{"id": 1}, {"id": 7}]}"#);
    assert!(e.contains("entity 7") && e.contains("at least 8"), "{e}");
    let e = err(r#"{"gamecraft": 1, "entities": [{"id": 0}]}"#);
    assert!(e.contains("entity 0") && e.contains("id"), "{e}");
    let e = err(r#"{"gamecraft": 1, "next_entity_id": 0}"#);
    assert!(e.contains("next_entity_id"), "{e}");
}

#[test]
fn bad_values_in_a_file_are_rejected_with_the_field() {
    let e = err(r#"{"gamecraft": 1, "entities": [{"id": 1, "scale": 0}]}"#);
    assert!(e.contains("entity 1") && e.contains("scale"), "{e}");
    let e = err(r#"{"gamecraft": 1, "environment": {"day_of_year": 400}}"#);
    assert!(e.contains("day_of_year") && e.contains("400"), "{e}");
    let e = err(r#"{"gamecraft": 1, "entities": [{"id": 1, "position": [1e999, 0, 0]}]}"#);
    assert!(!e.is_empty());
    let e = err(r#"{"gamecraft": 1, "seed": -1}"#);
    assert!(e.contains("line"), "{e}");
    let e = err(
        r#"{"gamecraft": 1, "entities": [{"id": 1, "components": [{"type": "plant", "species": "a"}, {"type": "plant", "species": "b"}]}]}"#,
    );
    assert!(e.contains("entity 1") && e.contains("plant"), "{e}");
}

#[test]
fn find_by_id_and_by_name() {
    let mut w = full_world();
    assert_eq!(w.find("2").unwrap(), EntityId(2));
    assert_eq!(w.find(" 2 ").unwrap(), EntityId(2));
    assert_eq!(w.find("oak").unwrap(), EntityId(2));
    assert_eq!(w.find("CRATE").unwrap(), EntityId(1));
    w.add_entity(EntityDraft {
        name: "OAK".into(),
        ..EntityDraft::default()
    })
    .unwrap();
    match w.find("Oak") {
        Err(e @ WorldError::Ambiguous { .. }) => {
            let s = e.to_string();
            assert!(s.contains("2 \"Oak\"") && s.contains("6 \"OAK\""), "{s}");
        }
        other => panic!("{other:?}"),
    }
    match w.find("Birch") {
        Err(e @ WorldError::NotFound { .. }) => {
            let s = e.to_string();
            assert!(s.contains("Birch") && s.contains("candidates"), "{s}");
        }
        other => panic!("{other:?}"),
    }
    // Close names are offered first.
    match w.find("Cra") {
        Err(WorldError::NotFound { candidates, .. }) => {
            assert_eq!(candidates.first().map(String::as_str), Some("1 \"Crate\""));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(w.find("99"), Err(WorldError::NotFound { .. })));
    assert!(
        World::new("e", 0)
            .find("x")
            .unwrap_err()
            .to_string()
            .contains("no entities")
    );
}

#[test]
fn ids_are_never_reused() {
    let mut w = World::new("Ids", 0);
    let a = w.add_entity(EntityDraft::default()).unwrap();
    let b = w.add_entity(EntityDraft::default()).unwrap();
    assert!(w.remove_entity(b).is_some());
    assert!(w.remove_entity(b).is_none());
    let c = w.add_entity(EntityDraft::default()).unwrap();
    assert_eq!((a.0, b.0, c.0), (1, 2, 3));
    // Also across save and load.
    let mut back = World::from_json(&w.to_json().unwrap()).unwrap();
    back.remove_entity(c);
    let d = back.add_entity(EntityDraft::default()).unwrap();
    assert_eq!(d.0, 4);
    assert_eq!(back.entity_count(), 2);
}

#[test]
fn add_entity_validates_and_keeps_the_id_on_failure() {
    let mut w = World::new("V", 0);
    let bad = EntityDraft {
        scale: crate::Scale::uniform(-1.0),
        ..EntityDraft::default()
    };
    let e = w.add_entity(bad).unwrap_err().to_string();
    assert!(e.contains("scale"), "{e}");
    assert_eq!(w.entity_count(), 0);
    assert_eq!(w.add_entity(EntityDraft::default()).unwrap(), EntityId(1));
}

#[test]
fn components_are_set_replaced_and_removed() {
    let mut w = example_world();
    let e = w.entity_mut(EntityId(2)).unwrap();
    e.set_component(Component::Plant(Plant {
        species: "fagus-sylvatica".into(),
        seed: 3,
        growth: 0.5,
    }));
    assert_eq!(e.components.len(), 1);
    assert_eq!(e.plant().unwrap().species, "fagus-sylvatica");
    e.set_component(Component::Character(Character::default()));
    assert_eq!(e.components.len(), 2);
    assert!(e.character().is_some());
    assert!(e.shape().is_none());
    assert_eq!(e.remove_component("plant").map(|c| c.kind()), Some("plant"));
    assert!(e.remove_component("plant").is_none());
    assert!(e.remove_component("nonsense").is_none());
    assert_eq!(Component::KINDS[..3], ["shape", "plant", "character"]);
    assert!(w.validate().is_ok());
}

#[test]
fn entity_id_displays_and_serialises_as_a_number() {
    assert_eq!(EntityId(12).to_string(), "12");
    assert_eq!(serde_json::to_string(&EntityId(12)).unwrap(), "12");
}

#[test]
fn to_json_refuses_an_invalid_world() {
    let mut w = example_world();
    w.entity_mut(EntityId(1)).unwrap().position[0] = f64::NAN;
    let e = w.to_json().unwrap_err().to_string();
    assert!(e.contains("entity 1") && e.contains("position[0]"), "{e}");
}
