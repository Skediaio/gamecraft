//! Running triggers: events from the simulation, `start` and `var` events, `once`, the
//! condition, all-or-nothing actions, nesting, and determinism.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use gamecraft_engine::Session;
use gamecraft_world::{ObjectiveStatus, VarValue, World};
use serde_json::{Value, json};

/// A flat world with a player (tagged `player`) at the origin and a gate zone 10 m north.
fn gate_world() -> Session {
    let mut s = Session::new(World::new("Gate", 1));
    let mut run = |c: &str, p: Value| s.run(c, &p).unwrap();
    run(
        "terrain.set",
        json!({"kind": "flat", "size_m": 128, "cell_m": 4}),
    );
    run(
        "entity.add",
        json!({"name": "Player", "at": [0, 0], "components": [
            {"type": "character"}, {"type": "tags", "tags": ["player"]}]}),
    );
    run(
        "entity.add",
        json!({"name": "Gate", "at": [0, -10], "components": [{"type": "zone", "radius_m": 2}]}),
    );
    run("var.set", json!({"name": "passed", "value": 0}));
    s
}

fn walk_north(s: &mut Session) {
    s.run(
        "sim.input",
        &json!({"entity": "Player", "forward": 1, "yaw_deg": 0}),
    )
    .unwrap();
}

fn var(s: &Session, name: &str) -> VarValue {
    s.current_world().vars[name].clone()
}

#[test]
fn a_start_trigger_fires_on_sim_start() {
    let mut s = gate_world();
    s.run(
        "trigger.add",
        &json!({"id": "hello", "when": {"event": "start"},
                "do": [{"command": "message.show", "params": {"text": "Go north"}}]}),
    )
    .unwrap();
    let r = s.run("sim.start", &json!({})).unwrap();
    assert_eq!(
        r["fired"],
        json!([{"trigger": "hello", "tick": 0, "event": "start", "ok": true}])
    );
    assert_eq!(r["events"], json!([{"tick": 0, "event": "start"}]));
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["messages"][0]["text"], json!("Go north"));
    // `once`: disabled in the running world, untouched in the document.
    assert!(!s.current_world().trigger("hello").unwrap().enabled);
    assert!(s.document().trigger("hello").unwrap().enabled);
}

#[test]
fn enter_and_leave_run_their_triggers_with_who_and_target() {
    let mut s = gate_world();
    s.run(
        "trigger.add",
        &json!({"id": "in", "when": {"event": "enter", "zone": "Gate", "who": "tag:player"},
                "do": [{"command": "var.set", "params": {"name": "passed", "add": 1}},
                       {"command": "entity.set", "params": {"id": "$target", "name": "Gate (passed by $who)"}},
                       {"command": "entity.set_component", "params": {"id": "$who", "component": {"type": "tags", "tags": ["player", "visitor"]}}}]}),
    )
    .unwrap();
    s.run(
        "trigger.add",
        &json!({"id": "out", "when": {"event": "leave", "zone": 2}, "once": false,
                "do": [{"command": "var.set", "params": {"name": "left", "value": true}}]}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    walk_north(&mut s);
    let r = s.run("sim.step", &json!({"seconds": 10})).unwrap();
    let events: Vec<&str> = r["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert_eq!(events, ["enter", "var", "leave", "var"], "{}", r["events"]);
    let fired: Vec<&str> = r["fired"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["trigger"].as_str().unwrap())
        .collect();
    assert_eq!(fired, ["in", "out"]);
    assert_eq!(var(&s, "passed"), VarValue::Number(1.0));
    assert_eq!(var(&s, "left"), VarValue::Bool(true));
    let w = s.current_world();
    let player = w.entities().find(|e| e.name == "Player").unwrap();
    assert!(player.has_tag("visitor"));
    // `$target` and `$who` are replaced only where a whole parameter is the placeholder.
    assert!(w.entities().any(|e| e.name == "Gate (passed by $who)"));
}

#[test]
fn the_who_filter_and_the_condition_must_hold() {
    let mut s = gate_world();
    s.run(
        "trigger.add",
        &json!({"id": "red-only", "when": {"event": "enter", "zone": "Gate", "who": "tag:red"},
                "do": [{"command": "var.set", "params": {"name": "red", "value": 1}}]}),
    )
    .unwrap();
    s.run(
        "trigger.add",
        &json!({"id": "rich-only", "when": {"event": "enter", "zone": "Gate"},
                "if": {"var": "passed", "op": ">", "value": 100},
                "do": [{"command": "var.set", "params": {"name": "rich", "value": 1}}]}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    walk_north(&mut s);
    let r = s.run("sim.step", &json!({"seconds": 10})).unwrap();
    assert_eq!(r["fired"], json!([]));
    assert!(!s.current_world().vars.contains_key("red"));
    assert!(!s.current_world().vars.contains_key("rich"));
}

#[test]
fn a_failing_trigger_changes_nothing_and_is_reported() {
    let mut s = gate_world();
    s.run(
        "trigger.add",
        &json!({"id": "broken", "when": {"event": "time", "at_s": 0.5},
                "do": [{"command": "var.set", "params": {"name": "passed", "value": 99}},
                       {"command": "environment.set", "params": {"mist": 5}}]}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    let r = s.run("sim.step", &json!({"seconds": 1})).unwrap();
    let fired = &r["fired"][0];
    assert_eq!(fired["trigger"], json!("broken"));
    assert_eq!(fired["tick"], json!(30));
    assert_eq!(fired["ok"], json!(false));
    let error = fired["error"].as_str().unwrap();
    assert!(
        error.contains("step 2") && error.contains("environment.set") && error.contains("mist"),
        "{error}"
    );
    assert_eq!(var(&s, "passed"), VarValue::Number(0.0), "all or nothing");
    assert!(
        s.current_world().trigger("broken").unwrap().enabled,
        "a failed trigger is not used up"
    );
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["trigger_errors"][0]["trigger"], json!("broken"));
    assert_eq!(state["trigger_errors"][0]["tick"], json!(30));
}

#[test]
fn a_reference_that_resolves_to_nothing_is_reported_when_the_trigger_runs() {
    let mut s = gate_world();
    s.run(
        "trigger.add",
        &json!({"id": "ghost", "when": {"event": "enter", "zone": "Nowhere"}, "do": []}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    walk_north(&mut s);
    let r = s.run("sim.step", &json!({"seconds": 10})).unwrap();
    let error = r["fired"][0]["error"].as_str().unwrap();
    assert!(
        error.contains("Nowhere") && error.contains("zone"),
        "{error}"
    );
}

#[test]
fn var_triggers_run_in_the_same_pass_and_their_depth_is_limited() {
    let mut s = gate_world();
    s.run(
        "trigger.add",
        &json!({"id": "loop", "when": {"event": "var", "name": "n"}, "once": false,
                "do": [{"command": "var.set", "params": {"name": "n", "add": 1}}]}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    // A variable changed by a command between steps fires at the start of the next step.
    s.run("var.set", &json!({"name": "n", "value": 0})).unwrap();
    let r = s.run("sim.step", &json!({"ticks": 1})).unwrap();
    let fired = r["fired"].as_array().unwrap();
    assert_eq!(fired.len(), 33, "32 levels, then the refusal");
    assert!(fired[..32].iter().all(|f| f["ok"] == json!(true)));
    let error = fired[32]["error"].as_str().unwrap();
    assert!(
        error.contains("32") && error.contains("loop > loop"),
        "{error}"
    );
    assert_eq!(fired[0]["tick"], json!(0));
    assert_eq!(var(&s, "n"), VarValue::Number(32.0));
}

#[test]
fn an_objective_trigger_marks_done_and_once_stops_it() {
    let mut s = gate_world();
    s.run(
        "objective.add",
        &json!({"id": "gate", "title": "Pass the gate"}),
    )
    .unwrap();
    s.run(
        "trigger.add",
        &json!({"id": "win", "when": {"event": "var", "name": "passed"},
                "if": {"var": "passed", "op": ">=", "value": 1},
                "do": [{"command": "objective.set", "params": {"id": "gate", "status": "done"}}]}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    s.run("var.set", &json!({"name": "passed", "value": 1}))
        .unwrap();
    s.run("objective.set", &json!({"id": "gate", "status": "open"}))
        .unwrap();
    s.run("sim.step", &json!({})).unwrap();
    assert_eq!(
        s.current_world().objective("gate").unwrap().status,
        ObjectiveStatus::Done
    );
    s.run("objective.set", &json!({"id": "gate", "status": "open"}))
        .unwrap();
    s.run("var.set", &json!({"name": "passed", "value": 2}))
        .unwrap();
    let r = s.run("sim.step", &json!({})).unwrap();
    assert_eq!(r["fired"], json!([]), "once");
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["objectives"][0]["status"], json!("open"));
}

/// One scripted run: the hash, events and fired triggers after every tick.
fn scripted() -> Vec<(Value, Value, Value)> {
    let mut s = gate_world();
    s.run(
        "trigger.add",
        &json!({"id": "in", "when": {"event": "enter", "zone": "Gate"}, "once": false,
                "do": [{"command": "var.set", "params": {"name": "passed", "add": 1}}]}),
    )
    .unwrap();
    s.run(
        "trigger.add",
        &json!({"id": "turn", "when": {"event": "leave", "zone": "Gate"}, "once": false,
                "do": [{"command": "environment.set", "params": {"time_of_day_h": 20}}]}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    let mut out = Vec::new();
    for tick in 0..900 {
        if tick % 150 == 0 {
            let yaw = if (tick / 150) % 2 == 0 { 0 } else { 180 };
            s.run(
                "sim.input",
                &json!({"entity": "Player", "forward": 1, "yaw_deg": yaw, "run": true}),
            )
            .unwrap();
        }
        let r = s.run("sim.step", &json!({})).unwrap();
        out.push((r["hash"].clone(), r["events"].clone(), r["fired"].clone()));
    }
    out
}

#[test]
fn the_same_scenario_and_inputs_give_the_same_hash_events_and_fired_every_tick() {
    let one = scripted();
    let two = scripted();
    assert_eq!(one, two);
    let fired: usize = one
        .iter()
        .map(|(_, _, f)| f.as_array().unwrap().len())
        .sum();
    assert!(fired >= 4, "the scenario did something: {fired}");
}

fn messages(s: &mut Session) -> Vec<String> {
    let state = s.run("sim.state", &json!({})).unwrap();
    state["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["text"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn messages_of_triggers_show_variables_and_the_names_of_the_event() {
    let mut s = gate_world();
    s.run(
        "trigger.add",
        &json!({"id": "in", "when": {"event": "enter", "zone": "Gate", "who": "tag:player"},
                "do": [{"command": "var.set", "params": {"name": "passed", "add": 1}},
                       {"command": "message.show", "params": {
                           "text": "{who} passed {target}: {var:passed} so far. {var:nope} {nothing}"}}]}),
    )
    .unwrap();
    s.run("var.set", &json!({"name": "colour", "value": "red"}))
        .unwrap();
    s.run(
        "trigger.add",
        &json!({"id": "count", "when": {"event": "var", "name": "passed"}, "once": false,
                "do": [{"command": "message.show", "params": {
                    "text": "{var:colour} {who}/{var:passed}"}}]}),
    )
    .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    walk_north(&mut s);
    s.run("sim.step", &json!({"seconds": 6})).unwrap();
    // Values at the moment the step runs: after the var.set before it. Text values are written
    // without quotes; unknown names stay as written.
    assert_eq!(
        messages(&mut s),
        vec![
            "Player passed Gate: 1 so far. {var:nope} {nothing}".to_string(),
            "red {who}/1".to_string()
        ]
    );
    // The unknown names were reported once each, naming the trigger; the triggers still ran.
    s.run("var.set", &json!({"name": "passed", "add": 1}))
        .unwrap();
    s.run("sim.step", &json!({"ticks": 1})).unwrap();
    assert!(messages(&mut s).contains(&"red {who}/2".to_string()));
    let state = s.run("sim.state", &json!({})).unwrap();
    let errors = state["trigger_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(errors[0]["trigger"], json!("in"));
    assert!(
        errors[0]["error"].as_str().unwrap().contains("{var:nope}"),
        "{errors:?}"
    );
    assert_eq!(errors[1]["trigger"], json!("count"));
    assert!(
        errors[1]["error"].as_str().unwrap().contains("{who}"),
        "{errors:?}"
    );
    let scenario = s.run("scenario.inspect", &json!({})).unwrap();
    let count = scenario["triggers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "count")
        .unwrap()
        .clone();
    assert_eq!(count["fired"], json!(2), "{count}");
}

#[test]
fn objective_conditions_gate_triggers_alone_and_inside_any() {
    let mut s = gate_world();
    s.run(
        "objective.add",
        &json!({"id": "goal", "title": "Pass the gate"}),
    )
    .unwrap();
    s.run(
        "trigger.add",
        &json!({"id": "while-open", "when": {"event": "var", "name": "passed"}, "once": false,
                "if": {"objective": "goal", "status": "open"},
                "do": [{"command": "var.set", "params": {"name": "open_hits", "add": 1}}]}),
    )
    .unwrap();
    s.run(
        "trigger.add",
        &json!({"id": "after", "when": {"event": "var", "name": "passed"}, "once": false,
                "if": {"any": [{"objective": "goal", "status": "done"},
                               {"objective": "goal", "status": "failed"}]},
                "do": [{"command": "var.set", "params": {"name": "late_hits", "add": 1}}]}),
    )
    .unwrap();
    s.run("var.set", &json!({"name": "open_hits", "value": 0}))
        .unwrap();
    s.run("var.set", &json!({"name": "late_hits", "value": 0}))
        .unwrap();
    s.run("sim.start", &json!({})).unwrap();
    s.run("var.set", &json!({"name": "passed", "add": 1}))
        .unwrap();
    s.run("sim.step", &json!({"ticks": 1})).unwrap();
    s.run("objective.set", &json!({"id": "goal", "status": "done"}))
        .unwrap();
    s.run("var.set", &json!({"name": "passed", "add": 1}))
        .unwrap();
    s.run("sim.step", &json!({"ticks": 1})).unwrap();
    assert_eq!(var(&s, "open_hits"), VarValue::Number(1.0));
    assert_eq!(var(&s, "late_hits"), VarValue::Number(1.0));
    assert_eq!(
        s.current_world().objectives[0].status,
        ObjectiveStatus::Done
    );
    // A bad objective condition is refused when the trigger is added.
    let e = s
        .run(
            "trigger.add",
            &json!({"id": "bad", "when": {"event": "start"},
                    "if": {"objective": "goal", "status": "won"}, "do": []}),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("open, done, failed"), "{e}");
}
