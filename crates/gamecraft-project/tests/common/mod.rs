//! Helpers shared by the tests of projects and packs.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gamecraft_project::{PackSource, Project, SavePlan};
use gamecraft_world::{
    Component, EntityDraft, Health, Prefab, PrefabChild, Shape, Spawn, Tags, World,
};
use std::collections::BTreeMap;

/// A pack's files from `(path, text)` pairs.
pub fn pack(location: &str, files: &[(&str, &str)]) -> PackSource {
    PackSource {
        location: location.to_string(),
        files: files
            .iter()
            .map(|(p, t)| ((*p).to_string(), (*t).to_string()))
            .collect(),
        assets: BTreeMap::new(),
    }
}

/// A pack's files from a save plan's writes.
pub fn pack_of(location: &str, plan: &SavePlan) -> PackSource {
    PackSource {
        location: location.to_string(),
        files: plan.writes.clone(),
        assets: BTreeMap::new(),
    }
}

/// Packs by id.
pub fn packs(list: Vec<(&str, PackSource)>) -> BTreeMap<String, PackSource> {
    list.into_iter()
        .map(|(id, p)| (id.to_string(), p))
        .collect()
}

/// Applies a save plan to a pack's files.
pub fn apply(source: &mut PackSource, plan: &SavePlan) {
    for (path, text) in &plan.writes {
        source.files.insert(path.clone(), text.clone());
    }
    for path in &plan.removes {
        source.files.remove(path);
    }
}

/// A small world with a prefab, two instances (one with a child), plain entities in two
/// layers and one in the default layer.
pub fn village() -> World {
    let mut w = World::new("Village", 9);
    w.environment.time_of_day_h = 10.0;
    w.put_prefab(Prefab {
        id: "soldier".into(),
        name: "Soldier".into(),
        components: vec![
            Component::Health(Health::default()),
            Component::Tags(Tags {
                tags: vec!["enemy".into()],
            }),
        ],
        children: vec![PrefabChild {
            name: "Hat".into(),
            offset: [0.0, 1.8, 0.0],
            rotation_y_deg: 0.0,
            components: vec![],
        }],
    })
    .unwrap();
    w.put_prefab(Prefab {
        id: "house".into(),
        name: "House".into(),
        components: vec![Component::Shape(Shape::default())],
        children: vec![],
    })
    .unwrap();
    for (name, x) in [("House 1", 0.0), ("House 2", 10.0)] {
        w.spawn_prefab(
            "house",
            Spawn {
                name: Some(name.into()),
                position: [x, 0.0, 0.0],
                layer: Some("village".into()),
                ..Spawn::default()
            },
        )
        .unwrap();
    }
    w.spawn_prefab(
        "soldier",
        Spawn {
            name: Some("Guard".into()),
            position: [5.0, 0.0, 5.0],
            layer: Some("enemies".into()),
            overrides: vec![Component::Health(Health {
                current: 50.0,
                ..Health::default()
            })],
            ..Spawn::default()
        },
    )
    .unwrap();
    for i in 0..3 {
        w.add_entity(EntityDraft {
            name: format!("Rock {i}"),
            position: [f64::from(i) * 3.0, 0.0, -20.0],
            layer: Some("terrain".into()),
            components: vec![Component::Shape(Shape::default())],
            ..EntityDraft::default()
        })
        .unwrap();
    }
    w.add_entity(EntityDraft {
        name: "Sign".into(),
        position: [1.0, 0.0, 1.0],
        ..EntityDraft::default()
    })
    .unwrap();
    w
}

/// The world without layers, its entities keyed by name with their parent's name: what two
/// worlds must share to be the same game, whatever ids and layers they use.
pub fn canonical(
    w: &World,
) -> (
    World,
    Vec<(String, Option<String>, gamecraft_world::Entity)>,
) {
    let mut fields_only = World::new(&w.name, w.seed);
    fields_only.terrain = w.terrain.clone();
    fields_only.environment = w.environment.clone();
    fields_only.vars = w.vars.clone();
    fields_only.objectives = w.objectives.clone();
    fields_only.triggers = w.triggers.clone();
    fields_only.prefabs = w.prefabs.clone();
    fields_only.prefabs.sort_by(|a, b| a.id.cmp(&b.id));
    let mut entities: Vec<(String, Option<String>, gamecraft_world::Entity)> = w
        .entities()
        .map(|e| {
            let parent = e.parent.and_then(|p| w.entity(p)).map(|p| p.name.clone());
            let mut e = e.clone();
            e.id = gamecraft_world::EntityId(0);
            e.parent = None;
            e.layer = None;
            (e.name.clone(), parent, e)
        })
        .collect();
    entities.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(format!("{:?}", a.2).cmp(&format!("{:?}", b.2)))
    });
    (fields_only, entities)
}

/// Loads a project of one pack.
pub fn load_one(id: &str, source: &PackSource) -> (Project, World) {
    Project::load(id, &packs(vec![(id, source.clone())]), None).unwrap()
}
