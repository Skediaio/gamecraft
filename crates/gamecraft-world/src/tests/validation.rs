//! Every validation rule: the error names the field (and the entity, for entity fields).

use super::full_world;
use crate::{Component, EntityDraft, EntityId, ShapeKind, Terrain, TerrainSource, World};

type Mutation = fn(&mut World);

fn entity(w: &mut World, id: u64) -> &mut crate::Entity {
    w.entity_mut(EntityId(id)).unwrap()
}

fn shape(w: &mut World, id: u64) -> &mut crate::Shape {
    match entity(w, id).components.first_mut() {
        Some(Component::Shape(s)) => s,
        _ => panic!("entity {id} has no shape first"),
    }
}

fn character(w: &mut World) -> &mut crate::Character {
    match entity(w, 3).components.first_mut() {
        Some(Component::Character(c)) => c,
        _ => panic!("no character"),
    }
}

fn plant(w: &mut World) -> &mut crate::Plant {
    match entity(w, 2).components.first_mut() {
        Some(Component::Plant(p)) => p,
        _ => panic!("no plant"),
    }
}

fn noise(w: &mut World) -> (&mut f64, &mut f64, &mut u32, &mut f64) {
    match &mut w.terrain {
        Some(Terrain {
            source:
                TerrainSource::Noise {
                    relief_m,
                    feature_m,
                    octaves,
                    roughness,
                    ..
                },
            ..
        }) => (relief_m, feature_m, octaves, roughness),
        _ => panic!("no noise terrain"),
    }
}

fn terrain(w: &mut World) -> &mut Terrain {
    w.terrain.as_mut().unwrap()
}

/// (description, mutation, text the error must contain)
fn cases() -> Vec<(&'static str, Mutation, &'static [&'static str])> {
    vec![
        (
            "world name",
            |w| w.name = "x".repeat(201),
            &["name", "201 characters", "200"],
        ),
        (
            "terrain size small",
            |w| terrain(w).size_m = 4.0,
            &["terrain.size_m", "4", "8"],
        ),
        (
            "terrain size large",
            |w| terrain(w).size_m = 20000.0,
            &["terrain.size_m", "16384"],
        ),
        (
            "terrain size nan",
            |w| terrain(w).size_m = f64::NAN,
            &["terrain.size_m", "NaN"],
        ),
        (
            "terrain cell small",
            |w| terrain(w).cell_m = 0.1,
            &["terrain.cell_m", "0.25"],
        ),
        (
            "terrain cell large",
            |w| terrain(w).cell_m = 100.0,
            &["terrain.cell_m", "64"],
        ),
        (
            "terrain cell not a divisor",
            |w| terrain(w).cell_m = 3.0,
            &["terrain.cell_m", "3", "multiple"],
        ),
        (
            "terrain too many samples",
            |w| {
                terrain(w).size_m = 16384.0;
                terrain(w).cell_m = 0.25;
            },
            &["terrain.cell_m", "65537", "2049", "use cell_m 8"],
        ),
        (
            "flat height",
            |w| terrain(w).source = TerrainSource::Flat { height_m: 2e6 },
            &["terrain.source.height_m", "2000000"],
        ),
        (
            "relief",
            |w| *noise(w).0 = 6000.0,
            &["terrain.source.relief_m", "5000"],
        ),
        (
            "feature",
            |w| *noise(w).1 = 0.5,
            &["terrain.source.feature_m", "0.5"],
        ),
        (
            "octaves low",
            |w| *noise(w).2 = 0,
            &["terrain.source.octaves", "0", "10"],
        ),
        (
            "octaves high",
            |w| *noise(w).2 = 11,
            &["terrain.source.octaves", "11"],
        ),
        (
            "roughness",
            |w| *noise(w).3 = 1.5,
            &["terrain.source.roughness", "1.5"],
        ),
        (
            "time of day 24",
            |w| w.environment.time_of_day_h = 24.0,
            &["environment.time_of_day_h", "24"],
        ),
        (
            "time of day negative",
            |w| w.environment.time_of_day_h = -1.0,
            &["environment.time_of_day_h"],
        ),
        (
            "day of year 0",
            |w| w.environment.day_of_year = 0,
            &["environment.day_of_year", "366"],
        ),
        (
            "day of year 367",
            |w| w.environment.day_of_year = 367,
            &["environment.day_of_year", "367"],
        ),
        (
            "latitude",
            |w| w.environment.latitude_deg = 91.0,
            &["environment.latitude_deg", "91"],
        ),
        (
            "wind direction",
            |w| w.environment.wind_direction_deg = f64::INFINITY,
            &["environment.wind_direction_deg", "inf"],
        ),
        (
            "wind speed",
            |w| w.environment.wind_speed_mps = 101.0,
            &["environment.wind_speed_mps", "100"],
        ),
        (
            "cloud cover",
            |w| w.environment.cloud_cover = 1.1,
            &["environment.cloud_cover"],
        ),
        ("mist", |w| w.environment.mist = -0.1, &["environment.mist"]),
        (
            "rain",
            |w| w.environment.rain = f64::NAN,
            &["environment.rain"],
        ),
        (
            "entity name",
            |w| entity(w, 1).name = "é".repeat(201),
            &["entity 1", "name"],
        ),
        (
            "position nan",
            |w| entity(w, 1).position[1] = f64::NAN,
            &["entity 1", "position[1]"],
        ),
        (
            "position far",
            |w| entity(w, 1).position[2] = -2e6,
            &["entity 1", "position[2]", "1000000"],
        ),
        (
            "rotation",
            |w| entity(w, 1).rotation_y_deg = f64::INFINITY,
            &["entity 1", "rotation_y_deg"],
        ),
        (
            "scale small",
            |w| entity(w, 1).scale = 0.0,
            &["entity 1", "scale", "0.001"],
        ),
        (
            "scale large",
            |w| entity(w, 1).scale = 1001.0,
            &["entity 1", "scale", "1000"],
        ),
        (
            "entity id mismatch",
            |w| entity(w, 1).id = EntityId(50),
            &["entity 1", "id", "50"],
        ),
        (
            "duplicate component",
            |w| {
                let c = entity(w, 1).components[0].clone();
                entity(w, 1).components.push(c);
            },
            &["entity 1", "shape", "at most one"],
        ),
        (
            "shape size small",
            |w| shape(w, 1).size[0] = 0.0,
            &["entity 1", "shape.size[0]", "0.001"],
        ),
        (
            "shape size large",
            |w| shape(w, 1).size[2] = 20000.0,
            &["entity 1", "shape.size[2]", "10000"],
        ),
        (
            "sphere unequal",
            |w| shape(w, 4).size[1] = 3.0,
            &["entity 4", "shape.size", "sphere"],
        ),
        (
            "cylinder unequal",
            |w| shape(w, 5).size[2] = 0.7,
            &["entity 5", "shape.size", "cylinder"],
        ),
        (
            "color",
            |w| shape(w, 1).color[2] = 1.5,
            &["entity 1", "shape.color[2]", "1.5"],
        ),
        (
            "color nan",
            |w| shape(w, 1).color[0] = f32::NAN,
            &["entity 1", "shape.color[0]"],
        ),
        (
            "species empty",
            |w| plant(w).species = String::new(),
            &["entity 2", "plant.species"],
        ),
        (
            "species long",
            |w| plant(w).species = "q".repeat(300),
            &["entity 2", "plant.species"],
        ),
        (
            "growth zero",
            |w| plant(w).growth = 0.0,
            &["entity 2", "plant.growth", "0"],
        ),
        (
            "growth above one",
            |w| plant(w).growth = 1.5,
            &["entity 2", "plant.growth", "1.5"],
        ),
        (
            "height",
            |w| character(w).height_m = 0.0,
            &["entity 3", "character.height_m"],
        ),
        (
            "radius",
            |w| character(w).radius_m = -0.3,
            &["entity 3", "character.radius_m"],
        ),
        (
            "eye",
            |w| character(w).eye_height_m = f64::NAN,
            &["entity 3", "character.eye_height_m"],
        ),
        (
            "walk",
            |w| character(w).walk_speed_mps = 0.0,
            &["entity 3", "character.walk_speed_mps"],
        ),
        (
            "run",
            |w| character(w).run_speed_mps = -1.0,
            &["entity 3", "character.run_speed_mps"],
        ),
        (
            "jump",
            |w| character(w).jump_speed_mps = f64::INFINITY,
            &["entity 3", "character.jump_speed_mps"],
        ),
        (
            "slope low",
            |w| character(w).max_slope_deg = 0.5,
            &["entity 3", "character.max_slope_deg", "89"],
        ),
        (
            "slope high",
            |w| character(w).max_slope_deg = 90.0,
            &["entity 3", "character.max_slope_deg", "90"],
        ),
        (
            "radius vs height",
            |w| character(w).radius_m = 1.0,
            &["entity 3", "character.radius_m", "height_m"],
        ),
        (
            "eye vs height",
            |w| character(w).eye_height_m = 2.0,
            &["entity 3", "character.eye_height_m", "height_m"],
        ),
    ]
}

#[test]
fn the_base_world_is_valid() {
    full_world().validate().unwrap();
    assert_eq!(
        full_world()
            .entity(EntityId(4))
            .unwrap()
            .shape()
            .unwrap()
            .kind,
        ShapeKind::Sphere
    );
}

#[test]
fn every_rule_names_its_field() {
    for (what, mutate, needles) in cases() {
        let mut w = full_world();
        mutate(&mut w);
        let e = match w.validate() {
            Ok(()) => panic!("{what}: accepted"),
            Err(e) => e.to_string(),
        };
        for n in needles {
            assert!(e.contains(n), "{what}: {e:?} does not contain {n:?}");
        }
    }
}

#[test]
fn entity_rules_also_apply_on_add_and_on_load() {
    for (what, mutate, needles) in cases() {
        if what == "entity id mismatch" {
            continue; // in a file the id is the key, so this case cannot be written
        }
        let mut w = full_world();
        mutate(&mut w);
        let text = serde_json::to_string(&probe(&w)).unwrap_or_default();
        if text.contains("null") || text.contains("NaN") || text.is_empty() {
            continue; // non-finite numbers cannot be written to JSON at all
        }
        let e = match World::from_json(&text) {
            Ok(_) => panic!("{what}: loaded"),
            Err(e) => e.to_string(),
        };
        let first = needles.first().copied().unwrap_or("");
        assert!(e.contains(first), "{what}: {e}");
    }
    let mut w = World::new("Add", 0);
    let e = w
        .add_entity(EntityDraft {
            position: [0.0, f64::NAN, 0.0],
            ..EntityDraft::default()
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("position[1]"), "{e}");
}

/// The world as JSON without going through `to_json` (which validates first).
fn probe(w: &World) -> serde_json::Value {
    let entities: Vec<&crate::Entity> = w.entities().collect();
    serde_json::json!({
        "gamecraft": 1,
        "name": w.name,
        "seed": w.seed,
        "terrain": w.terrain,
        "environment": w.environment,
        "next_entity_id": w.next_entity_id(),
        "entities": entities,
    })
}

#[test]
fn too_many_entities_is_an_error() {
    let mut text = String::from(r#"{"gamecraft": 1, "entities": ["#);
    for i in 1..=(crate::limits::MAX_ENTITIES + 1) {
        if i > 1 {
            text.push(',');
        }
        text.push_str(&format!("{{\"id\":{i}}}"));
    }
    text.push_str("]}");
    let e = World::from_json(&text).unwrap_err().to_string();
    assert!(e.contains("1000001") && e.contains("1000000"), "{e}");
}
