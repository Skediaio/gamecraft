//! Unit tests of the crate, one file per area.

mod combat;
mod document;
mod fuzz;
mod layer;
mod model;
mod prefab;
mod scenario;
mod sculpt;
mod sun;
mod terrain;
mod validation;
mod view;

use crate::{
    Character, Component, EntityDraft, Environment, Plant, Shape, ShapeKind, Terrain,
    TerrainSource, World,
};

/// The example document of the format specification, compact.
pub(crate) const EXAMPLE_COMPACT: &str = r#"{
  "gamecraft": 1,
  "name": "Valley",
  "seed": 7,
  "terrain": {"size_m": 512.0, "cell_m": 2.0, "source": {"kind": "noise", "seed": 7, "relief_m": 40.0, "feature_m": 200.0, "octaves": 5, "roughness": 0.5}},
  "environment": {"time_of_day_h": 14.0, "day_of_year": 172, "latitude_deg": 50.85, "wind_direction_deg": 250.0, "wind_speed_mps": 3.0, "cloud_cover": 0.3, "mist": 0.0, "rain": 0.0},
  "next_entity_id": 3,
  "entities": [
    {"id": 1, "name": "Crate", "position": [4.0, 12.5, -3.0], "rotation_y_deg": 30.0, "scale": 1.0,
     "components": [{"type": "shape", "kind": "box", "size": [1.0, 1.0, 1.0], "color": [0.6, 0.5, 0.4], "solid": true}]},
    {"id": 2, "name": "Oak", "position": [10.0, 11.0, 5.0], "rotation_y_deg": 0.0, "scale": 1.0,
     "components": [{"type": "plant", "species": "quercus-robur", "seed": 11, "growth": 1.0}]}
  ]
}"#;

/// The same document as `to_json` prints it.
pub(crate) const EXAMPLE_PRETTY: &str = r#"{
  "gamecraft": 1,
  "name": "Valley",
  "seed": 7,
  "terrain": {
    "size_m": 512.0,
    "cell_m": 2.0,
    "source": {
      "kind": "noise",
      "seed": 7,
      "relief_m": 40.0,
      "feature_m": 200.0,
      "octaves": 5,
      "roughness": 0.5
    }
  },
  "environment": {
    "time_of_day_h": 14.0,
    "day_of_year": 172,
    "latitude_deg": 50.85,
    "wind_direction_deg": 250.0,
    "wind_speed_mps": 3.0,
    "cloud_cover": 0.3,
    "mist": 0.0,
    "rain": 0.0
  },
  "next_entity_id": 3,
  "entities": [
    {
      "id": 1,
      "name": "Crate",
      "position": [
        4.0,
        12.5,
        -3.0
      ],
      "rotation_y_deg": 30.0,
      "scale": 1.0,
      "components": [
        {
          "type": "shape",
          "kind": "box",
          "size": [
            1.0,
            1.0,
            1.0
          ],
          "color": [
            0.6,
            0.5,
            0.4
          ],
          "solid": true
        }
      ]
    },
    {
      "id": 2,
      "name": "Oak",
      "position": [
        10.0,
        11.0,
        5.0
      ],
      "rotation_y_deg": 0.0,
      "scale": 1.0,
      "components": [
        {
          "type": "plant",
          "species": "quercus-robur",
          "seed": 11,
          "growth": 1.0
        }
      ]
    }
  ]
}"#;

/// The example document built through the API.
pub(crate) fn example_world() -> World {
    let mut w = World::new("Valley", 7);
    w.terrain = Some(Terrain {
        size_m: 512.0,
        cell_m: 2.0,
        source: TerrainSource::Noise {
            seed: 7,
            relief_m: 40.0,
            feature_m: 200.0,
            octaves: 5,
            roughness: 0.5,
        },
        ..Default::default()
    });
    w.environment = Environment::default();
    let crate_id = w
        .add_entity(EntityDraft {
            name: "Crate".into(),
            position: [4.0, 12.5, -3.0],
            rotation_y_deg: 30.0,
            scale: 1.0,
            components: vec![Component::Shape(Shape {
                kind: ShapeKind::Box,
                size: [1.0, 1.0, 1.0],
                color: [0.6, 0.5, 0.4],
                solid: true,
            })],
            layer: None,
        })
        .unwrap();
    assert_eq!(crate_id.0, 1);
    let oak = w
        .add_entity(EntityDraft {
            name: "Oak".into(),
            position: [10.0, 11.0, 5.0],
            components: vec![Component::Plant(Plant {
                species: "quercus-robur".into(),
                seed: 11,
                growth: 1.0,
            })],
            ..EntityDraft::default()
        })
        .unwrap();
    assert_eq!(oak.0, 2);
    w
}

/// A world with one entity of every component type, valid.
pub(crate) fn full_world() -> World {
    let mut w = example_world();
    w.add_entity(EntityDraft {
        name: "Walker".into(),
        position: [0.0, 10.0, 0.0],
        components: vec![Component::Character(Character::default())],
        ..EntityDraft::default()
    })
    .unwrap();
    w.add_entity(EntityDraft {
        name: "Ball".into(),
        components: vec![Component::Shape(Shape {
            kind: ShapeKind::Sphere,
            size: [2.0, 2.0, 2.0],
            color: [1.0, 0.0, 0.0],
            solid: false,
        })],
        ..EntityDraft::default()
    })
    .unwrap();
    w.add_entity(EntityDraft {
        name: "Pillar".into(),
        components: vec![Component::Shape(Shape {
            kind: ShapeKind::Cylinder,
            size: [0.5, 3.0, 0.5],
            color: [0.5, 0.5, 0.5],
            solid: true,
        })],
        ..EntityDraft::default()
    })
    .unwrap();
    w
}
