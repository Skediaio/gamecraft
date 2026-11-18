//! GPU-free tests: input to input records, the fixed step, interpolation, the camera rig, the
//! notices, the arguments and the game loop on the example world.

use crate::args::{self, Args};
use crate::clock::{FixedStep, MAX_TICKS_PER_FRAME};
use crate::game::Game;
use crate::input::{Action, Controls, Key};
use crate::notices::Notices;
use crate::pose::{self, Pose, Poses};
use crate::rig::{self, ViewMode};
use gamecraft_engine::example_world;
use gamecraft_math::{DVec3, yaw_forward};
use gamecraft_render::Quality;
use gamecraft_sim::DT;
use gamecraft_world::{
    Character, Component, EntityDraft, EntityId, Objective, ObjectiveStatus, Terrain,
    TerrainSource, World,
};
use serde_json::json;

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

// ---- input ----

#[test]
fn keys_become_an_input_record() {
    let mut c = Controls::new(30.0);
    let r = c.record();
    assert_eq!((r.forward, r.right, r.yaw_deg), (0.0, 0.0, 30.0));
    assert!(!r.run && !r.jump && !r.interact);

    c.key(Key::Forward, true);
    assert_eq!(c.record().forward, 1.0);
    c.key(Key::Back, true);
    assert_eq!(c.record().forward, 0.0, "forward and back cancel");
    c.key(Key::Forward, false);
    assert_eq!(c.record().forward, -1.0);
    c.key(Key::Back, false);
    c.key(Key::Left, true);
    assert_eq!(c.record().right, -1.0);
    c.key(Key::Right, true);
    assert_eq!(c.record().right, 0.0);
    c.key(Key::Left, false);
    assert_eq!(c.record().right, 1.0);
    c.key(Key::Run, true);
    assert!(c.record().run);
    c.key(Key::Interact, true);
    assert!(c.record().interact);

    let params = c.record().params(EntityId(3));
    assert_eq!(
        params,
        json!({"entity": 3, "forward": 0.0, "right": 1.0, "yaw_deg": 30.0, "run": true,
               "jump": false, "interact": true})
    );

    c.release_all();
    let r = c.record();
    assert!(r.forward == 0.0 && r.right == 0.0 && !r.run && !r.interact);
}

#[test]
fn a_tap_between_two_ticks_still_reaches_one_tick() {
    let mut c = Controls::new(0.0);
    c.key(Key::Jump, true);
    c.key(Key::Jump, false);
    c.key(Key::Interact, true);
    c.key(Key::Interact, false);
    let r = c.record();
    assert!(r.jump && r.interact, "held until a tick has seen them");
    c.ticked();
    let r = c.record();
    assert!(!r.jump && !r.interact);
    // Held keys stay after a tick.
    c.key(Key::Jump, true);
    c.ticked();
    assert!(c.record().jump);
}

#[test]
fn the_window_keys_are_actions_on_press_only() {
    let mut c = Controls::new(0.0);
    assert_eq!(c.key(Key::ToggleView, true), Some(Action::ToggleView));
    assert_eq!(c.key(Key::ToggleView, false), None);
    assert_eq!(c.key(Key::Restart, true), Some(Action::Restart));
    assert_eq!(c.key(Key::Pause, true), Some(Action::Pause));
    assert_eq!(c.key(Key::Release, true), Some(Action::Release));
    assert_eq!(c.key(Key::Forward, true), None);
}

#[test]
fn the_mouse_turns_and_tilts_within_limits() {
    let mut c = Controls::new(0.0);
    c.look(100.0, 0.0);
    // Moving the mouse right turns right: clockwise seen from above, so the yaw falls.
    assert!(
        c.yaw_deg() > 180.0 && c.yaw_deg() < 360.0,
        "{}",
        c.yaw_deg()
    );
    c.look(-100.0, 0.0);
    assert!(close(c.yaw_deg(), 0.0, 1e-9) || close(c.yaw_deg(), 360.0, 1e-9));
    c.look(0.0, -100.0);
    assert!(c.pitch_deg() > 0.0, "moving the mouse up looks up");
    for _ in 0..10 {
        c.look(0.0, -1000.0);
    }
    assert!(close(c.pitch_deg(), crate::input::PITCH_LIMIT_DEG, 1e-9));
    for _ in 0..10 {
        c.look(0.0, 1000.0);
    }
    assert!(close(c.pitch_deg(), -crate::input::PITCH_LIMIT_DEG, 1e-9));
    // One wild event turns at most MAX_LOOK_PIXELS' worth.
    let mut d = Controls::new(0.0);
    d.look(0.0, 1e9);
    assert!(
        close(
            d.pitch_deg(),
            -crate::input::MAX_LOOK_PIXELS * crate::input::LOOK_DEG_PER_PIXEL,
            1e-9
        ) || close(d.pitch_deg(), -crate::input::PITCH_LIMIT_DEG, 1e-9)
    );
    d.look(1e9, 0.0);
    let turned = 360.0 - d.yaw_deg();
    assert!(
        close(
            turned,
            crate::input::MAX_LOOK_PIXELS * crate::input::LOOK_DEG_PER_PIXEL,
            1e-9
        ),
        "{turned}"
    );
    c.look(f64::NAN, f64::INFINITY);
    assert!(c.yaw_deg().is_finite() && c.pitch_deg().is_finite());
    assert!((0.0..360.0).contains(&c.yaw_deg()));
}

// ---- the fixed step ----

#[test]
fn a_second_of_frames_is_sixty_ticks_at_any_frame_rate() {
    for hz in [30.0, 60.0, 75.0, 144.0, 240.0] {
        let mut step = FixedStep::new(DT);
        let frames = hz as u32;
        let ticks: u32 = (0..frames).map(|_| step.advance(1.0 / hz)).sum();
        assert!((59..=61).contains(&ticks), "{hz} Hz: {ticks} ticks");
        assert!((0.0..1.0).contains(&step.alpha()));
    }
}

#[test]
fn a_long_frame_runs_a_few_ticks_and_drops_the_rest() {
    let mut step = FixedStep::new(DT);
    assert_eq!(step.advance(2.0), MAX_TICKS_PER_FRAME);
    assert!((0.0..1.0).contains(&step.alpha()), "{}", step.alpha());
    // No debt is carried into the next frame: a normal frame is a normal tick again.
    assert!(step.advance(DT) <= 1);
    for bad in [f64::NAN, f64::NEG_INFINITY, -1.0, f64::INFINITY] {
        let mut step = FixedStep::new(DT);
        let ticks = step.advance(bad);
        assert!(ticks <= MAX_TICKS_PER_FRAME, "{bad}: {ticks}");
        assert!((0.0..1.0).contains(&step.alpha()));
    }
    let mut step = FixedStep::new(DT);
    assert_eq!(step.advance(-1.0), 0);
    assert_eq!(step.advance(f64::NAN), 0);
}

#[test]
fn alpha_is_the_share_of_a_tick_since_the_last() {
    let mut step = FixedStep::new(DT);
    assert_eq!(step.advance(DT * 0.25), 0);
    assert!(close(step.alpha(), 0.25, 1e-9));
    assert_eq!(step.advance(DT), 1);
    assert!(close(step.alpha(), 0.25, 1e-6));
    step.reset();
    assert_eq!(step.alpha(), 0.0);
}

// ---- interpolation ----

#[test]
fn poses_are_interpolated_between_two_ticks() {
    let a = Pose {
        position: [0.0, 0.0, 0.0],
        yaw_deg: 350.0,
    };
    let b = Pose {
        position: [0.1, 0.2, -0.3],
        yaw_deg: 10.0,
    };
    let half = pose::interpolate(&a, &b, 0.5);
    assert!(close(half.position[0], 0.05, 1e-12) && close(half.position[2], -0.15, 1e-12));
    assert!(
        close(half.yaw_deg, 0.0, 1e-9) || close(half.yaw_deg, 360.0, 1e-9),
        "the short way round: {}",
        half.yaw_deg
    );
    assert_eq!(pose::interpolate(&a, &b, 0.0).position, a.position);
    assert_eq!(pose::interpolate(&a, &b, 1.0).position, b.position);
    // A jump (a game master moved it) is not smeared over a tick.
    let far = Pose {
        position: [50.0, 0.0, 0.0],
        yaw_deg: 0.0,
    };
    assert_eq!(pose::interpolate(&a, &far, 0.3).position, far.position);
    // Out-of-range alphas are clamped.
    assert_eq!(pose::interpolate(&a, &b, 7.0).position, b.position);
    assert_eq!(pose::interpolate(&a, &b, f64::NAN).position, b.position);
}

fn one_walker(at: [f64; 3]) -> (World, EntityId) {
    let mut world = World::new("walk", 1);
    let id = world
        .add_entity(EntityDraft {
            position: at,
            components: vec![Component::Character(Character::default())],
            ..EntityDraft::default()
        })
        .unwrap();
    (world, id)
}

#[test]
fn poses_follow_ticks_and_snap_to_other_changes() {
    let (mut world, id) = one_walker([0.0, 0.0, 0.0]);
    let mut poses = Poses::new(&world);
    world.entity_mut(id).unwrap().position = [0.0, 0.0, -0.1];
    poses.ticked(&world);
    let mid = poses.at(id, 0.5).unwrap();
    assert!(close(mid.position[2], -0.05, 1e-12), "{mid:?}");
    // Between ticks, a change that is no tick (a command) is shown at once.
    world.entity_mut(id).unwrap().position = [0.0, 0.0, -0.2];
    poses.observe(&world);
    assert!(close(poses.at(id, 0.0).unwrap().position[2], -0.2, 1e-12));
    // The display world carries the interpolated poses.
    world.entity_mut(id).unwrap().position = [0.0, 0.0, -0.3];
    poses.ticked(&world);
    let mut shown = world.clone();
    poses.apply(&mut shown, 0.5);
    assert!(close(shown.entity(id).unwrap().position[2], -0.25, 1e-12));
}

// ---- the camera rig ----

fn flat_heights(height: f64) -> gamecraft_world::HeightField {
    Terrain {
        size_m: 64.0,
        cell_m: 1.0,
        source: TerrainSource::Flat { height_m: height },
    }
    .build()
    .unwrap()
}

/// A ridge 3 m high from z = 1 southwards: right behind a character at the origin that faces
/// north.
fn hill_behind(_x: f64, z: f64) -> Option<f64> {
    Some(if z >= 1.0 { 3.0 } else { 0.0 })
}

#[test]
fn first_person_looks_from_the_eyes_where_the_mouse_points() {
    let pose = Pose {
        position: [1.0, 2.0, 3.0],
        yaw_deg: 0.0,
    };
    let c = rig::first_person(&pose, 1.65, 90.0, 0.0);
    assert_eq!(c.fov_y_deg, rig::FOV_DEG);
    assert_eq!(c.fov_y_deg, 70.0);
    assert!(close(c.position[1], 3.65, 1e-12));
    let dir = (DVec3::from_array(c.target) - DVec3::from_array(c.position)).normalize();
    assert!(
        dir.distance(yaw_forward(90.0)) < 1e-9,
        "level, facing west: {dir:?}"
    );
    let up = rig::first_person(&pose, 1.65, 90.0, 30.0);
    let dir = (DVec3::from_array(up.target) - DVec3::from_array(up.position)).normalize();
    assert!(close(dir.y, 0.5, 1e-9), "30 degrees up: {dir:?}");
    assert!(c.validate_ok());
}

#[test]
fn third_person_stands_behind_and_above_on_open_ground() {
    let pose = Pose {
        position: [0.0, 0.0, 0.0],
        yaw_deg: 0.0,
    };
    let heights = flat_heights(0.0);
    let c = rig::third_person(&pose, 1.65, 0.0, 0.0, Some(&heights));
    assert_eq!(c.fov_y_deg, 70.0);
    let focus = DVec3::new(0.0, 1.65, 0.0);
    let boom = DVec3::from_array(c.position) - focus;
    assert!(close(boom.length(), rig::BOOM_M, 1e-9), "{boom:?}");
    assert!(
        boom.z > 0.0 && boom.y > 0.0,
        "behind (south) and above: {boom:?}"
    );
    assert_eq!(c.target, focus.to_array());
    assert!(c.validate_ok());
}

#[test]
fn the_boom_is_pulled_in_when_the_terrain_is_in_the_way() {
    let pose = Pose {
        position: [0.0, 0.0, 0.0],
        yaw_deg: 0.0,
    };
    let c = rig::third_person_over(&pose, 1.65, 0.0, 0.0, &hill_behind);
    let focus = DVec3::new(0.0, 1.65, 0.0);
    let length = (DVec3::from_array(c.position) - focus).length();
    assert!(
        (rig::BOOM_MIN_M..rig::BOOM_M).contains(&length),
        "pulled in: {length}"
    );
    // Every point of the boom up to the camera is clear of the ground.
    let dir = (DVec3::from_array(c.position) - focus).normalize();
    for k in 0..=20 {
        let p = focus + dir * (length * f64::from(k) / 20.0);
        let ground = hill_behind(p.x, p.z).unwrap();
        assert!(p.y >= ground, "{p:?} is under the ground at {ground}");
    }
    // Without a height field the boom is full.
    let free = rig::third_person(&pose, 1.65, 0.0, 0.0, None);
    let free_length = (DVec3::from_array(free.position) - focus).length();
    assert!(close(free_length, rig::BOOM_M, 1e-9));
    // Looking up lowers the camera, and the ground still holds it.
    let low = rig::third_person(&pose, 1.65, 0.0, 60.0, Some(&flat_heights(0.0)));
    assert!(low.position[1] >= 0.0, "{:?}", low.position);
    assert!(low.validate_ok());
}

#[test]
fn the_view_toggles() {
    assert_eq!(ViewMode::FirstPerson.toggled(), ViewMode::ThirdPerson);
    assert_eq!(ViewMode::ThirdPerson.toggled(), ViewMode::FirstPerson);
}

// ---- notices ----

#[test]
fn new_messages_and_objective_changes_are_told_once() {
    let mut n = Notices::default();
    let mut objectives = vec![Objective {
        id: "find-oak".into(),
        title: "Find the oak".into(),
        status: ObjectiveStatus::Open,
        hidden: false,
    }];
    let state = json!({"playing": true, "messages": []});
    let lines = n.update(&state, &objectives);
    assert_eq!(lines, vec!["Objective: Find the oak".to_string()]);
    assert!(n.update(&state, &objectives).is_empty(), "told once");

    let state = json!({"playing": true, "messages": [{"text": "The mist rises.", "to": null}]});
    assert_eq!(
        n.update(&state, &objectives),
        vec!["Message: The mist rises.".to_string()]
    );
    assert_eq!(n.message(), Some("The mist rises."));
    assert!(n.update(&state, &objectives).is_empty());

    objectives[0].status = ObjectiveStatus::Done;
    objectives.push(Objective {
        id: "secret".into(),
        title: "Hidden".into(),
        status: ObjectiveStatus::Open,
        hidden: true,
    });
    let state = json!({"playing": true, "messages": []});
    assert_eq!(
        n.update(&state, &objectives),
        vec!["Objective done: Find the oak".to_string()]
    );
    assert_eq!(n.message(), None);
}

// ---- arguments ----

fn parse(list: &[&str]) -> Result<Args, String> {
    args::parse(list.iter().map(|s| s.to_string()))
}

#[test]
fn the_arguments_name_a_world_the_control_channel_and_a_quality() {
    let a = parse(&[]).unwrap();
    assert_eq!(a, Args::default());
    assert_eq!(a.quality, Quality::Medium);
    let a = parse(&["valley.gamecraft", "--stdin", "--quality", "low"]).unwrap();
    assert_eq!(
        a.world.as_deref(),
        Some(std::path::Path::new("valley.gamecraft"))
    );
    assert!(a.stdin);
    assert_eq!(a.quality, Quality::Low);
    assert!(parse(&["--help"]).unwrap().help);
    let e = parse(&["--quality", "ultra"]).unwrap_err();
    assert!(
        e.contains("ultra") && e.contains("low, medium, high"),
        "{e}"
    );
    let e = parse(&["--quality"]).unwrap_err();
    assert!(e.contains("--quality"), "{e}");
    let e = parse(&["--stdn"]).unwrap_err();
    assert!(e.contains("--stdn") && e.contains("--stdin"), "{e}");
    let e = parse(&["a.gamecraft", "b.gamecraft"]).unwrap_err();
    assert!(e.contains("b.gamecraft"), "{e}");
}

// ---- the game loop, without a window ----

fn frames(game: &mut Game, n: u32) -> u32 {
    (0..n).map(|_| game.frame(DT)).sum()
}

#[test]
fn the_game_starts_the_example_world_with_scout_as_the_player() {
    let game = Game::new(example_world());
    assert!(game.session().is_playing());
    let scout = game.player().unwrap();
    let name = &game.session().current_world().entity(scout).unwrap().name;
    assert_eq!(name, "Scout");
    assert!(game.title().contains("Example valley"), "{}", game.title());
}

#[test]
fn walking_forward_moves_the_player_north_through_the_session() {
    let mut game = Game::new(example_world());
    let scout = game.player().unwrap();
    let start = game
        .session()
        .current_world()
        .entity(scout)
        .unwrap()
        .position;
    game.key(Key::Forward, true);
    let ticks = frames(&mut game, 60);
    assert!((59..=61).contains(&ticks), "{ticks}");
    let end = game
        .session()
        .current_world()
        .entity(scout)
        .unwrap()
        .position;
    let walked = start[2] - end[2];
    let walk = Character::default().walk_speed_mps;
    assert!(
        walked > 0.8 * walk && walked < 1.1 * walk,
        "about a second of walking north: {walked} m"
    );
    assert!(close(end[0], start[0], 1e-6));
    // The input went through sim.input: the session holds the record.
    let state = game.session_mut().run("sim.state", &json!({})).unwrap();
    assert_eq!(state["inputs"][0]["forward"], json!(1.0));
    // The display is between the last two ticks.
    let shown = game.display_world();
    let z = shown.entity(scout).unwrap().position[2];
    assert!(
        z >= end[2] - 1e-9 && z <= end[2] + walk * DT + 1e-9,
        "{z} {end:?}"
    );
}

#[test]
fn pause_stops_the_ticks_and_restart_goes_back_to_the_start() {
    let mut game = Game::new(example_world());
    let scout = game.player().unwrap();
    let start = game
        .session()
        .current_world()
        .entity(scout)
        .unwrap()
        .position;
    game.key(Key::Forward, true);
    frames(&mut game, 30);
    game.key(Key::Pause, true);
    game.key(Key::Pause, false);
    assert!(game.is_paused());
    assert_eq!(frames(&mut game, 30), 0);
    game.key(Key::Pause, true);
    assert!(!game.is_paused());
    assert!(frames(&mut game, 10) > 0);
    game.key(Key::Forward, false);
    game.key(Key::Restart, true);
    let tick = game.session_mut().run("sim.state", &json!({})).unwrap()["tick"].clone();
    assert_eq!(tick, json!(0));
    let back = game
        .session()
        .current_world()
        .entity(scout)
        .unwrap()
        .position;
    assert_eq!(back, start);
}

#[test]
fn the_view_key_toggles_the_camera() {
    let mut game = Game::new(example_world());
    assert_eq!(game.view(), ViewMode::ThirdPerson);
    let third = game.camera(16.0 / 9.0);
    game.key(Key::ToggleView, true);
    assert_eq!(game.view(), ViewMode::FirstPerson);
    let first = game.camera(16.0 / 9.0);
    assert_ne!(first.position, third.position);
    let scout = game.player().unwrap();
    let e = game.session().current_world().entity(scout).unwrap();
    let eye = e.position[1] + e.character().unwrap().eye_height_m;
    assert!(close(first.position[1], eye, 1e-9));
}

#[test]
fn an_agent_drives_the_same_session_over_json_lines() {
    let mut game = Game::new(example_world());
    let reply = game
        .control(r#"{"id": 1, "method": "engine.execute", "params": {"command": "sim.input", "params": {"entity": "Scout", "forward": 1}}}"#)
        .unwrap();
    let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["ok"], json!(true), "{reply}");
    let scout = game.player().unwrap();
    let start = game
        .session()
        .current_world()
        .entity(scout)
        .unwrap()
        .position;
    frames(&mut game, 30);
    let end = game
        .session()
        .current_world()
        .entity(scout)
        .unwrap()
        .position;
    assert!(
        start[2] - end[2] > 0.3,
        "the agent's input walks Scout: {start:?} {end:?}"
    );
    // A bad line gets an error reply, not a crash.
    let reply = game.control("not json").unwrap();
    assert!(reply.contains("\"ok\":false"), "{reply}");
    assert_eq!(game.control("   "), None);
    // The agent stops the simulation: the window shows the document and does not step.
    game.control(r#"{"id": 2, "method": "engine.execute", "params": {"command": "sim.stop"}}"#);
    assert!(!game.session().is_playing());
    assert_eq!(frames(&mut game, 10), 0);
    let _ = game.camera(1.0);
}

#[test]
fn a_world_without_characters_is_shown_from_a_framing_camera() {
    let mut world = example_world();
    let ids: Vec<EntityId> = world
        .entities()
        .filter(|e| e.character().is_some())
        .map(|e| e.id)
        .collect();
    for id in ids {
        world.remove_entity(id).unwrap();
    }
    let mut game = Game::new(world);
    assert_eq!(game.player(), None);
    game.key(Key::Forward, true);
    frames(&mut game, 5);
    let c = game.camera(16.0 / 9.0);
    assert!(c.validate_ok());
}

#[test]
fn the_objective_of_the_example_is_told_at_the_start() {
    let mut game = Game::new(example_world());
    frames(&mut game, 1);
    let told = game.take_notices();
    assert!(
        told.iter().any(|l| l == "Objective: Find the oak"),
        "{told:?}"
    );
    assert!(game.take_notices().is_empty());
}

/// `Camera::validate` as a boolean.
trait ValidateOk {
    fn validate_ok(&self) -> bool;
}

impl ValidateOk for gamecraft_render::Camera {
    fn validate_ok(&self) -> bool {
        self.validate().is_ok()
    }
}

#[test]
fn a_player_far_from_the_terrain_still_gets_a_camera() {
    let (world, _) = one_walker([1e6, 50.0, -1e6]);
    let game = Game::new(world);
    assert!(game.camera(1.5).validate_ok());
    let _ = pose::characters(game.session().current_world());
}
