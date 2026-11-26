//! Water: the even-odd rule over an outline, an area's coverage and depth, the checks of the
//! component, and the file format with and without water.

use crate::limits::WATER_POINTS_MAX;
use crate::water::{Water, WaterArea, depth_at, inside_outline, surface_at, water_areas};
use crate::{Component, EntityDraft, World};

fn square() -> Vec<[f64; 2]> {
    vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]]
}

#[test]
fn the_even_odd_rule_holds_a_square_and_its_hole() {
    let s = square();
    assert!(inside_outline([0.0, 0.0], &s));
    assert!(inside_outline([4.9, -4.9], &s));
    assert!(!inside_outline([5.1, 0.0], &s));
    assert!(!inside_outline([0.0, -5.1], &s));
    assert!(!inside_outline([0.0, 0.0], &[]));
    assert!(!inside_outline([0.0, 0.0], &s[..2]));
    assert!(!inside_outline([f64::NAN, 0.0], &s));
    // A square inside a square: the inner ring is a hole by the even-odd rule.
    let ring = vec![
        [-5.0, -5.0],
        [5.0, -5.0],
        [5.0, 5.0],
        [-5.0, 5.0],
        [-2.0, -2.0],
        [-2.0, 2.0],
        [2.0, 2.0],
        [2.0, -2.0],
    ];
    assert!(!inside_outline([0.0, 0.0], &ring), "the hole");
    assert!(inside_outline([3.5, 0.0], &ring), "the ring itself");
}

#[test]
fn an_area_covers_and_measures_depth() {
    let mut w = World::new("w", 1);
    let id = w
        .add_entity(EntityDraft {
            name: "Pond".into(),
            position: [10.0, 2.0, -3.0],
            rotation_y_deg: 90.0,
            components: vec![Component::Water(Water::new(0.5, square()))],
            ..EntityDraft::default()
        })
        .unwrap();
    let e = w.entity(id).unwrap();
    let water = e.water().unwrap();
    assert_eq!(water.surface_y(e), Some(2.5));
    let outline = water.outline_in_world(e);
    // Turned 90 degrees: the square stays a square about the entity's position.
    assert_eq!(outline.len(), 4);
    for p in &outline {
        assert!(
            (p[0] - 10.0).abs() == 5.0 || (p[1] + 3.0).abs() == 5.0,
            "{p:?}"
        );
    }
    let areas = water_areas(&w);
    assert_eq!(areas.len(), 1);
    assert_eq!(areas[0].level, 2.5);
    assert_eq!(areas[0].signed_area_m2().abs(), 100.0);
    assert!(areas[0].covers(10.0, -3.0));
    assert_eq!(depth_at(&areas, 10.0, -3.0, 2.0), Some(0.5));
    assert_eq!(depth_at(&areas, 10.0, -3.0, 2.5), None, "at the surface");
    assert_eq!(depth_at(&areas, 10.0, -3.0, 3.0), None, "above it");
    assert_eq!(depth_at(&areas, 20.0, -3.0, 0.0), None, "dry land");
    // Everywhere: the sea.
    let sea = w
        .add_entity(EntityDraft {
            name: "Sea".into(),
            position: [0.0, 0.0, 0.0],
            components: vec![Component::Water(Water::default())],
            ..EntityDraft::default()
        })
        .unwrap();
    let areas = water_areas(&w);
    assert_eq!(areas.len(), 2);
    assert_eq!(areas[1].entity, sea);
    assert!(areas[1].covers(1e5, -1e5));
    assert_eq!(depth_at(&areas, 20.0, -3.0, -1.0), Some(1.0));
    // The highest surface counts where two overlap.
    assert_eq!(surface_at(&areas, 10.0, -3.0).map(|a| a.entity), Some(id));
    assert_eq!(depth_at(&areas, 10.0, -3.0, 0.0), Some(2.5));
}

#[test]
fn an_outline_is_checked() {
    let mut w = World::new("w", 1);
    let mut add = |outline: Vec<[f64; 2]>, level: f64| {
        w.add_entity(EntityDraft {
            name: "Water".into(),
            components: vec![Component::Water(Water::new(level, outline))],
            ..EntityDraft::default()
        })
    };
    assert!(add(vec![], 0.0).is_ok());
    assert!(add(square(), 1e6).is_ok());
    let e = add(vec![[0.0, 0.0], [1.0, 0.0]], 0.0)
        .unwrap_err()
        .to_string();
    assert!(e.contains("water.outline") && e.contains("2 points"), "{e}");
    let mut many = Vec::new();
    for i in 0..=WATER_POINTS_MAX {
        many.push([i as f64, 0.0]);
    }
    let e = add(many, 0.0).unwrap_err().to_string();
    assert!(e.contains("65 points"), "{e}");
    let e = add(vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]], 0.0);
    assert!(
        e.is_ok(),
        "a flat outline covers nothing but is not an error"
    );
    let e = add(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]], 0.0)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("water.outline[0]") && e.contains("0.01 m apart"),
        "{e}"
    );
    let e = add(vec![[0.0, 0.0], [1.0, 0.0], [1.0, f64::NAN]], 0.0)
        .unwrap_err()
        .to_string();
    assert!(e.contains("water.outline[2][1]"), "{e}");
    let e = add(vec![], f64::INFINITY).unwrap_err().to_string();
    assert!(e.contains("water.level_m"), "{e}");
    let e = add(vec![], 2e6).unwrap_err().to_string();
    assert!(e.contains("water.level_m") && e.contains("1000000"), "{e}");
}

#[test]
fn water_round_trips_through_the_file_and_leaves_other_worlds_as_they_were() {
    let mut w = World::new("w", 1);
    w.add_entity(EntityDraft {
        name: "Pond".into(),
        position: [4.0, 1.5, -2.0],
        components: vec![Component::Water(Water::new(0.25, square()))],
        ..EntityDraft::default()
    })
    .unwrap();
    let text = w.to_json().unwrap();
    assert!(
        text.contains("\"type\": \"water\"") && text.contains("\"level_m\": 0.25"),
        "{text}"
    );
    let back = World::from_json(&text).unwrap();
    assert_eq!(back, w);
    // The component alone, with its defaults left out.
    let json = serde_json::to_string(&Component::Water(Water::default())).unwrap();
    assert_eq!(json, r#"{"type":"water"}"#);
    let json = serde_json::to_string(&Component::Water(Water::new(1.0, square()))).unwrap();
    assert_eq!(
        json,
        r#"{"type":"water","level_m":1.0,"outline":[[-5.0,-5.0],[5.0,-5.0],[5.0,5.0],[-5.0,5.0]]}"#
    );
    let parsed: Component =
        serde_json::from_str(r#"{"type":"water","outline":[[0,0],[2,0],[2,2]]}"#).unwrap();
    assert_eq!(
        parsed,
        Component::Water(Water::new(0.0, vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]]))
    );
    // Unknown fields and a broken outline are refused.
    assert!(serde_json::from_str::<Component>(r#"{"type":"water","level":1.0}"#).is_err());
    let mut broken = World::new("w", 1);
    let e = broken
        .add_entity(EntityDraft {
            name: "Pond".into(),
            components: vec![Component::Water(Water::new(
                0.0,
                vec![[0.0, 0.0], [2.0, 0.0], [2.0, 0.0]],
            ))],
            ..EntityDraft::default()
        })
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("water.outline[2]") && e.contains("0.01 m apart"),
        "{e}"
    );
}

#[test]
fn an_area_of_water_is_read_from_its_entity_s_frame() {
    // Scaled and turned: the outline follows the entity as a road's points do.
    let mut w = World::new("w", 1);
    let id = w
        .add_entity(EntityDraft {
            name: "Pool".into(),
            position: [0.0, 0.0, 0.0],
            rotation_y_deg: 90.0,
            scale: crate::Scale::per_axis(2.0, 1.0, 4.0),
            components: vec![Component::Water(Water::new(0.0, square()))],
            ..EntityDraft::default()
        })
        .unwrap();
    let e = w.entity(id).unwrap();
    let outline = e.water().unwrap().outline_in_world(e);
    // Local x (scaled 2) turns to -z... local z (scaled 4) turns to +x: a 10 by 20 rectangle.
    for p in &outline {
        assert!(
            (p[0].abs() - 20.0).abs() < 1e-9 && (p[1].abs() - 10.0).abs() < 1e-9,
            "{p:?}"
        );
    }
    let areas = water_areas(&w);
    assert_eq!(areas[0].signed_area_m2().abs(), 800.0);
    assert!(
        areas[0].covers(19.0, 9.0) && !areas[0].covers(21.0, 0.0) && !areas[0].covers(0.0, 11.0)
    );
    // Moving the entity moves its water; an entity far above the limit has no surface.
    w.place_entity(id, [100.0, 5.0, 100.0], 0.0).unwrap();
    let areas = water_areas(&w);
    assert!(areas[0].covers(100.0 + 4.0, 100.0 + 4.0));
    assert!(!areas[0].covers(0.0, 0.0));
    w.entity_mut(id).unwrap().position[1] = 1e6;
    w.entity_mut(id).unwrap().water_mut().unwrap().level_m = 1.0;
    let areas = water_areas(&w);
    assert!(areas.is_empty(), "beyond the coordinate limit: no surface");
    // An area with an outline whose points collapsed (a zero scale is refused by the entity,
    // so through the file): fewer than three points cover nothing.
    let odd = WaterArea {
        entity: id,
        level: 0.0,
        outline: vec![[0.0, 0.0], [1.0, 0.0]],
    };
    assert!(!odd.covers(0.5, 0.0));
}
