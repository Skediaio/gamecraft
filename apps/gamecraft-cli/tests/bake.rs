//! `project bake`: a script of commands saved as a celled project in one step. The same script
//! gives the same files; a changed script changes only the cells it touches and a layer edited
//! by hand survives; a bad line names its line and writes nothing; a hundred thousand entities
//! bake in one go (the time is printed, for the report).

// Test helpers: clippy.toml allows these inside #[test] functions only.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

mod common;
use common::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The steps of a script of `count` boxes with their layer named, over a kilometre, after a
/// flat terrain.
fn steps(count: usize) -> Vec<Value> {
    let mut steps = vec![json!([
        "terrain.set",
        {"kind": "flat", "size_m": 1024.0, "cell_m": 8.0, "height_m": 0.0}
    ])];
    for i in 0..count {
        let x = (i % 20) as f64 * 50.0;
        let z = (i / 20) as f64 * 50.0;
        steps.push(json!([
            "entity.add",
            {
                "name": format!("Box {i}"),
                "layer": "main",
                "position": [x, 0.0, z],
                "components": [{"type": "shape", "kind": "box", "size": [4, 3, 4],
                                "color": [0.6, 0.5, 0.4], "solid": true}]
            }
        ]));
    }
    steps
}

/// A script of `count` boxes as the text of its file: one step per line, so that a step's
/// line is its number plus one.
fn script(count: usize) -> String {
    let mut text = String::from("[\n");
    for step in steps(count) {
        text.push_str(&format!("  {step},\n"));
    }
    // The last step's comma is a trailing one: take it back.
    text.truncate(text.len() - 2);
    text.push_str("\n]\n");
    text
}

/// Every file under `dir`, by path relative to it (the index and the terrain cache left out:
/// derived, ignored by git).
fn files(dir: &Path) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.ends_with(".gamecraft-index") || path.ends_with("cache") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(
                    path.strip_prefix(dir).unwrap().to_path_buf(),
                    String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned(),
                );
            }
        }
    }
    out
}

fn bake(temp: &Temp, script: &str, into: &str, more: &[&str]) -> Ran {
    let mut args = vec![
        "project".to_string(),
        "bake".to_string(),
        script.to_string(),
        "--into".to_string(),
        into.to_string(),
    ];
    args.extend(more.iter().map(|s| s.to_string()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run_in(&temp.0, &args)
}

/// The state hash of what a server session has after `project.open` of `dir`, or of the script
/// run step by step on a new world.
fn hash_of(steps: &[Value], project: Option<&str>, temp: &Temp) -> String {
    let root = temp.0.display().to_string();
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
    match project {
        Some(dir) => {
            server.ok("project.open", json!({"path": dir}));
        }
        None => {
            server.ok("file.new", json!({}));
            for step in steps {
                let (id, params) = (step[0].as_str().unwrap(), &step[1]);
                server.ok(id, params.clone());
            }
        }
    }
    server.ok("sim.start", json!({}));
    server.ok("sim.state", json!({}))["hash"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn a_script_baked_into_a_new_project_is_celled_and_hashes_as_run() {
    let temp = Temp::new("bake-new");
    let text = script(400);
    temp.write("gen.json", &text);
    let ran = bake(
        &temp,
        "gen.json",
        "world",
        &["--id", "world", "--name", "World", "--cell-size-m", "256"],
    );
    assert_eq!(ran.code, 0, "{} {}", ran.out, ran.err);
    let answer: Value = serde_json::from_str(ran.out.lines().next().unwrap()).unwrap();
    assert_eq!(answer["steps"], json!(401), "{answer}");
    assert_eq!(answer["entities"], json!(400), "{answer}");
    assert_eq!(
        (answer["world"].clone(), answer["cell_size_m"].clone()),
        (json!("main"), json!(256.0))
    );
    assert_eq!(answer["layers"], json!(["main"]));
    // Cells of 256 m over a kilometre of boxes: sixteen files, and the record.
    let files = files(&temp.path("world"));
    let cells: Vec<&PathBuf> = files
        .keys()
        .filter(|p| p.to_string_lossy().starts_with("worlds/main/layers/main/"))
        .collect();
    assert_eq!(cells.len(), 16, "{cells:?}");
    assert!(
        files.contains_key(Path::new("worlds/main/bake.json")),
        "{files:?}"
    );
    assert!(files.contains_key(Path::new("pack.json")));
    assert!(!files.contains_key(Path::new("worlds/main/layers/main.json")));
    // The same entities as running the script directly. In cells the ids follow the cell
    // order, so the state hash differs from a run that adds in another order; with one file
    // per layer (a cell size of 0) the order, and so the hash, is the script's.
    let steps = steps(400);
    let direct = hash_of(&steps, None, &temp);
    let whole = bake(
        &temp,
        "gen.json",
        "whole",
        &["--id", "world", "--cell-size-m", "0"],
    );
    assert_eq!(whole.code, 0, "{} {}", whole.out, whole.err);
    assert_eq!(hash_of(&steps, Some("whole"), &temp), direct);
    let by_name = |file: &str| {
        let text = std::fs::read_to_string(temp.path(file)).unwrap();
        let world: Value = serde_json::from_str(&text).unwrap();
        let mut entities: Vec<Value> = world["entities"].as_array().unwrap().clone();
        for e in &mut entities {
            e["id"] = Value::Null;
        }
        entities.sort_by_key(|e| e["name"].to_string());
        entities
    };
    let _ = run_in(
        &temp.0,
        &["project", "export", "world", "--out", "celled.gamecraft"],
    );
    let _ = run_in(
        &temp.0,
        &["project", "export", "whole", "--out", "whole.gamecraft"],
    );
    assert_eq!(by_name("celled.gamecraft"), by_name("whole.gamecraft"));
    // project open reads the cells back.
    let open = run_in(&temp.0, &["project", "open", "world"]);
    assert_eq!(open.code, 0, "{}", open.err);
    let opened: Value = serde_json::from_str(open.out.lines().next().unwrap()).unwrap();
    assert_eq!(opened["entities"], json!(400), "{opened}");
    assert_eq!(opened["cell_size_m"], json!(256.0));
}

#[test]
fn the_same_script_baked_again_gives_the_same_files() {
    let temp = Temp::new("bake-again");
    temp.write("gen.json", &script(400));
    let a = bake(
        &temp,
        "gen.json",
        "a",
        &["--id", "a", "--cell-size-m", "256"],
    );
    assert_eq!(a.code, 0, "{} {}", a.out, a.err);
    let b = bake(
        &temp,
        "gen.json",
        "b",
        &["--id", "a", "--cell-size-m", "256"],
    );
    assert_eq!(b.code, 0, "{} {}", b.out, b.err);
    assert_eq!(
        files(&temp.path("a")),
        files(&temp.path("b")),
        "two bakes of one script"
    );
    // Into the project itself: nothing to write.
    let again = bake(&temp, "gen.json", "a", &["--cell-size-m", "256"]);
    assert_eq!(again.code, 0, "{} {}", again.out, again.err);
    let answer: Value = serde_json::from_str(again.out.lines().next().unwrap()).unwrap();
    assert_eq!(
        (answer["unchanged"].clone(), answer["written"].clone()),
        (json!(true), json!([])),
        "{answer}"
    );
    assert_eq!(files(&temp.path("a")), files(&temp.path("b")));
}

#[test]
fn a_changed_script_changes_only_the_cells_it_touches_and_a_hand_edited_layer_survives() {
    let temp = Temp::new("bake-change");
    temp.write("gen.json", &script(400));
    let first = bake(
        &temp,
        "gen.json",
        "world",
        &["--id", "world", "--cell-size-m", "256"],
    );
    assert_eq!(first.code, 0, "{} {}", first.out, first.err);
    // A layer edited by hand.
    let root = temp.0.display().to_string();
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
    server.ok("project.open", json!({"path": "world"}));
    server.ok(
        "entity.add",
        json!({"name": "Hand made", "layer": "hand", "position": [12.0, 0.0, 12.0],
               "components": [{"type": "shape", "kind": "box", "size": [1, 1, 1], "color": [1, 0, 0]}]}),
    );
    assert_eq!(
        server.ok("project.save", json!({}))["unchanged"],
        json!(false)
    );
    assert!(server.finish(), "the server wrote the hand layer");
    let before = files(&temp.path("world"));
    let hand: Vec<&PathBuf> = before
        .keys()
        .filter(|p| p.to_string_lossy().contains("/layers/hand/"))
        .collect();
    assert_eq!(hand.len(), 1, "{hand:?}");
    let hand_text = before[hand[0]].clone();
    // The script moves one box from the cell (1, 0) into the cell (3, 3).
    let mut edited = steps(400);
    assert_eq!(edited[11][1]["name"], json!("Box 10"));
    edited[11][1]["position"] = json!([900.0, 0.0, 900.0]);
    let moved = serde_json::to_string_pretty(&edited).unwrap();
    temp.write("gen.json", &moved);
    let second = bake(&temp, "gen.json", "world", &["--cell-size-m", "256"]);
    assert_eq!(second.code, 0, "{} {}", second.out, second.err);
    let answer: Value = serde_json::from_str(second.out.lines().next().unwrap()).unwrap();
    let written: Vec<String> = answer["written"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        (1..=2).contains(&written.len()),
        "only the cells the box left and entered: {written:?}"
    );
    assert!(
        written
            .iter()
            .all(|w| w.starts_with("worlds/main/layers/main/")),
        "{written:?}"
    );
    let after = files(&temp.path("world"));
    assert_eq!(after[hand[0]], hand_text, "the hand layer is as it was");
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>()
    );
    let changed: Vec<&PathBuf> = after.keys().filter(|k| after[*k] != before[*k]).collect();
    assert!(changed.len() <= 2, "{changed:?}");
    let (old_cell, new_cell) = (
        PathBuf::from("worlds/main/layers/main/1_0.json"),
        PathBuf::from("worlds/main/layers/main/3_3.json"),
    );
    let named = "\"name\":\"Box 10\",";
    assert!(
        before[&old_cell].contains(named) && !after[&old_cell].contains(named),
        "the box left its cell"
    );
    assert!(after[&new_cell].contains(named), "and reached the other");
    // The script's four hundred boxes and the hand-made one.
    let open = run_in(&temp.0, &["project", "open", "world"]);
    let opened: Value = serde_json::from_str(open.out.lines().next().unwrap()).unwrap();
    assert_eq!(opened["entities"], json!(401), "{opened}");
}

#[test]
fn a_bad_line_is_reported_with_its_line_and_nothing_is_written() {
    let temp = Temp::new("bake-bad");
    let good = script(3);
    let text = format!(
        "{},\n  [\"entity.add\", {{\"name\": \"No position\", \"layer\": \"main\"}}]\n]\n",
        good.trim_end_matches("\n]\n")
    );
    // Line 1 `[`, line 2 terrain.set, lines 3 to 5 boxes, line 6 the bad step.
    temp.write("gen.json", &text);
    let ran = bake(&temp, "gen.json", "world", &["--id", "world"]);
    assert_eq!(ran.code, 1, "{} {}", ran.out, ran.err);
    assert!(
        ran.err.contains("gen.json:6: step 4 `entity.add`") && ran.err.contains("position"),
        "{}",
        ran.err
    );
    assert!(ran.out.is_empty(), "{}", ran.out);
    assert!(
        !temp.path("world").exists()
            || std::fs::read_dir(temp.path("world"))
                .unwrap()
                .next()
                .is_none(),
        "nothing is written for a script that fails"
    );
    // Into an existing project: the same sentence, and the files as they were.
    temp.write("gen.json", &script(3));
    let first = bake(&temp, "gen.json", "world", &["--id", "world"]);
    assert_eq!(first.code, 0, "{} {}", first.out, first.err);
    let before = files(&temp.path("world"));
    temp.write("bad.json", &text);
    let again = bake(&temp, "bad.json", "world", &[]);
    assert_eq!(again.code, 1, "{} {}", again.out, again.err);
    assert!(
        again.err.contains("bad.json:6: step 4 `entity.add`"),
        "{}",
        again.err
    );
    assert_eq!(files(&temp.path("world")), before);
    // A script that is not a list, and one step that is not a step.
    temp.write("notalist.json", "{\"command\": \"world.inspect\"}");
    let ran = bake(&temp, "notalist.json", "other", &["--id", "other"]);
    assert_eq!(ran.code, 1, "{}", ran.err);
    assert!(
        ran.err.contains("not a JSON list") || ran.err.contains("step 0"),
        "{}",
        ran.err
    );
    temp.write("notastep.json", "[\n  [\"world.inspect\"],\n  7\n]\n");
    let ran = bake(&temp, "notastep.json", "other", &["--id", "other"]);
    assert_eq!(ran.code, 1, "{}", ran.err);
    assert!(
        ran.err.contains("notastep.json: line 3, step 1"),
        "{}",
        ran.err
    );
    // A trailing comma is refused, as `run --script` refuses it.
    temp.write("comma.json", "[[\"world.inspect\"],]\n");
    let ran = bake(&temp, "comma.json", "other", &["--id", "other"]);
    assert_eq!(ran.code, 1, "{}", ran.err);
    assert!(ran.err.contains("comma before"), "{}", ran.err);
}

#[test]
fn a_hundred_thousand_entities_bake_in_one_go() {
    let temp = Temp::new("bake-100k");
    let text = script(100_000);
    temp.write("gen.json", &text);
    let started = Instant::now();
    let ran = bake(
        &temp,
        "gen.json",
        "world",
        &["--id", "world", "--cell-size-m", "512"],
    );
    let seconds = started.elapsed().as_secs_f64();
    println!("bake of 100 000 entities: {seconds:.1} s");
    assert_eq!(ran.code, 0, "{} {}", ran.out, ran.err);
    let answer: Value = serde_json::from_str(ran.out.lines().next().unwrap()).unwrap();
    assert_eq!(answer["entities"], json!(100_000), "{answer}");
    let open = run_in(&temp.0, &["project", "open", "world"]);
    let opened: Value = serde_json::from_str(open.out.lines().next().unwrap()).unwrap();
    assert_eq!(opened["entities"], json!(100_000), "{opened}");
    assert!(
        seconds < 600.0,
        "a hundred thousand entities in {seconds:.0} s is too slow"
    );
    // Baked again into the same project: every entity removed and added again, and the
    // files as they were.
    let started = Instant::now();
    let again = bake(&temp, "gen.json", "world", &["--cell-size-m", "512"]);
    let again_s = started.elapsed().as_secs_f64();
    println!("bake of 100 000 entities again: {again_s:.1} s");
    assert_eq!(again.code, 0, "{} {}", again.out, again.err);
    let answer: Value = serde_json::from_str(again.out.lines().next().unwrap()).unwrap();
    assert_eq!(answer["unchanged"], json!(true), "{answer}");
    assert!(again_s < 600.0, "the re-bake took {again_s:.0} s");
}
