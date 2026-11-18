//! `entity.scatter`: copies of a template placed on the ground, deterministically.

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
use gamecraft_engine::{EXAMPLE_SPECIES, Session};
use gamecraft_world::{Entity, EntityId, World};
use serde_json::{Value, json};

fn oak_template() -> Value {
    json!([{"type": "plant", "species": EXAMPLE_SPECIES, "seed": 5, "growth": 0.7}])
}

fn scatter(s: &mut Session, params: Value) -> Value {
    s.run("entity.scatter", &params).unwrap()
}

/// The entities the scatter result names, in id order.
fn placed<'a>(s: &'a Session, r: &Value) -> Vec<&'a Entity> {
    if r["placed"] == json!(0) {
        return Vec::new();
    }
    let (first, last) = (
        r["first_id"].as_u64().unwrap(),
        r["last_id"].as_u64().unwrap(),
    );
    s.current_world()
        .entities()
        .filter(|e| (first..=last).contains(&e.id.0))
        .collect()
}

#[test]
fn the_same_parameters_give_the_same_entities() {
    let params = json!({
        "count": 40, "components": oak_template(), "name": "Oak",
        "min_distance_m": 3, "scale_range": [0.5, 1.5], "max_slope_deg": 20
    });
    let mut a = example();
    let mut b = example();
    let ra = scatter(&mut a, params.clone());
    let rb = scatter(&mut b, params.clone());
    assert_eq!(ra, rb);
    assert_eq!(a.document(), b.document());
    assert_eq!(ra["requested"], json!(40));
    assert_eq!(ra["placed"], json!(40), "{ra}");
    assert_eq!(ra["first_id"], json!(4));
    assert_eq!(ra["last_id"], json!(43));

    // Another seed gives another layout; the default seed is the world's.
    let mut c = example();
    let mut with_seed = params.clone();
    with_seed["seed"] = json!(8);
    scatter(&mut c, with_seed);
    assert_ne!(c.document(), a.document());
    let mut d = example();
    let mut world_seed = params;
    world_seed["seed"] = json!(7);
    scatter(&mut d, world_seed);
    assert_eq!(d.document(), a.document());
}

#[test]
fn names_scale_rotation_and_seeds() {
    let mut s = example();
    let r = scatter(
        &mut s,
        json!({"count": 30, "components": oak_template(), "name": "Oak", "scale_range": [0.5, 1.5]}),
    );
    let es = placed(&s, &r);
    assert_eq!(es.len(), 30);
    for (i, e) in es.iter().enumerate() {
        assert_eq!(e.name, format!("Oak {}", i + 1));
        assert!((0.5..=1.5).contains(&e.scale), "{}", e.scale);
        assert!((0.0..360.0).contains(&e.rotation_y_deg));
        let p = e.plant().unwrap();
        assert_eq!((p.species.as_str(), p.growth), (EXAMPLE_SPECIES, 0.7));
    }
    let mut seeds: Vec<u64> = es.iter().map(|e| e.plant().unwrap().seed).collect();
    seeds.sort_unstable();
    seeds.dedup();
    assert!(seeds.len() > 25, "plant seeds vary: {seeds:?}");
    assert!(es.iter().any(|e| e.rotation_y_deg != es[0].rotation_y_deg));

    // Defaults: no name, scale 1, no random rotation, the template's seed.
    let mut s = example();
    let r = scatter(
        &mut s,
        json!({"count": 10, "components": oak_template(), "random_rotation": false, "vary_seed": false}),
    );
    for e in placed(&s, &r) {
        assert_eq!(e.name, "");
        assert_eq!((e.scale, e.rotation_y_deg), (1.0, 0.0));
        assert_eq!(e.plant().unwrap().seed, 5);
    }
}

#[test]
fn copies_stand_on_the_ground_inside_the_area() {
    let mut s = example();
    let r = scatter(
        &mut s,
        json!({"count": 50, "components": [{"type": "shape", "kind": "box"}], "center": [10, -20], "radius_m": 15}),
    );
    assert_eq!(r["placed"], json!(50));
    let field = s.current_heights().unwrap();
    for e in placed(&s, &r) {
        let [x, y, z] = e.position;
        let d = ((x - 10.0) * (x - 10.0) + (z + 20.0) * (z + 20.0)).sqrt();
        assert!(d <= 15.0 + 1e-9, "{d}");
        assert_eq!(y, field.height_at(x, z).unwrap());
    }

    // The whole terrain when no area is given: within 64 m of the origin on both axes, and
    // spread over it.
    let mut s = example();
    let r = scatter(
        &mut s,
        json!({"count": 200, "components": [{"type": "shape", "kind": "box"}]}),
    );
    let es = placed(&s, &r);
    assert!(
        es.iter()
            .all(|e| e.position[0].abs() <= 64.0 && e.position[2].abs() <= 64.0)
    );
    assert!(es.iter().any(|e| e.position[0] > 40.0) && es.iter().any(|e| e.position[0] < -40.0));
    assert!(es.iter().any(|e| e.position[2] > 40.0) && es.iter().any(|e| e.position[2] < -40.0));
}

#[test]
fn min_distance_and_max_slope_are_respected() {
    let mut s = example();
    let r = scatter(
        &mut s,
        json!({"count": 300, "components": oak_template(), "min_distance_m": 6, "max_slope_deg": 8}),
    );
    let placed_n = r["placed"].as_u64().unwrap();
    assert!(placed_n > 20, "{r}");
    let es = placed(&s, &r);
    assert_eq!(es.len() as u64, placed_n);
    for (i, a) in es.iter().enumerate() {
        for b in &es[i + 1..] {
            let (dx, dz) = (a.position[0] - b.position[0], a.position[2] - b.position[2]);
            let d = (dx * dx + dz * dz).sqrt();
            assert!(d >= 6.0, "{} and {} are {d} m apart", a.id, b.id);
        }
    }
    let positions: Vec<[f64; 3]> = es.iter().map(|e| e.position).collect();
    for [x, _, z] in positions {
        let g = s
            .run("query.ground_height", &json!({"x": x, "z": z}))
            .unwrap();
        let slope = g["slope_deg"].as_f64().unwrap();
        assert!(slope <= 8.0 + 1e-9, "slope {slope} at {x}, {z}");
    }

    // The example valley is steeper than 1 degree nearly everywhere: few or none fit, and the
    // command still ends.
    let mut s = example();
    let r = scatter(
        &mut s,
        json!({"count": 5000, "components": oak_template(), "max_slope_deg": 0.01}),
    );
    assert!(r["placed"].as_u64().unwrap() < 5000);
    // More than fit: bounded tries.
    let mut s = example();
    let r = scatter(
        &mut s,
        json!({"count": 5000, "components": oak_template(), "min_distance_m": 10}),
    );
    let n = r["placed"].as_u64().unwrap();
    assert!(n > 50 && n < 400, "{r}");
}

#[test]
fn one_undo_step_removes_all_of_it() {
    let mut s = example();
    let before = s.document().clone();
    let r = scatter(&mut s, json!({"count": 25, "components": oak_template()}));
    assert_eq!(s.document().entity_count(), 28);
    assert!(
        s.document()
            .entity(EntityId(r["last_id"].as_u64().unwrap()))
            .is_some()
    );
    s.undo().unwrap();
    assert_eq!(*s.document(), before);
    assert!(!s.can_undo());
}

#[test]
fn works_on_a_running_world() {
    let mut s = playing();
    let doc = s.document().clone();
    let r = scatter(
        &mut s,
        json!({"count": 5, "components": [{"type": "character"}], "name": "Guard"}),
    );
    assert_eq!(r["placed"], json!(5));
    assert_eq!(s.current_world().entity_count(), 8);
    assert_eq!(*s.document(), doc);
    // The new characters take part in the simulation.
    let state = s.run("sim.state", &json!({})).unwrap();
    assert_eq!(state["characters"].as_array().unwrap().len(), 6, "{state}");
}

#[test]
fn bad_parameters_are_named() {
    let mut s = example();
    let c = oak_template();
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 0, "components": c}),
        &["`count`", "5000"],
    );
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 5001, "components": c}),
        &["`count`"],
    );
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 3}),
        &["`components`"],
    );
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 3, "components": [{"type": "shap"}]}),
        &["components[0]", "shape"],
    );
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 3, "components": [{"type": "shape", "kind": "box"}, {"type": "shape", "kind": "box"}]}),
        &["shape"],
    );
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 3, "components": c, "center": [0, 0]}),
        &["`radius_m`"],
    );
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 3, "components": c, "radius_m": 5}),
        &["`center`"],
    );
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 3, "components": c, "center": [0, 0], "radius_m": 0}),
        &["`radius_m`"],
    );
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 3, "components": c, "min_distance_m": -1}),
        &["`min_distance_m`"],
    );
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 3, "components": c, "max_slope_deg": 91}),
        &["`max_slope_deg`", "90"],
    );
    for range in [
        json!([2, 1]),
        json!([0, 1]),
        json!([1]),
        json!([1, "a"]),
        json!([1, 2000]),
    ] {
        refuse(
            &mut s,
            "entity.scatter",
            json!({"count": 3, "components": c, "scale_range": range}),
            &["`scale_range`"],
        );
    }
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 3000, "components": c, "name": "n".repeat(198)}),
        &["`name`", "200"],
    );
    let mut bare = example().document().clone();
    bare.terrain = None;
    let mut s = Session::new(bare);
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 3, "components": c}),
        &["terrain", "terrain.set"],
    );
    let mut s = Session::new(World::new("", 0));
    refuse(
        &mut s,
        "entity.scatter",
        json!({"count": 3, "components": c}),
        &["terrain"],
    );
}

#[test]
fn an_area_outside_the_terrain_places_nothing() {
    let mut s = example();
    let r = scatter(
        &mut s,
        json!({"count": 3, "components": oak_template(), "center": [1000, 1000], "radius_m": 10}),
    );
    assert_eq!(
        r,
        json!({"placed": 0, "requested": 3, "first_id": null, "last_id": null})
    );
}

#[test]
fn exclude_keeps_circles_clear() {
    let mut s = example();
    let clear = [([0.0, 0.0], 25.0), ([40.0, -30.0], 15.0)];
    let r = scatter(
        &mut s,
        json!({"count": 80, "components": oak_template(), "name": "Oak", "seed": 3,
               "exclude": [{"center": [0, 0], "radius_m": 25}, {"center": [40, -30], "radius_m": 15}]}),
    );
    let es = placed(&s, &r);
    assert_eq!(es.len(), 80, "{r}");
    for e in &es {
        for ([cx, cz], radius) in clear {
            let (dx, dz) = (e.position[0] - cx, e.position[2] - cz);
            assert!(
                (dx * dx + dz * dz).sqrt() >= radius,
                "{} at {:?} is inside the clearing at [{cx}, {cz}]",
                e.name,
                e.position
            );
        }
    }
    // Numbered from 1 within the call, every name its own.
    let names: std::collections::BTreeSet<&str> = es.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names.len(), 80);
    assert!(names.contains("Oak 1") && names.contains("Oak 80"));
    // Without `exclude` the same seed puts copies there.
    let mut t = example();
    let r = scatter(
        &mut t,
        json!({"count": 80, "components": oak_template(), "seed": 3}),
    );
    assert!(placed(&t, &r).iter().any(|e| {
        let (dx, dz) = (e.position[0], e.position[2]);
        (dx * dx + dz * dz).sqrt() < 25.0
    }));
}

#[test]
fn bad_exclusions_are_named() {
    let mut s = example();
    let c = oak_template();
    for (exclude, needle) in [
        (json!({"center": [0, 0], "radius_m": 5}), "`exclude`"),
        (json!([{"center": [0, 0]}]), "`exclude[0].radius_m`"),
        (
            json!([{"center": [0, 0], "radius_m": -1}]),
            "`exclude[0].radius_m`",
        ),
        (
            json!([{"center": [0], "radius_m": 1}]),
            "`exclude[0].center`",
        ),
        (
            json!([{"center": [0, 0], "radius_m": 1, "size": 3}]),
            "`size`",
        ),
    ] {
        refuse(
            &mut s,
            "entity.scatter",
            json!({"count": 3, "components": c, "exclude": exclude}),
            &[needle],
        );
    }
}
