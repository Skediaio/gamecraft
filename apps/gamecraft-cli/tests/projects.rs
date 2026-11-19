//! The Skirmish as a project and its night mod, through the built binary: the project is the
//! committed world, the mod changes exactly what it says, an edit of a game entity is saved
//! into the mod, the playthrough still wins, and the index finds the enemies near the village.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
use common::*;
use gamecraft_world::{Prefab, Spawn, World};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn examples() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .join("examples")
}

fn text(path: &Path) -> String {
    path.display().to_string()
}

/// Runs the binary; it must exit with code 0 and print one JSON answer.
fn json_of(args: &[&str]) -> Value {
    let ran = run(args);
    assert_eq!(ran.code, 0, "{args:?}: {}{}", ran.out, ran.err);
    serde_json::from_str(ran.out.trim()).unwrap_or_else(|e| panic!("{args:?}: not JSON: {e}"))
}

/// The world a project resolves to, through `project export`.
fn exported(project: &Path, temp: &Temp) -> World {
    let out = temp.path("export.gamecraft");
    json_of(&["project", "export", &text(project), "--out", &text(&out)]);
    World::from_json(&std::fs::read_to_string(&out).unwrap()).unwrap()
}

/// `w` without layers and with its prefabs sorted by id.
fn without_layers(mut w: World) -> World {
    let ids: Vec<_> = w.entities().map(|e| e.id).collect();
    for id in ids {
        w.entity_mut(id).unwrap().layer = None;
    }
    w.prefabs.sort_by(|a, b| a.id.cmp(&b.id));
    w
}

/// Copies the folder `from` to `to`, recursively (without an index).
fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == ".gamecraft-index" {
            continue;
        }
        let target = to.join(&name);
        if entry.path().is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Every file under `dir` with its text.
fn files(dir: &Path) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            // The index and the terrain's tile cache are derived, ignored by git.
            if path.ends_with(".gamecraft-index") || path.ends_with("terrain/cache") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(path.clone(), std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    out
}

/// The Skirmish and its mod copied into a temporary folder, side by side as in `examples/`.
fn copied(tag: &str) -> Temp {
    let temp = Temp::new(tag);
    copy(
        &examples().join("skirmish/project"),
        &temp.path("skirmish/project"),
    );
    copy(
        &examples().join("skirmish-night-mod"),
        &temp.path("skirmish-night-mod"),
    );
    temp
}

#[test]
fn the_skirmish_project_resolves_to_the_committed_world() {
    let temp = Temp::new("skirmish-project");
    let project = exported(&examples().join("skirmish/project"), &temp);
    let committed = World::from_json(
        &std::fs::read_to_string(examples().join("skirmish/world.gamecraft")).unwrap(),
    )
    .unwrap();
    // The layers keep the committed order, so even the runtime ids are the same.
    assert_eq!(without_layers(project.clone()), without_layers(committed));
    let layer_of = |name: &str| {
        project
            .entity(project.find(name).unwrap())
            .unwrap()
            .layer
            .clone()
            .unwrap()
    };
    assert_eq!(layer_of("Oak 1"), "terrain-features");
    assert_eq!(layer_of("Wall 3"), "terrain-features");
    assert_eq!(layer_of("House 2"), "village");
    assert_eq!(layer_of("Patrol Alpha"), "enemies");
}

#[test]
fn the_night_mod_changes_exactly_what_it_says() {
    let temp = Temp::new("night-mod");
    let base = exported(&examples().join("skirmish/project"), &temp);
    let night = exported(&examples().join("skirmish-night-mod"), &temp);
    let mut expected = base.clone();
    expected.environment.time_of_day_h = 23.0;
    expected.environment.mist = 0.5;
    expected.environment.cloud_cover = 0.6;
    let lit: Prefab = serde_json::from_str(
        &std::fs::read_to_string(examples().join("skirmish-night-mod/prefabs/enemy-soldier.json"))
            .unwrap(),
    )
    .unwrap();
    assert!(lit.component("light").is_some());
    expected.put_prefab(lit).unwrap();
    expected
        .spawn_prefab(
            "enemy-soldier",
            Spawn {
                name: Some("Night Watch".into()),
                position: [4.0, 5.0, -62.0],
                rotation_y_deg: 180.0,
                layer: Some("night-watch".into()),
                ..Spawn::default()
            },
        )
        .unwrap();
    assert_eq!(night, expected);
    let lanterns = night
        .entities()
        .filter(|e| e.has_tag("enemy") && e.light().is_some())
        .count();
    assert_eq!(
        lanterns, 5,
        "four enemies and the night watch carry lanterns"
    );
    let sun = night.environment.sun();
    assert!(sun.elevation_deg < 0.0, "night: the sun is down ({sun:?})");
}

#[test]
fn an_edit_of_a_game_entity_is_saved_into_the_mod_never_the_game() {
    let temp = copied("night-save");
    let game_before = files(&temp.path("skirmish/project"));
    let mod_before = files(&temp.path("skirmish-night-mod"));
    let root = text(&temp.0);
    let mut server = Server::start(
        "serve",
        &[
            "--automation-read-root",
            &root,
            "--automation-write-root",
            &root,
        ],
        &temp.0,
    );
    // Opened and saved unchanged, neither project writes a file.
    let opened = server.ok("project.open", json!({"path": "skirmish/project"}));
    assert_eq!(opened["entities"], json!(55));
    assert_eq!(
        server.ok("project.save", json!({}))["unchanged"],
        json!(true)
    );
    let opened = server.ok("project.open", json!({"path": "skirmish-night-mod"}));
    assert_eq!(opened["packs"], json!(["skirmish", "skirmish-night"]));
    assert_eq!(opened["entities"], json!(56));
    assert_eq!(
        server.ok("project.save", json!({}))["unchanged"],
        json!(true)
    );
    assert_eq!(files(&temp.path("skirmish/project")), game_before);
    assert_eq!(files(&temp.path("skirmish-night-mod")), mod_before);
    // The guard moves: the override goes into the mod.
    server.ok(
        "entity.set",
        json!({"id": "Village Guard", "position": [-6.0, 3.6, -57.0]}),
    );
    let saved = server.ok("project.save", json!({}));
    assert_eq!(
        saved["written"],
        json!(["worlds/village/layers/enemies.json"])
    );
    assert_eq!(
        files(&temp.path("skirmish/project")),
        game_before,
        "the game is untouched"
    );
    let layer =
        std::fs::read_to_string(temp.path("skirmish-night-mod/worlds/village/layers/enemies.json"))
            .unwrap();
    let lines: Vec<&str> = layer.lines().filter(|l| l.contains("\"id\"")).collect();
    assert_eq!(lines.len(), 1, "{layer}");
    assert!(lines[0].contains("\"id\":\"village-guard\"") && lines[0].contains("-57.0"));
    // Opened again, the mod has the moved guard and the game has it where it was.
    let reopened = server.ok("project.open", json!({"path": "skirmish-night-mod"}));
    assert_eq!(reopened["entities"], json!(56));
    let guard = server.ok("entity.get", json!({"id": "Village Guard"}));
    assert_eq!(guard["position"], json!([-6.0, 3.6, -57.0]));
    server.ok("project.open", json!({"path": "skirmish/project"}));
    let guard = server.ok("entity.get", json!({"id": "Village Guard"}));
    assert_eq!(guard["position"][0], json!(-8.0));
    assert!(server.finish());
}

#[test]
fn the_playthrough_still_wins_on_the_base_game_project() {
    let project = examples().join("skirmish/project");
    let script = text(&examples().join("skirmish/playthrough.json"));
    let state = json_of(&[
        "play",
        &text(&project),
        "--script",
        &script,
        "--seconds",
        "60",
    ]);
    for id in ["clear", "alive"] {
        let status = state["objectives"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == id)
            .unwrap()["status"]
            .clone();
        assert_eq!(status, json!("done"), "{id}: {state}");
    }
    let player = state["characters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Player")
        .unwrap()
        .clone();
    assert_eq!(player["health"]["down"], json!(false));
}

#[test]
fn a_query_finds_the_four_enemies_near_the_village() {
    let temp = copied("query");
    let project = temp.path("skirmish/project");
    let sql = "SELECT e.name FROM entities e JOIN entity_tags t ON t.entity_id = e.id \
               WHERE t.tag = 'enemy' \
               AND (e.x - (SELECT avg(x) FROM entities WHERE prefab = 'house')) \
                 * (e.x - (SELECT avg(x) FROM entities WHERE prefab = 'house')) \
               + (e.z - (SELECT avg(z) FROM entities WHERE prefab = 'house')) \
                 * (e.z - (SELECT avg(z) FROM entities WHERE prefab = 'house')) <= 100 * 100 \
               ORDER BY e.name";
    let answer = json_of(&["project", "query", &text(&project), sql]);
    assert_eq!(answer["columns"], json!(["name"]));
    assert_eq!(
        answer["rows"],
        json!([
            ["Hill Sentry"],
            ["Patrol Alpha"],
            ["Patrol Bravo"],
            ["Village Guard"]
        ]),
        "{answer}"
    );
    assert!(project.join(".gamecraft-index/index.db").exists());
    let by_layer = json_of(&[
        "project",
        "query",
        &text(&project),
        "SELECT layer, count(*) FROM entities GROUP BY layer ORDER BY layer",
    ]);
    assert_eq!(
        by_layer["rows"],
        json!([["enemies", 4], ["terrain-features", 47], ["village", 4]])
    );
    // The mod's index knows the packs.
    let packs = json_of(&[
        "project",
        "query",
        &text(&temp.path("skirmish-night-mod")),
        "SELECT pack, count(*) FROM entities GROUP BY pack ORDER BY pack",
    ]);
    assert_eq!(
        packs["rows"],
        json!([["skirmish", 55], ["skirmish-night", 1]])
    );
    let refused = run(&["project", "query", &text(&project), "DELETE FROM entities"]);
    assert_eq!(refused.code, 1, "{}", refused.out);
    assert!(refused.err.contains("only one SELECT"), "{}", refused.err);
}

#[test]
fn run_info_and_project_new_take_projects() {
    let temp = Temp::new("project-cli");
    let project = examples().join("skirmish/project");
    let info = json_of(&["info", &text(&project)]);
    assert_eq!(info["entity_count"], json!(55));
    let ran = run(&[
        "run",
        &text(&project),
        "--cmd",
        "entity.get",
        "--params",
        r#"{"id": "Barn"}"#,
    ]);
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(ran.out.contains("\"layer\":\"village\""), "{}", ran.out);
    let made = json_of(&[
        "project",
        "new",
        &text(&temp.path("copy")),
        "--id",
        "copy",
        "--name",
        "Copy",
        "--world",
        "village",
        "--from",
        &text(&examples().join("skirmish/world.gamecraft")),
    ]);
    assert_eq!(made["id"], json!("copy"));
    let info = json_of(&["project", "info", &text(&temp.path("copy"))]);
    assert_eq!(info["entities"], json!(55));
    assert_eq!(info["changes"]["files"], json!([]));
    let bad = run(&["project", "frobnicate"]);
    assert_eq!(bad.code, 2);
}

#[test]
fn the_committed_layer_and_prefab_files_are_in_the_form_a_save_writes() {
    let dir = examples().join("skirmish/project");
    let source = gamecraft_project::PackSource {
        location: text(&dir),
        files: files(&dir)
            .into_iter()
            .map(|(p, t)| {
                let rel = p
                    .strip_prefix(&dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                (rel, t)
            })
            .collect(),
    };
    let packs = BTreeMap::from([("skirmish".to_string(), source.clone())]);
    let (_, world) = gamecraft_project::Project::load("skirmish", &packs, None).unwrap();
    let (_, saving) =
        gamecraft_project::Project::create("skirmish", "Skirmish", "village", &world).unwrap();
    let mut compared = 0;
    for (path, written) in &saving.plan.writes {
        if path.starts_with("prefabs/") || path.contains("/layers/") {
            assert_eq!(
                source.files.get(path),
                Some(written),
                "{path} is not in the form a save writes, so an edit would rewrite more than \
                 its line"
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 5, "three layers and two prefabs");
}
