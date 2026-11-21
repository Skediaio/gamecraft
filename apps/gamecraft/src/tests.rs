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
    Character, Component, EntityDraft, EntityId, FireMode, Health, HeldModel, Launcher, Objective,
    ObjectiveStatus, Recoil, Terrain, TerrainSource, ViewMode as WorldView, ViewRules, World,
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
               "jump": false, "interact": true, "pitch_deg": 0.0, "fire": false, "ads": false,
               "hold_breath": false, "switch_mode": false, "reload": false})
    );

    c.release_all();
    let r = c.record();
    assert!(r.forward == 0.0 && r.right == 0.0 && !r.run && !r.interact);
}

#[test]
fn the_fire_button_and_the_camera_tilt_go_into_the_record() {
    let mut c = Controls::new(0.0);
    assert!(!c.record().fire);
    c.look(0.0, -50.0);
    c.set_fire(true);
    let r = c.record();
    assert!(r.fire);
    assert!(close(r.pitch_deg, c.pitch_deg(), 1e-12) && r.pitch_deg > 0.0);
    let params = r.params(EntityId(7));
    assert_eq!(params["fire"], json!(true));
    assert_eq!(params["pitch_deg"], json!(r.pitch_deg));
    c.set_fire(false);
    assert!(
        c.record().fire,
        "a click reaches a tick even when let go before"
    );
    c.ticked();
    assert!(!c.record().fire);
    c.set_fire(true);
    c.release_all();
    assert!(
        !c.record().fire,
        "letting go of everything lets go of the trigger"
    );
    assert!(c.pitch_deg() > 0.0, "the look is kept");
}

#[test]
fn the_sights_the_breath_the_selector_and_a_reload_go_into_the_record() {
    let mut c = Controls::new(0.0);
    // Shift runs; with the sights held it holds the breath instead.
    c.key(Key::Run, true);
    let r = c.record();
    assert!(r.run && !r.ads && !r.hold_breath);
    c.set_ads(true);
    let r = c.record();
    assert!(!r.run && r.ads && r.hold_breath);
    c.key(Key::Run, false);
    let r = c.record();
    assert!(r.ads && !r.hold_breath);
    // B and R are taps that reach one tick, held keys stay.
    c.key(Key::SwitchMode, true);
    c.key(Key::SwitchMode, false);
    c.key(Key::Reload, true);
    let r = c.record();
    assert!(r.switch_mode && r.reload);
    c.ticked();
    let r = c.record();
    assert!(!r.switch_mode && r.reload, "R is still held");
    let params = r.params(EntityId(2));
    assert_eq!(
        (params["ads"].clone(), params["reload"].clone()),
        (json!(true), json!(true))
    );
    c.release_all();
    let r = c.record();
    assert!(!r.ads && !r.reload && !r.switch_mode && !r.hold_breath);
}

/// A flat world with one player holding a rifle that kicks 10 mrad up per shot (semi, then
/// auto) and a held model with sights.
fn rifleman() -> World {
    let mut world = World::new("Rifle range", 4);
    world.terrain = Some(Terrain {
        size_m: 128.0,
        cell_m: 4.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    world.view = ViewRules {
        modes: vec![WorldView::First],
        fov_y_deg: 70.0,
        ..Default::default()
    };
    world
        .add_entity(EntityDraft {
            name: "Rifleman".into(),
            components: vec![
                Component::Character(Character::default()),
                Component::Launcher(Launcher {
                    dispersion_mrad: 0.0,
                    magazine: 30,
                    modes: vec![FireMode::Semi, FireMode::Auto],
                    max_flight_s: 0.5,
                    recoil: Recoil {
                        kick_up_mrad: 10.0,
                        kick_side_mrad: 0.0,
                        recovery_s: 1.0,
                    },
                    ..Launcher::default()
                }),
                Component::HeldModel(HeldModel {
                    ads_offset_m: Some([0.0, -0.05, 0.25]),
                    ads_fov_factor: 0.8,
                    ..HeldModel::of("models/rifle.glb")
                }),
            ],
            ..EntityDraft::default()
        })
        .unwrap();
    world
}

#[test]
fn the_recoil_climbs_the_view_and_the_player_pulls_it_back() {
    let mut game = Game::new(rifleman());
    frames(&mut game, 2);
    let pitch_of = |g: &Game| {
        let c = g.camera(16.0 / 9.0);
        let d = DVec3::from_array(c.target) - DVec3::from_array(c.position);
        (d.y / d.length()).asin().to_degrees()
    };
    let level = pitch_of(&game);
    assert!(close(level, 0.0, 1e-9), "{level}");
    assert!(
        game.hud().is_empty(),
        "the ammunition is hidden until the weapon does something"
    );
    // A click: one shot; the view climbs by the kick (10 mrad, 0.57 degrees).
    game.fire(true);
    game.fire(false);
    frames(&mut game, 1);
    let kicked = pitch_of(&game);
    assert!(close(kicked, 0.01_f64.to_degrees(), 1e-6), "{kicked}");
    assert_eq!(
        game.hud().corner[0].text,
        "SEMI",
        "a shot shows the ammunition"
    );
    let aim = game.view_model_aim().unwrap();
    assert!(
        close(aim.pitch_deg, kicked, 1e-9),
        "the held model climbs too"
    );
    // The player pulls the mouse down by as much: the view is level again while the kick
    // decays, so it then sinks below (the input's aim is the player's).
    game.look(0.0, kicked / crate::input::LOOK_DEG_PER_PIXEL);
    assert!(close(pitch_of(&game), 0.0, 1e-9));
    frames(&mut game, 60);
    assert!(
        pitch_of(&game) < -0.4,
        "the kick settled: {}",
        pitch_of(&game)
    );
    // B switches to auto.
    game.key(Key::SwitchMode, true);
    game.key(Key::SwitchMode, false);
    frames(&mut game, 1);
    assert_eq!(game.hud().corner[0].text, "AUTO");
}

#[test]
fn down_the_sights_the_held_model_comes_up_and_the_view_narrows() {
    let mut game = Game::new(rifleman());
    frames(&mut game, 2);
    assert_eq!(game.ads_blend(), 0.0);
    let wide = game.camera(16.0 / 9.0).fov_y_deg;
    game.aim_down_sights(true);
    assert!(game.is_aiming());
    // 0.2 s at 60 ticks a second: up at the sights after 12 ticks. The picture shows the
    // tick before the last (like the camera), so it is one frame behind.
    frames(&mut game, 7);
    assert!(close(game.ads_blend(), 0.5, 1e-9), "{}", game.ads_blend());
    frames(&mut game, 6);
    assert!(close(game.ads_blend(), 1.0, 1e-9));
    assert!(close(game.camera(16.0 / 9.0).fov_y_deg, wide * 0.8, 1e-9));
    assert!(close(game.view_model_aim().unwrap().ads, 1.0, 1e-9));
    // The simulation knows: the input holds `ads`.
    let state = game.session_mut().run("sim.state", &json!({})).unwrap();
    assert_eq!(state["inputs"][0]["ads"], json!(true));
    game.aim_down_sights(false);
    frames(&mut game, 13);
    assert_eq!(game.ads_blend(), 0.0);
    assert!(close(game.camera(16.0 / 9.0).fov_y_deg, wide, 1e-9));
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
        ..Default::default()
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

#[test]
fn the_field_of_view_flag_takes_30_to_120_degrees() {
    assert_eq!(parse(&["--fov", "90"]).unwrap().fov, Some(90.0));
    assert_eq!(parse(&["--fov", "30"]).unwrap().fov, Some(30.0));
    assert_eq!(parse(&["--fov", "120"]).unwrap().fov, Some(120.0));
    assert_eq!(parse(&[]).unwrap().fov, None);
    for bad in ["29", "121", "-70", "NaN", "inf"] {
        let e = parse(&["--fov", bad]).unwrap_err();
        assert!(e.contains("--fov") && e.contains("30 to 120"), "{bad}: {e}");
    }
    let e = parse(&["--fov", "wide"]).unwrap_err();
    assert!(e.contains("`wide`") && e.contains("not a number"), "{e}");
    let e = parse(&["--fov"]).unwrap_err();
    assert!(e.contains("--fov needs a value"), "{e}");
    let e = parse(&["--fv", "90"]).unwrap_err();
    assert!(e.contains("did you mean --fov"), "{e}");
    assert_eq!(args::parse_fov(" 75.5 "), Ok(75.5));
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

fn ruled(modes: &[WorldView], fov_y_deg: f64) -> World {
    let mut world = example_world();
    world.view = ViewRules {
        modes: modes.to_vec(),
        fov_y_deg,
        ..Default::default()
    };
    world
}

#[test]
fn the_views_cycle_among_those_the_rules_allow() {
    let rules = |modes: &[WorldView]| ViewRules {
        modes: modes.to_vec(),
        fov_y_deg: 70.0,
        ..Default::default()
    };
    let first = rules(&[WorldView::First]);
    assert_eq!(ViewMode::first_allowed(&first), ViewMode::FirstPerson);
    assert_eq!(
        ViewMode::FirstPerson.next_allowed(&first),
        ViewMode::FirstPerson
    );
    assert_eq!(
        ViewMode::ThirdPerson.allowed(&first),
        ViewMode::FirstPerson,
        "a view the rules forbid falls back to the first allowed"
    );
    let both = rules(&[WorldView::First, WorldView::Third]);
    assert_eq!(ViewMode::first_allowed(&both), ViewMode::FirstPerson);
    assert_eq!(
        ViewMode::FirstPerson.next_allowed(&both),
        ViewMode::ThirdPerson
    );
    assert_eq!(
        ViewMode::ThirdPerson.next_allowed(&both),
        ViewMode::FirstPerson
    );
    assert_eq!(
        ViewMode::first_allowed(&ViewRules::default()),
        ViewMode::ThirdPerson,
        "a world without rules starts in third person, as before"
    );
}

#[test]
fn a_world_that_allows_only_first_person_stays_in_first_person() {
    let mut game = Game::new(ruled(&[WorldView::First], 70.0));
    assert_eq!(game.view(), ViewMode::FirstPerson);
    assert_eq!(game.hidden_character(), game.player());
    for _ in 0..3 {
        game.key(Key::ToggleView, true);
        game.key(Key::ToggleView, false);
        assert_eq!(game.view(), ViewMode::FirstPerson, "F1 does nothing");
    }
    let mut game = Game::new(ruled(&[WorldView::First, WorldView::Third], 70.0));
    assert_eq!(game.view(), ViewMode::FirstPerson, "the first listed");
    game.key(Key::ToggleView, true);
    assert_eq!(game.view(), ViewMode::ThirdPerson);
    // A game master forbids the third-person view while it is played.
    game.session_mut()
        .run("view.set", &json!({"modes": ["first"]}))
        .unwrap();
    assert_eq!(game.view(), ViewMode::FirstPerson);
    game.key(Key::ToggleView, true);
    assert_eq!(game.view(), ViewMode::FirstPerson);
}

#[test]
fn the_camera_sees_as_wide_as_the_world_or_the_player_says() {
    let mut game = Game::new(example_world());
    assert_eq!(game.camera(16.0 / 9.0).fov_y_deg, 70.0);
    let mut game_ruled = Game::new(ruled(&[WorldView::Third, WorldView::First], 55.0));
    assert_eq!(game_ruled.camera(16.0 / 9.0).fov_y_deg, 55.0);
    game_ruled.set_fov(Some(95.0));
    assert_eq!(
        game_ruled.camera(16.0 / 9.0).fov_y_deg,
        95.0,
        "the player's own"
    );
    game_ruled.key(Key::ToggleView, true);
    assert_eq!(
        game_ruled.camera(16.0 / 9.0).fov_y_deg,
        95.0,
        "in both views"
    );
    game.session_mut()
        .run("view.set", &json!({"fov_y_deg": 80}))
        .unwrap();
    assert_eq!(
        game.camera(16.0 / 9.0).fov_y_deg,
        80.0,
        "a running world's rules"
    );
    game.set_fov(None);
    assert_eq!(game.fov_y_deg(), 80.0);
}

#[test]
fn free_look_turns_the_camera_but_not_the_aim() {
    let mut game = Game::new(ruled(&[WorldView::First], 70.0));
    frames(&mut game, 2);
    let aim = game.view_model_aim().unwrap();
    let player = game.player().unwrap();
    let facing_before = game
        .session()
        .current_world()
        .entity(player)
        .unwrap()
        .rotation_y_deg;
    let before = game.camera(16.0 / 9.0);
    let look = |c: &gamecraft_render::Camera| {
        let d = DVec3::from_array(c.target) - DVec3::from_array(c.position);
        d / d.length()
    };
    game.key(Key::Freelook, true);
    assert!(game.is_freelooking());
    // 500 pixels right (60 degrees) and 200 up.
    game.look(250.0, 0.0);
    game.look(250.0, -200.0);
    let turned = game.camera(16.0 / 9.0);
    let (a, b) = (look(&before), look(&turned));
    assert!(a.dot(b) < 0.6, "the head turned: {a:?} {b:?}");
    assert!(b.y > 0.3, "and looks up: {b:?}");
    let after = game.view_model_aim().unwrap();
    assert_eq!(
        (after.yaw_deg, after.pitch_deg),
        (aim.yaw_deg, aim.pitch_deg),
        "the aim, and so the held model, stays"
    );
    // The character the simulation steps keeps its facing.
    frames(&mut game, 2);
    let scout = game.player().unwrap();
    let facing = |g: &Game| {
        g.session()
            .current_world()
            .entity(scout)
            .unwrap()
            .rotation_y_deg
    };
    assert_eq!(facing(&game), facing_before);
    // At most 100 degrees to the side.
    game.look(10_000.0, 0.0);
    game.look(10_000.0, 0.0);
    let far = look(&game.camera(16.0 / 9.0));
    let side = yaw_forward(aim.yaw_deg);
    let flat = DVec3::new(far.x, 0.0, far.z).normalize();
    let angle = flat.dot(side).clamp(-1.0, 1.0).acos().to_degrees();
    assert!(close(angle, 100.0, 1e-6), "{angle}");
    // Let go: back on the aim at once.
    game.key(Key::Freelook, false);
    assert!(!game.is_freelooking());
    let back = look(&game.camera(16.0 / 9.0));
    assert!(back.dot(look(&before)) > 1.0 - 1e-9, "{back:?}");
    // And the mouse turns the aim again.
    game.look(100.0, 0.0);
    assert!(game.view_model_aim().unwrap().yaw_deg != aim.yaw_deg);
}

#[test]
fn the_view_model_is_held_only_in_first_person() {
    let mut game = Game::new(example_world());
    assert_eq!(game.view(), ViewMode::ThirdPerson);
    assert!(game.view_model_aim().is_none());
    game.key(Key::ToggleView, true);
    let aim = game.view_model_aim().unwrap();
    let camera = game.camera(16.0 / 9.0);
    assert_eq!(aim.eye, camera.position, "held at the eyes");
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

// ---- combat effects ----

fn skirmish() -> World {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/skirmish/world.gamecraft");
    crate::app::load_world_file(&path).unwrap()
}

#[test]
fn firing_with_the_mouse_puts_tracers_in_the_air_and_puffs_where_they_hit() {
    let mut game = Game::new(skirmish());
    assert_eq!(game.effects(), gamecraft_render::Effects::default());
    game.fire(true);
    // Level, due north: the wall line 60 m ahead stops the shot.
    let (mut tracers, mut impacts) = (0, 0);
    for _ in 0..240 {
        game.frame(DT);
        let effects = game.effects();
        tracers = tracers.max(effects.tracers.len());
        impacts = impacts.max(effects.impacts.len());
        for t in &effects.tracers {
            let length = t
                .from
                .iter()
                .zip(&t.to)
                .map(|(a, b)| (b - a) * (b - a))
                .sum::<f64>()
                .sqrt();
            assert!(length > 0.0 && length < 40.0, "a short streak: {length} m");
        }
    }
    game.fire(false);
    game.frame(DT);
    let state = game.session_mut().run("sim.state", &json!({})).unwrap();
    assert_eq!(state["inputs"][0]["fire"], json!(false));
    assert!(tracers >= 1, "a tracer was in the air");
    assert!(impacts >= 1, "a shot hit the wall or the ground");
    // A second after the last hit the puffs are gone; a restart clears at once.
    game.restart();
    assert_eq!(game.effects(), gamecraft_render::Effects::default());
}

#[test]
fn invert_look_is_a_flag() {
    assert!(parse(&["--invert-look"]).unwrap().invert_look);
    assert!(!parse(&[]).unwrap().invert_look);
}

#[test]
fn the_left_stick_walks_the_player_north_and_start_pauses() {
    use crate::pad::PadState;
    let mut game = Game::new(example_world());
    let scout = game.player().unwrap();
    let start = game
        .session()
        .current_world()
        .entity(scout)
        .unwrap()
        .position;
    let stick = PadState {
        left_y: 1.0,
        ..PadState::default()
    };
    for _ in 0..60 {
        game.pad(&stick, 1.0 / 60.0);
        game.frame(1.0 / 60.0);
    }
    let end = game
        .session()
        .current_world()
        .entity(scout)
        .unwrap()
        .position;
    assert!(start[2] - end[2] > 0.5, "{start:?} {end:?}");
    game.pad(
        &PadState {
            start: true,
            ..stick
        },
        1.0 / 60.0,
    );
    assert!(game.is_paused());
}

// ---- projects and their models ----

#[test]
fn a_project_folder_opens_with_its_models_handed_to_the_view_once() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/assets-demo");
    let mut game = Game::from_project(&dir).unwrap();
    assert!(game.session().is_playing());
    let mut paths: Vec<String> = game.take_assets().into_iter().map(|(p, _)| p).collect();
    paths.sort();
    assert_eq!(paths, ["models/crate.glb", "models/fence.glb"]);
    assert!(game.take_assets().is_empty(), "handed over once");
    // The crates collide: their bounds reached the running world.
    assert!(
        game.session()
            .current_world()
            .model_bounds("models/crate.glb")
            .is_some()
    );
    let e = Game::from_project(&dir.join("no-such-project"))
        .err()
        .unwrap();
    assert!(e.contains("cannot be opened"), "{e}");
}

// ---- the HUD: ammunition, hits and death ----

/// `rifleman()` with a health component on the player and a sniper 40 m north that fires
/// south at it through `sim.fire`.
fn duel(spare: Option<u32>) -> World {
    let mut world = rifleman();
    let player = world.entities().next().unwrap().id;
    let e = world.entity_mut(player).unwrap();
    e.set_component(Component::Health(Health::default()));
    if let Some(l) = e.launcher().cloned() {
        e.set_component(Component::Launcher(Launcher {
            spare_magazines: spare,
            ..l
        }));
    }
    world
        .add_entity(EntityDraft {
            name: "Sniper".into(),
            position: [0.0, 0.0, -40.0],
            components: vec![Component::Launcher(Launcher {
                dispersion_mrad: 0.0,
                max_flight_s: 1.0,
                ..Launcher::default()
            })],
            ..EntityDraft::default()
        })
        .unwrap();
    world
}

fn sniper_fires(game: &mut Game) {
    let sniper = game
        .session()
        .current_world()
        .entities()
        .find(|e| e.name == "Sniper")
        .unwrap()
        .id;
    let line = json!({"id": 1, "method": "engine.execute", "params": {"command": "sim.fire",
            "params": {"entity": sniper.0, "yaw_deg": 180}}});
    let reply = game.control(&line.to_string()).unwrap();
    assert!(reply.contains("\"ok\":true"), "{reply}");
}

#[test]
fn the_ammunition_shows_after_a_shot_a_reload_or_a_check_and_hides_after_three_seconds() {
    let mut game = Game::new(duel(Some(2)));
    frames(&mut game, 2);
    assert!(game.hud().is_empty());
    // T: check the magazine.
    game.key(Key::CheckMagazine, true);
    game.key(Key::CheckMagazine, false);
    let lines: Vec<String> = game.hud().corner.iter().map(|l| l.text.clone()).collect();
    assert_eq!(lines, ["SEMI", "MAGAZINE FULL", "2 FULL"]);
    frames(&mut game, (crate::hud::AMMO_SHOW_S / DT) as u32 - 5);
    assert!(!game.hud().is_empty(), "still up just before three seconds");
    frames(&mut game, 10);
    assert!(game.hud().is_empty(), "gone after three seconds");
    // A shot shows it again, counted from the last shot.
    game.fire(true);
    game.fire(false);
    frames(&mut game, 2);
    assert!(!game.hud().is_empty());
    frames(&mut game, 150);
    assert!(!game.hud().is_empty(), "2.5 s after the shot");
    frames(&mut game, 60);
    assert!(game.hud().is_empty());
    // A reload shows it; the partly used magazine went back in the pouch, so the count stays.
    game.key(Key::Reload, true);
    game.key(Key::Reload, false);
    frames(&mut game, 2);
    let lines: Vec<String> = game.hud().corner.iter().map(|l| l.text.clone()).collect();
    assert_eq!(lines, ["SEMI", "RELOADING", "1 FULL, 1 PARTLY USED"]);
}

#[test]
fn a_reloading_weapon_is_lowered_by_the_reload_progress() {
    let mut game = Game::new(duel(Some(2)));
    frames(&mut game, 2);
    assert_eq!(game.reload_progress(), None);
    assert_eq!(game.view_model_aim().unwrap().reload, None);
    game.fire(true);
    game.fire(false);
    frames(&mut game, 2);
    game.key(Key::Reload, true);
    game.key(Key::Reload, false);
    frames(&mut game, 60);
    let early = game.reload_progress().expect("reloading");
    assert!(early > 0.0 && early < 1.0, "{early}");
    assert_eq!(game.view_model_aim().unwrap().reload, Some(early));
    frames(&mut game, 400);
    assert_eq!(game.reload_progress(), None, "the reload is over");
}

#[test]
fn the_crosshair_follows_the_view_rules() {
    let mut world = duel(Some(2));
    assert!(Game::new(world.clone()).crosshair_allowed());
    world.view.crosshair = false;
    assert!(!Game::new(world).crosshair_allowed());
}

#[test]
fn being_shot_flashes_the_screen_and_dying_shows_the_death_screen_until_enter() {
    let mut game = Game::new(duel(None));
    frames(&mut game, 2);
    assert!(!game.is_dead());
    sniper_fires(&mut game);
    // The bullet needs about 50 ms for 40 m.
    let mut flashed = false;
    for _ in 0..30 {
        game.frame(DT);
        flashed |= game.hud().vignette.is_some() && !game.is_dead();
        if game.is_dead() {
            break;
        }
    }
    assert!(game.is_dead(), "one hit of 160 against 100 health");
    let hud = game.hud();
    assert!(flashed || hud.veil.is_some());
    assert!(hud.veil.is_some() && hud.corner.is_empty());
    frames(&mut game, 120);
    let hud = game.hud();
    let lines: Vec<&str> = hud.centre.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        lines,
        [
            "YOU ARE DEAD",
            "KILLED BY SNIPER AT 40 M",
            "PRESS ENTER TO RESTART"
        ]
    );
    // The dead player's trigger does nothing, and Enter restarts the world from its document.
    game.fire(true);
    frames(&mut game, 5);
    game.key(Key::Confirm, true);
    game.key(Key::Confirm, false);
    assert!(!game.is_dead());
    assert!(game.hud().is_empty());
    let player = game.player().unwrap();
    let health = game
        .session()
        .current_world()
        .entity(player)
        .unwrap()
        .health()
        .unwrap()
        .current;
    assert_eq!(health, 100.0);
}

#[test]
fn enter_does_nothing_while_the_player_lives() {
    let mut game = Game::new(duel(None));
    frames(&mut game, 5);
    let tick = game.session().live().unwrap().tick();
    game.key(Key::Confirm, true);
    game.key(Key::Confirm, false);
    frames(&mut game, 1);
    assert!(game.session().live().unwrap().tick() > tick, "no restart");
}

// ---- tracers and the camera ----

#[test]
fn a_shot_fired_this_tick_by_a_moving_player_starts_at_the_muzzle_the_camera_shows() {
    use crate::game::tracer_of;
    use gamecraft_sim::Projectile;
    let held = HeldModel {
        muzzle_m: Some([0.0, 0.0, -0.9]),
        ..HeldModel::of("models/rifle.glb")
    };
    let eye_height_m = 1.6;
    let eye = |p: &Pose| {
        let [x, y, z] = p.position;
        [x, y + eye_height_m, z]
    };
    // Walking north at 6 m/s: the last tick left the player 0.1 m beyond the one before.
    let before = Pose {
        position: [3.0, 0.0, -2.0],
        yaw_deg: 0.0,
    };
    let last = Pose {
        position: [3.0, 0.0, -2.1],
        yaw_deg: 0.0,
    };
    let fire_point = held.muzzle_point(eye(&last), 0.0, 0.0, 0.0).unwrap();
    let velocity = [0.0, 0.0, -800.0];
    // Fired in the last tick, which also flew it for DT.
    let shot = Projectile {
        id: 1,
        shooter: EntityId(1),
        position: [
            fire_point[0],
            fire_point[1],
            fire_point[2] + velocity[2] * DT,
        ],
        velocity,
        age_s: DT,
        mass_kg: 0.004,
        ballistic_coefficient: 200.0,
        damage_per_joule: 0.01,
        max_flight_s: 1.0,
    };
    for alpha in [0.1, 0.25, 0.5, 0.75, 0.95] {
        let shown = pose::interpolate(&before, &last, alpha);
        let lag = [
            shown.position[0] - last.position[0],
            shown.position[1] - last.position[1],
            shown.position[2] - last.position[2],
        ];
        let muzzle = held.muzzle_point(eye(&shown), 0.0, 0.0, 0.0).unwrap();
        let tracer = tracer_of(&shot, alpha, lag).expect("a tracer");
        let off = DVec3::from_array(tracer.from) - DVec3::from_array(muzzle);
        assert!(
            off.length() < 0.01,
            "alpha {alpha}: the tail is {} m from the muzzle on screen",
            off.length()
        );
    }
}
