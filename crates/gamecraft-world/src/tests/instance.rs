//! Prefab instances in the file: children derived from the prefab, only differences written.

use crate::{
    Component, EntityDraft, EntityId, Prefab, PrefabChild, Shape, ShapeKind, Spawn, Tags, World,
};
use serde_json::Value;

fn boxed(size: [f64; 3]) -> Vec<Component> {
    vec![Component::Shape(Shape {
        visible: true,
        kind: ShapeKind::Box,
        size,
        color: [0.58, 0.54, 0.48],
        solid: true,
    })]
}

fn child(name: &str, offset: [f64; 3], size: [f64; 3]) -> PrefabChild {
    PrefabChild {
        name: name.into(),
        offset,
        rotation_y_deg: 0.0,
        prefab: None,
        components: boxed(size),
    }
}

/// A courtyard lot: a house, four walls and a gate (6 children).
fn lot() -> Prefab {
    Prefab {
        id: "lot".into(),
        name: "Lot".into(),
        components: vec![Component::Tags(Tags {
            tags: vec!["lot".into()],
        })],
        children: vec![
            child("house", [0.0, 0.0, 0.0], [8.8, 3.0, 8.6]),
            child("wall-gate", [-1.25, -0.25, -5.0], [7.5, 2.5, 0.4]),
            child("wall-back", [0.0, -0.25, 5.0], [10.0, 2.5, 0.4]),
            child("wall-left", [-5.0, -0.25, 0.0], [0.4, 2.5, 10.0]),
            child("wall-right", [5.0, -0.25, 0.0], [0.4, 2.5, 10.0]),
            child("gate", [3.75, 0.0, -5.0], [2.5, 2.2, 0.1]),
        ],
    }
}

fn at(i: usize) -> Spawn {
    let f = i as f64;
    Spawn {
        name: Some(format!("lot-{i:05}")),
        position: [f * 13.37 + 0.5, 1.25, (i % 37) as f64 * 11.1],
        rotation_y_deg: (f * 7.3) % 360.0,
        ..Spawn::default()
    }
}

pub(crate) fn lots(n: usize) -> World {
    let mut w = World::new("Lots", 3);
    w.put_prefab(lot()).unwrap();
    for i in 0..n {
        w.spawn_prefab("lot", at(i)).unwrap();
    }
    w
}

/// Reads `w` back from its text, checks the world and the text are the same, returns the
/// text as JSON.
fn round_trip(w: &World) -> Value {
    let text = w.to_json().unwrap();
    let back = World::from_json(&text).unwrap();
    assert_eq!(&back, w, "the world reads back the same");
    assert_eq!(back.to_json().unwrap(), text, "the text is stable");
    serde_json::from_str(&text).unwrap()
}

fn entry(file: &Value, id: EntityId) -> Option<&Value> {
    file["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"].as_u64() == Some(id.0))
}

#[test]
fn an_instance_is_written_without_its_children() {
    let mut w = lots(3);
    // An entity of its own in between, so ids are not all from instances.
    w.add_entity(EntityDraft::default()).unwrap();
    w.spawn_prefab("lot", at(9)).unwrap();
    let file = round_trip(&w);
    assert_eq!(file["derived_children"], true);
    assert_eq!(file["entities"].as_array().unwrap().len(), 5);
    let first = entry(&file, EntityId(1)).unwrap();
    assert!(first.get("children").is_none(), "default ids: {first}");
    assert!(
        first.get("child_edits").is_none(),
        "nothing edited: {first}"
    );
    let back = World::from_json(&w.to_json().unwrap()).unwrap();
    assert_eq!(back.entity_count(), 4 * 7 + 1);
    assert_eq!(back.children(EntityId(1)).len(), 6);
    assert_eq!(back.entity(EntityId(7)).unwrap().prefab_child, Some(5));
}

#[test]
fn a_world_without_children_writes_no_flag() {
    let mut w = World::new("plain", 1);
    w.add_entity(EntityDraft::default()).unwrap();
    let mut p = lot();
    p.children.clear();
    w.put_prefab(p).unwrap();
    w.spawn_prefab("lot", Spawn::default()).unwrap();
    let file = round_trip(&w);
    assert!(file.get("derived_children").is_none(), "{file}");
}

#[test]
fn an_edited_child_writes_only_what_differs() {
    let mut w = lots(2);
    let gate = EntityId(1 + 6);
    let wall = EntityId(1 + 2);
    {
        let g = w.entity_mut(gate).unwrap();
        g.name = "open gate".into();
        g.rotation_y_deg += 90.0;
    }
    w.entity_mut(wall)
        .unwrap()
        .set_component(Component::Tags(Tags {
            tags: vec!["breach".into()],
        }));
    w.entity_mut(wall).unwrap().remove_component("shape");
    let file = round_trip(&w);
    let edits = entry(&file, EntityId(1)).unwrap()["child_edits"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(edits.len(), 2, "{edits:?}");
    let wall_edit = &edits[0];
    assert_eq!(wall_edit["index"], 1);
    assert_eq!(
        wall_edit["removed_components"],
        serde_json::json!(["shape"])
    );
    assert_eq!(wall_edit["overrides"][0]["type"], "tags");
    assert!(wall_edit.get("name").is_none() && wall_edit.get("position").is_none());
    let gate_edit = &edits[1];
    assert_eq!(gate_edit["index"], 5);
    assert_eq!(gate_edit["name"], "open gate");
    assert!(gate_edit.get("rotation_y_deg").is_some());
    assert!(gate_edit.get("position").is_none(), "{gate_edit}");
    assert!(
        entry(&file, EntityId(8))
            .unwrap()
            .get("child_edits")
            .is_none()
    );
}

#[test]
fn a_removed_child_is_written_as_null_and_stays_removed() {
    let mut w = lots(2);
    w.remove_entity(EntityId(3)).unwrap();
    let file = round_trip(&w);
    let ids = &entry(&file, EntityId(1)).unwrap()["children"];
    assert_eq!(*ids, serde_json::json!([2, null, 4, 5, 6, 7]));
    let back = World::from_json(&w.to_json().unwrap()).unwrap();
    assert!(back.entity(EntityId(3)).is_none());
    assert_eq!(back.entity_count(), 13);
}

#[test]
fn an_entity_added_under_an_instance_is_written_in_full() {
    let mut w = lots(1);
    let flag = w
        .add_entity(EntityDraft {
            name: "flag".into(),
            components: boxed([0.1, 3.0, 0.1]),
            ..EntityDraft::default()
        })
        .unwrap();
    w.entity_mut(flag).unwrap().parent = Some(EntityId(1));
    // And one under a child.
    let lamp = w.add_entity(EntityDraft::default()).unwrap();
    w.entity_mut(lamp).unwrap().parent = Some(EntityId(2));
    let file = round_trip(&w);
    assert_eq!(entry(&file, flag).unwrap()["parent"], 1);
    assert_eq!(entry(&file, lamp).unwrap()["parent"], 2);
    assert!(
        entry(&file, EntityId(2)).is_none(),
        "the child itself stays derived"
    );
}

#[test]
fn moving_an_instance_keeps_its_children_derived() {
    let mut w = lots(1);
    w.entity_mut(EntityId(4)).unwrap().name = "edited".into();
    w.place_entity(EntityId(1), [500.25, 3.5, -77.125], 123.4)
        .unwrap();
    w.place_entity(EntityId(1), [-3.3, 1.0, 9.9], 271.7)
        .unwrap();
    let file = round_trip(&w);
    let edits = &entry(&file, EntityId(1)).unwrap()["child_edits"];
    assert_eq!(edits.as_array().unwrap().len(), 1, "{edits}");
    assert_eq!(edits[0]["name"], "edited");
    assert!(edits[0].get("position").is_none(), "{edits}");
}

#[test]
fn ids_of_children_survive_removals_of_others_and_new_spawns() {
    let mut w = lots(3);
    w.remove_entity(EntityId(8)).unwrap(); // the second instance with its children
    w.spawn_prefab("lot", at(5)).unwrap();
    let file = round_trip(&w);
    assert!(
        entry(&file, EntityId(15))
            .unwrap()
            .get("children")
            .is_none()
    );
    let back = World::from_json(&w.to_json().unwrap()).unwrap();
    assert_eq!(
        back.children(EntityId(22)),
        (23..=28).map(EntityId).collect::<Vec<_>>()
    );
    assert_eq!(back.next_entity_id(), EntityId(29));
}

fn house() -> Prefab {
    Prefab {
        id: "house".into(),
        name: "House".into(),
        components: boxed([8.0, 3.0, 8.0]),
        children: vec![child("door", [0.0, 0.0, -4.0], [1.0, 2.0, 0.1])],
    }
}

fn compound() -> Prefab {
    let mut p = lot();
    p.id = "compound".into();
    p.children.truncate(3);
    p.children[0] = PrefabChild {
        name: String::new(),
        offset: [1.0, 0.0, 1.0],
        rotation_y_deg: 90.0,
        prefab: Some("house".into()),
        components: vec![Component::Tags(Tags {
            tags: vec!["home".into()],
        })],
    };
    p
}

#[test]
fn a_prefab_inside_a_prefab_places_its_children_too() {
    let mut w = World::new("nested", 1);
    w.put_prefab(house()).unwrap();
    w.put_prefab(compound()).unwrap();
    let s = w.spawn_prefab("compound", at(0)).unwrap();
    // compound 1; house 2 (its door 3); walls 4, 5.
    assert_eq!(s.children, vec![EntityId(2), EntityId(4), EntityId(5)]);
    let home = w.entity(EntityId(2)).unwrap();
    assert_eq!(home.prefab.as_deref(), Some("house"));
    assert_eq!(home.name, "House");
    assert!(home.has_tag("home") && home.shape().is_some());
    assert_eq!(w.entity(EntityId(3)).unwrap().parent, Some(EntityId(2)));
    assert_eq!(w.next_entity_id(), EntityId(6));
    let file = round_trip(&w);
    assert_eq!(file["entities"].as_array().unwrap().len(), 1);

    // An edit deep inside: the door of the house of the compound.
    w.entity_mut(EntityId(3)).unwrap().name = "back door".into();
    let file = round_trip(&w);
    let edit = &entry(&file, EntityId(1)).unwrap()["child_edits"][0];
    assert_eq!(edit["index"], 0);
    assert_eq!(edit["child_edits"][0]["name"], "back door");

    // Removing the whole house removes its door with it.
    w.remove_entity(EntityId(2)).unwrap();
    let file = round_trip(&w);
    assert_eq!(
        entry(&file, EntityId(1)).unwrap()["children"],
        serde_json::json!([null, 4, 5])
    );
}

#[test]
fn nesting_is_limited_and_never_loops() {
    let mut w = World::new("deep", 1);
    let level = |i: usize| Prefab {
        id: format!("p{i}"),
        name: String::new(),
        components: Vec::new(),
        children: vec![PrefabChild {
            name: String::new(),
            offset: [0.0; 3],
            rotation_y_deg: 0.0,
            prefab: (i > 0).then(|| format!("p{}", i - 1)),
            components: Vec::new(),
        }],
    };
    for i in 0..crate::limits::MAX_PREFAB_NESTING {
        w.put_prefab(level(i)).unwrap();
    }
    let s = w.spawn_prefab("p7", Spawn::default()).unwrap();
    assert_eq!(w.entity_count(), 9);
    round_trip(&w);
    let err = w
        .put_prefab(level(crate::limits::MAX_PREFAB_NESTING))
        .unwrap_err()
        .to_string();
    assert!(err.contains("levels deep"), "{err}");

    // A loop: p0 placing p7, which places p0 again.
    let mut looped = level(0);
    looped.children[0].prefab = Some("p7".into());
    let err = w.put_prefab(looped).unwrap_err().to_string();
    assert!(
        err.contains("p0") && err.contains("never inside themselves"),
        "{err}"
    );
    // An unknown prefab inside a prefab.
    let mut unknown = level(0);
    unknown.children[0].prefab = Some("ghost".into());
    let err = w.put_prefab(unknown).unwrap_err().to_string();
    assert!(
        err.contains("children[0].prefab") && err.contains("ghost"),
        "{err}"
    );
    // A prefab placed inside another cannot be removed.
    let err = w.remove_prefab("p0").unwrap_err().to_string();
    assert!(err.contains("p1"), "{err}");
    assert!(w.entity(s.id).is_some());
}

#[test]
fn a_file_with_a_loop_of_prefabs_is_an_error() {
    let text = r#"{"gamecraft": 1, "prefabs": [
        {"id": "a", "children": [{"prefab": "b"}]},
        {"id": "b", "children": [{"prefab": "a"}]}
      ], "entities": [{"id": 1, "prefab": "a"}], "derived_children": true}"#;
    let err = World::from_json(text).unwrap_err().to_string();
    assert!(err.contains("never inside themselves"), "{err}");
}

/// The children of `w` written in full, as an engine before derived children wrote them
/// (keys sorted, the same bytes otherwise).
fn old_text(w: &World) -> String {
    let mut old = w.clone();
    let ids: Vec<EntityId> = old.entities().map(|e| e.id).collect();
    for id in ids {
        old.entity_mut(id).unwrap().prefab_child = None;
    }
    let mut file: Value = serde_json::from_str(&old.to_json().unwrap()).unwrap();
    file.as_object_mut().unwrap().remove("derived_children");
    for e in file["entities"].as_array_mut().unwrap() {
        e.as_object_mut().unwrap().remove("children");
    }
    serde_json::to_string_pretty(&file).unwrap()
}

#[test]
fn a_file_of_an_earlier_engine_reads_and_its_children_become_derived() {
    let mut w = lots(3);
    w.entity_mut(EntityId(3)).unwrap().name = "edited".into();
    w.remove_entity(EntityId(10)).unwrap();
    let old = old_text(&w);
    let back = World::from_json(&old).unwrap();
    assert_eq!(back, w, "every written child is taken as its prefab child");
    assert_eq!(back.to_json().unwrap(), w.to_json().unwrap());
    assert!(old.len() > w.to_json().unwrap().len());
}

#[test]
fn a_child_written_in_full_in_a_new_file_is_merged_not_doubled() {
    let w = lots(1);
    let mut file: Value = serde_json::from_str(&w.to_json().unwrap()).unwrap();
    let mut written = serde_json::to_value(w.entity(EntityId(3)).unwrap()).unwrap();
    written.as_object_mut().unwrap().remove("prefab_child");
    file["entities"].as_array_mut().unwrap().push(written);
    let back = World::from_json(&file.to_string()).unwrap();
    assert_eq!(back, w);
}

#[test]
fn bad_children_and_edits_are_errors_that_name_the_field() {
    let w = lots(2);
    let text = w.to_json().unwrap();
    let with = |f: &dyn Fn(&mut Value)| {
        let mut v: Value = serde_json::from_str(&text).unwrap();
        f(&mut v);
        World::from_json(&v.to_string()).unwrap_err().to_string()
    };
    let err = with(&|v| v["entities"][0]["children"] = serde_json::json!([2, 3]));
    assert!(err.contains("children") && err.contains("has 6"), "{err}");
    let err = with(&|v| {
        v["entities"][0]["child_edits"] = serde_json::json!([{"index": 6, "name": "x"}]);
    });
    assert!(err.contains("child_edits[0].index"), "{err}");
    let err = with(&|v| {
        v["entities"][0]["child_edits"] =
            serde_json::json!([{"index": 1, "name": "x"}, {"index": 1}]);
    });
    assert!(err.contains("at most once"), "{err}");
    let err = with(&|v| {
        v["entities"][0]["children"] = serde_json::json!([2, 3, 4, 5, 6, 8]);
    });
    assert!(err.contains("children[5]"), "{err}");
    let err = with(&|v| {
        v["entities"][0]["children"] = serde_json::json!([2, null, 4, 5, 6, 7]);
        v["entities"][0]["child_edits"] = serde_json::json!([{"index": 1, "name": "x"}]);
    });
    assert!(err.contains("removed"), "{err}");
    let err = with(&|v| {
        v["entities"][0]["child_edits"] =
            serde_json::json!([{"index": 1, "removed_components": ["laser"]}]);
    });
    assert!(err.contains("removed_components"), "{err}");
    let err = with(&|v| {
        v.as_object_mut().unwrap().remove("derived_children");
        v["entities"][0]["children"] = serde_json::json!([2, 3, 4, 5, 6, 7]);
    });
    assert!(err.contains("derived_children"), "{err}");
    // A derived child whose id another entity has.
    let err = with(&|v| {
        v["entities"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"id": 4, "name": "squatter"}));
    });
    assert!(err.contains("children[2]"), "{err}");
}

#[test]
fn changing_a_prefab_brings_the_children_of_its_instances_along() {
    // Instance `a` is 1 (house 2, wall-gate 3, wall-back 4, wall-left 5, wall-right 6,
    // gate 7); `b` is 8 (9 to 14).
    let mut w = lots(2);
    let (a, b) = (EntityId(1), EntityId(8));
    // In `a`: the back wall renamed, the gate removed, a flag of its own under the left wall.
    w.entity_mut(EntityId(4)).unwrap().name = "old back".into();
    w.remove_entity(EntityId(7)).unwrap();
    let flag = w.add_entity(EntityDraft::default()).unwrap();
    w.entity_mut(flag).unwrap().parent = Some(EntityId(5));

    let mut p = lot();
    p.children[1].offset = [-1.25, -0.25, -6.0]; // the gate wall moves out
    p.children[1].components = boxed([7.5, 3.0, 0.4]); // and is higher
    p.children[2].components = boxed([10.0, 3.0, 0.4]);
    p.children.truncate(5); // the gate goes ...
    p.children.remove(3); // ... and so does the left wall
    p.children
        .push(child("well", [2.0, 0.0, 2.0], [1.0, 1.0, 1.0])); // a well comes
    w.put_prefab(p).unwrap();
    w.validate().unwrap();

    // The unedited gate walls moved out and grew.
    for x in [a, b] {
        let wall = w.entity(EntityId(x.0 + 2)).unwrap();
        assert_eq!(wall.shape().unwrap().size, [7.5, 3.0, 0.4]);
        assert_eq!(wall.prefab_child, Some(1));
    }
    let moved = w.entity(EntityId(10)).unwrap().position;
    let before = lots(2).entity(EntityId(10)).unwrap().position;
    assert_ne!(moved, before);
    // The renamed back wall kept its name and took the new size.
    let back = w.entity(EntityId(4)).unwrap();
    assert_eq!(back.name, "old back");
    assert_eq!(back.shape().unwrap().size, [10.0, 3.0, 0.4]);
    // The left wall is gone from the prefab: `a`'s has a flag below it, so it stays as an
    // entity of its own; `b`'s was untouched and is removed.
    let left = w.entity(EntityId(5)).unwrap();
    assert_eq!((left.prefab_child, left.name.as_str()), (None, "wall-left"));
    assert!(w.entity(EntityId(12)).is_none());
    // The right wall is recognised by its name at its new index.
    assert_eq!(w.entity(EntityId(6)).unwrap().prefab_child, Some(3));
    assert_eq!(w.entity(EntityId(13)).unwrap().prefab_child, Some(3));
    // The gate is gone where untouched and stays removed in `a`; the well is in both.
    assert!(w.entity(EntityId(14)).is_none() && w.entity(EntityId(7)).is_none());
    for x in [a, b] {
        let well = w
            .children(x)
            .into_iter()
            .find(|c| w.entity(*c).unwrap().name == "well");
        assert_eq!(
            w.entity(well.unwrap()).unwrap().prefab_child,
            Some(4),
            "{x}"
        );
    }
    let file = round_trip(&w);
    assert!(
        entry(&file, EntityId(5)).is_some(),
        "the kept wall is written in full"
    );
    // A new spawn of the changed prefab has its children at the default ids again.
    let c = w.spawn_prefab("lot", at(7)).unwrap();
    let file = round_trip(&w);
    assert!(entry(&file, c.id).unwrap().get("children").is_none());
}

#[test]
fn a_bigger_prefab_that_does_not_fit_changes_nothing() {
    let mut w = lots(1);
    let before = w.clone();
    let mut p = lot();
    p.children[0].offset = [f64::MAX, 0.0, 0.0];
    assert!(w.put_prefab(p).is_err());
    let mut p = lot();
    p.children[0].offset = [9.0e5, 0.0, 0.0];
    w.place_entity(EntityId(1), [9.0e5, 0.0, 0.0], 0.0).unwrap();
    let moved = w.clone();
    assert!(w.put_prefab(p).is_err(), "a child beyond 1e6 m");
    assert_eq!(w, moved);
    assert_ne!(before, moved);
}

/// Bytes per instance of a prefab with 6 children, with the children written in full (as
/// before) and derived. Counts bytes, not time (AGENTS.md rule 18).
#[test]
fn bytes_per_instance_of_a_six_child_prefab() {
    let n = 1000;
    let empty = lots(0).to_json().unwrap().len();
    let w = lots(n);
    let derived = w.to_json().unwrap().len();
    let old = old_text(&w).len();
    let per = |total: usize| (total - empty) as f64 / n as f64;
    let (before, after) = (per(old), per(derived));
    println!("bytes per 6-child instance: before {before:.1}, after {after:.1}");
    assert!(before > 3000.0, "{before}");
    assert!(after < 300.0, "{after}");
}
