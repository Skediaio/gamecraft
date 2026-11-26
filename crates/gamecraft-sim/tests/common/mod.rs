//! Small worlds built in code, shared by the integration tests.

#![allow(dead_code)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use gamecraft_math::{Portable, deg_to_rad};
use gamecraft_sim::{Input, Sim};
use gamecraft_world::{
    Character, Component, EntityDraft, EntityId, Shape, ShapeKind, Terrain, TerrainSource, World,
};

/// A world with flat terrain at `height_m`, 256 m square, cells of 4 m.
pub fn flat_world(height_m: f64) -> World {
    let mut world = World::new("test", 7);
    world.terrain = Some(Terrain {
        size_m: 256.0,
        cell_m: 4.0,
        source: TerrainSource::Flat { height_m },
        ..Default::default()
    });
    world
}

/// A world with rolling noise terrain, 256 m square, cells of 2 m.
pub fn noise_world(seed: u64, relief_m: f64, feature_m: f64) -> World {
    let mut world = World::new("noise", seed);
    world.terrain = Some(Terrain {
        size_m: 256.0,
        cell_m: 2.0,
        source: TerrainSource::Noise {
            seed,
            relief_m,
            feature_m,
            octaves: 3,
            roughness: 0.5,
        },
        ..Default::default()
    });
    world
}

/// Adds a default character standing at `position`.
pub fn add_character(world: &mut World, position: [f64; 3]) -> EntityId {
    add_character_with(world, position, Character::default())
}

/// Adds a character with the given parameters at `position`.
pub fn add_character_with(world: &mut World, position: [f64; 3], character: Character) -> EntityId {
    world
        .add_entity(EntityDraft {
            name: "hero".to_string(),
            position,
            components: vec![Component::Character(character)],
            ..EntityDraft::default()
        })
        .unwrap()
}

/// Adds a shape entity.
pub fn add_shape(
    world: &mut World,
    kind: ShapeKind,
    size: [f64; 3],
    position: [f64; 3],
    rotation_y_deg: f64,
    solid: bool,
) -> EntityId {
    world
        .add_entity(EntityDraft {
            name: kind.name().to_string(),
            position,
            rotation_y_deg,
            components: vec![Component::Shape(Shape {
                visible: true,
                kind,
                size,
                color: [0.5, 0.5, 0.5],
                solid,
            })],
            ..EntityDraft::default()
        })
        .unwrap()
}

/// An input walking along the facing `yaw_deg`.
pub fn walk(yaw_deg: f64) -> Input {
    Input {
        forward: 1.0,
        yaw_deg,
        ..Input::default()
    }
}

/// The position of an entity in the running world.
pub fn pos(sim: &Sim, id: EntityId) -> [f64; 3] {
    sim.world().entity(id).unwrap().position
}

/// Horizontal distance from point `p` (x, z) to the footprint of a box centred on `c` with half
/// extents `half` (x, z), rotated by `rotation_y_deg`. Negative inside.
pub fn box_distance(p: [f64; 2], c: [f64; 2], half: [f64; 2], rotation_y_deg: f64) -> f64 {
    let (s, co) = deg_to_rad(rotation_y_deg).psin_cos();
    let dx = p[0] - c[0];
    let dz = p[1] - c[1];
    // World to local: the inverse of the rotation counter-clockwise seen from above.
    let lx = dx * co - dz * s;
    let lz = dx * s + dz * co;
    let qx = lx.abs() - half[0];
    let qz = lz.abs() - half[1];
    if qx <= 0.0 && qz <= 0.0 {
        qx.max(qz)
    } else {
        let ox = qx.max(0.0);
        let oz = qz.max(0.0);
        (ox * ox + oz * oz).sqrt()
    }
}

/// Lets new bodies settle on the ground: one tick without input.
pub fn settle(sim: &mut Sim) {
    sim.step(1);
}
