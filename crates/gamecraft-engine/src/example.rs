//! The example world every spec's `example` runs on.

use gamecraft_world::{
    Character, CompareOp, Component, Condition, EntityDraft, Health, Launcher, Objective,
    ObjectiveStatus, Plant, Prefab, Shape, ShapeKind, Step, Tags, Terrain, TerrainSource, Trigger,
    VarValue, When, World, WorldError,
};

/// The plant library species of the example oak.
pub const EXAMPLE_SPECIES: &str = "quercus-robur";

/// A small valley: noise terrain of 128 m at 2 m cells (seed 7, relief 12 m); entity 1 a solid
/// box named `Crate` at `[-6, 0, 4]`, not on the ground (`entity.snap_to_ground` puts it there),
/// entity 2 a plant `Oak` (`quercus-robur`) on the ground at x 8, z -10,
/// entity 3 a character named `Scout` on the ground at (0, 0), facing north, with a default
/// `launcher` and `health`. One prefab, `marker` ("Marker": a cylinder 0.4 m wide and 1.5 m
/// high, tagged `marker`), placed nowhere. Its scenario: a
/// variable `score` = 0, an objective `find-oak` ("Find the oak") and a trigger `oak-found`
/// that marks it done when `score` changes to 3 or more.
pub fn example_world() -> World {
    build().unwrap_or_else(|_| World::new("Example valley", 7))
}

fn build() -> Result<World, WorldError> {
    let mut world = World::new("Example valley", 7);
    let terrain = Terrain {
        size_m: 128.0,
        cell_m: 2.0,
        source: TerrainSource::Noise {
            seed: 7,
            relief_m: 12.0,
            feature_m: 80.0,
            octaves: 4,
            roughness: 0.5,
        },
    };
    let field = terrain.build()?;
    world.terrain = Some(terrain);
    let on_ground = |x: f64, z: f64| [x, field.height_at(x, z).unwrap_or(0.0), z];
    world.add_entity(EntityDraft {
        name: "Crate".into(),
        position: [-6.0, 0.0, 4.0],
        components: vec![Component::Shape(Shape {
            kind: ShapeKind::Box,
            size: [1.0, 1.0, 1.0],
            color: [0.55, 0.4, 0.25],
            solid: true,
        })],
        ..EntityDraft::default()
    })?;
    world.add_entity(EntityDraft {
        name: "Oak".into(),
        position: on_ground(8.0, -10.0),
        components: vec![Component::Plant(Plant {
            species: EXAMPLE_SPECIES.into(),
            seed: 1,
            growth: 1.0,
        })],
        ..EntityDraft::default()
    })?;
    world.add_entity(EntityDraft {
        name: "Scout".into(),
        position: on_ground(0.0, 0.0),
        components: vec![
            Component::Character(Character::default()),
            Component::Launcher(Launcher::default()),
            Component::Health(Health::default()),
        ],
        ..EntityDraft::default()
    })?;
    world.put_prefab(Prefab {
        id: "marker".into(),
        name: "Marker".into(),
        components: vec![
            Component::Shape(Shape {
                kind: ShapeKind::Cylinder,
                size: [0.4, 1.5, 0.4],
                color: [0.9, 0.8, 0.1],
                solid: false,
            }),
            Component::Tags(Tags {
                tags: vec!["marker".into()],
            }),
        ],
        children: Vec::new(),
    })?;
    world.vars.insert("score".into(), VarValue::Number(0.0));
    world.objectives.push(Objective {
        id: "find-oak".into(),
        title: "Find the oak".into(),
        status: ObjectiveStatus::Open,
        hidden: false,
    });
    world.triggers.push(Trigger {
        id: "oak-found".into(),
        when: When::Var {
            name: "score".into(),
        },
        condition: Some(Condition::Compare {
            var: "score".into(),
            op: CompareOp::Ge,
            value: VarValue::Number(3.0),
        }),
        actions: vec![Step {
            command: "objective.set".into(),
            params: serde_json::json!({"id": "find-oak", "status": "done"}),
        }],
        once: true,
        enabled: true,
    });
    world.validate()?;
    Ok(world)
}
