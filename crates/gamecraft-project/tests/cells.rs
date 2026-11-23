//! Layers split into spatial cells: one file per cell, entities placed by their root, the same
//! world back, partial loads and the area query.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{apply, canonical, load_one, pack_of};
use gamecraft_project::{Cell, DEFAULT_CELL_SIZE_M, PackSource, Project, cells_in_area};
use gamecraft_world::{Component, EntityDraft, Prefab, PrefabChild, Shape, Spawn, World};
use std::collections::BTreeSet;

/// Lots in a row along x, each with a wall 30 m to its east, and rocks on their own.
fn town() -> World {
    let mut w = World::new("Town", 4);
    w.put_prefab(Prefab {
        id: "lot".into(),
        name: "Lot".into(),
        components: vec![Component::Shape(Shape::default())],
        children: vec![PrefabChild {
            name: "Far wall".into(),
            offset: [30.0, 0.0, 0.0],
            rotation_y_deg: 0.0,
            prefab: None,
            components: vec![Component::Shape(Shape::default())],
        }],
    })
    .unwrap();
    for i in 0..6 {
        w.spawn_prefab(
            "lot",
            Spawn {
                name: Some(format!("Lot {i}")),
                position: [f64::from(i) * 50.0 - 100.0, 0.0, f64::from(i % 3) * 40.0],
                rotation_y_deg: 90.0,
                layer: Some("town".into()),
                ..Spawn::default()
            },
        )
        .unwrap();
    }
    for i in 0..4 {
        w.add_entity(EntityDraft {
            name: format!("Rock {i}"),
            position: [f64::from(i) * 70.0, 0.0, -90.0],
            layer: Some("rocks".into()),
            components: vec![Component::Shape(Shape::default())],
            ..EntityDraft::default()
        })
        .unwrap();
    }
    w
}

fn celled(size: f64) -> (PackSource, Project, World) {
    let world = town();
    let (mut project, saving) =
        Project::create_cells("town", "Town", "main", &world, Some(size)).unwrap();
    let source = pack_of("town", &saving.plan);
    project.finish_save(saving);
    (source, project, world)
}

fn cell_files(source: &PackSource) -> Vec<&str> {
    source
        .files
        .keys()
        .filter(|p| p.contains("/layers/"))
        .map(String::as_str)
        .collect()
}

#[test]
fn cells_name_themselves_and_cover_areas() {
    let c = Cell { x: 3, z: -2 };
    assert_eq!(c.to_string(), "3_-2");
    assert_eq!(Cell::parse("3_-2"), Some(c));
    for bad in ["+3_-2", "03_1", "3", "3_x", "3_1_2", "_1", ""] {
        assert_eq!(Cell::parse(bad), None, "{bad}");
    }
    assert_eq!(Cell::of(-0.5, 511.9, 512.0), Cell { x: -1, z: 0 });
    assert_eq!(Cell::of(f64::NAN, 1e300, 16.0), Cell { x: 0, z: i32::MAX });
    let around = cells_in_area([-600.0, -10.0], [600.0, 10.0], DEFAULT_CELL_SIZE_M).unwrap();
    assert_eq!(around.len(), 4 * 2);
    assert_eq!(around[0], Cell { x: -2, z: -1 });
    // Corners in any order.
    assert_eq!(
        cells_in_area([600.0, 10.0], [-600.0, -10.0], 512.0).unwrap(),
        around
    );
    assert!(
        cells_in_area([0.0; 2], [1e6, 1e6], 16.0).is_err(),
        "too many"
    );
    assert!(cells_in_area([0.0; 2], [f64::NAN, 0.0], 512.0).is_err());
    assert!(
        cells_in_area([0.0; 2], [1.0, 1.0], 1.0).is_err(),
        "a cell too small"
    );
}

#[test]
fn a_world_in_cells_has_one_file_per_cell_and_children_go_with_their_root() {
    let (source, _, world) = celled(64.0);
    let files = cell_files(&source);
    assert!(files.iter().all(|f| f.split('/').count() == 5), "{files:?}");
    assert!(
        files.contains(&"worlds/main/layers/town/-2_0.json"),
        "{files:?}"
    );
    let world_json = &source.files["worlds/main/world.json"];
    assert!(world_json.contains("\"cell_size_m\": 64.0"), "{world_json}");
    // Each lot's far wall (30 m east, often in the next cell) is in its lot's file.
    for text in source.files.values().filter(|t| t.contains("lot-")) {
        let lots = text.matches("\"prefab\":\"lot\"").count();
        let walls = text.matches("\"parent\":\"lot-").count();
        assert_eq!(lots, walls, "{text}");
    }
    let (project, loaded) = load_one("town", &source);
    assert_eq!(canonical(&loaded), canonical(&world));
    assert_eq!(project.cell_size(), Some(64.0));
    assert!(project.plan_save(&loaded).unwrap().plan.is_empty());
    // Deterministic: the same world gives the same files, byte for byte.
    let (again, _, _) = celled(64.0);
    assert_eq!(again.files, source.files);
    // And a second load is the very same world (ids included).
    let (_, reloaded) = load_one("town", &source);
    assert_eq!(reloaded, loaded);
    assert_eq!(reloaded.to_json().unwrap(), loaded.to_json().unwrap());
}

#[test]
fn moving_an_entity_rewrites_only_the_cells_it_left_and_entered() {
    let (mut source, _, _) = celled(64.0);
    // The project as opened: its ids are the ones a load gives.
    let (project, mut world) = load_one("town", &source);
    let rock = world.entities().find(|e| e.name == "Rock 0").unwrap().id;
    world.place_entity(rock, [300.0, 0.0, 300.0], 0.0).unwrap();
    let saving = project.plan_save(&world).unwrap();
    let written: Vec<&str> = saving.plan.writes.keys().map(String::as_str).collect();
    assert_eq!(
        written,
        vec!["worlds/main/layers/rocks/4_4.json"],
        "{written:?}"
    );
    assert_eq!(
        saving.plan.removes,
        vec!["worlds/main/layers/rocks/0_-2.json"]
    );
    apply(&mut source, &saving.plan);
    let (_, back) = load_one("town", &source);
    assert_eq!(canonical(&back), canonical(&world));
}

#[test]
fn switching_between_cells_and_one_file_per_layer_moves_the_files() {
    let world = town();
    let (mut project, saving) = Project::create("town", "Town", "main", &world).unwrap();
    let mut source = pack_of("town", &saving.plan);
    project.finish_save(saving);
    let (mut project, loaded) = load_one("town", &source);
    project.set_cell_size(Some(128.0)).unwrap();
    let saving = project.plan_save(&loaded).unwrap();
    assert!(
        saving
            .plan
            .removes
            .contains(&"worlds/main/layers/town.json".to_string())
    );
    assert!(
        saving
            .plan
            .writes
            .keys()
            .any(|p| p.starts_with("worlds/main/layers/town/"))
    );
    assert!(saving.plan.writes.contains_key("worlds/main/world.json"));
    apply(&mut source, &saving.plan);
    let (mut project, back) = load_one("town", &source);
    assert_eq!(project.cell_size(), Some(128.0));
    assert_eq!(back, loaded, "the same world, the same ids");
    project.set_cell_size(None).unwrap();
    let saving = project.plan_save(&back).unwrap();
    assert!(
        saving
            .plan
            .writes
            .contains_key("worlds/main/layers/town.json")
    );
    assert!(
        saving
            .plan
            .removes
            .iter()
            .all(|p| p.contains("/layers/town/") || p.contains("/layers/rocks/"))
    );
    apply(&mut source, &saving.plan);
    assert!(!source.files.keys().any(|p| p.matches('/').count() == 4));
    let (mut project, again) = load_one("town", &source);
    assert_eq!(project.cell_size(), None);
    assert_eq!(again, loaded);
    assert!(project.set_cell_size(Some(1.0)).is_err());
}

#[test]
fn a_world_loads_in_part_by_cells_and_then_refuses_to_save() {
    let (source, _, world) = celled(64.0);
    let only: BTreeSet<Cell> = [Cell { x: -2, z: 0 }].into();
    let all = gamecraft_project::Project::load_cells(
        "town",
        &common::packs(vec![("town", source.clone())]),
        None,
        Some(&only),
    );
    let (project, part) = all.unwrap();
    let names: Vec<&str> = part.entities().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Lot 0", "Far wall"], "{names:?}");
    assert!(part.entity_count() < world.entity_count());
    let err = project.plan_save(&part).unwrap_err().to_string();
    assert!(err.contains("open it whole"), "{err}");
    let info = project.info(&part).unwrap();
    assert_eq!(info["cells"], serde_json::json!([[-2, 0]]));
    assert_eq!(info["changes"]["files"], serde_json::json!([]));
}

#[test]
fn bad_cell_files_are_refused_with_a_sentence() {
    let (source, _, _) = celled(64.0);
    let with = |path: &str, text: &str| {
        let mut s = source.clone();
        s.files.insert(path.into(), text.into());
        Project::load("town", &common::packs(vec![("town", s)]), None)
            .unwrap_err()
            .to_string()
    };
    let layer = "{\"gamecraft_layer\": 1, \"entities\": []}";
    let err = with("worlds/main/layers/town/1_01.json", layer);
    assert!(err.contains("<cx>_<cz>.json"), "{err}");
    let err = with("worlds/main/layers/town.json", layer);
    assert!(err.contains("both one file"), "{err}");
    let err = with(
        "worlds/main/layers/town/9_9.json",
        "{\"gamecraft_layer\": 1, \"entities\": [",
    );
    assert!(err.contains("9_9.json"), "{err}");
    let mut s = source.clone();
    let wj =
        s.files["worlds/main/world.json"].replace("\"cell_size_m\": 64.0", "\"cell_size_m\": 0.5");
    s.files.insert("worlds/main/world.json".into(), wj);
    let err = Project::load("town", &common::packs(vec![("town", s)]), None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("cell_size_m"), "{err}");
}

/// Regression: naming a new entity tried every number from 2 up for each one, so a project of
/// tens of thousands of entities with one name (the walls of a city) took hours to make.
#[test]
fn many_entities_of_one_name_get_their_keys_in_one_try_each() {
    let mut w = World::new("Walls", 1);
    for i in 0..20_000 {
        w.add_entity(EntityDraft {
            name: "wall-gate".into(),
            position: [f64::from(i % 200) * 5.0, 0.0, f64::from(i / 200) * 5.0],
            ..EntityDraft::default()
        })
        .unwrap();
    }
    let (_, saving) = Project::create_cells("walls", "Walls", "main", &w, Some(512.0)).unwrap();
    let text: String = saving
        .plan
        .writes
        .iter()
        .filter(|(p, _)| p.contains("/layers/"))
        .map(|(_, t)| t.as_str())
        .collect();
    assert!(text.contains("\"id\":\"wall-gate\""));
    assert!(text.contains("\"id\":\"wall-gate-20000\""));
    assert!(!text.contains("\"id\":\"wall-gate-20001\""));
}
