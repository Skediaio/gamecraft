//! The P2 miniature: a small valley, a player with a rifle, three enemies with behaviours
//! patrolling, a wall to hide behind, and a trigger that completes an objective when all three
//! are down. The player is scripted through inputs only (`face`, `pitch_deg` from a simple
//! ballistic aim helper computed here, `fire`), exactly as an agent or a keyboard plays.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use gamecraft_engine::Session;
use gamecraft_math::{Portable, rad_to_deg, yaw_forward};
use gamecraft_sim::ballistics::{Flight, advance, air_at};
use gamecraft_sim::combat::aim_direction;
use gamecraft_world::{Environment, ObjectiveStatus, World};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// The rifle: a 7.62 mm 168 grain bullet with a G7 BC of 0.223 lb/in2 (199.6 kg/m2).
const MUZZLE_VELOCITY_MPS: f64 = 800.0;
const BALLISTIC_COEFFICIENT: f64 = 199.6;
const EYE_HEIGHT_M: f64 = 1.65;
const CHEST_HEIGHT_M: f64 = 1.8 * 0.65;
const PLAYER: u64 = 1;

/// The valley: gentle noise terrain, the player (team blue) at the south behind a wall, three
/// red enemies patrolling `range_m` to the north, and the objective with its triggers.
fn skirmish(range_m: f64, wind_mps: f64) -> Session {
    let mut s = Session::new(World::new("Skirmish", 21));
    let mut run = |c: &str, p: Value| s.run(c, &p).unwrap_or_else(|e| panic!("{c} {p}: {e}"));
    run(
        "terrain.set",
        json!({"kind": "noise", "size_m": 512, "cell_m": 4, "seed": 5, "relief_m": 3,
               "feature_m": 300, "octaves": 2}),
    );
    run(
        "environment.set",
        json!({"time_of_day_h": 11, "wind_speed_mps": wind_mps, "wind_direction_deg": 270,
               "mist": 0, "rain": 0}),
    );
    let z0 = range_m / 2.0;
    run(
        "entity.add",
        json!({"name": "Player", "at": [0, z0], "components": [
            {"type": "character"},
            {"type": "health", "max": 2000, "current": 2000},
            {"type": "launcher", "muzzle_velocity_mps": MUZZLE_VELOCITY_MPS, "mass_kg": 0.0109,
             "ballistic_coefficient": BALLISTIC_COEFFICIENT, "dispersion_mrad": 0.2,
             "rate_s": 0.6, "magazine": 10, "reload_s": 2.5, "damage_per_joule": 0.05,
             "sound_range_m": 800},
            {"type": "tags", "tags": ["player", "team:blue"]}]}),
    );
    // The wall: 12 m wide, 3 m tall, 4 m north of the player.
    run(
        "entity.add",
        json!({"name": "Wall", "at": [0, z0 - 4.0], "components": [
            {"type": "shape", "kind": "box", "size": [12, 3, 1]}]}),
    );
    run("entity.snap_to_ground", json!({"id": "Wall"}));
    for (i, x) in [-25.0, 0.0, 25.0].into_iter().enumerate() {
        let z = -z0;
        run(
            "entity.add",
            json!({"name": format!("Enemy {}", i + 1), "at": [x, z], "rotation_y_deg": 180,
                   "components": [
                {"type": "character"},
                {"type": "health"},
                {"type": "launcher", "muzzle_velocity_mps": 700, "dispersion_mrad": 2,
                 "rate_s": 1.5, "magazine": 5, "reload_s": 4, "damage_per_joule": 0.01},
                {"type": "behaviour", "mode": "patrol", "route": [[x - 6.0, z], [x + 6.0, z]],
                 "team": "red", "view_range_m": 400, "engage_range_m": 350,
                 "aim_error_mrad": 15, "reaction_s": 1.0},
                {"type": "tags", "tags": ["enemy"]}]}),
        );
    }
    run(
        "objective.add",
        json!({"id": "clear", "title": "Down all three enemies"}),
    );
    run(
        "trigger.add",
        json!({"id": "enemy-down", "when": {"event": "down", "target": "tag:enemy"},
               "once": false,
               "do": [{"command": "var.set", "params": {"name": "downed", "add": 1}}]}),
    );
    run(
        "trigger.add",
        json!({"id": "all-down", "when": {"event": "var", "name": "downed"},
               "if": {"var": "downed", "op": ">=", "value": 3},
               "do": [{"command": "objective.set", "params": {"id": "clear", "status": "done"}}]}),
    );
    s
}

fn sq(x: f64) -> f64 {
    x * x
}

/// The aim (yaw, pitch in degrees) from `eye` at `target`: a few rounds of flying a test
/// shot with the engine's own flight function and turning by the miss. Without
/// `correct_wind` the test shot flies in calm air, so only the drop is allowed for.
fn aim(eye: [f64; 3], target: [f64; 3], env: &Environment, correct_wind: bool) -> (f64, f64) {
    let (dx, dy, dz) = (target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]);
    let range = (dx * dx + dz * dz).sqrt();
    let mut yaw = rad_to_deg((-dx).patan2(-dz));
    let mut pitch = rad_to_deg(dy.patan2(range));
    let mut air_env = env.clone();
    if !correct_wind {
        air_env.wind_speed_mps = 0.0;
    }
    let ground = eye[1] - EYE_HEIGHT_M;
    for _ in 0..4 {
        let [ux, uy, uz] = aim_direction(yaw, pitch);
        let mut f = Flight {
            position: eye,
            velocity: [
                ux * MUZZLE_VELOCITY_MPS,
                uy * MUZZLE_VELOCITY_MPS,
                uz * MUZZLE_VELOCITY_MPS,
            ],
        };
        let flat = |p: [f64; 3]| (sq(p[0] - eye[0]) + sq(p[2] - eye[2])).sqrt();
        for _ in 0..100_000 {
            if flat(f.position) >= range {
                break;
            }
            f = advance(
                f,
                BALLISTIC_COEFFICIENT,
                &air_at(&air_env, Some(ground), f.position),
            );
        }
        let p = f.position;
        let fwd = yaw_forward(yaw);
        // Right of the facing: the facing turned clockwise.
        let (rx, rz) = (-fwd.z, fwd.x);
        let side = (p[0] - target[0]) * rx + (p[2] - target[2]) * rz;
        let high = p[1] - target[1];
        pitch -= rad_to_deg(high.patan2(range));
        yaw += rad_to_deg(side.patan2(range));
    }
    (yaw, pitch)
}

/// What a scripted fight did.
#[derive(Debug)]
struct Outcome {
    done: bool,
    hash: String,
    shots: u64,
    enemy_hits: u64,
    player_hits: u64,
    player_heard: u64,
}

/// The scripted player: hidden behind the wall for 3 s, then it steps out to the east and
/// engages the nearest enemy that is up, leading it by its velocity over the flight time.
fn play(mut s: Session, correct_wind: bool, seconds: f64) -> Outcome {
    s.run("sim.start", &json!({})).unwrap();
    let enemies: BTreeSet<u64> = [3, 4, 5].into();
    let mut outcome = Outcome {
        done: false,
        hash: String::new(),
        shots: 0,
        enemy_hits: 0,
        player_hits: 0,
        player_heard: 0,
    };
    let env = s.current_world().environment.clone();
    let ticks_per_look = 3;
    let mut tick = 0;
    let z0 = s
        .current_world()
        .entity(gamecraft_world::EntityId(PLAYER))
        .unwrap()
        .position[2];
    while f64::from(tick) / 60.0 < seconds {
        let state = s.run("sim.state", &json!({})).unwrap();
        let chars = state["characters"].as_array().unwrap();
        let me = chars.iter().find(|c| c["id"] == PLAYER).unwrap();
        let pos: Vec<f64> = me["position"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let input = if tick < 180 {
            json!({"entity": "Player", "fire": false})
        } else if pos[0] < 9.0 {
            // Step out from behind the wall.
            json!({"entity": "Player", "face": [20, z0], "forward": 1, "run": true, "fire": false})
        } else {
            let eye = [pos[0], pos[1] + EYE_HEIGHT_M, pos[2]];
            let target = chars
                .iter()
                .filter(|c| enemies.contains(&c["id"].as_u64().unwrap()))
                .filter(|c| c["health"]["down"] == false)
                .min_by(|a, b| {
                    let d = |c: &Value| {
                        let p = &c["position"];
                        sq(p[0].as_f64().unwrap() - eye[0]) + sq(p[2].as_f64().unwrap() - eye[2])
                    };
                    d(a).total_cmp(&d(b))
                });
            match target {
                Some(t) => {
                    let p = |i: usize| t["position"][i].as_f64().unwrap();
                    let v = |i: usize| t["velocity"][i].as_f64().unwrap();
                    let flat = (sq(p(0) - eye[0]) + sq(p(2) - eye[2])).sqrt();
                    let lead = flat / MUZZLE_VELOCITY_MPS * 1.1;
                    let chest = [
                        p(0) + v(0) * lead,
                        p(1) + CHEST_HEIGHT_M,
                        p(2) + v(2) * lead,
                    ];
                    let (yaw, pitch) = aim(eye, chest, &env, correct_wind);
                    json!({"entity": "Player", "forward": 0, "yaw_deg": yaw, "pitch_deg": pitch,
                           "fire": true})
                }
                None => json!({"entity": "Player", "forward": 0, "fire": false}),
            }
        };
        s.run("sim.input", &input).unwrap();
        let r = s
            .run("sim.step", &json!({"ticks": ticks_per_look}))
            .unwrap();
        tick += ticks_per_look;
        for e in r["events"].as_array().unwrap() {
            let who = e["who"].as_u64();
            let target = e["target"].as_u64();
            match e["event"].as_str().unwrap() {
                "hit" => {
                    let shooter = e["shooter"].as_u64();
                    if shooter == Some(PLAYER) && target.is_some_and(|t| enemies.contains(&t)) {
                        outcome.enemy_hits += 1;
                    }
                    if target == Some(PLAYER) {
                        outcome.player_hits += 1;
                    }
                }
                "heard" if who == Some(PLAYER) && target.is_some_and(|t| enemies.contains(&t)) => {
                    outcome.player_heard += 1;
                }
                _ => {}
            }
        }
        outcome.hash = r["hash"].as_str().unwrap().to_string();
        let world = s.current_world();
        if world.objective("clear").unwrap().status == ObjectiveStatus::Done {
            outcome.done = true;
            break;
        }
    }
    let state = s.run("sim.state", &json!({})).unwrap();
    let me = state["characters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == PLAYER)
        .unwrap()
        .clone();
    outcome.shots = me["magazine"]["shots"].as_u64().unwrap();
    outcome
}

#[test]
fn the_player_downs_three_enemies_who_fire_back_and_the_replay_matches() {
    let first = play(skirmish(160.0, 3.0), true, 60.0);
    println!("{first:?}");
    assert!(first.done, "the objective is done: {first:?}");
    assert!(first.enemy_hits >= 3, "{first:?}");
    assert!(
        first.player_hits > 0 || first.player_heard > 0,
        "the enemies fired back: {first:?}"
    );
    let second = play(skirmish(160.0, 3.0), true, 60.0);
    assert_eq!(first.hash, second.hash, "the same fight twice");
    assert_eq!(first.shots, second.shots);
}

#[test]
fn a_strong_crosswind_without_correction_misses_more() {
    let miss_rate = |o: &Outcome| {
        assert!(o.shots > 0, "{o:?}");
        1.0 - o.enemy_hits as f64 / o.shots as f64
    };
    let corrected = play(skirmish(260.0, 18.0), true, 40.0);
    let uncorrected = play(skirmish(260.0, 18.0), false, 40.0);
    println!("corrected {corrected:?}\nuncorrected {uncorrected:?}");
    assert!(
        miss_rate(&uncorrected) > miss_rate(&corrected),
        "corrected {corrected:?} uncorrected {uncorrected:?}"
    );
}
