//! Projects and packs: round trips, layering by id across packs, saving changes to the files
//! they belong to, dependency order, and hostile files.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{apply, canonical, load_one, pack, pack_of, packs, village};
use gamecraft_project::{Manifest, PackSource, Project, dependency_order};
use gamecraft_world::{Component, Health, Light, World};
use serde_json::json;
use std::collections::BTreeMap;

/// The village saved as a new project `game`, its files and the project.
fn game() -> (PackSource, Project, World) {
    let world = village();
    let (mut project, saving) = Project::create("game", "Game", "village", &world).unwrap();
    let source = pack_of("game", &saving.plan);
    project.finish_save(saving);
    (source, project, world)
}

/// The lines that differ between two texts, as (before, after).
fn changed_lines(a: &str, b: &str) -> Vec<(String, String)> {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    assert_eq!(la.len(), lb.len(), "the line count changed:\n{a}\n---\n{b}");
    la.iter()
        .zip(&lb)
        .filter(|(x, y)| x != y)
        .map(|(x, y)| (x.to_string(), y.to_string()))
        .collect()
}

#[test]
fn a_new_project_has_the_files_of_the_design() {
    let (source, _, _) = game();
    let paths: Vec<&str> = source.files.keys().map(String::as_str).collect();
    assert_eq!(
        paths,
        vec![
            "pack.json",
            "prefabs/house.json",
            "prefabs/soldier.json",
            "worlds/village/layers/enemies.json",
            "worlds/village/layers/main.json",
            "worlds/village/layers/terrain.json",
            "worlds/village/layers/village.json",
            "worlds/village/world.json",
        ]
    );
    let enemies = &source.files["worlds/village/layers/enemies.json"];
    assert!(enemies.starts_with("{\n  \"gamecraft_layer\": 1,\n  \"entities\": [\n"));
    // One entity per line: the guard and its hat.
    let lines: Vec<&str> = enemies.lines().filter(|l| l.contains("\"id\"")).collect();
    assert_eq!(lines.len(), 2, "{enemies}");
    assert!(lines[0].contains("\"id\":\"guard\"") && lines[0].contains("\"overrides\""));
    assert!(lines[1].contains("\"id\":\"hat\"") && lines[1].contains("\"parent\":\"guard\""));
    let world_json: serde_json::Value =
        serde_json::from_str(&source.files["worlds/village/world.json"]).unwrap();
    assert_eq!(world_json["gamecraft_world"], json!(1));
    assert_eq!(world_json["default_layer"], json!("main"));
    assert_eq!(world_json["name"], json!("Village"));
    let manifest = Manifest::parse(&source.files["pack.json"], "pack.json").unwrap();
    assert_eq!(manifest.worlds, vec!["village".to_string()]);
    assert!(source.files.values().all(|t| t.ends_with('\n')));
}

#[test]
fn load_gives_the_same_world_and_saving_it_unchanged_writes_nothing() {
    let (source, _, original) = game();
    let (project, loaded) = load_one("game", &source);
    assert_eq!(canonical(&loaded), canonical(&original));
    let sign = loaded.entities().find(|e| e.name == "Sign").unwrap();
    assert_eq!(sign.layer.as_deref(), Some("main"));
    let saving = project.plan_save(&loaded).unwrap();
    assert!(saving.plan.is_empty(), "{:?}", saving.plan);
    // And through a second load: still nothing.
    let (again, reloaded) = load_one("game", &source);
    assert_eq!(reloaded, loaded);
    assert!(again.plan_save(&reloaded).unwrap().plan.is_empty());
}

#[test]
fn a_hand_written_file_in_another_layout_is_not_rewritten_by_an_unchanged_save() {
    let files = [
        (
            "pack.json",
            r#"{"gamecraft_pack": 1, "id": "tiny", "worlds": ["w"]}"#,
        ),
        (
            "worlds/w/world.json",
            r#"{"gamecraft_world": 1, "seed": 4}"#,
        ),
        (
            "worlds/w/layers/main.json",
            r#"{"gamecraft_layer": 1, "entities": [ {"id": "a", "name": "A",
                "position": [1, 2, 3]} ]}"#,
        ),
    ];
    let source = pack("tiny", &files);
    let (project, world) = load_one("tiny", &source);
    assert_eq!(world.name, "w", "the world's name defaults to its folder");
    assert_eq!(world.seed, 4);
    assert!(project.plan_save(&world).unwrap().plan.is_empty());
}

#[test]
fn editing_one_entity_changes_one_line_of_one_file() {
    let (mut source, _, _) = game();
    let (mut project, mut world) = load_one("game", &source);
    let rock = world.find("Rock 1").unwrap();
    world.entity_mut(rock).unwrap().position[0] = 42.5;
    let saving = project.plan_save(&world).unwrap();
    let paths: Vec<&String> = saving.plan.writes.keys().collect();
    assert_eq!(paths, vec!["worlds/village/layers/terrain.json"]);
    let before = &source.files["worlds/village/layers/terrain.json"];
    let after = &saving.plan.writes["worlds/village/layers/terrain.json"];
    let diff = changed_lines(before, after);
    assert_eq!(diff.len(), 1, "{diff:?}");
    assert!(diff[0].0.contains("rock-1") && diff[0].1.contains("42.5"));
    apply(&mut source, &saving.plan);
    project.finish_save(saving);
    assert!(project.plan_save(&world).unwrap().plan.is_empty());
    let (_, reloaded) = load_one("game", &source);
    assert_eq!(reloaded.entity(rock).unwrap().position[0], 42.5);
}

#[test]
fn adding_moving_and_removing_entities_touch_only_their_layers() {
    let (mut source, _, _) = game();
    let (mut project, mut world) = load_one("game", &source);
    // Move the sign to the village, remove a rock, add a tree in a new layer.
    let sign = world.find("Sign").unwrap();
    world.entity_mut(sign).unwrap().layer = Some("village".into());
    let rock = world.find("Rock 0").unwrap();
    world.remove_entity(rock);
    let tree = world
        .add_entity(gamecraft_world::EntityDraft {
            name: "Old Oak".into(),
            layer: Some("forest".into()),
            ..Default::default()
        })
        .unwrap();
    let saving = project.plan_save(&world).unwrap();
    let mut paths: Vec<&str> = saving.plan.writes.keys().map(String::as_str).collect();
    paths.sort_unstable();
    assert_eq!(
        paths,
        vec![
            "worlds/village/layers/forest.json",
            "worlds/village/layers/main.json",
            "worlds/village/layers/terrain.json",
            "worlds/village/layers/village.json",
        ]
    );
    assert!(saving.plan.writes["worlds/village/layers/forest.json"].contains("\"id\":\"old-oak\""));
    assert!(saving.plan.writes["worlds/village/layers/main.json"].contains("\"entities\": []"));
    apply(&mut source, &saving.plan);
    project.finish_save(saving);
    assert_eq!(
        project.provenance().entities[&tree].key,
        "old-oak",
        "a saved entity keeps its new string id"
    );
    let (_, reloaded) = load_one("game", &source);
    assert_eq!(reloaded.entity_count(), world.entity_count());
    assert!(reloaded.find("Rock 0").is_err());
    let sign = reloaded.find("Sign").unwrap();
    assert_eq!(
        reloaded.entity(sign).unwrap().layer.as_deref(),
        Some("village")
    );
}

#[test]
fn prefab_and_world_field_changes_go_to_their_own_files() {
    let (mut source, _, _) = game();
    let (mut project, mut world) = load_one("game", &source);
    let mut soldier = world.prefab("soldier").unwrap().clone();
    soldier.name = "Veteran".into();
    world.put_prefab(soldier).unwrap();
    world.environment.mist = 0.5;
    let saving = project.plan_save(&world).unwrap();
    let paths: Vec<&str> = saving.plan.writes.keys().map(String::as_str).collect();
    assert_eq!(
        paths,
        vec!["prefabs/soldier.json", "worlds/village/world.json"]
    );
    apply(&mut source, &saving.plan);
    project.finish_save(saving);
    // Removing the unused prefab deletes its file.
    for id in world.prefab_instances("house") {
        world.remove_entity(id);
    }
    world.remove_prefab("house").unwrap();
    let saving = project.plan_save(&world).unwrap();
    assert_eq!(saving.plan.removes, vec!["prefabs/house.json".to_string()]);
    apply(&mut source, &saving.plan);
    let (_, reloaded) = load_one("game", &source);
    assert!(reloaded.prefab("house").is_none());
    assert_eq!(reloaded.prefab("soldier").unwrap().name, "Veteran");
    assert_eq!(reloaded.environment.mist, 0.5);
}

/// The game, a mod over it and a second mod over the first: replace, remove and add by id.
fn three_packs() -> BTreeMap<String, PackSource> {
    let (game, _, _) = game();
    let balance = pack(
        "balance",
        &[
            (
                "pack.json",
                r#"{"gamecraft_pack": 1, "id": "balance", "depends": [{"id": "game"}]}"#,
            ),
            (
                "prefabs/soldier.json",
                r#"{"id": "soldier", "name": "Soldier", "components": [
                    {"type": "health", "max": 200, "current": 200},
                    {"type": "tags", "tags": ["enemy"]},
                    {"type": "light", "intensity_cd": 50, "range_m": 15}],
                  "children": [{"name": "Hat", "offset": [0, 1.8, 0]}]}"#,
            ),
            (
                "worlds/village/world.json",
                r#"{"gamecraft_world": 1, "environment": {"time_of_day_h": 23.0, "mist": 0.6}}"#,
            ),
            (
                "worlds/village/layers/terrain.json",
                r#"{"gamecraft_layer": 1, "entities": [
                    {"id": "rock-1", "name": "Big rock", "position": [3, 0, -20], "components": []},
                    {"id": "rock-2", "removed": true}
                ]}"#,
            ),
        ],
    );
    let night = pack(
        "night",
        &[
            (
                "pack.json",
                r#"{"gamecraft_pack": 1, "id": "night", "depends": [{"id": "balance"}]}"#,
            ),
            (
                "worlds/village/world.json",
                r#"{"gamecraft_world": 1, "environment": {"rain": 0.3}}"#,
            ),
            (
                "worlds/village/layers/more.json",
                r#"{"gamecraft_layer": 1, "entities": [
                    {"id": "scout", "name": "Scout", "prefab": "soldier", "position": [9, 0, 9]}
                ]}"#,
            ),
        ],
    );
    packs(vec![("game", game), ("balance", balance), ("night", night)])
}

#[test]
fn three_packs_layer_by_id_replace_remove_and_add() {
    let all = three_packs();
    let (project, world) = Project::load("night", &all, None).unwrap();
    let ids: Vec<&str> = project
        .packs()
        .iter()
        .map(|p| p.manifest.id.as_str())
        .collect();
    assert_eq!(ids, vec!["game", "balance", "night"]);
    assert_eq!(project.world(), "village");
    // world.json merged field by field.
    assert_eq!(world.environment.time_of_day_h, 23.0);
    assert_eq!(world.environment.mist, 0.6);
    assert_eq!(world.environment.rain, 0.3);
    assert_eq!(
        world.name, "Village",
        "fields no mod touches stay the game's"
    );
    // The prefab replaced: every soldier carries a light, the guard keeps its override.
    let guard = world.entity(world.find("Guard").unwrap()).unwrap();
    assert!(guard.light().is_some());
    assert_eq!(guard.health().unwrap().current, 50.0);
    assert_eq!(
        guard.health().unwrap().max,
        100.0,
        "the override is the whole component"
    );
    // Entities replaced, removed and added.
    assert!(world.find("Rock 1").is_err());
    let big = world.entity(world.find("Big rock").unwrap()).unwrap();
    assert!(big.components.is_empty());
    assert!(world.find("Rock 2").is_err());
    let scout = world.entity(world.find("Scout").unwrap()).unwrap();
    assert_eq!(scout.layer.as_deref(), Some("more"));
    assert_eq!(scout.health().unwrap().max, 200.0);
    // Provenance.
    let p = project.provenance();
    let src = |name: &str| &p.entities[&world.find(name).unwrap()];
    assert_eq!(src("Big rock").pack, "balance");
    assert_eq!(src("Big rock").key, "rock-1");
    assert_eq!(src("Scout").pack, "night");
    assert_eq!(src("Rock 0").pack, "game");
    assert_eq!(p.prefabs["soldier"], "balance");
    assert_eq!(p.prefabs["house"], "game");
    assert_eq!(p.world_fields["environment"], "night");
    assert_eq!(p.world_fields["name"], "game");
    assert!(project.plan_save(&world).unwrap().plan.is_empty());
}

#[test]
fn a_replacing_layer_drops_the_earlier_layer_of_its_name() {
    let mut all = three_packs();
    all.get_mut("night").unwrap().files.insert(
        "worlds/village/layers/village.json".into(),
        r#"{"gamecraft_layer": 1, "replace": true, "entities": [
            {"id": "tent", "name": "Tent", "components": []}]}"#
            .into(),
    );
    let (project, world) = Project::load("night", &all, None).unwrap();
    assert!(world.find("House 1").is_err() && world.find("House 2").is_err());
    assert!(world.find("Tent").is_ok());
    assert!(project.plan_save(&world).unwrap().plan.is_empty());
}

#[test]
fn editing_a_dependency_writes_an_override_into_the_root_pack_never_the_dependency() {
    let all = three_packs();
    let (mut project, mut world) = Project::load("night", &all, None).unwrap();
    // A game entity moved, another removed, a balance prefab changed.
    let house = world.find("House 2").unwrap();
    world.entity_mut(house).unwrap().position = [11.0, 0.0, 1.0];
    let sign = world.find("Sign").unwrap();
    world.remove_entity(sign);
    let mut soldier = world.prefab("soldier").unwrap().clone();
    soldier.components.push(Component::Light(Light::default()));
    soldier.components.retain(|c| c.kind() != "light");
    soldier.components.push(Component::Light(Light {
        range_m: 30.0,
        ..Light::default()
    }));
    world.put_prefab(soldier).unwrap();
    let saving = project.plan_save(&world).unwrap();
    let paths: Vec<&str> = saving.plan.writes.keys().map(String::as_str).collect();
    assert_eq!(
        paths,
        vec![
            "prefabs/soldier.json",
            "worlds/village/layers/main.json",
            "worlds/village/layers/village.json",
        ],
        "every write is a path in the root pack `night`"
    );
    let village = &saving.plan.writes["worlds/village/layers/village.json"];
    let lines: Vec<&str> = village.lines().filter(|l| l.contains("\"id\"")).collect();
    assert_eq!(lines.len(), 1, "only the moved house: {village}");
    assert!(lines[0].contains("\"id\":\"house-2\"") && lines[0].contains("11.0"));
    assert!(
        saving.plan.writes["worlds/village/layers/main.json"]
            .contains(r#"{"id":"sign","removed":true}"#)
    );
    // Saved into `night`, the project resolves to the edited world.
    let mut all = all;
    apply(all.get_mut("night").unwrap(), &saving.plan);
    project.finish_save(saving);
    assert!(project.plan_save(&world).unwrap().plan.is_empty());
    let (again, reloaded) = Project::load("night", &all, None).unwrap();
    assert!(reloaded.find("Sign").is_err());
    let house = reloaded.entity(reloaded.find("House 2").unwrap()).unwrap();
    assert_eq!(house.position, [11.0, 0.0, 1.0]);
    assert_eq!(house.layer.as_deref(), Some("village"));
    assert_eq!(again.provenance().prefabs["soldier"], "night");
    assert!(again.plan_save(&reloaded).unwrap().plan.is_empty());
    // The game and the first mod are as they were.
    assert_eq!(all["game"], three_packs()["game"]);
    assert_eq!(all["balance"], three_packs()["balance"]);
}

#[test]
fn removing_a_dependency_prefab_writes_a_removal_marker() {
    let all = three_packs();
    let (project, mut world) = Project::load("night", &all, None).unwrap();
    for id in world.prefab_instances("house") {
        world.remove_entity(id);
    }
    world.remove_prefab("house").unwrap();
    let saving = project.plan_save(&world).unwrap();
    let marker: serde_json::Value =
        serde_json::from_str(&saving.plan.writes["prefabs/house.json"]).unwrap();
    assert_eq!(marker, json!({"id": "house", "removed": true}));
    let mut all = all;
    apply(all.get_mut("night").unwrap(), &saving.plan);
    let (_, reloaded) = Project::load("night", &all, None).unwrap();
    assert!(reloaded.prefab("house").is_none());
}

#[test]
fn a_mod_world_change_is_a_minimal_patch() {
    let all = three_packs();
    let (project, mut world) = Project::load("night", &all, None).unwrap();
    world.environment.wind_speed_mps = 12.0;
    let saving = project.plan_save(&world).unwrap();
    let text = &saving.plan.writes["worlds/village/world.json"];
    let patch: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        patch,
        json!({"gamecraft_world": 1, "environment": {"rain": 0.3, "wind_speed_mps": 12.0}}),
        "{text}"
    );
}

#[test]
fn dependency_order_names_cycles_and_missing_packs_with_their_chain() {
    let m = |id: &str, deps: &[&str]| {
        let mut manifest = Manifest::new(id, id, vec![]);
        for d in deps {
            manifest.add_dependency(d, &format!("../{d}")).unwrap();
        }
        (id.to_string(), manifest)
    };
    let ok: BTreeMap<_, _> = [
        m("mod", &["game", "nature"]),
        m("game", &["nature"]),
        m("nature", &[]),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        dependency_order("mod", &ok).unwrap(),
        vec!["nature", "game", "mod"]
    );
    let cycle: BTreeMap<_, _> = [m("a", &["b"]), m("b", &["c"]), m("c", &["a"])]
        .into_iter()
        .collect();
    let e = dependency_order("a", &cycle).unwrap_err().to_string();
    assert!(e.contains("cycle: a -> b -> c -> a"), "{e}");
    let missing: BTreeMap<_, _> = [m("mod", &["game"]), m("game", &["nature"])]
        .into_iter()
        .collect();
    let e = dependency_order("mod", &missing).unwrap_err().to_string();
    assert!(
        e.contains("`nature` is missing (needed by mod -> game -> nature)"),
        "{e}"
    );
    // Through load, too.
    let mut all = three_packs();
    all.remove("balance");
    let e = Project::load("night", &all, None).unwrap_err().to_string();
    assert!(e.contains("needed by night -> balance"), "{e}");
}

/// Loads `files` as the single pack `p` and returns the error.
fn refused(files: &[(&str, &str)]) -> String {
    let all = packs(vec![("p", pack("packs/p", files))]);
    Project::load("p", &all, None)
        .map(|_| ())
        .expect_err(&format!("should be refused: {files:?}"))
        .to_string()
}

const MANIFEST: (&str, &str) = (
    "pack.json",
    r#"{"gamecraft_pack": 1, "id": "p", "worlds": ["w"]}"#,
);

#[test]
fn hostile_files_are_refused_with_a_sentence_naming_the_file() {
    let e = refused(&[
        MANIFEST,
        (
            "worlds/w/layers/a.json",
            r#"{"gamecraft_layer": 1, "entities": [{"id": "x", "na"#,
        ),
    ]);
    assert!(
        e.contains("`packs/p/worlds/w/layers/a.json`") && e.contains("EOF"),
        "{e}"
    );
    let e = refused(&[
        MANIFEST,
        (
            "worlds/w/layers/a.json",
            r#"{"gamecraft_layer": 1, "entities": [{"id": "x"}]}"#,
        ),
        (
            "worlds/w/layers/b.json",
            r#"{"gamecraft_layer": 1, "entities": [{"id": "x"}]}"#,
        ),
    ]);
    assert!(
        e.contains("`x` is used twice in the pack `p`")
            && e.contains("a.json")
            && e.contains("b.json"),
        "{e}"
    );
    let e = refused(&[
        MANIFEST,
        (
            "worlds/w/layers/../../../escape.json",
            r#"{"gamecraft_layer": 1}"#,
        ),
    ]);
    assert!(e.contains("leads out of its pack"), "{e}");
    let e = refused(&[
        MANIFEST,
        (
            "worlds/w/world.json",
            r#"{"gamecraft_world": 1, "default_layer": "../up"}"#,
        ),
    ]);
    assert!(e.contains("default_layer"), "{e}");
    let e = refused(&[
        MANIFEST,
        (
            "worlds/w/layers/a.json",
            r#"{"gamecraft_layer": 1, "entities": [{"id": "x", "parent": "nobody"}]}"#,
        ),
    ]);
    assert!(e.contains("parent `nobody`"), "{e}");
    let e = refused(&[
        MANIFEST,
        (
            "worlds/w/layers/a.json",
            r#"{"gamecraft_layer": 1, "entities": [{"id": "x", "scale": -1}]}"#,
        ),
    ]);
    assert!(
        e.contains("`packs/p/worlds/w/layers/a.json`: the entity `x`") && e.contains("scale"),
        "{e}"
    );
    let e = refused(&[
        MANIFEST,
        (
            "worlds/w/layers/a.json",
            r#"{"gamecraft_layer": 1, "entities": [{"id": "x", "prefab": "ghost"}]}"#,
        ),
    ]);
    assert!(e.contains("ghost"), "{e}");
    let e = refused(&[MANIFEST, ("prefabs/a.json", r#"{"id": "b"}"#)]);
    assert!(e.contains("named after its id"), "{e}");
    let e = refused(&[(
        "pack.json",
        r#"{"gamecraft_pack": 1, "id": "p", "worlds": ["w"], "depends": [{"id": "p"}]}"#,
    )]);
    assert!(e.contains("depends on itself"), "{e}");
    let e = refused(&[("pack.json", r#"{"gamecraft_pack": 2, "id": "p"}"#)]);
    assert!(e.contains("gamecraft_pack"), "{e}");
    let e = refused(&[]);
    assert!(e.contains("pack.json") && e.contains("missing"), "{e}");
    let e = refused(&[
        MANIFEST,
        ("worlds/other/world.json", r#"{"gamecraft_world": 1}"#),
    ]);
    assert!(e.contains("not listed in the `worlds`"), "{e}");
    let e = refused(&[
        MANIFEST,
        (
            "worlds/w/layers/a.json",
            r#"{"gamecraft_layer": 1, "entities": [{"id": "../x"}]}"#,
        ),
    ]);
    assert!(e.contains("is not allowed"), "{e}");
    let e = refused(&[
        MANIFEST,
        (
            "worlds/w/layers/a.json",
            r#"{"gamecraft_layer": 1, "entities": [{"id": "x", "removed": false}]}"#,
        ),
    ]);
    assert!(e.contains("a removal is exactly"), "{e}");
}

#[test]
fn every_truncation_of_every_file_is_an_error_not_a_panic() {
    let (source, _, _) = game();
    for (path, text) in &source.files {
        let mut cut = text.len();
        while cut > 0 {
            cut = cut.saturating_sub(7).min(cut - 1);
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            let mut broken = source.clone();
            broken.files.insert(path.clone(), text[..cut].to_string());
            let all = packs(vec![("game", broken)]);
            let outcome =
                std::panic::catch_unwind(|| Project::load("game", &all, None).map(|_| ()));
            assert!(outcome.is_ok(), "{path} cut at {cut} panicked");
        }
    }
}

#[test]
fn info_lists_packs_layers_prefabs_and_what_changed() {
    let all = three_packs();
    let (project, mut world) = Project::load("night", &all, None).unwrap();
    let info = project.info(&world).unwrap();
    assert_eq!(info["root"], json!("night"));
    assert_eq!(info["packs"].as_array().unwrap().len(), 3);
    let layers = info["layers"].as_array().unwrap();
    let count = |name: &str| {
        layers
            .iter()
            .find(|l| l["name"] == json!(name))
            .map(|l| l["entities"].as_u64().unwrap())
    };
    assert_eq!(count("more"), Some(1), "the scout");
    assert_eq!(count("terrain"), Some(2), "rock 0 and the big rock");
    assert_eq!(info["changes"]["files"], json!([]));
    let id = world.find("Guard").unwrap();
    world.entity_mut(id).unwrap().name = "Captain".into();
    let info = project.info(&world).unwrap();
    assert_eq!(info["changes"]["entities_changed"], json!(1));
    assert_eq!(
        info["changes"]["files"],
        json!(["worlds/village/layers/enemies.json"])
    );
}

#[test]
fn health_override_round_trip_keeps_bits() {
    let (source, _, _) = game();
    let (_, w) = load_one("game", &source);
    let guard = w.entity(w.find("Guard").unwrap()).unwrap();
    assert_eq!(
        guard.health(),
        Some(&Health {
            current: 50.0,
            ..Health::default()
        })
    );
}

#[test]
fn the_id_of_a_removed_entity_of_the_root_pack_is_free_again() {
    // A bake replaces a layer's entities: removed and added again, they must get the ids they
    // had, so that the same script writes the same files.
    let (source, _, _) = game();
    let (project, mut world) = load_one("game", &source);
    let sign = world.find("Sign").unwrap();
    let removed = world.remove_entity(sign).unwrap();
    let again = gamecraft_world::EntityDraft {
        name: removed.name.clone(),
        position: removed.position,
        rotation_y_deg: removed.rotation_y_deg,
        scale: removed.scale,
        layer: removed.layer.clone(),
        components: removed.components.clone(),
    };
    world.add_entity(again).unwrap();
    let saving = project.plan_save(&world).unwrap();
    assert!(
        saving.plan.is_empty(),
        "the new Sign took the old one's id and place: {:?}",
        saving.plan
    );
    // A second entity of that name is numbered, as always.
    world
        .add_entity(gamecraft_world::EntityDraft {
            name: "Sign".into(),
            ..gamecraft_world::EntityDraft::default()
        })
        .unwrap();
    let saving = project.plan_save(&world).unwrap();
    let main = &saving.plan.writes["worlds/village/layers/main.json"];
    assert!(
        main.contains("\"sign-2\"") && main.contains("\"sign\""),
        "{main}"
    );
}
