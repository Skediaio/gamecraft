//! Terrain edits in a project: `worlds/<world>/terrain/edits.json`, layered across packs and
//! saved to the root pack.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{apply, pack, pack_of, packs, village};
use gamecraft_project::{PackSource, Project, TERRAIN_EDITS_FORMAT};
use gamecraft_world::{PaintRule, Terrain, TerrainEdit, TerrainLayer, TerrainSource, World};
use serde_json::Value;

const EDITS: &str = "worlds/village/terrain/edits.json";

fn raise(x: f64, strength_m: f64) -> TerrainEdit {
    TerrainEdit::Raise {
        center: [x, 0.0],
        radius_m: 5.0,
        strength_m,
        falloff: 0.5,
    }
}

/// The village on sculpted, painted ground.
fn sculpted() -> World {
    let mut w = village();
    let mut t = Terrain::new(
        128.0,
        2.0,
        TerrainSource::Noise {
            seed: 4,
            relief_m: 10.0,
            feature_m: 50.0,
            octaves: 3,
            roughness: 0.5,
        },
    );
    t.layers = vec![
        TerrainLayer {
            name: "grass".into(),
            color: [0.1, 0.2, 0.05],
            roughness: 0.9,
        },
        TerrainLayer {
            name: "rock".into(),
            color: [0.3, 0.3, 0.3],
            roughness: 0.8,
        },
    ];
    t.paint_rules = vec![PaintRule {
        layer: "rock".into(),
        slope_min_deg: Some(35.0),
        ..PaintRule::default()
    }];
    t.edits = vec![raise(0.0, 2.0), raise(20.0, 1.0)];
    w.terrain = Some(t);
    w
}

fn game() -> (PackSource, Project, World) {
    let world = sculpted();
    let (mut project, saving) = Project::create("game", "Game", "village", &world).unwrap();
    let source = pack_of("game", &saving.plan);
    project.finish_save(saving);
    (source, project, world)
}

#[test]
fn edits_live_in_their_own_file_one_per_line() {
    let (source, _, world) = game();
    let text = &source.files[EDITS];
    assert!(
        text.starts_with(&format!(
            "{{\n  \"gamecraft_terrain_edits\": {TERRAIN_EDITS_FORMAT},\n  \"edits\": [\n"
        )),
        "{text}"
    );
    let lines: Vec<&str> = text.lines().filter(|l| l.contains("\"op\"")).collect();
    assert_eq!(lines.len(), 2, "{text}");
    // world.json keeps the recipe without its edits; the layers and rules stay there.
    let world_json: Value =
        serde_json::from_str(&source.files["worlds/village/world.json"]).unwrap();
    assert!(world_json["terrain"].get("edits").is_none());
    assert_eq!(world_json["terrain"]["layers"][1]["name"], "rock");
    // Loaded again, the world is the same, edits in order.
    let (project, loaded) = Project::load("game", &packs(vec![("game", source)]), None).unwrap();
    assert_eq!(loaded.terrain, world.terrain);
    assert!(project.plan_save(&loaded).unwrap().plan.is_empty());
}

#[test]
fn an_added_edit_writes_one_more_line_of_the_edits_file_only() {
    let (mut source, mut project, mut world) = game();
    world
        .terrain
        .as_mut()
        .unwrap()
        .edits
        .push(raise(-20.0, 3.0));
    let saving = project.plan_save(&world).unwrap();
    let paths: Vec<&str> = saving.plan.writes.keys().map(String::as_str).collect();
    assert_eq!(paths, vec![EDITS]);
    let before = source.files[EDITS].clone();
    apply(&mut source, &saving.plan);
    project.finish_save(saving);
    let after = &source.files[EDITS];
    assert_eq!(after.lines().count(), before.lines().count() + 1);
    let (_, loaded) = Project::load("game", &packs(vec![("game", source)]), None).unwrap();
    assert_eq!(loaded.terrain, world.terrain);
    // Removing every edit writes an empty list, and a world without terrain no file at all.
    assert!(project.plan_save(&world).unwrap().plan.is_empty());
}

#[test]
fn a_mod_replaces_the_edits_and_saving_never_touches_the_game() {
    let (game_source, _, base) = game();
    let modded = pack(
        "mod",
        &[
            (
                "pack.json",
                r#"{"gamecraft_pack": 1, "id": "mod", "name": "Mod", "version": "0.1.0", "depends": [{"id": "game"}]}"#,
            ),
            (
                EDITS,
                "{\"gamecraft_terrain_edits\": 1, \"edits\": [\n{\"op\": \"lower\", \"center\": [0, 0], \"radius_m\": 4, \"strength_m\": 3}\n]}\n",
            ),
        ],
    );
    let all = packs(vec![("game", game_source.clone()), ("mod", modded)]);
    let (project, mut world) = Project::load("mod", &all, None).unwrap();
    let edits = &world.terrain.as_ref().unwrap().edits;
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].op(), "lower");
    // Back to the game's edits: the mod's file now holds them.
    world.terrain.as_mut().unwrap().edits = base.terrain.as_ref().unwrap().edits.clone();
    let saving = project.plan_save(&world).unwrap();
    let paths: Vec<&str> = saving.plan.writes.keys().map(String::as_str).collect();
    assert_eq!(paths, vec![EDITS]);
    assert_eq!(saving.plan.writes[EDITS].matches("\"op\"").count(), 2);
    // A mod without an edits file of its own takes the game's and saves nothing.
    let bare = pack(
        "mod",
        &[(
            "pack.json",
            r#"{"gamecraft_pack": 1, "id": "mod", "name": "Mod", "version": "0.1.0", "depends": [{"id": "game"}]}"#,
        )],
    );
    let all = packs(vec![("game", game_source), ("mod", bare)]);
    let (project, world) = Project::load("mod", &all, None).unwrap();
    assert_eq!(world.terrain, base.terrain);
    assert!(project.plan_save(&world).unwrap().plan.is_empty());
}

fn refused(edits: &str) -> String {
    let (mut source, _, _) = game();
    source.files.insert(EDITS.into(), edits.into());
    Project::load("game", &packs(vec![("game", source)]), None)
        .map(|_| ())
        .unwrap_err()
        .to_string()
}

#[test]
fn hostile_edit_files_are_refused_naming_the_file() {
    for (text, needle) in [
        ("{", "not valid JSON"),
        ("[]", "gamecraft_terrain_edits"),
        (
            r#"{"gamecraft_terrain_edits": 2, "edits": []}"#,
            "gamecraft_terrain_edits",
        ),
        (
            r#"{"gamecraft_terrain_edits": 1, "edits": [{"op": "dig"}]}"#,
            "dig",
        ),
        (
            r#"{"gamecraft_terrain_edits": 1, "edits": [], "more": 1}"#,
            "more",
        ),
        (
            r#"{"gamecraft_terrain_edits": 1, "edits": [{"op": "raise", "center": [0, 0], "radius_m": -1, "strength_m": 1}]}"#,
            "radius_m",
        ),
        (
            r#"{"gamecraft_terrain_edits": 1, "edits": [{"op": "paint", "layer": "snow", "center": [0, 0], "radius_m": 1}]}"#,
            "snow",
        ),
    ] {
        let e = refused(text);
        assert!(e.contains(EDITS) || e.contains("terrain"), "{text}: {e}");
        assert!(e.contains(needle), "{text}: {e}");
    }
    // Edits for a world without terrain.
    let mut flat = village();
    flat.terrain = None;
    let (_, saving) = Project::create("game", "Game", "village", &flat).unwrap();
    let mut source = pack_of("game", &saving.plan);
    source.files.insert(
        EDITS.into(),
        r#"{"gamecraft_terrain_edits": 1, "edits": [{"op": "raise", "center": [0, 0], "radius_m": 4, "strength_m": 1}]}"#.into(),
    );
    let e = Project::load("game", &packs(vec![("game", source)]), None)
        .map(|_| ())
        .unwrap_err()
        .to_string();
    assert!(e.contains(EDITS) && e.contains("no terrain"), "{e}");
}

#[test]
fn every_truncation_of_the_edits_file_is_an_error_not_a_panic() {
    let (source, _, _) = game();
    let text = source.files[EDITS].clone();
    for cut in 0..text.len() {
        let mut broken = source.clone();
        broken.files.insert(EDITS.into(), text[..cut].to_string());
        let all = packs(vec![("game", broken)]);
        let outcome = std::panic::catch_unwind(|| Project::load("game", &all, None).map(|_| ()));
        assert!(outcome.is_ok(), "cut at {cut} panicked");
    }
}
