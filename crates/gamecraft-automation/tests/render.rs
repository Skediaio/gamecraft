//! `view.render`: the request a renderer is handed, the answer, and the errors.

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
use gamecraft_automation::{CameraRequest, base64};
use gamecraft_world::EntityId;
use serde_json::json;

#[test]
fn without_pictures_a_renderer_is_needed() {
    let mut h = host();
    let error = err(&mut h, "view.render", json!({}));
    assert!(error.starts_with("view.render: "), "{error}");
    assert!(error.contains("renderer is needed"), "{error}");
}

#[test]
fn the_default_frames_the_whole_world() {
    let (mut h, seen) = host_with_pictures(host());
    let r = ok(&mut h, "view.render", json!({}));
    assert_eq!(r["mime"], "image/png");
    assert_eq!(r["base64"], base64(PNG_1X1));
    assert_eq!(
        (r["width"].clone(), r["height"].clone()),
        (json!(960), json!(540))
    );
    assert_eq!(r["renderer"], "fake");
    let seen = seen.borrow();
    let s = seen.last().unwrap();
    assert_eq!(s.request.camera, CameraRequest::Framing);
    assert_eq!((s.request.width, s.request.height), (960, 540));
    assert_eq!(s.request.time_s, 0.0);
    assert!(s.has_heights);
    assert_eq!(s.view_revision, h.session().view_revision());
    assert_eq!(&s.world, h.session().document());
}

#[test]
fn the_camera_variants() {
    let (mut h, seen) = host_with_pictures(host());
    ok(
        &mut h,
        "view.render",
        json!({"width": 64, "height": 32, "camera": {"position": [0, 20, 30], "target": [0, 0, 0]}}),
    );
    assert_eq!(
        seen.borrow().last().unwrap().request.camera,
        CameraRequest::LookAt {
            position: [0.0, 20.0, 30.0],
            target: [0.0, 0.0, 0.0],
            fov_y_deg: 60.0
        }
    );
    ok(
        &mut h,
        "view.render",
        json!({"camera": {"position": [1, 2, 3], "target": [4, 5, 6], "fov_y_deg": 45}}),
    );
    assert!(matches!(
        seen.borrow().last().unwrap().request.camera,
        CameraRequest::LookAt { fov_y_deg, .. } if fov_y_deg == 45.0
    ));
    ok(
        &mut h,
        "view.render",
        json!({"follow": {"entity": "scout"}}),
    );
    assert_eq!(
        seen.borrow().last().unwrap().request.camera,
        CameraRequest::ThirdPerson {
            entity: EntityId(3),
            distance_m: 6.0
        }
    );
    ok(
        &mut h,
        "view.render",
        json!({"follow": {"entity": 3, "view": "first_person"}}),
    );
    assert_eq!(
        seen.borrow().last().unwrap().request.camera,
        CameraRequest::FirstPerson {
            entity: EntityId(3)
        }
    );
    ok(
        &mut h,
        "view.render",
        json!({"follow": {"entity": "Crate", "view": "third_person", "distance_m": 12.5}, "time_s": 4}),
    );
    let last = seen.borrow().last().unwrap().request.clone();
    assert_eq!(
        last.camera,
        CameraRequest::ThirdPerson {
            entity: EntityId(1),
            distance_m: 12.5
        }
    );
    assert_eq!(last.time_s, 4.0);
}

#[test]
fn bad_requests_are_refused_before_drawing() {
    let (mut h, seen) = host_with_pictures(host());
    for (params, says) in [
        (json!({"width": 0}), "`width`"),
        (json!({"width": 5000}), "`width`"),
        (json!({"height": -3}), "`height`"),
        (json!({"height": 1.5}), "`height`"),
        (json!({"camera": {"position": [0, 1, 0]}}), "target"),
        (
            json!({"camera": {"position": [0, 1, 0], "target": [0, 1, 0]}}),
            "target",
        ),
        (
            json!({"camera": {"position": [0, 1, 0], "target": [0, 0, 0], "fov_y_deg": 0}}),
            "fov_y_deg",
        ),
        (
            json!({"camera": {"position": [0, 1], "target": [0, 0, 0]}}),
            "position",
        ),
        (
            json!({"camera": {"position": [0, 1, 0], "target": [0, 0, 0], "zoom": 2}}),
            "zoom",
        ),
        (
            json!({"camera": {"position": [0, 9, 9], "target": [0, 0, 0]}, "follow": {"entity": 3}}),
            "not both",
        ),
        (json!({"follow": {"entity": "Nobody"}}), "Nobody"),
        (json!({"follow": {"entity": 99}}), "99"),
        (json!({"follow": {}}), "entity"),
        (json!({"follow": {"entity": 3, "view": "top"}}), "view"),
        (
            json!({"follow": {"entity": 3, "distance_m": 0}}),
            "distance_m",
        ),
        (
            json!({"follow": {"entity": 3, "view": "first_person", "distance_m": 4}}),
            "distance_m",
        ),
        (json!({"time_s": -1}), "time_s"),
        (json!({"camera": 5}), "camera"),
        (json!({"zoom": 2}), "zoom"),
    ] {
        let error = err(&mut h, "view.render", params.clone());
        assert!(error.starts_with("view.render: "), "{params}: {error}");
        assert!(error.contains(says), "{params}: {error}");
    }
    assert!(seen.borrow().is_empty());
}

#[test]
fn while_playing_the_running_world_is_drawn() {
    let (mut h, seen) = host_with_pictures(host());
    ok(&mut h, "sim.start", json!({}));
    ok(&mut h, "sim.step", json!({"ticks": 30}));
    ok(
        &mut h,
        "entity.add",
        json!({"name": "Dropped", "at": [2, 2]}),
    );
    let r = ok(&mut h, "view.render", json!({}));
    assert_eq!(r["entities"], 4);
    let seen = seen.borrow();
    let s = seen.last().unwrap();
    assert_eq!(s.world.entity_count(), 4);
    assert_eq!(h.session().document().entity_count(), 3);
    assert_eq!(&s.world, h.session().current_world());
    assert_eq!(s.request.time_s, 0.5);
    assert_eq!(s.view_revision, h.session().view_revision());
}

#[test]
fn a_picture_can_be_written_to_a_file() {
    let dir = TempDir::new("render");
    let (mut h, _) = host_with_pictures(host_in(&dir));
    let r = ok(
        &mut h,
        "view.render",
        json!({"path": "shot.png", "width": 32, "height": 16}),
    );
    assert!(r["path"].as_str().unwrap().ends_with("shot.png"));
    assert_eq!(r["bytes"], PNG_1X1.len());
    assert_eq!(
        (r["width"].clone(), r["height"].clone()),
        (json!(32), json!(16))
    );
    assert!(r.get("base64").is_none());
    assert_eq!(std::fs::read(dir.join("shot.png")).unwrap(), PNG_1X1);
    // A picture is drawn again at will: it may replace one.
    ok(&mut h, "view.render", json!({"path": "shot.png"}));
    let error = err(&mut h, "view.render", json!({"path": "shot.jpg"}));
    assert!(error.contains(".png"), "{error}");
    let error = err(&mut h, "view.render", json!({"path": "../shot.png"}));
    assert!(error.contains("`..`"), "{error}");

    let (mut rootless, _) = host_with_pictures(host());
    let error = err(&mut rootless, "view.render", json!({"path": "shot.png"}));
    assert!(error.contains("--automation-write-root"), "{error}");
}

#[test]
fn a_failing_renderer_is_an_error() {
    struct Broken;
    impl gamecraft_automation::Pictures for Broken {
        fn render(
            &mut self,
            _: &gamecraft_world::World,
            _: Option<&gamecraft_world::HeightField>,
            _: u64,
            _: &gamecraft_automation::RenderRequest,
        ) -> Result<gamecraft_automation::Rendered, String> {
            Err("the adapter was lost".into())
        }
    }
    let mut h = host().with_pictures(Box::new(Broken));
    let error = err(&mut h, "view.render", json!({}));
    assert_eq!(error, "view.render: the adapter was lost");
}
