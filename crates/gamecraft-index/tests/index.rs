//! The index answers SQL about a world and refuses to be written to.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use gamecraft_index::{INDEX_DIR, INDEX_FILE, Index, MAX_ROWS};
use gamecraft_project::Project;
use gamecraft_world::{Component, EntityDraft, Health, Shape, Tags, World};
use serde_json::json;

fn town(n: usize) -> World {
    let mut w = World::new("Town", 1);
    for i in 0..n {
        let enemy = i % 10 == 0;
        let mut components = vec![Component::Shape(Shape::default())];
        if enemy {
            components.push(Component::Tags(Tags {
                tags: vec!["enemy".into(), "team:red".into()],
            }));
            components.push(Component::Health(Health::default()));
        }
        let (x, z) = ((i % 100) as f64 * 2.0, (i / 100) as f64 * 2.0);
        w.add_entity(EntityDraft {
            name: format!("{} {i}", if enemy { "Enemy" } else { "Crate" }),
            position: [x, 0.0, z],
            layer: Some(if enemy { "enemies" } else { "props" }.into()),
            components,
            ..EntityDraft::default()
        })
        .unwrap();
    }
    w
}

#[test]
fn the_index_answers_queries_about_the_world() {
    let world = town(200);
    let mut index = Index::open(None).unwrap();
    let done = index.rebuild(&world, None).unwrap();
    assert_eq!((done.entities, done.prefabs), (200, 0));
    let count = index
        .query("SELECT count(*) AS n FROM entities", 10)
        .unwrap();
    assert_eq!(count.columns, vec!["n"]);
    assert_eq!(count.rows, vec![vec![json!(200)]]);
    let enemies = index
        .query(
            "SELECT e.name FROM entities e JOIN entity_tags t ON t.entity_id = e.id \
             WHERE t.tag = 'enemy' AND (e.x - 0) * (e.x - 0) + (e.z - 0) * (e.z - 0) <= 20 * 20 \
             ORDER BY e.id",
            100,
        )
        .unwrap();
    let names: Vec<&str> = enemies
        .rows
        .iter()
        .map(|r| r[0].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Enemy 0", "Enemy 10", "Enemy 100"]);
    let layers = index
        .query(
            "SELECT layer, count(*) FROM entities GROUP BY layer ORDER BY layer",
            10,
        )
        .unwrap();
    assert_eq!(
        layers.rows,
        vec![
            vec![json!("enemies"), json!(20)],
            vec![json!("props"), json!(180)]
        ]
    );
    let health = index
        .query(
            "SELECT count(*) FROM entity_components WHERE type = 'health'",
            10,
        )
        .unwrap();
    assert_eq!(health.rows, vec![vec![json!(20)]]);
    let capped = index.query("SELECT id FROM entities", 50).unwrap();
    assert_eq!(capped.rows.len(), 50);
    assert!(capped.truncated);
    // Rebuilt after a change: the rows follow the world.
    let mut changed = world.clone();
    changed.remove_entity(changed.find("Enemy 0").unwrap());
    index.rebuild(&changed, None).unwrap();
    let count = index.query("SELECT count(*) FROM entities", 10).unwrap();
    assert_eq!(count.rows, vec![vec![json!(199)]]);
}

#[test]
fn writes_and_other_statements_are_refused() {
    let world = town(20);
    let mut index = Index::open(None).unwrap();
    index.rebuild(&world, None).unwrap();
    for bad in [
        "DELETE FROM entities",
        "UPDATE entities SET name = 'x'",
        "INSERT INTO entities (id) VALUES (999)",
        "DROP TABLE entities",
        "SELECT 1; DELETE FROM entities",
        "PRAGMA query_only = 0",
        "SELECT load_extension('x')",
    ] {
        let e = index.query(bad, 10).unwrap_err();
        assert!(!e.is_empty(), "{bad}");
    }
    let e = index.query("SELECT nope FROM nowhere", 10).unwrap_err();
    assert!(e.contains("not valid SQL") || e.contains("failed"), "{e}");
    let count = index.query("SELECT count(*) FROM entities", 10).unwrap();
    assert_eq!(count.rows, vec![vec![json!(20)]], "nothing was written");
}

#[test]
fn the_index_lives_beside_the_project_and_carries_string_ids_and_packs() {
    let world = town(30);
    let (mut project, saving) = Project::create("town", "Town", "main", &world).unwrap();
    project.finish_save(saving);
    let dir = std::env::temp_dir().join(format!("gamecraft-index-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut index = Index::open(Some(&dir)).unwrap();
    assert_eq!(
        index.path(),
        Some(dir.join(INDEX_DIR).join(INDEX_FILE).as_path())
    );
    index.rebuild(&world, Some(&project)).unwrap();
    let rows = index
        .query(
            "SELECT key, pack, layer FROM entities WHERE name = 'Enemy 10'",
            10,
        )
        .unwrap();
    assert_eq!(
        rows.rows,
        vec![vec![json!("enemy-10"), json!("town"), json!("enemies")]]
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(INDEX_DIR).join(".gitignore")).unwrap(),
        "*\n"
    );
    drop(index);
    // Opening again starts from empty tables.
    let index = Index::open(Some(&dir)).unwrap();
    let count = index.query("SELECT count(*) FROM entities", 10).unwrap();
    assert_eq!(count.rows, vec![vec![json!(0)]]);
    drop(index);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_rebuild_of_ten_thousand_entities_is_measured() {
    let world = town(10_000);
    let mut index = Index::open(None).unwrap();
    let start = std::time::Instant::now();
    let done = index.rebuild(&world, None).unwrap();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("rebuild of {} entities: {ms:.1} ms", done.entities);
    assert_eq!(done.entities, 10_000);
    let start = std::time::Instant::now();
    let answer = index
        .query(
            "SELECT e.id FROM entities e JOIN entity_tags t ON t.entity_id = e.id WHERE t.tag = 'enemy'",
            MAX_ROWS,
        )
        .unwrap();
    eprintln!(
        "tag query over 10 000 entities: {:.1} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(answer.rows.len(), 1000);
    assert!(!answer.truncated, "exactly 1000 enemies");
}
