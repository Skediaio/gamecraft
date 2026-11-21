//! A rifle in a character's hands through the commands: `sim.input`'s `fire_mode`,
//! `switch_mode`, `reload`, `ads` and `hold_breath`, and what `sim.state`, `sim.step` and
//! `entity.get` show of the selector, the magazine, recoil and sway.

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

/// A flat range with a rifleman holding a 5.56 service rifle (safe, semi, auto at 650 a minute,
/// 30 rounds, recoil and sway) and an unarmed walker.
fn range() -> Session {
    let mut s = Session::new(World::new("Range", 3));
    let mut run = |c: &str, p: Value| s.run(c, &p).unwrap();
    run(
        "terrain.set",
        json!({"kind": "flat", "size_m": 256, "cell_m": 4}),
    );
    run("environment.set", json!({"wind_speed_mps": 0}));
    run(
        "entity.add",
        json!({"name": "Rifleman", "at": [0, 0], "components": [
            {"type": "character"},
            {"type": "launcher", "muzzle_velocity_mps": 870, "mass_kg": 0.004,
             "dispersion_mrad": 0.5, "rate_s": 0.1, "magazine": 30, "reload_s": 2.5,
             "reload_empty_s": 3.5, "modes": ["safe", "semi", "auto"], "cyclic_rpm": 650,
             "max_flight_s": 1,
             "recoil": {"kick_up_mrad": 1.5, "kick_side_mrad": 0.6, "recovery_s": 0.25},
             "sway": {"standing_mrad": 2}}]}),
    );
    run(
        "entity.add",
        json!({"name": "Walker", "at": [5, 5], "components": [{"type": "character"}]}),
    );
    run("sim.start", json!({}));
    s
}

fn rifleman(s: &mut Session) -> Value {
    let state = s.run("sim.state", &json!({})).unwrap();
    state["characters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Rifleman")
        .unwrap()
        .clone()
}

#[test]
fn the_selector_the_trigger_and_a_reload_through_sim_input() {
    let mut s = range();
    let me = rifleman(&mut s);
    assert_eq!(me["magazine"]["mode"], "safe");
    assert_eq!(me["handling"]["breath"], "normal");
    // On safe the trigger does nothing.
    s.run("sim.input", &json!({"entity": "Rifleman", "fire": true}))
        .unwrap();
    s.run("sim.step", &json!({"ticks": 30})).unwrap();
    assert_eq!(rifleman(&mut s)["magazine"]["shots"], 0);
    // The selector, pressed: semi. Held, one shot.
    let stored = s
        .run(
            "sim.input",
            &json!({"entity": "Rifleman", "fire": false, "switch_mode": true}),
        )
        .unwrap();
    assert_eq!(stored["switch_mode"], true);
    s.run("sim.step", &json!({"ticks": 1})).unwrap();
    assert_eq!(rifleman(&mut s)["magazine"]["mode"], "semi");
    s.run(
        "sim.input",
        &json!({"entity": "Rifleman", "fire": true, "switch_mode": false}),
    )
    .unwrap();
    s.run("sim.step", &json!({"ticks": 60})).unwrap();
    let me = rifleman(&mut s);
    assert_eq!(me["magazine"]["shots"], 1);
    assert!(me["handling"]["recoil_mrad"][1].as_f64().unwrap() >= 0.0);
    // fire_mode sets it at once: auto, a second held at 650 a minute.
    s.run(
        "sim.input",
        &json!({"entity": "Rifleman", "fire": true, "fire_mode": "auto"}),
    )
    .unwrap();
    assert_eq!(rifleman(&mut s)["magazine"]["mode"], "auto");
    let answer = s.run("sim.step", &json!({"ticks": 60})).unwrap();
    let me = answer["characters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Rifleman")
        .unwrap()
        .clone();
    let shots = me["magazine"]["shots"].as_u64().unwrap();
    assert!((10..=12).contains(&shots), "{shots} shots");
    // The climb shows in the handling, and what it adds to the aim in degrees.
    let up = me["handling"]["recoil_mrad"][1].as_f64().unwrap();
    assert!(up > 0.0, "auto fire kicks the aim up: {up}");
    let pitch = me["handling"]["aim_offset_deg"][1].as_f64().unwrap();
    let sway = me["handling"]["sway_mrad"][1].as_f64().unwrap();
    assert!(((up + sway) * 1e-3_f64.to_degrees() - pitch).abs() < 1e-9);
    // A reload asked for.
    s.run(
        "sim.input",
        &json!({"entity": "Rifleman", "fire": false, "reload": true, "ads": true}),
    )
    .unwrap();
    s.run("sim.step", &json!({"ticks": 1})).unwrap();
    let me = rifleman(&mut s);
    assert_eq!(me["magazine"]["reloading"], true);
    assert_eq!(me["handling"]["ads"], true);
    let got = s.run("entity.get", &json!({"id": "Rifleman"})).unwrap();
    assert_eq!(got["handling"]["ads"], true);
    assert_eq!(got["magazine"]["mode"], "auto");
    assert_eq!(got["input"]["reload"], true);
    // A character without a launcher has no handling.
    let state = s.run("sim.state", &json!({})).unwrap();
    let walker = state["characters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Walker")
        .unwrap();
    assert_eq!(walker["handling"], Value::Null);
}

#[test]
fn a_bad_fire_mode_names_the_parameter_and_changes_nothing() {
    let mut s = range();
    refuse(
        &mut s,
        "sim.input",
        json!({"entity": "Rifleman", "fire_mode": "full"}),
        &["fire_mode", "safe", "burst"],
    );
    refuse(
        &mut s,
        "sim.input",
        json!({"entity": "Rifleman", "fire_mode": "burst"}),
        &["fire_mode", "safe, semi, auto"],
    );
    refuse(
        &mut s,
        "sim.input",
        json!({"entity": "Walker", "fire_mode": "semi"}),
        &["fire_mode", "launcher"],
    );
    refuse(
        &mut s,
        "sim.input",
        json!({"entity": "Rifleman", "ads": 1}),
        &["ads"],
    );
    refuse(
        &mut s,
        "sim.input",
        json!({"entity": "Rifleman", "reload": "yes"}),
        &["reload"],
    );
}

#[test]
fn the_component_list_shows_the_handling_fields_with_their_defaults() {
    let mut s = Session::new(World::new("w", 1));
    let list = s.run("component.list", &json!({})).unwrap();
    let kinds = list["components"].as_array().unwrap();
    let field = |kind: &str, name: &str| -> Value {
        let k = kinds.iter().find(|k| k["type"] == kind).unwrap();
        k["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == name)
            .unwrap_or_else(|| panic!("{kind}.{name}"))
            .clone()
    };
    assert_eq!(field("launcher", "modes")["default"], json!(["auto"]));
    assert_eq!(field("launcher", "burst_count")["default"], 3);
    assert_eq!(field("launcher", "cyclic_rpm")["default"], Value::Null);
    assert_eq!(field("launcher", "recoil")["default"]["recovery_s"], 0.25);
    assert_eq!(field("launcher", "sway")["default"]["standing_mrad"], 0.0);
    assert_eq!(field("launcher", "ads_sway_factor")["default"], 0.5);
    assert_eq!(field("held_model", "ads_fov_factor")["default"], 0.85);
    assert_eq!(field("held_model", "ads_offset_m")["default"], Value::Null);
    assert_eq!(field("held_model", "reticle")["default"], Value::Null);
    assert_eq!(field("held_model", "attachments")["default"], json!([]));
}

#[test]
fn a_held_model_takes_an_optic_and_a_reticle_and_refuses_bad_ones() {
    let mut s = Session::new(World::new("Range", 3));
    let held = |extra: Value| {
        let mut h = json!({"type": "held_model", "asset": "models/rifle.glb",
            "attachments": [{"asset": "models/optic.glb", "offset_m": [0, 0.03, -0.05]}],
            "reticle": {"color_srgb": [1, 0.1, 0.05], "radius_mrad": 0.6, "brightness": 1}});
        if let (Value::Object(h), Value::Object(extra)) = (&mut h, extra) {
            h.extend(extra);
        }
        json!({"name": "Rifleman", "position": [0, 0, 0], "components": [{"type": "character"}, h]})
    };
    s.run("entity.add", &held(json!({}))).unwrap();
    let got = s.run("entity.get", &json!({"id": "Rifleman"})).unwrap();
    let text = got.to_string();
    assert!(
        text.contains("models/optic.glb") && text.contains("radius_mrad"),
        "{text}"
    );
    refuse(
        &mut s,
        "entity.add",
        held(json!({"reticle": {"radius_mrad": 50}})),
        &["held_model.reticle.radius_mrad"],
    );
    refuse(
        &mut s,
        "entity.add",
        held(json!({"attachments": [{"asset": "optic.obj"}]})),
        &["held_model.attachments[0].asset"],
    );
    refuse(
        &mut s,
        "entity.add",
        held(json!({"reticle": {"glow": 1}})),
        &["glow"],
    );
}
