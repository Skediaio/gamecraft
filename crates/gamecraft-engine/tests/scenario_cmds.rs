//! The scenario commands: variables, objectives, triggers, messages, and their queries.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, playing, refuse};
use gamecraft_world::{ObjectiveStatus, VarValue};
use serde_json::json;

#[test]
fn var_set_sets_adds_and_reports_the_previous_value() {
    let mut s = example();
    let r = s
        .run("var.set", &json!({"name": "gold", "value": 10}))
        .unwrap();
    assert_eq!(r, json!({"name": "gold", "value": 10.0, "previous": null}));
    let r = s
        .run("var.set", &json!({"name": "gold", "add": 2.5}))
        .unwrap();
    assert_eq!(r["value"], json!(12.5));
    assert_eq!(r["previous"], json!(10.0));
    // A missing variable counts from 0.
    s.run("var.set", &json!({"name": "fresh", "add": -1}))
        .unwrap();
    assert_eq!(s.document().vars["fresh"], VarValue::Number(-1.0));
    s.run("var.set", &json!({"name": "flag", "value": true}))
        .unwrap();
    s.run("var.set", &json!({"name": "who", "value": "red"}))
        .unwrap();
    assert_eq!(s.document().vars["who"], VarValue::Text("red".into()));
}

#[test]
fn var_set_refuses_what_it_cannot_do() {
    let mut s = example();
    refuse(
        &mut s,
        "var.set",
        json!({"name": "x"}),
        &["`value` or `add`"],
    );
    refuse(
        &mut s,
        "var.set",
        json!({"name": "x", "value": 1, "add": 1}),
        &["not both"],
    );
    refuse(
        &mut s,
        "var.set",
        json!({"name": "x", "value": [1]}),
        &["`value`", "number, a boolean or a string"],
    );
    refuse(
        &mut s,
        "var.set",
        json!({"name": "", "value": 1}),
        &["name"],
    );
    s.run("var.set", &json!({"name": "s", "value": "text"}))
        .unwrap();
    refuse(
        &mut s,
        "var.set",
        json!({"name": "s", "add": 1}),
        &["`add`", "\"s\"", "string"],
    );
    s.run("var.set", &json!({"name": "big", "value": 1e308}))
        .unwrap();
    refuse(
        &mut s,
        "var.set",
        json!({"name": "big", "add": 1e308}),
        &["finite"],
    );
}

#[test]
fn var_remove_names_the_variables_there_are() {
    let mut s = example();
    s.run("var.remove", &json!({"name": "score"})).unwrap();
    assert!(!s.document().vars.contains_key("score"));
    s.run("var.set", &json!({"name": "a", "value": 1})).unwrap();
    refuse(&mut s, "var.remove", json!({"name": "b"}), &["\"b\"", "a"]);
}

#[test]
fn objectives_are_added_changed_listed_and_removed() {
    let mut s = example();
    let r = s
        .run(
            "objective.add",
            &json!({"id": "reach-crate", "title": "Reach the crate", "hidden": true}),
        )
        .unwrap();
    assert_eq!(
        r,
        json!({"id": "reach-crate", "title": "Reach the crate", "status": "open", "hidden": true})
    );
    refuse(
        &mut s,
        "objective.add",
        json!({"id": "reach-crate", "title": "Again"}),
        &["reach-crate", "objective.set"],
    );
    refuse(
        &mut s,
        "objective.add",
        json!({"id": "x", "title": "X", "status": "won"}),
        &["`status`", "\"open\""],
    );
    s.run(
        "objective.set",
        &json!({"id": "reach-crate", "status": "done", "hidden": false}),
    )
    .unwrap();
    let o = s.document().objective("reach-crate").unwrap();
    assert_eq!((o.status, o.hidden), (ObjectiveStatus::Done, false));
    assert_eq!(o.title, "Reach the crate");
    refuse(
        &mut s,
        "objective.set",
        json!({"id": "nope", "status": "done"}),
        &["\"nope\"", "find-oak"],
    );
    let list = s.run("objective.list", &json!({})).unwrap();
    assert_eq!(list["objectives"].as_array().unwrap().len(), 2);
    assert_eq!(
        (
            list["open"].clone(),
            list["done"].clone(),
            list["failed"].clone()
        ),
        (json!(1), json!(1), json!(0))
    );
    s.run("objective.remove", &json!({"id": "reach-crate"}))
        .unwrap();
    refuse(
        &mut s,
        "objective.remove",
        json!({"id": "reach-crate"}),
        &["reach-crate"],
    );
}

fn crate_trigger() -> serde_json::Value {
    json!({
        "id": "at-crate",
        "when": {"event": "enter", "zone": "Crate", "who": "tag:player"},
        "if": {"var": "score", "op": "<", "value": 10},
        "do": [
            {"command": "var.set", "params": {"name": "score", "add": 1}},
            {"command": "entity.set_component", "params": {"id": "$who", "component": {"type": "tags", "tags": ["visited"]}}},
            {"command": "message.show", "params": {"text": "Crate!", "to": "$who"}}
        ]
    })
}

#[test]
fn trigger_add_stores_the_trigger_with_its_defaults() {
    let mut s = example();
    let r = s.run("trigger.add", &crate_trigger()).unwrap();
    assert_eq!(r["id"], json!("at-crate"));
    assert_eq!(r["once"], json!(true));
    assert_eq!(r["enabled"], json!(true));
    let t = s.document().trigger("at-crate").unwrap();
    assert_eq!(t.actions.len(), 3);
    assert!(t.condition.is_some());
    refuse(
        &mut s,
        "trigger.add",
        crate_trigger(),
        &["at-crate", "trigger.set"],
    );
}

#[test]
fn trigger_add_checks_every_step_against_its_command() {
    let mut s = example();
    let with_do =
        |steps: serde_json::Value| json!({"id": "t", "when": {"event": "start"}, "do": steps});
    refuse(
        &mut s,
        "trigger.add",
        with_do(
            json!([{"command": "var.set", "params": {"name": "a", "value": 1}}, {"command": "enviroment.set", "params": {}}]),
        ),
        &[
            "trigger \"t\"",
            "step 2",
            "enviroment.set",
            "environment.set",
        ],
    );
    refuse(
        &mut s,
        "trigger.add",
        with_do(json!([{"command": "environment.set", "params": {"mists": 1}}])),
        &[
            "trigger \"t\"",
            "step 1",
            "environment.set",
            "mists",
            "mist",
        ],
    );
    refuse(
        &mut s,
        "trigger.add",
        with_do(json!([{"command": "sim.step", "params": {}}])),
        &["step 1", "sim.step", "world command"],
    );
    refuse(
        &mut s,
        "trigger.add",
        with_do(json!([{"command": "entity.set", "params": {"id": "$who", "name": "x"}}])),
        &["step 1", "$who", "start"],
    );
    refuse(
        &mut s,
        "trigger.add",
        json!({"id": "t", "when": {"event": "enter", "zone": "Crate"},
               "do": [{"command": "var.set", "params": {"name": "$who", "value": 1}}]}),
        &["step 1", "$who", "`name`", "entity"],
    );
    refuse(
        &mut s,
        "trigger.add",
        with_do(json!([{"command": "var.set"}, 3])),
        &["`do[1]`"],
    );
    refuse(
        &mut s,
        "trigger.add",
        json!({"id": "t", "when": {"event": "explode"}, "do": []}),
        &["`when`", "explode"],
    );
    refuse(
        &mut s,
        "trigger.add",
        json!({"id": "t", "when": {"event": "time", "at_s": -5}, "do": []}),
        &["when.at_s", "-5"],
    );
    refuse(
        &mut s,
        "trigger.add",
        json!({"id": "t", "when": {"event": "start"}, "if": {"var": "a", "op": "~", "value": 1}, "do": []}),
        &["`if`", "op"],
    );
    // A trigger may add triggers; their own steps are checked when they are added.
    s.run(
        "trigger.add",
        &with_do(json!([{"command": "trigger.add", "params": {"id": "later", "when": {"event": "start"}, "do": []}}])),
    )
    .unwrap();
}

#[test]
fn trigger_set_changes_only_what_is_given_and_checks_again() {
    let mut s = example();
    s.run("trigger.add", &crate_trigger()).unwrap();
    s.run(
        "trigger.set",
        &json!({"id": "at-crate", "enabled": false, "once": false}),
    )
    .unwrap();
    let t = s.document().trigger("at-crate").unwrap().clone();
    assert!(!t.enabled && !t.once);
    assert_eq!(t.actions.len(), 3);
    // Changing `when` to an event without an actor makes `$who` wrong.
    refuse(
        &mut s,
        "trigger.set",
        json!({"id": "at-crate", "when": {"event": "time", "at_s": 3}}),
        &["at-crate", "$who"],
    );
    refuse(
        &mut s,
        "trigger.set",
        json!({"id": "nope", "enabled": true}),
        &["\"nope\"", "at-crate"],
    );
    s.run("trigger.remove", &json!({"id": "at-crate"})).unwrap();
    assert!(s.document().trigger("at-crate").is_none());
}

#[test]
fn scenario_commands_change_the_running_world_and_not_the_document() {
    let mut s = playing();
    let doc = s.document().clone();
    s.run("var.set", &json!({"name": "score", "value": 5}))
        .unwrap();
    s.run(
        "objective.set",
        &json!({"id": "find-oak", "status": "failed"}),
    )
    .unwrap();
    s.run("trigger.add", &crate_trigger()).unwrap();
    assert_eq!(s.document(), &doc);
    let live = s.current_world();
    assert_eq!(live.vars["score"], VarValue::Number(5.0));
    assert_eq!(
        live.objective("find-oak").unwrap().status,
        ObjectiveStatus::Failed
    );
    assert!(live.trigger("at-crate").is_some());
}

#[test]
fn messages_show_while_playing_and_expire() {
    let mut s = example();
    refuse(
        &mut s,
        "message.show",
        json!({"text": "Hello"}),
        &["sim.start"],
    );
    s.run("sim.start", &json!({})).unwrap();
    refuse(&mut s, "message.show", json!({"text": ""}), &["`text`"]);
    refuse(
        &mut s,
        "message.show",
        json!({"text": "x", "seconds": 0}),
        &["`seconds`"],
    );
    refuse(
        &mut s,
        "message.show",
        json!({"text": "x", "to": "Nobody"}),
        &["Nobody"],
    );
    let r = s
        .run(
            "message.show",
            &json!({"text": "Find the oak", "to": "Scout", "seconds": 1}),
        )
        .unwrap();
    assert_eq!(r["to"], json!(3));
    assert_eq!(r["until_s"], json!(1.0));
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["messages"][0]["text"], json!("Find the oak"));
    s.run("sim.step", &json!({"seconds": 0.5})).unwrap();
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["messages"].as_array().unwrap().len(), 1);
    s.run("sim.step", &json!({"seconds": 0.5})).unwrap();
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["messages"], json!([]), "expired at 1 s");
}

#[test]
fn the_queries_describe_the_scenario() {
    let mut s = example();
    let r = s.run("scenario.inspect", &json!({})).unwrap();
    assert_eq!(r["playing"], json!(false));
    assert_eq!(r["vars"], json!({"score": 0.0}));
    assert_eq!(r["objectives"][0]["id"], json!("find-oak"));
    assert_eq!(r["triggers"][0]["id"], json!("oak-found"));
    assert_eq!(r["triggers"][0]["when"]["event"], json!("var"));
    assert_eq!(r["triggers"][0]["fired"], json!(0));
    assert_eq!(r["messages"], json!([]));
    let r = s.run("world.inspect", &json!({"entities": false})).unwrap();
    assert_eq!(r["vars"], json!({"score": 0.0}));
    assert_eq!(r["objectives"][0]["title"], json!("Find the oak"));
    assert_eq!(r["trigger_count"], json!(1));
}

#[test]
fn entity_find_filters_by_tag() {
    let mut s = example();
    s.run(
        "entity.set_component",
        &json!({"id": "Crate", "component": {"type": "tags", "tags": ["loot", "wood"]}}),
    )
    .unwrap();
    let r = s.run("entity.find", &json!({"tag": "loot"})).unwrap();
    assert_eq!(r["total"], json!(1));
    assert_eq!(r["entities"][0]["name"], json!("Crate"));
    let r = s.run("entity.find", &json!({"tag": "Loot"})).unwrap();
    assert_eq!(r["total"], json!(0), "tags match exactly");
}

#[test]
fn component_list_has_the_scenario_components() {
    let mut s = example();
    let r = s.run("component.list", &json!({})).unwrap();
    let types: Vec<&str> = r["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["type"].as_str().unwrap())
        .collect();
    for t in ["zone", "interactable", "light", "tags"] {
        assert!(types.contains(&t), "{t}");
    }
}
