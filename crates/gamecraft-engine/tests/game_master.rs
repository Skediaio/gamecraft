//! The game master's tools: `gm.overview`, `gm.sight`, `gm.order`, and `message.show` to a
//! tag with `sim.state {for}` per player.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::refuse;
use gamecraft_engine::Session;
use gamecraft_world::World;
use serde_json::{Value, json};

const PLAYER: u64 = 1;
const GUARD: u64 = 2;
const PATROL: u64 = 3;
const CRATE: u64 = 4;

/// A flat field: the player (blue) at [0, 20] facing north, a red guard at [0, -20] facing
/// south (no launcher: it aims but never fires), a red patrol at [30, -20] that sees only 5 m,
/// a low crate at [3, 0] and an objective.
fn field() -> Session {
    let mut s = Session::new(World::new("Field", 3));
    let mut run = |c: &str, p: Value| s.run(c, &p).unwrap_or_else(|e| panic!("{c} {p}: {e}"));
    run(
        "terrain.set",
        json!({"kind": "flat", "size_m": 256, "cell_m": 4}),
    );
    run(
        "environment.set",
        json!({"time_of_day_h": 12, "mist": 0, "rain": 0}),
    );
    run(
        "entity.add",
        json!({"name": "Player", "at": [0, 20], "components": [
            {"type": "character"}, {"type": "health", "max": 100, "current": 100},
            {"type": "launcher"}, {"type": "tags", "tags": ["player", "team:blue"]}]}),
    );
    run(
        "entity.add",
        json!({"name": "Guard", "at": [0, -20], "rotation_y_deg": 180, "components": [
            {"type": "character"}, {"type": "health"},
            {"type": "behaviour", "mode": "guard", "team": "red", "reaction_s": 0.5},
            {"type": "tags", "tags": ["enemy", "squad", "team:red"]}]}),
    );
    run(
        "entity.add",
        json!({"name": "Patrol", "at": [30, -20], "components": [
            {"type": "character"}, {"type": "health"},
            {"type": "behaviour", "mode": "patrol", "team": "red", "view_range_m": 5,
             "route": [[30, -20], [30, -40]]},
            {"type": "tags", "tags": ["enemy", "squad", "team:red"]}]}),
    );
    run(
        "entity.add",
        json!({"name": "Crate", "at": [3, 0], "components": [
            {"type": "shape", "kind": "box", "size": [1, 0.5, 1], "solid": true}]}),
    );
    run(
        "objective.add",
        json!({"id": "hold", "title": "Hold the field"}),
    );
    s
}

fn ok(s: &mut Session, c: &str, p: Value) -> Value {
    s.run(c, &p).unwrap_or_else(|e| panic!("{c} {p}: {e}"))
}

fn character(overview: &Value, id: u64) -> Value {
    overview["characters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("no character {id} in {overview}"))
}

fn ids(list: &Value) -> Vec<u64> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_u64().unwrap_or_else(|| v.as_u64().unwrap()))
        .collect()
}

fn is_cm(v: f64) -> bool {
    ((v * 100.0).round() / 100.0 - v).abs() < 1e-12
}

#[test]
fn overview_while_editing_lists_the_document() {
    let mut s = field();
    let o = ok(&mut s, "gm.overview", json!({}));
    assert_eq!(o["playing"], false, "{o}");
    assert_eq!(o["tick"], Value::Null);
    assert_eq!(ids(&o["characters"]), vec![PLAYER, GUARD, PATROL]);
    let guard = character(&o, GUARD);
    assert_eq!(guard["team"], "red");
    assert_eq!(
        guard["state"],
        Value::Null,
        "no state while editing: {guard}"
    );
    assert_eq!(guard["sees"], Value::Null);
    assert_eq!(o["objectives"][0]["id"], "hold");
    assert_eq!(o["events"], json!([]));
}

#[test]
fn overview_tells_what_is_going_on() {
    let mut s = field();
    ok(&mut s, "sim.start", json!({}));
    ok(&mut s, "sim.step", json!({"ticks": 3}));
    ok(
        &mut s,
        "message.show",
        json!({"text": "Hold on.", "to": "Player", "seconds": 10}),
    );
    let o = ok(&mut s, "gm.overview", json!({}));
    assert_eq!(o["playing"], true);
    assert_eq!(o["tick"], 3);
    assert!(o["time_s"].as_f64().unwrap() > 0.0);
    assert_eq!(ids(&o["characters"]), vec![PLAYER, GUARD, PATROL]);
    let guard = character(&o, GUARD);
    assert_eq!(guard["name"], "Guard");
    assert_eq!(guard["team"], "red");
    assert_eq!(guard["state"], "alert", "the guard saw the player: {guard}");
    assert_eq!(guard["target"], PLAYER);
    assert_eq!(guard["sees"], json!([PLAYER]));
    assert_eq!(guard["down"], false);
    assert_eq!(guard["taken_over"], false);
    let player = character(&o, PLAYER);
    assert_eq!(player["team"], "blue");
    assert_eq!(player["health"], 100.0);
    assert_eq!(player["state"], Value::Null);
    assert_eq!(player["sees"], json!([GUARD, PATROL]));
    for v in player["position"].as_array().unwrap() {
        assert!(
            is_cm(v.as_f64().unwrap()),
            "rounded to centimetres: {player}"
        );
    }
    let patrol = character(&o, PATROL);
    assert_eq!(patrol["state"], "patrol");
    assert_eq!(patrol["sees"], json!([]));
    assert_eq!(
        o["teams"],
        json!([{"team": "blue", "alive": 1, "down": 0}, {"team": "red", "alive": 2, "down": 0}])
    );
    assert_eq!(
        o["objectives"],
        json!([{"id": "hold", "title": "Hold the field", "status": "open", "hidden": false}])
    );
    assert_eq!(o["messages"][0]["text"], "Hold on.");
    assert_eq!(o["messages"][0]["to"], PLAYER);
    assert_eq!(o["weather"]["mist"], 0.0);
    assert!(
        o["weather"]["visibility_m"].is_null()
            || o["weather"]["visibility_m"].as_f64().unwrap() > 1000.0
    );
    assert!(o["weather"]["time_of_day_h"].is_number());
    assert_eq!(o["events"][0]["event"], "start", "{o}");
    // An agent takes the guard over.
    ok(
        &mut s,
        "sim.input",
        json!({"entity": "Guard", "forward": 0}),
    );
    let o = ok(&mut s, "gm.overview", json!({}));
    assert_eq!(character(&o, GUARD)["taken_over"], true);
}

#[test]
fn overview_filters_by_area_and_team() {
    let mut s = field();
    ok(&mut s, "sim.start", json!({}));
    let o = ok(
        &mut s,
        "gm.overview",
        json!({"near": [0, -20], "radius_m": 15}),
    );
    assert_eq!(ids(&o["characters"]), vec![GUARD]);
    // The teams always count everyone.
    assert_eq!(o["teams"][1]["alive"], 2);
    let o = ok(&mut s, "gm.overview", json!({"team": "blue"}));
    assert_eq!(ids(&o["characters"]), vec![PLAYER]);
    let o = ok(&mut s, "gm.overview", json!({"near": [0, -20]}));
    assert_eq!(
        ids(&o["characters"]),
        vec![PLAYER, GUARD, PATROL],
        "default radius 100 m"
    );
    refuse(&mut s, "gm.overview", json!({"radius_m": 10}), &["`near`"]);
    refuse(
        &mut s,
        "gm.overview",
        json!({"near": [0, 0], "radius_m": -1}),
        &["`radius_m`"],
    );
    refuse(
        &mut s,
        "gm.overview",
        json!({"team": "green"}),
        &["green", "blue", "red"],
    );
}

#[test]
fn overview_keeps_the_last_fifty_events_with_names() {
    let mut s = field();
    // 60 small zones around the player: 60 `enter` events in the first tick.
    for i in 0..60 {
        ok(
            &mut s,
            "entity.add",
            json!({"name": format!("Zone {i}"), "at": [0, 20], "components": [
                {"type": "zone", "shape": "circle", "radius_m": 2, "height_m": 3}]}),
        );
    }
    ok(&mut s, "sim.start", json!({}));
    ok(&mut s, "sim.step", json!({"ticks": 2}));
    let o = ok(&mut s, "gm.overview", json!({}));
    let events = o["events"].as_array().unwrap();
    assert_eq!(events.len(), 50, "{o}");
    let last = events.last().unwrap();
    assert_eq!(last["event"], "enter");
    assert_eq!(last["who"], PLAYER);
    assert!(last["tick"].is_u64());
    let zone = last["zone"].as_u64().unwrap();
    assert_eq!(o["names"][zone.to_string()], "Zone 59");
    assert_eq!(o["names"][PLAYER.to_string()], "Player");
}

#[test]
fn overview_counts_the_down() {
    let mut s = field();
    ok(&mut s, "sim.start", json!({}));
    ok(
        &mut s,
        "entity.set_component",
        json!({"id": "Patrol", "component": {"type": "health", "max": 100, "current": 0}}),
    );
    let o = ok(&mut s, "gm.overview", json!({}));
    assert_eq!(character(&o, PATROL)["down"], true);
    assert_eq!(o["teams"][1], json!({"team": "red", "alive": 1, "down": 1}));
}

#[test]
fn sight_uses_the_simulations_perception() {
    let mut s = field();
    ok(&mut s, "sim.start", json!({}));
    let seen = ok(&mut s, "gm.sight", json!({"from": "Guard"}));
    assert_eq!(seen["from"], GUARD);
    let chars = seen["characters"].as_array().unwrap();
    assert_eq!(chars.len(), 1, "{seen}");
    assert_eq!(chars[0]["id"], PLAYER);
    assert_eq!(chars[0]["name"], "Player");
    assert_eq!(chars[0]["enemy"], true);
    let d = chars[0]["distance_m"].as_f64().unwrap();
    assert!((d - 40.0).abs() < 0.5, "{d}");
    assert!(is_cm(d));
    let things = ids(&seen["entities"]);
    assert!(things.contains(&CRATE), "{seen}");
    assert!(seen["range_m"].as_f64().unwrap() > 200.0);
    // A wall between them: the guard sees the wall, not the player.
    ok(
        &mut s,
        "entity.add",
        json!({"name": "Wall", "at": [0, 0], "components": [
            {"type": "shape", "kind": "box", "size": [8, 4, 1], "solid": true}]}),
    );
    let seen = ok(&mut s, "gm.sight", json!({"from": "Guard"}));
    assert_eq!(seen["characters"], json!([]), "{seen}");
    assert!(ids(&seen["entities"]).contains(&5), "the wall: {seen}");
    // Fog of 1 hides everything beyond 120 m and the night shortens the range.
    ok(&mut s, "environment.set", json!({"mist": 1}));
    let seen = ok(&mut s, "gm.sight", json!({"from": "Player"}));
    assert!(seen["range_m"].as_f64().unwrap() <= 120.0);
    refuse(
        &mut s,
        "gm.sight",
        json!({"from": "Crate"}),
        &["Crate", "character"],
    );
    refuse(&mut s, "gm.sight", json!({"from": "Nobody"}), &["Nobody"]);
}

#[test]
fn sight_works_while_editing() {
    let mut s = field();
    let seen = ok(&mut s, "gm.sight", json!({"from": "Player"}));
    assert_eq!(ids(&seen["characters"]), vec![GUARD, PATROL]);
}

fn behaviour(s: &Session, id: u64) -> Value {
    let e = s
        .current_world()
        .entity(gamecraft_world::EntityId(id))
        .unwrap();
    serde_json::to_value(e.behaviour().unwrap()).unwrap()
}

#[test]
fn order_moves_patrols_and_holds_while_playing() {
    let mut s = field();
    ok(&mut s, "sim.start", json!({}));
    let r = ok(
        &mut s,
        "gm.order",
        json!({"entity": "Patrol", "order": "move", "point": [10, -20]}),
    );
    assert_eq!(r["ordered"][0]["id"], PATROL, "{r}");
    assert_eq!(r["ordered"][0]["mode"], "patrol");
    let b = behaviour(&s, PATROL);
    assert_eq!(b["mode"], "patrol");
    assert_eq!(b["route"], json!([[10.0, -20.0]]));
    assert_eq!(b["view_range_m"], 5.0, "the rest of the behaviour is kept");
    ok(&mut s, "sim.step", json!({"seconds": 20}));
    let p = s
        .current_world()
        .entity(gamecraft_world::EntityId(PATROL))
        .unwrap()
        .position;
    let (dx, dz) = (p[0] - 10.0, p[2] + 20.0);
    let d = (dx * dx + dz * dz).sqrt();
    assert!(d < 1.5, "the patrol went to the point: {p:?}");
    // A new patrol route.
    ok(
        &mut s,
        "gm.order",
        json!({"entity": 3, "order": "patrol", "route": [[10, 10], [20, 10]]}),
    );
    assert_eq!(
        behaviour(&s, PATROL)["route"],
        json!([[10.0, 10.0], [20.0, 10.0]])
    );
    // The squad holds, in one command.
    let r = ok(
        &mut s,
        "gm.order",
        json!({"entity": "tag:squad", "order": "hold"}),
    );
    assert_eq!(ids(&r["ordered"]), vec![GUARD, PATROL]);
    assert_eq!(behaviour(&s, GUARD)["mode"], "idle");
    assert_eq!(behaviour(&s, PATROL)["mode"], "idle");
    // Patrol keeps the route it has when none is given; guard goes back to its post.
    ok(
        &mut s,
        "gm.order",
        json!({"entity": "Patrol", "order": "patrol"}),
    );
    assert_eq!(behaviour(&s, PATROL)["mode"], "patrol");
    let r = ok(
        &mut s,
        "gm.order",
        json!({"entity": ["Guard", 3], "order": "guard"}),
    );
    assert_eq!(ids(&r["ordered"]), vec![GUARD, PATROL]);
    // It is recorded and replays to the same hash.
    ok(&mut s, "sim.step", json!({"seconds": 2}));
    let journal = s.journal().unwrap();
    assert!(journal.entries.iter().any(|e| e.command == "gm.order"));
    let replay = journal.replay(None).unwrap();
    assert!(replay.divergence.is_none(), "{:?}", replay.divergence);
}

#[test]
fn order_gives_a_behaviour_to_a_character_without_one() {
    let mut s = field();
    ok(
        &mut s,
        "gm.order",
        json!({"entity": "Player", "order": "move", "point": [0, 0]}),
    );
    assert_eq!(behaviour(&s, PLAYER)["mode"], "patrol");
    assert!(s.can_undo(), "on the document, one undo step");
}

#[test]
fn order_refuses_what_it_cannot_do() {
    let mut s = field();
    ok(&mut s, "sim.start", json!({}));
    refuse(
        &mut s,
        "gm.order",
        json!({"entity": "Patrol", "order": "dance"}),
        &["`order`", "dance"],
    );
    refuse(
        &mut s,
        "gm.order",
        json!({"entity": "Patrol", "order": "move"}),
        &["`point`"],
    );
    refuse(
        &mut s,
        "gm.order",
        json!({"entity": "Patrol", "order": "move", "route": [[1, 1]]}),
        &["`route`", "move"],
    );
    refuse(
        &mut s,
        "gm.order",
        json!({"entity": "Patrol", "order": "patrol", "point": [1, 1]}),
        &["`point`", "patrol"],
    );
    refuse(
        &mut s,
        "gm.order",
        json!({"entity": "Guard", "order": "patrol"}),
        &["`route`", "Guard"],
    );
    refuse(
        &mut s,
        "gm.order",
        json!({"entity": "Patrol", "order": "hold", "point": [1, 1]}),
        &["`point`"],
    );
    refuse(
        &mut s,
        "gm.order",
        json!({"entity": "Crate", "order": "hold"}),
        &["Crate", "character"],
    );
    refuse(
        &mut s,
        "gm.order",
        json!({"entity": "tag:nobody", "order": "hold"}),
        &["nobody"],
    );
    refuse(
        &mut s,
        "gm.order",
        json!({"entity": [], "order": "hold"}),
        &["`entity`"],
    );
    refuse(
        &mut s,
        "gm.order",
        json!({"entity": true, "order": "hold"}),
        &["`entity`"],
    );
    refuse(
        &mut s,
        "gm.order",
        json!({"entity": "Patrol", "order": "patrol", "route": []}),
        &["`route`"],
    );
}

#[test]
fn messages_to_a_tag_reach_those_players() {
    let mut s = field();
    ok(&mut s, "sim.start", json!({}));
    let m = ok(
        &mut s,
        "message.show",
        json!({"text": "HQ: hold the line.", "to": "tag:player"}),
    );
    assert_eq!(m["to"], "tag:player");
    assert_eq!(m["recipients"], json!([PLAYER]));
    ok(&mut s, "message.show", json!({"text": "All hear this."}));
    ok(
        &mut s,
        "message.show",
        json!({"text": "For the guard.", "to": "Guard"}),
    );
    let all = ok(&mut s, "sim.state", json!({}));
    assert_eq!(all["messages"].as_array().unwrap().len(), 3);
    let player = ok(&mut s, "sim.state", json!({"for": "Player"}));
    let texts: Vec<&str> = player["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, vec!["HQ: hold the line.", "All hear this."]);
    let guard = ok(&mut s, "sim.state", json!({"for": "Guard"}));
    assert_eq!(guard["messages"].as_array().unwrap().len(), 2);
    refuse(
        &mut s,
        "message.show",
        json!({"text": "x", "to": "tag:nobody"}),
        &["nobody"],
    );
    refuse(
        &mut s,
        "message.show",
        json!({"text": "x", "to": "tag:"}),
        &["tag"],
    );
    refuse(&mut s, "sim.state", json!({"for": "Nobody"}), &["Nobody"]);
}
