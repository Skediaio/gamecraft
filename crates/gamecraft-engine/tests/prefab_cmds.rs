//! Prefab commands: describe once, place many, with overrides and children; spawning on a
//! running world.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, refuse};
use gamecraft_engine::Session;
use gamecraft_world::World;
use serde_json::{Value, json};

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.run(command, &params)
        .unwrap_or_else(|e| panic!("{command} {params}: {e}"))
}

fn soldier() -> Value {
    json!({
        "id": "soldier",
        "name": "Soldier",
        "components": [
            {"type": "character"},
            {"type": "health", "max": 100, "current": 100},
            {"type": "behaviour", "mode": "patrol", "route": [[0, 0], [10, 0]], "team": "red"},
            {"type": "tags", "tags": ["enemy"]}
        ]
    })
}

fn house() -> Value {
    json!({
        "id": "house",
        "name": "House",
        "components": [{"type": "shape", "kind": "box", "size": [6, 4, 8]}],
        "children": [
            {"name": "Door", "offset": [0, 0, 4.5],
             "components": [{"type": "interactable", "prompt": "Open"}]},
            {"name": "Yard", "offset": [0, 0, 8], "rotation_y_deg": 90,
             "components": [{"type": "zone", "shape": "box", "size": [6, 4]}]}
        ]
    })
}

#[test]
fn overrides_replace_the_prefabs_components_by_type() {
    let mut s = example();
    run(&mut s, "prefab.add", soldier());
    let spawned = run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "soldier", "at": [3, 3], "name": "Sergeant",
               "overrides": [{"type": "health", "max": 250, "current": 250}]}),
    );
    let id = spawned["id"].as_u64().unwrap();
    assert_eq!(spawned["children"], json!([]));
    let got = run(&mut s, "entity.get", json!({"id": id}));
    assert_eq!(got["name"], "Sergeant");
    assert_eq!(got["prefab"], "soldier");
    assert_eq!(got["parent"], Value::Null);
    let kinds: Vec<&str> = got["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["character", "health", "behaviour", "tags"]);
    assert_eq!(got["components"][1]["max"], 250.0);
    assert_eq!(got["overrides"].as_array().unwrap().len(), 1);
    assert_eq!(got["overrides"][0]["type"], "health");
    assert_eq!(got["health"]["max"], 250.0);

    // world.inspect shows the prefab, its instances, and the entity's reference.
    let inspected = run(&mut s, "world.inspect", json!({}));
    let prefabs = inspected["prefabs"].as_array().unwrap();
    let soldier = prefabs.iter().find(|p| p["id"] == "soldier").unwrap();
    assert_eq!(soldier["instances"], json!([id]));
    let entity = inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap();
    assert_eq!(entity["prefab"], "soldier");
    assert_eq!(entity["components"].as_array().unwrap().len(), 4);

    // A changed prefab changes its instance, which keeps its override.
    let set = run(
        &mut s,
        "prefab.set",
        json!({"id": "soldier", "components": [
            {"type": "character", "walk_speed_mps": 1},
            {"type": "health", "max": 80, "current": 80}]}),
    );
    assert_eq!(set["instances"], json!([id]));
    let got = run(&mut s, "entity.get", json!({"id": id}));
    assert_eq!(got["components"][0]["walk_speed_mps"], 1.0);
    assert_eq!(got["health"]["max"], 250.0);
    // The behaviour and the tags came from the prefab, which has them no more.
    assert_eq!(got["components"].as_array().unwrap().len(), 2);

    let list = run(&mut s, "prefab.list", json!({}));
    assert_eq!(list["prefabs"].as_array().unwrap().len(), 2);
}

#[test]
fn a_placed_prefab_is_not_removed() {
    let mut s = example();
    run(&mut s, "prefab.add", soldier());
    let a = run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "soldier", "at": [1, 1]}),
    )["id"]
        .as_u64()
        .unwrap();
    let b = run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "soldier", "at": [2, 2]}),
    )["id"]
        .as_u64()
        .unwrap();
    let a_text = format!("{a} \"Soldier\"");
    let b_text = format!("{b} \"Soldier\"");
    refuse(
        &mut s,
        "prefab.remove",
        json!({"id": "soldier"}),
        &["\"soldier\"", &a_text, &b_text, "remove them first"],
    );
    run(&mut s, "entity.remove", json!({"id": a}));
    run(&mut s, "entity.remove", json!({"id": b}));
    run(&mut s, "prefab.remove", json!({"id": "soldier"}));
    assert!(s.document().prefab("soldier").is_none());
}

#[test]
fn children_are_entities_that_follow_their_parent() {
    let mut s = example();
    run(&mut s, "prefab.add", house());
    let spawned = run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "house", "position": [10, 0, 10], "rotation_y_deg": 180}),
    );
    let id = spawned["id"].as_u64().unwrap();
    let children: Vec<u64> = spawned["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_u64().unwrap())
        .collect();
    assert_eq!(children.len(), 2);
    let door = run(&mut s, "entity.get", json!({"id": children[0]}));
    assert_eq!(door["parent"], id);
    assert_eq!(door["name"], "Door");
    // Turned half round, the local south (+Z) is the world's north.
    let p = door["position"].as_array().unwrap();
    assert!((p[0].as_f64().unwrap() - 10.0).abs() < 1e-9, "{door}");
    assert!((p[2].as_f64().unwrap() - 5.5).abs() < 1e-9, "{door}");
    let house = run(&mut s, "entity.get", json!({"id": id}));
    assert_eq!(house["children"], json!(children));

    // Moving the house carries the door.
    run(
        &mut s,
        "entity.set",
        json!({"id": id, "position": [0, 0, 0], "rotation_y_deg": 0}),
    );
    let door = run(&mut s, "entity.get", json!({"id": children[0]}));
    let p = door["position"].as_array().unwrap();
    assert!(p[0].as_f64().unwrap().abs() < 1e-9, "{door}");
    assert!((p[2].as_f64().unwrap() - 4.5).abs() < 1e-9, "{door}");
    let yard = run(&mut s, "entity.get", json!({"id": children[1]}));
    assert!((yard["rotation_y_deg"].as_f64().unwrap() - 90.0).abs() < 1e-9);

    // Removing the house removes its children, as one undo step.
    let count = s.document().entity_count();
    run(&mut s, "entity.remove", json!({"id": id}));
    assert_eq!(s.document().entity_count(), count - 3);
    s.undo().unwrap();
    assert_eq!(s.document().entity_count(), count);
}

#[test]
fn spawning_on_a_running_skirmish_adds_a_soldier_who_patrols() {
    let mut s = Session::new(World::new("Skirmish", 3));
    run(
        &mut s,
        "terrain.set",
        json!({"kind": "flat", "size_m": 128, "cell_m": 4}),
    );
    run(&mut s, "prefab.add", soldier());
    run(
        &mut s,
        "entity.add",
        json!({"name": "Player", "at": [0, 40], "components": [
            {"type": "character"}, {"type": "tags", "tags": ["player", "team:red"]}]}),
    );
    run(&mut s, "sim.start", json!({}));
    run(&mut s, "sim.step", json!({"seconds": 1}));
    let spawned = run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "soldier", "at": [-20, -20], "name": "Reinforcement",
               "overrides": [{"type": "behaviour", "mode": "patrol",
                              "route": [[-20, -20], [20, -20]], "team": "red"}]}),
    );
    let id = spawned["id"].as_u64().unwrap();
    assert!(
        s.document().find("Reinforcement").is_err(),
        "the document is untouched"
    );
    run(&mut s, "sim.step", json!({"seconds": 6}));
    let got = run(&mut s, "entity.get", json!({"id": id}));
    assert_eq!(got["behaviour"]["state"], "patrol", "{got}");
    let x = got["position"][0].as_f64().unwrap();
    assert!(x > -12.0, "the soldier walks its route east: x {x}");
    assert!((got["position"][2].as_f64().unwrap() + 20.0).abs() < 1.0);
    // The journal recorded the spawn, and it replays.
    let journal = s.journal().unwrap();
    assert!(journal.entries.iter().any(|e| e.command == "prefab.spawn"));
    assert_eq!(journal.replay(None).unwrap().divergence, None);
}

#[test]
fn bad_prefab_parameters_are_named() {
    let mut s = example();
    refuse(
        &mut s,
        "prefab.add",
        json!({"id": "marker"}),
        &["\"marker\"", "prefab.set"],
    );
    refuse(
        &mut s,
        "prefab.add",
        json!({"id": "x", "components": [{"type": "shape", "kind": "box", "size": [-1, 1, 1]}]}),
        &["prefab \"x\"", "components.shape.size[0]"],
    );
    refuse(
        &mut s,
        "prefab.add",
        json!({"id": "x", "children": [{"ofset": [0, 0, 0]}]}),
        &["children[0]", "`offset`"],
    );
    refuse(
        &mut s,
        "prefab.add",
        json!({"id": "x", "children": [{"offset": [0, "up", 0]}]}),
        &["children[0].offset[1]"],
    );
    refuse(
        &mut s,
        "prefab.add",
        json!({"id": "x", "components": [{"type": "shap"}]}),
        &["components[0].type", "`shape`"],
    );
    refuse(
        &mut s,
        "prefab.add",
        json!({"id": ""}),
        &["prefab \"\"", "id"],
    );
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "markr", "at": [0, 0]}),
        &["\"markr\"", "\"marker\""],
    );
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "marker"}),
        &["`position` or `at`"],
    );
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "marker", "position": [0, 0, 2e6]}),
        &["position[2]"],
    );
    refuse(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "marker", "at": [0, 0],
               "overrides": [{"type": "tags", "tags": []}, {"type": "tags", "tags": []}]}),
        &["overrides", "tags"],
    );
    refuse(
        &mut s,
        "prefab.set",
        json!({"id": "nothing"}),
        &["\"nothing\""],
    );
    refuse(
        &mut s,
        "prefab.remove",
        json!({"id": "nothing"}),
        &["\"nothing\""],
    );
    // An instance cannot lose a component type of its prefab.
    let id = run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "marker", "at": [1, 1]}),
    )["id"]
        .as_u64()
        .unwrap();
    refuse(
        &mut s,
        "entity.remove_component",
        json!({"id": id, "type": "shape"}),
        &["\"marker\"", "shape"],
    );
}

#[test]
fn a_spawn_is_one_undo_step_and_round_trips_through_the_file() {
    let mut s = example();
    run(&mut s, "prefab.add", house());
    let before = s.document().clone();
    run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "house", "at": [5, 5], "rotation_y_deg": 30,
               "overrides": [{"type": "tags", "tags": ["home"]}]}),
    );
    let after = s.document().clone();
    let text = after.to_json().unwrap();
    assert_eq!(World::from_json(&text).unwrap(), after);
    s.undo().unwrap();
    assert_eq!(*s.document(), before);
}

/// The state hash of `s`'s document after a second of play.
fn hash_after_play(world: World) -> Value {
    let mut s = Session::new(world);
    run(&mut s, "sim.start", json!({}));
    run(&mut s, "sim.step", json!({"ticks": 60}));
    run(&mut s, "sim.state", json!({}))["hash"].clone()
}

#[test]
fn children_are_derived_in_the_file_and_the_state_hash_survives_save_and_load() {
    let mut s = example();
    run(&mut s, "prefab.add", house());
    let a = run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "house", "at": [5, 5]}),
    );
    let b = run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "house", "at": [-6, 9], "rotation_y_deg": 33}),
    );
    let door = a["children"][0].as_u64().unwrap();
    let yard = b["children"][1].as_u64().unwrap();
    let got = run(&mut s, "entity.get", json!({"id": door}));
    assert_eq!(got["prefab_child"], 0);
    run(
        &mut s,
        "entity.set",
        json!({"id": door, "name": "Front door", "rotation_y_deg": 15}),
    );
    run(&mut s, "entity.remove", json!({"id": yard}));
    // Move a whole house: its children go along and stay derived.
    run(
        &mut s,
        "entity.set",
        json!({"id": b["id"], "at": [-7.5, 11.25], "rotation_y_deg": 200}),
    );

    let world = s.document().clone();
    let text = world.to_json().unwrap();
    let file: Value = serde_json::from_str(&text).unwrap();
    let written: Vec<u64> = file["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_u64().unwrap())
        .collect();
    assert!(
        !written.contains(&door),
        "the edited door is written on its house"
    );
    let house_a = file["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == a["id"])
        .unwrap();
    assert_eq!(house_a["child_edits"][0]["name"], "Front door");
    let back = World::from_json(&text).unwrap();
    assert_eq!(back, world);
    assert_eq!(back.to_json().unwrap(), text);
    assert_eq!(hash_after_play(back), hash_after_play(world));
}

#[test]
fn undo_takes_back_a_spawn_and_an_edit_of_a_child() {
    let mut s = example();
    run(&mut s, "prefab.add", house());
    let before = s.document().to_json().unwrap();
    let a = run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "house", "at": [5, 5]}),
    );
    let spawned = s.document().to_json().unwrap();
    let door = a["children"][0].as_u64().unwrap();
    run(
        &mut s,
        "entity.set",
        json!({"id": door, "name": "Back door"}),
    );
    assert!(s.document().to_json().unwrap().contains("Back door"));
    s.undo().unwrap();
    assert_eq!(s.document().to_json().unwrap(), spawned);
    assert_eq!(
        s.document()
            .entity(gamecraft_world::EntityId(door))
            .unwrap()
            .name,
        "Door"
    );
    s.undo().unwrap();
    assert_eq!(s.document().to_json().unwrap(), before);
    s.redo().unwrap();
    s.redo().unwrap();
    let back = World::from_json(&s.document().to_json().unwrap()).unwrap();
    assert_eq!(&back, s.document());
}

#[test]
fn a_prefab_inside_a_prefab_by_command() {
    let mut s = example();
    run(&mut s, "prefab.add", house());
    run(
        &mut s,
        "prefab.add",
        json!({"id": "farm", "children": [
            {"prefab": "house", "offset": [4, 0, 0], "components": [{"type": "tags", "tags": ["farmhouse"]}]},
            {"name": "Fence", "offset": [0, 0, -6], "components": [{"type": "shape", "kind": "box", "size": [12, 1, 0.1]}]}
        ]}),
    );
    let farm = run(
        &mut s,
        "prefab.spawn",
        json!({"prefab": "farm", "at": [0, 0]}),
    );
    let home = farm["children"][0].as_u64().unwrap();
    let got = run(&mut s, "entity.get", json!({"id": home}));
    assert_eq!(got["prefab"], "house");
    assert_eq!(got["children"].as_array().unwrap().len(), 2);
    refuse(
        &mut s,
        "prefab.remove",
        json!({"id": "house"}),
        &["farm", "child 0"],
    );
    refuse(
        &mut s,
        "prefab.set",
        json!({"id": "house", "children": [{"prefab": "farm"}]}),
        &["never inside themselves"],
    );
    let world = s.document().clone();
    let back = World::from_json(&world.to_json().unwrap()).unwrap();
    assert_eq!(back, world);
}
