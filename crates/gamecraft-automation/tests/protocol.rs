//! The JSON-lines protocol: the envelope, the command list, batches and the history.
//!
//! Adapted from VisionCraft's `crates/visioncraft-automation/tests/protocol.rs` (MIT).

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use gamecraft_automation::{MAX_BATCH_STEPS, MAX_REQUEST_BYTES, rpc};
use serde_json::{Value, json};

const HOST_COMMANDS: [&str; 14] = [
    "edit.redo",
    "edit.undo",
    "file.new",
    "file.open",
    "file.save",
    "journal.save",
    "pack.add_dependency",
    "project.export",
    "project.info",
    "project.new",
    "project.open",
    "project.query",
    "project.save",
    "view.render",
];

#[test]
fn a_command_that_succeeds_has_a_time() {
    let mut h = host();
    let reply = execute(&mut h, "world.set", json!({"name": "Green valley"}));
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["result"], Value::Null);
    assert!(reply["ms"].is_number(), "{reply}");
    assert_eq!(h.session().document().name, "Green valley");
    assert!(h.is_dirty());
}

#[test]
fn a_failed_command_starts_with_its_id_and_has_a_time() {
    let mut h = host();
    let reply = execute(&mut h, "entity.add", json!({"name": "Rock"}));
    assert_eq!(reply["ok"], false);
    assert!(reply["ms"].is_number(), "{reply}");
    let error = reply["error"].as_str().unwrap();
    assert!(error.starts_with("entity.add: "), "{error}");
    let error = err(&mut h, "entity.ad", json!({}));
    assert!(error.contains("did you mean `entity.add`"), "{error}");
    // A host command is one of the candidates too.
    let error = err(&mut h, "view.rendr", json!({}));
    assert!(error.contains("did you mean `view.render`"), "{error}");
    let error = err(&mut h, "zzzzzzzz", json!({}));
    assert!(error.contains("engine.commands"), "{error}");
}

#[test]
fn the_envelope_is_checked() {
    let mut h = host();
    let reply = ask(&mut h, json!({"id": 7, "method": "nothing"}));
    assert_eq!(
        (reply["id"].clone(), reply["ok"].clone()),
        (json!(7), json!(false))
    );
    let error = reply["error"].as_str().unwrap();
    assert!(error.contains("unknown method `nothing`"), "{error}");
    assert!(error.contains("engine.execute"), "{error}");

    let reply = ask(&mut h, json!({"id": 8, "method": "methods", "extra": 1}));
    assert_eq!(reply["id"], 8);
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("unknown request field `extra`")
    );

    let line = rpc::handle_line(&mut h, "{\"id\": 1, \"method\": ").unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["id"], Value::Null);
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().unwrap().contains("not valid JSON"));

    let reply = ask(
        &mut h,
        json!({"id": 9, "method": "engine.execute", "params": [1]}),
    );
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("must be a JSON object"),
        "{reply}"
    );
    let reply = ask(
        &mut h,
        json!({"id": 10, "method": "engine.execute", "params": {"command": "world.set", "params": "x"}}),
    );
    assert!(
        reply["error"].as_str().unwrap().contains("`params`"),
        "{reply}"
    );
    let reply = ask(
        &mut h,
        json!({"id": 11, "method": "engine.execute", "params": {}}),
    );
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("missing `command`"),
        "{reply}"
    );
    let reply = ask(&mut h, json!({"id": 12, "method": 5}));
    assert!(
        reply["error"].as_str().unwrap().contains("`method`"),
        "{reply}"
    );
    let reply = ask(&mut h, json!([1, 2]));
    assert!(
        reply["error"].as_str().unwrap().contains("JSON object"),
        "{reply}"
    );

    let long = format!(
        "{{\"id\": 1, \"method\": \"methods\", \"params\": {{\"x\": \"{}\"}}}}",
        "a".repeat(MAX_REQUEST_BYTES)
    );
    let reply: Value = serde_json::from_str(&rpc::handle_line(&mut h, &long).unwrap()).unwrap();
    assert_eq!(reply["id"], Value::Null);
    assert!(
        reply["error"].as_str().unwrap().contains("limit"),
        "{reply}"
    );

    assert_eq!(rpc::handle_line(&mut h, "   "), None);
}

#[test]
fn no_line_stops_the_server() {
    let mut h = host();
    let long = "x".repeat(MAX_REQUEST_BYTES + 10);
    let input = [
        r#"{"id": 1, "method": "methods"}"#,
        "not json",
        "",
        r#"{"id": 2, "method": "nope"}"#,
        long.as_str(),
        r#"{"id": 3, "method": "engine.execute", "params": {"command": "world.inspect", "params": {"entities": false}}}"#,
        r#"{"id": 4, "extra": true}"#,
        "[]",
        r#"{"id": 5, "method": "engine.execute", "params": {"command": "sim.step"}}"#,
        r#"{"id": 6, "method": "engine.execute", "params": {"command": "world.set", "params": {"name": "Still here"}}}"#,
    ]
    .join("\n");
    let replies = serve_rpc(&mut h, &input);
    assert_eq!(replies.len(), 9, "{replies:#?}");
    let last: Value = serde_json::from_str(replies.last().unwrap()).unwrap();
    assert_eq!(last["ok"], true, "{last}");
    assert_eq!(h.session().document().name, "Still here");
    let ok: Vec<bool> = replies
        .iter()
        .map(|r| serde_json::from_str::<Value>(r).unwrap()["ok"] == true)
        .collect();
    assert_eq!(
        ok,
        [true, false, false, false, true, false, false, false, true]
    );
}

#[test]
fn methods_lists_the_methods_and_the_limits() {
    let mut h = host();
    let reply = ask(&mut h, json!({"id": 1, "method": "methods", "params": {}}));
    let names: Vec<&str> = reply["result"]["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["engine.execute", "engine.commands", "batch", "methods"]
    );
    assert_eq!(reply["result"]["limits"]["batchSteps"], MAX_BATCH_STEPS);
    let reply = ask(
        &mut h,
        json!({"id": 1, "method": "methods", "params": {"x": 1}}),
    );
    assert_eq!(reply["ok"], false);
}

fn commands(h: &mut Host, params: Value) -> Vec<Value> {
    let reply = ask(
        h,
        json!({"id": 1, "method": "engine.commands", "params": params}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    reply["result"]["commands"].as_array().unwrap().clone()
}

use gamecraft_automation::Host;

#[test]
fn every_command_is_listed_sorted_with_its_parts() {
    let mut h = host();
    let list = commands(&mut h, json!({}));
    let ids: Vec<String> = list
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    for spec in h.session().specs() {
        assert!(ids.contains(&spec.name), "{} is not listed", spec.name);
    }
    for id in HOST_COMMANDS {
        assert!(ids.iter().any(|i| i == id), "{id} is not listed");
    }
    assert_eq!(ids.len(), h.session().specs().len() + HOST_COMMANDS.len());
    for c in &list {
        for key in [
            "id",
            "label",
            "params",
            "summary",
            "parameters",
            "openParams",
            "result",
            "example",
            "undoable",
            "scope",
            "enabled",
            "disabledReason",
        ] {
            assert!(c.get(key).is_some(), "{} lacks {key}", c["id"]);
        }
        assert!(c["example"].is_object(), "{}: {}", c["id"], c["example"]);
        assert!(c["params"].as_str().unwrap().starts_with('{'));
        let id = c["id"].as_str().unwrap();
        let scope = c["scope"].as_str().unwrap();
        let want = if HOST_COMMANDS.contains(&id) {
            "host"
        } else {
            h.session().spec(id).unwrap().scope.name()
        };
        assert_eq!(scope, want, "{id}");
        for p in c["parameters"].as_array().unwrap() {
            for key in [
                "name", "type", "unit", "required", "default", "allowed", "doc",
            ] {
                assert!(p.get(key).is_some(), "{id}: a parameter lacks {key}");
            }
        }
    }
    let by_id = |id: &str| list.iter().find(|c| c["id"] == id).unwrap().clone();
    assert_eq!(by_id("entity.add")["scope"], "world");
    assert_eq!(by_id("world.inspect")["scope"], "query");
    assert_eq!(by_id("sim.step")["scope"], "play");
    // Nothing to undo yet, and no renderer.
    let undo = by_id("edit.undo");
    assert_eq!(undo["enabled"], false);
    assert!(
        undo["disabledReason"]
            .as_str()
            .unwrap()
            .contains("nothing to undo")
    );
    assert_eq!(by_id("view.render")["enabled"], false);
    assert_eq!(by_id("sim.step")["enabled"], false);
    assert_eq!(by_id("sim.start")["enabled"], true);
    assert_eq!(by_id("entity.add")["enabled"], true);
    assert_eq!(by_id("entity.add")["disabledReason"], Value::Null);
}

#[test]
fn every_listed_example_runs() {
    let (mut h, _) = host_with_pictures(host());
    let list = commands(&mut h, json!({}));
    let mut ran = 0;
    for c in list {
        let id = c["id"].as_str().unwrap();
        // Files need roots (tests/files.rs, tests/journal.rs and tests/projects.rs run them).
        if id.starts_with("file.")
            || id.starts_with("edit.")
            || id.starts_with("project.")
            || id.starts_with("pack.")
            || id == "journal.save"
        {
            continue;
        }
        let (mut fresh, _) = host_with_pictures(host());
        // Play commands other than sim.start need a running simulation.
        if (id.starts_with("sim.") && id != "sim.start" && id != "sim.state")
            || id == "message.show"
        {
            ok(&mut fresh, "sim.start", json!({}));
        }
        ok(&mut fresh, id, c["example"].clone());
        ran += 1;
    }
    assert!(ran > 15, "{ran}");
}

#[test]
fn brief_filter_and_id_narrow_the_list() {
    let mut h = host();
    let brief = commands(&mut h, json!({"brief": true}));
    for c in &brief {
        let mut keys: Vec<&String> = c.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["enabled", "id", "params", "summary"], "{c}");
        assert!(!c["summary"].as_str().unwrap().contains(". "), "{c}");
    }
    let entity = commands(&mut h, json!({"filter": "ENTITY"}));
    assert!(entity.len() >= 6);
    assert!(entity.iter().all(|c| {
        let id = c["id"].as_str().unwrap();
        let label = c["label"].as_str().unwrap().to_lowercase();
        id.contains("entity") || label.contains("entity")
    }));
    let one = commands(&mut h, json!({"id": "sim.step"}));
    assert_eq!(one.len(), 1);
    assert_eq!(one[0]["id"], "sim.step");
    let reply = ask(
        &mut h,
        json!({"id": 1, "method": "engine.commands", "params": {"id": "sim.stp"}}),
    );
    assert_eq!(reply["ok"], false);
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("did you mean `sim.step`"),
        "{reply}"
    );
    let reply = ask(
        &mut h,
        json!({"id": 1, "method": "engine.commands", "params": {"brief": 1}}),
    );
    assert!(
        reply["error"].as_str().unwrap().contains("`brief`"),
        "{reply}"
    );
    let reply = ask(
        &mut h,
        json!({"id": 1, "method": "engine.commands", "params": {"fliter": "x"}}),
    );
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("unknown parameter `fliter`"),
        "{reply}"
    );
}

#[test]
fn command_docs_matches_the_listing() {
    let h = host();
    let docs = h.command_docs();
    let ids: Vec<&str> = docs.iter().map(|d| d.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted);
    let render = docs.iter().find(|d| d.id == "view.render").unwrap();
    assert_eq!(render.scope, "host");
    assert!(!render.undoable);
    let names: Vec<&str> = render.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["path", "width", "height", "camera", "follow", "time_s"]
    );
}

fn add(name: &str, x: f64) -> Value {
    json!({"command": "entity.add", "params": {"name": name, "position": [x, 0, 0]}})
}

fn batch(h: &mut Host, params: Value) -> Value {
    let reply = ask(h, json!({"id": 1, "method": "batch", "params": params}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(reply.get("ms").is_none(), "{reply}");
    reply["result"].clone()
}

#[test]
fn a_batch_runs_in_order_and_stops_at_the_first_error() {
    let mut h = empty_host();
    let r = batch(
        &mut h,
        json!({"intent": "three steps", "steps": [add("A", 0.0), add("B", 1.0), {"command": "world.inspect", "params": {"limit": 5}, "intent": "look"}]}),
    );
    assert_eq!(
        (r["completed"].clone(), r["failed"].clone()),
        (json!(3), json!(0))
    );
    let results = r["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["result"]["id"], 1);
    assert_eq!(results[1]["result"]["id"], 2);
    assert_eq!(results[2]["result"]["entity_count"], 2);
    assert!(r["ms"].is_number());

    let bad = json!({"command": "entity.remove", "params": {"id": 99}});
    let r = batch(
        &mut h,
        json!({"steps": [add("C", 2.0), bad.clone(), add("D", 3.0)]}),
    );
    assert_eq!(
        (r["completed"].clone(), r["failed"].clone()),
        (json!(1), json!(1))
    );
    assert_eq!(r["results"].as_array().unwrap().len(), 2);
    assert_eq!(r["results"][1]["ok"], false);
    assert!(
        r["results"][1]["error"]
            .as_str()
            .unwrap()
            .starts_with("entity.remove")
    );
    assert_eq!(h.session().document().entity_count(), 3);
    // Not a transaction: each step is its own undo step.
    ok(&mut h, "edit.undo", json!({}));
    assert_eq!(h.session().document().entity_count(), 2);

    let r = batch(
        &mut h,
        json!({"stopOnError": false, "steps": [add("E", 4.0), bad.clone(), add("F", 5.0)]}),
    );
    assert_eq!(
        (r["completed"].clone(), r["failed"].clone()),
        (json!(2), json!(1))
    );
    assert_eq!(h.session().document().entity_count(), 4);

    let r = batch(
        &mut h,
        json!({"terse": true, "stopOnError": false, "steps": [add("G", 6.0), bad, add("H", 7.0)]}),
    );
    assert!(r.get("results").is_none());
    assert_eq!(r["errors"].as_array().unwrap().len(), 1);
    assert_eq!(r["errors"][0]["step"], 1);
    assert_eq!(r["last"]["step"], 2);
    assert!(r["last"]["result"]["id"].is_number());
}

#[test]
fn a_batch_is_checked_before_anything_runs() {
    let mut h = empty_host();
    let too_many: Vec<Value> = (0..=MAX_BATCH_STEPS).map(|i| add("X", i as f64)).collect();
    let reply = ask(
        &mut h,
        json!({"id": 1, "method": "batch", "params": {"steps": too_many}}),
    );
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().unwrap().contains("256"), "{reply}");
    for steps in [
        json!([add("A", 0.0), 5]),
        json!([add("A", 0.0), {"command": "entity.add", "parms": {}}]),
        json!([add("A", 0.0), {"params": {}}]),
        json!([add("A", 0.0), {"command": "world.inspect", "intent": 3}]),
    ] {
        let reply = ask(
            &mut h,
            json!({"id": 1, "method": "batch", "params": {"steps": steps}}),
        );
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(
            reply["error"].as_str().unwrap().contains("steps[1]"),
            "{reply}"
        );
    }
    let reply = ask(&mut h, json!({"id": 1, "method": "batch", "params": {}}));
    assert!(
        reply["error"].as_str().unwrap().contains("missing `steps`"),
        "{reply}"
    );
    let reply = ask(
        &mut h,
        json!({"id": 1, "method": "batch", "params": {"steps": [], "intent": ["no"]}}),
    );
    assert!(
        reply["error"].as_str().unwrap().contains("`intent`"),
        "{reply}"
    );
    assert_eq!(h.session().document().entity_count(), 0);
}

#[test]
fn an_atomic_batch_is_all_or_nothing() {
    let mut h = empty_host();
    ok(
        &mut h,
        "entity.add",
        json!({"name": "First", "position": [0, 0, 0]}),
    );
    let before = h.session().document().clone();
    let revision = h.session().revision();

    let r = batch(
        &mut h,
        json!({"atomic": true, "steps": [add("A", 1.0), add("B", 2.0), {"command": "entity.remove", "params": {"id": 99}}, add("C", 3.0)]}),
    );
    assert_eq!(r["rolledBack"], true);
    assert_eq!(r["failed"], 1);
    assert_eq!(r["completed"], 2);
    assert_eq!(h.session().document(), &before);
    assert_eq!(h.session().revision(), revision);
    // The history is as before: one undo step, the first entity.
    ok(&mut h, "edit.undo", json!({}));
    assert_eq!(h.session().document().entity_count(), 0);
    assert!(!h.session().can_undo());
    ok(&mut h, "edit.redo", json!({}));

    let r = batch(
        &mut h,
        json!({"atomic": true, "terse": true, "steps": [add("A", 1.0), add("B", 2.0), add("C", 3.0)]}),
    );
    assert_eq!(
        (r["rolledBack"].clone(), r["completed"].clone()),
        (json!(false), json!(3))
    );
    assert_eq!(h.session().document().entity_count(), 4);
    ok(&mut h, "edit.undo", json!({}));
    assert_eq!(h.session().document(), &before);
}

#[test]
fn some_commands_cannot_be_part_of_an_atomic_batch() {
    let mut h = empty_host();
    for forbidden in ["file.new", "file.open", "edit.undo", "edit.redo"] {
        let r = batch(
            &mut h,
            json!({"atomic": true, "steps": [add("A", 1.0), {"command": forbidden}]}),
        );
        assert_eq!(r["rolledBack"], true, "{forbidden}");
        assert_eq!(r["completed"], 0);
        let error = r["results"][0]["error"].as_str().unwrap();
        assert!(error.starts_with(forbidden), "{error}");
        assert!(error.contains("atomic"), "{error}");
        assert_eq!(h.session().document().entity_count(), 0);
    }
}

#[test]
fn undo_and_redo_answer_with_the_history() {
    let mut h = empty_host();
    let error = err(&mut h, "edit.undo", json!({}));
    assert!(
        error.starts_with("edit.undo: ") && error.contains("nothing to undo"),
        "{error}"
    );
    let error = err(&mut h, "edit.redo", json!({}));
    assert!(error.contains("nothing to redo"), "{error}");
    ok(&mut h, "entity.add", json!({"position": [0, 0, 0]}));
    assert_eq!(
        ok(&mut h, "edit.undo", json!({})),
        json!({"can_undo": false, "can_redo": true})
    );
    assert_eq!(
        ok(&mut h, "edit.redo", json!({})),
        json!({"can_undo": true, "can_redo": false})
    );
    let error = err(&mut h, "edit.undo", json!({"steps": 2}));
    assert!(error.contains("unknown parameter `steps`"), "{error}");
    ok(&mut h, "sim.start", json!({}));
    let error = err(&mut h, "edit.undo", json!({}));
    assert!(error.contains("sim.stop"), "{error}");
}

#[test]
fn intent_is_accepted_and_checked() {
    let mut h = empty_host();
    let reply = ask(
        &mut h,
        json!({"id": 1, "method": "engine.execute", "params": {"command": "world.set", "params": {"seed": 3}, "intent": "a new seed"}}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let reply = ask(
        &mut h,
        json!({"id": 1, "method": "engine.execute", "params": {"command": "world.set", "params": {"seed": 4}, "intent": 5}}),
    );
    assert_eq!(reply["ok"], false);
    assert!(
        reply["error"].as_str().unwrap().contains("`intent`"),
        "{reply}"
    );
    assert_eq!(h.session().document().seed, 3);
}

#[test]
fn execute_runs_both_kinds_of_command() {
    let mut h = empty_host();
    let r = h
        .execute("entity.add", &json!({"name": "A", "position": [0, 0, 0]}))
        .unwrap();
    assert_eq!(r["id"], 1);
    assert_eq!(
        h.execute("edit.undo", &Value::Null).unwrap()["can_redo"],
        true
    );
    let e = h.execute("entity.add", &json!(3)).unwrap_err();
    assert!(e.starts_with("entity.add"), "{e}");
    let e = h.execute("edit.undo", &json!([1])).unwrap_err();
    assert!(e.starts_with("edit.undo"), "{e}");
}
