//! The view rules: their defaults, their checks and their place in the file.

use crate::{DEFAULT_FOV_Y_DEG, ViewMode, ViewRules, World};

#[test]
fn a_world_without_view_rules_allows_both_views_at_70_degrees_and_writes_none() {
    let w = World::new("v", 1);
    assert_eq!(w.view.modes, [ViewMode::Third, ViewMode::First]);
    assert_eq!(w.view.fov_y_deg, DEFAULT_FOV_Y_DEG);
    assert_eq!(DEFAULT_FOV_Y_DEG, 70.0);
    assert!(w.view.is_default());
    let text = w.to_json().unwrap();
    assert!(!text.contains("\"view\""), "{text}");
    let back = World::from_json(r#"{"gamecraft": 1}"#).unwrap();
    assert_eq!(back.view, ViewRules::default());
}

#[test]
fn view_rules_round_trip_through_the_file() {
    let mut w = World::new("v", 1);
    w.view = ViewRules {
        modes: vec![ViewMode::First],
        fov_y_deg: 62.5,
    };
    let text = w.to_json().unwrap();
    assert!(
        text.contains(
            "\"view\": {\n    \"modes\": [\n      \"first\"\n    ],\n    \"fov_y_deg\": 62.5\n  }"
        ),
        "{text}"
    );
    let back = World::from_json(&text).unwrap();
    assert_eq!(back, w);
    // A field left out takes its default.
    let partial = World::from_json(r#"{"gamecraft": 1, "view": {"modes": ["first"]}}"#).unwrap();
    assert_eq!(partial.view.modes, [ViewMode::First]);
    assert_eq!(partial.view.fov_y_deg, 70.0);
    let partial = World::from_json(r#"{"gamecraft": 1, "view": {"fov_y_deg": 90}}"#).unwrap();
    assert_eq!(partial.view.modes, [ViewMode::Third, ViewMode::First]);
}

#[test]
fn bad_view_rules_name_the_field_and_what_is_allowed() {
    let err = |text: &str| World::from_json(text).unwrap_err().to_string();
    let e = err(r#"{"gamecraft": 1, "view": {"modes": []}}"#);
    assert!(
        e.contains("view.modes") && e.contains("at least one"),
        "{e}"
    );
    let e = err(r#"{"gamecraft": 1, "view": {"modes": ["first", "first"]}}"#);
    assert!(e.contains("view.modes") && e.contains("once"), "{e}");
    let e = err(r#"{"gamecraft": 1, "view": {"modes": ["second"]}}"#);
    assert!(e.contains("second") && e.contains("first"), "{e}");
    for fov in ["29.9", "120.5", "-70", "0"] {
        let e = err(&format!(
            r#"{{"gamecraft": 1, "view": {{"fov_y_deg": {fov}}}}}"#
        ));
        assert!(
            e.contains("view.fov_y_deg") && e.contains("30 to 120"),
            "{fov}: {e}"
        );
    }
    let e = err(r#"{"gamecraft": 1, "view": {"fov": 60}}"#);
    assert!(e.contains("fov"), "{e}");
    for fov in [30.0, 120.0] {
        let mut w = World::new("v", 1);
        w.view.fov_y_deg = fov;
        w.validate().unwrap();
    }
    let mut w = World::new("v", 1);
    w.view.fov_y_deg = f64::NAN;
    assert!(
        w.validate()
            .unwrap_err()
            .to_string()
            .contains("view.fov_y_deg")
    );
    assert!(w.to_json().is_err(), "an invalid world is never written");
}
