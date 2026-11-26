//! The queries go through the session's index of what a ray can meet, which follows the
//! document: a ray after edits finds what a session that made its index only after the edits
//! finds, and so it does after undo, and on a running world a command changed.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::refuse;
use gamecraft_engine::Session;
use gamecraft_world::{EntityId, Terrain, TerrainSource, World};
use serde_json::{Value, json};

/// A session of a flat terrain at height 0, 400 m square, without entities.
fn flat() -> Session {
    let mut w = World::new("Flat", 1);
    w.terrain = Some(Terrain {
        size_m: 400.0,
        cell_m: 2.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    Session::new(w)
}

/// The edits, always in the same order (so ids agree): the first `count` of them.
fn edits(s: &mut Session, count: usize) {
    let steps: [(&str, Value); 7] = [
        (
            "entity.add",
            json!({"name": "Wall", "position": [0, 0, 0], "components": [{"type": "shape", "kind": "box", "size": [20, 4, 1]}]}),
        ),
        (
            "entity.add",
            json!({"name": "Tower", "position": [40, 0, 40], "components": [{"type": "shape", "kind": "cylinder", "size": [3, 30, 3]}]}),
        ),
        (
            "prefab.add",
            json!({"id": "gate", "name": "Gate", "children": [
                {"name": "Post", "offset": [-2, 0, 0], "components": [{"type": "shape", "kind": "box", "size": [0.4, 3, 0.4]}]},
                {"name": "Post", "offset": [2, 0, 0], "components": [{"type": "shape", "kind": "box", "size": [0.4, 3, 0.4]}]}
            ]}),
        ),
        (
            "prefab.spawn",
            json!({"prefab": "gate", "position": [-60, 0, 30]}),
        ),
        (
            "entity.set",
            json!({"id": "Wall", "position": [10, 1, -20], "rotation_y_deg": 30, "scale": [1, 2, 4]}),
        ),
        (
            "entity.set",
            json!({"id": "Tower", "position": [-30, 0, 80]}),
        ),
        ("entity.remove", json!({"id": 3})),
    ];
    for (command, params) in steps.iter().take(count) {
        s.run(command, params).unwrap();
    }
}

/// A session of the flat world with the first `count` edits applied.
fn edited(count: usize) -> Session {
    let mut s = flat();
    edits(&mut s, count);
    s
}

/// Where the edits put things, and around them: the probes aim there.
const TARGETS: [[f64; 3]; 8] = [
    [10.0, 1.0, -20.0],
    [10.0, 6.0, -22.0],
    [-60.0, 1.5, 30.0],
    [-62.0, 2.0, 30.0],
    [40.0, 15.0, 40.0],
    [-30.0, 15.0, 80.0],
    [0.0, 2.0, 0.0],
    [20.0, 3.0, 10.0],
];

/// Thirty rays and sixteen lines of sight aimed at the places the edits touch, as JSON, each
/// with its number: rays from above and beside (some too short to arrive), lines of sight
/// between the targets.
fn probes(s: &mut Session) -> Vec<Value> {
    let mut out = Vec::new();
    for (i, target) in TARGETS.iter().enumerate() {
        let origins = [
            [
                target[0] - 60.0 - i as f64,
                25.0 + (i as f64) * 7.0,
                target[2] - 90.0,
            ],
            [target[0] + 50.0, 2.0 + i as f64, target[2] + 120.0],
            [target[0], 60.0, target[2] + 0.5],
            [target[0] + 30.0, 1.0, target[2] - 60.0],
        ];
        for (k, origin) in origins.iter().enumerate() {
            let dir = [
                target[0] - origin[0],
                target[1] - origin[1],
                target[2] - origin[2],
            ];
            // Every fifth ray stops short of its target.
            let max = if (i + k) % 5 == 0 { 20.0 } else { 2000.0 };
            let mut ray = s
                .run(
                    "query.raycast",
                    &json!({"origin": origin, "direction": dir, "max_m": max}),
                )
                .unwrap();
            if let Value::Object(map) = &mut ray {
                map.insert("probe".into(), json!(i * 4 + k));
            }
            out.push(ray);
        }
    }
    for (i, from) in TARGETS.iter().enumerate() {
        for to in TARGETS.iter().skip(i + 1).step_by(2) {
            let mut sight = s
                .run("query.line_of_sight", &json!({"from": from, "to": to}))
                .unwrap();
            if let Value::Object(map) = &mut sight {
                map.insert("probe".into(), json!(100 + i));
            }
            out.push(sight);
        }
    }
    out
}

#[test]
fn rays_after_edits_find_what_a_fresh_session_finds() {
    // A session that made its index before the edits, then is edited.
    let mut early = flat();
    let before = probes(&mut early);
    edits(&mut early, 7);
    let after_early = probes(&mut early);
    // The same edits on a session that made no index before them.
    let mut late = flat();
    edits(&mut late, 7);
    let after_late = probes(&mut late);
    assert_eq!(after_early, after_late);
    // The edits changed what the rays meet, and something is met.
    assert_ne!(before, after_early);
    assert!(
        before.iter().all(|r| r["kind"] != "entity"),
        "an empty world"
    );
    assert!(
        after_early.iter().filter(|r| r["kind"] == "entity").count() >= 3,
        "{after_early:?}"
    );
    // Undo takes the edits back one at a time: after each, the rays are those of a session
    // that only ever had the edits that remain, through the index that followed the undos.
    for left in (0..7).rev() {
        early.undo().unwrap();
        assert_eq!(
            probes(&mut early),
            probes(&mut edited(left)),
            "{left} edits left"
        );
    }
    assert_eq!(probes(&mut early), before, "back to the start, to the bit");
    for left in 1..=7 {
        early.redo().unwrap();
        assert_eq!(
            probes(&mut early),
            probes(&mut edited(left)),
            "{left} edits back"
        );
    }
    // A failed command leaves the index as it was.
    let snapshot = probes(&mut early);
    refuse(
        &mut early,
        "entity.set",
        json!({"id": "Nobody"}),
        &["Nobody"],
    );
    assert_eq!(probes(&mut early), snapshot);
}

#[test]
fn a_running_world_changed_by_a_command_is_indexed_from_the_next_ray() {
    let mut s = flat();
    s.run(
        "entity.add",
        &json!({"name": "Rock", "position": [0, 0, 0], "components": [{"type": "shape", "kind": "box", "size": [2, 2, 2]}]}),
    )
    .unwrap();
    s.run("sim.start", &Value::Null).unwrap();
    let down = json!({"origin": [30, 20, 0], "direction": [0, -1, 0]});
    assert_eq!(s.run("query.raycast", &down).unwrap()["kind"], "terrain");
    // The game master drops a block: the next ray meets it.
    let dropped = s
        .run(
            "entity.add",
            &json!({"name": "Block", "position": [30, 0, 0], "components": [{"type": "shape", "kind": "box", "size": [4, 5, 4]}]}),
        )
        .unwrap();
    let hit = s.run("query.raycast", &down).unwrap();
    assert_eq!(hit["entity"], dropped["id"], "{hit}");
    assert_eq!(hit["point"][1], json!(5.0));
    // Moved out of the ray's way, and back; removed.
    s.run(
        "entity.set",
        &json!({"id": dropped["id"].clone(), "position": [50, 0, 0]}),
    )
    .unwrap();
    assert_eq!(s.run("query.raycast", &down).unwrap()["kind"], "terrain");
    s.run(
        "entity.set",
        &json!({"id": dropped["id"].clone(), "position": [30, 0, 0], "scale": [2, 1, 2]}),
    )
    .unwrap();
    assert_eq!(
        s.run("query.raycast", &down).unwrap()["entity"],
        dropped["id"]
    );
    let id = dropped["id"].as_u64().unwrap();
    s.run("entity.remove", &json!({"id": id})).unwrap();
    assert_eq!(s.run("query.raycast", &down).unwrap()["kind"], "terrain");
    // The document was never touched by any of it.
    assert!(!s.document().entity(EntityId(id)).is_some());
    assert!(s.live().unwrap().world().entity(EntityId(id)).is_none());
}
