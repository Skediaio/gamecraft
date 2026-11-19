//! Generators in a project: `worlds/<world>/generators.json`, layered across packs by id and
//! saved to the root pack as the difference from the dependencies.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{apply, pack, pack_of, packs, village};
use gamecraft_project::{GENERATORS_FORMAT, PackSource, Project};
use gamecraft_world::{
    Area, Circle, Component, Generator, GeneratorItem, GeneratorKind, Plant, Terrain,
    TerrainSource, World,
};

const FILE: &str = "worlds/village/generators.json";
const MOD: &str = r#"{"gamecraft_pack": 1, "id": "mod", "name": "Mod", "version": "0.1.0", "depends": [{"id": "game"}]}"#;

fn wood(id: &str, x: f64, density: f64) -> Generator {
    Generator {
        id: id.into(),
        kind: GeneratorKind::Forest,
        area: Area::Circle(Circle {
            center: [x, 0.0],
            radius_m: 30.0,
        }),
        seed: 4,
        density_per_ha: density,
        min_distance_m: Some(3.0),
        slope_max_deg: None,
        height_range_m: None,
        exclude: Vec::new(),
        items: vec![GeneratorItem::of(
            1.0,
            vec![Component::Plant(Plant {
                species: "picea-abies".into(),
                seed: 2,
                growth: 1.0,
            })],
        )],
    }
}

fn wooded() -> World {
    let mut w = village();
    w.terrain = Some(Terrain::new(
        256.0,
        4.0,
        TerrainSource::Flat { height_m: 0.0 },
    ));
    w.generators = vec![wood("north", 0.0, 100.0), wood("south", 60.0, 50.0)];
    w
}

fn game() -> (PackSource, Project, World) {
    let world = wooded();
    let (mut project, saving) = Project::create("game", "Game", "village", &world).unwrap();
    let source = pack_of("game", &saving.plan);
    project.finish_save(saving);
    (source, project, world)
}

#[test]
fn generators_live_in_their_own_file_one_per_line() {
    let (source, _, world) = game();
    let text = &source.files[FILE];
    assert!(
        text.starts_with(&format!(
            "{{\n  \"gamecraft_generators\": {GENERATORS_FORMAT},\n  \"generators\": [\n"
        )),
        "{text}"
    );
    assert_eq!(text.lines().filter(|l| l.contains("\"kind\"")).count(), 2);
    assert!(!source.files["worlds/village/world.json"].contains("generators"));
    let (project, loaded) = Project::load("game", &packs(vec![("game", source)]), None).unwrap();
    assert_eq!(loaded.generators, world.generators);
    assert!(project.plan_save(&loaded).unwrap().plan.is_empty());
}

#[test]
fn a_changed_generator_rewrites_only_the_generators_file() {
    let (mut source, mut project, mut world) = game();
    world.generators[1].density_per_ha = 80.0;
    let saving = project.plan_save(&world).unwrap();
    let paths: Vec<&str> = saving.plan.writes.keys().map(String::as_str).collect();
    assert_eq!(paths, vec![FILE]);
    apply(&mut source, &saving.plan);
    project.finish_save(saving);
    let (_, loaded) = Project::load("game", &packs(vec![("game", source)]), None).unwrap();
    assert_eq!(loaded.generators, world.generators);
}

#[test]
fn a_mod_overrides_by_id_and_saving_never_touches_the_game() {
    let (game_source, _, base) = game();
    let lines = format!(
        "{{\"gamecraft_generators\": 1, \"generators\": [\n{},\n{},\n{{\"id\": \"south\", \"removed\": true}}\n]}}\n",
        serde_json::to_string(&wood("north", 0.0, 300.0)).unwrap(),
        serde_json::to_string(&wood("east", 100.0, 20.0)).unwrap(),
    );
    let modded = pack("mod", &[("pack.json", MOD), (FILE, &lines)]);
    let all = packs(vec![("game", game_source.clone()), ("mod", modded)]);
    let (project, mut world) = Project::load("mod", &all, None).unwrap();
    let ids: Vec<&str> = world.generators.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(ids, vec!["north", "east"]);
    assert_eq!(world.generators[0].density_per_ha, 300.0);
    assert!(project.plan_save(&world).unwrap().plan.is_empty());
    // Back to the game's north: the mod's file keeps only what still differs.
    world.generators[0] = base.generators[0].clone();
    let saving = project.plan_save(&world).unwrap();
    let paths: Vec<&str> = saving.plan.writes.keys().map(String::as_str).collect();
    assert_eq!(paths, vec![FILE]);
    let text = &saving.plan.writes[FILE];
    assert!(
        text.contains("\"east\"") && text.contains("\"removed\":true"),
        "{text}"
    );
    assert!(!text.contains("\"north\""), "{text}");
    let mut mod_source = pack("mod", &[("pack.json", MOD)]);
    apply(&mut mod_source, &saving.plan);
    let all = packs(vec![("game", game_source.clone()), ("mod", mod_source)]);
    let (_, again) = Project::load("mod", &all, None).unwrap();
    assert_eq!(again.generators, world.generators);
    // A mod without a generators file takes the game's and saves nothing.
    let bare = pack("mod", &[("pack.json", MOD)]);
    let all = packs(vec![("game", game_source), ("mod", bare)]);
    let (project, world) = Project::load("mod", &all, None).unwrap();
    assert_eq!(world.generators, base.generators);
    assert!(project.plan_save(&world).unwrap().plan.is_empty());
}

fn refused(text: &str) -> String {
    let (mut source, _, _) = game();
    source.files.insert(FILE.into(), text.into());
    Project::load("game", &packs(vec![("game", source)]), None)
        .map(|_| ())
        .unwrap_err()
        .to_string()
}

#[test]
fn hostile_generator_files_are_refused_naming_the_file() {
    let good = serde_json::to_string(&wood("north", 0.0, 100.0)).unwrap();
    let bad_density = good.replace("\"density_per_ha\":100.0", "\"density_per_ha\":-5.0");
    assert_ne!(bad_density, good);
    for (text, needle) in [
        ("{", "not valid JSON".to_string()),
        ("[]", "gamecraft_generators".to_string()),
        (
            r#"{"gamecraft_generators": 2, "generators": []}"#,
            "gamecraft_generators".to_string(),
        ),
        (
            r#"{"gamecraft_generators": 1, "generators": [], "more": 1}"#,
            "more".to_string(),
        ),
        (
            r#"{"gamecraft_generators": 1, "generators": [{"id": "x", "kind": "jungle"}]}"#,
            "jungle".to_string(),
        ),
        (
            r#"{"gamecraft_generators": 1, "generators": [{"id": "x", "removed": false}]}"#,
            "removal".to_string(),
        ),
        (
            &format!(r#"{{"gamecraft_generators": 1, "generators": [{good}, {good}]}}"#),
            "once in a file".to_string(),
        ),
        (
            &format!(r#"{{"gamecraft_generators": 1, "generators": [{bad_density}]}}"#),
            "density_per_ha".to_string(),
        ),
    ] {
        let e = refused(text);
        assert!(e.contains(FILE), "{text}: {e}");
        assert!(e.contains(&needle), "{text}: {e}");
    }
}

#[test]
fn every_truncation_of_the_generators_file_is_an_error_not_a_panic() {
    let (source, _, _) = game();
    let text = source.files[FILE].clone();
    for cut in 0..text.len() {
        let mut broken = source.clone();
        broken.files.insert(FILE.into(), text[..cut].to_string());
        let all = packs(vec![("game", broken)]);
        let outcome = std::panic::catch_unwind(|| Project::load("game", &all, None).map(|_| ()));
        assert!(outcome.is_ok(), "cut at {cut} panicked");
    }
}
