//! Generators: the recipe, its checks, and the oracles of what it places.

use crate::generator::{MAX_GENERATED_PER_GENERATOR, Trunk};
use crate::{
    Area, Circle, Component, Generator, GeneratorItem, GeneratorKind, Instance, Plant, Prefab,
    Shape, ShapeKind, Terrain, TerrainEdit, TerrainSource, World, WorldError,
};
use gamecraft_math::Portable;

fn plant(species: &str) -> Component {
    Component::Plant(Plant {
        species: species.into(),
        seed: 100,
        growth: 1.0,
    })
}

fn rock() -> Component {
    Component::Shape(Shape {
        kind: ShapeKind::Sphere,
        size: [1.0, 1.0, 1.0],
        color: [0.4, 0.4, 0.4],
        solid: true,
    })
}

fn forest(id: &str, center: [f64; 2], radius_m: f64, density: f64) -> Generator {
    Generator {
        id: id.into(),
        kind: GeneratorKind::Forest,
        area: Area::Circle(Circle { center, radius_m }),
        seed: 42,
        density_per_ha: density,
        min_distance_m: Some(3.0),
        slope_max_deg: None,
        height_range_m: None,
        exclude: Vec::new(),
        items: vec![
            GeneratorItem {
                growth_range: Some([0.5, 1.0]),
                ..GeneratorItem::of(3.0, vec![plant("quercus-robur")])
            },
            GeneratorItem::of(1.0, vec![plant("betula-pendula")]),
        ],
    }
}

fn flat_world(size_m: f64) -> World {
    let mut w = World::new("Woods", 1);
    w.terrain = Some(Terrain {
        size_m,
        cell_m: 4.0,
        source: TerrainSource::Flat { height_m: 2.0 },
        ..Default::default()
    });
    w
}

fn hilly_world() -> World {
    let mut w = World::new("Hills", 1);
    w.terrain = Some(Terrain {
        size_m: 1024.0,
        cell_m: 4.0,
        source: TerrainSource::Noise {
            seed: 5,
            relief_m: 120.0,
            feature_m: 250.0,
            octaves: 5,
            roughness: 0.5,
        },
        ..Default::default()
    });
    w
}

/// Regenerates `w` over its own terrain.
fn regen(w: &mut World) {
    let field = w.terrain.as_ref().map(|t| t.build().unwrap());
    w.regenerate(field.as_ref());
}

fn instances(w: &World, id: &str) -> Vec<Instance> {
    w.generated().set(id).unwrap().instances().to_vec()
}

#[test]
fn the_same_seed_gives_the_same_instances_in_the_same_order() {
    let mut a = flat_world(1024.0);
    a.generators.push(forest("woods", [0.0, 0.0], 300.0, 150.0));
    let mut b = a.clone();
    regen(&mut a);
    regen(&mut b);
    let (ia, ib) = (instances(&a, "woods"), instances(&b, "woods"));
    assert!(ia.len() > 1000, "{}", ia.len());
    assert_eq!(ia, ib);
    assert!(ia.windows(2).all(|w| w[0].number < w[1].number));
    // Another seed is another forest.
    let mut c = a.clone();
    c.generators[0].seed = 43;
    regen(&mut c);
    assert_ne!(instances(&c, "woods"), ia);
    // A clone shares the derived data; equality ignores it.
    assert_eq!(a, b);
}

#[test]
fn density_is_held_within_two_percent_on_open_ground() {
    let mut w = flat_world(1024.0);
    w.generators
        .push(forest("woods", [10.0, -20.0], 400.0, 120.0));
    regen(&mut w);
    let set = w.generated().set("woods").unwrap();
    let requested = w.generators[0].requested() as f64;
    let placed = set.instances().len() as f64;
    assert!(
        (placed - requested).abs() / requested < 0.02,
        "placed {placed} of {requested}"
    );
    // Weights 3 : 1, within a few percent.
    let per = set.per_item();
    let share = per[0] as f64 / placed;
    assert!((share - 0.75).abs() < 0.03, "{per:?}");
    // Spacing.
    let all = set.instances();
    for (i, a) in all.iter().enumerate().take(400) {
        for b in all.iter().skip(i + 1) {
            let (dx, dz) = (a.position[0] - b.position[0], a.position[2] - b.position[2]);
            assert!(dx * dx + dz * dz >= 9.0 - 1e-9, "{a:?} {b:?}");
        }
    }
}

#[test]
fn nothing_stands_inside_an_exclude_or_outside_the_area() {
    let mut w = flat_world(1024.0);
    let polygon = vec![
        [-300.0, -200.0],
        [250.0, -260.0],
        [300.0, 200.0],
        [-100.0, 320.0],
    ];
    let road = Area::Polygon(vec![
        [-400.0, -10.0],
        [400.0, -10.0],
        [400.0, 10.0],
        [-400.0, 10.0],
    ]);
    let clearing = Area::Circle(Circle {
        center: [100.0, 100.0],
        radius_m: 60.0,
    });
    let mut g = forest("woods", [0.0, 0.0], 1.0, 200.0);
    g.area = Area::Polygon(polygon.clone());
    g.exclude = vec![road.clone(), clearing.clone()];
    w.generators.push(g);
    regen(&mut w);
    let all = instances(&w, "woods");
    assert!(all.len() > 3000, "{}", all.len());
    let area = Area::Polygon(polygon);
    for i in &all {
        let p = [i.position[0], i.position[2]];
        assert!(area.contains(p), "{p:?} outside");
        assert!(!road.contains(p) && !clearing.contains(p), "{p:?} excluded");
        assert_eq!(i.position[1], 2.0, "on the ground");
    }
}

#[test]
fn slope_and_height_limits_are_held() {
    let mut w = hilly_world();
    let mut g = forest("woods", [0.0, 0.0], 450.0, 60.0);
    g.slope_max_deg = Some(12.0);
    g.height_range_m = Some([-20.0, 25.0]);
    w.generators.push(g);
    regen(&mut w);
    let field = w.terrain.as_ref().unwrap().build().unwrap();
    let all = instances(&w, "woods");
    let requested = w.generators[0].requested() as usize;
    assert!(all.len() > 100 && all.len() < requested, "{}", all.len());
    for i in &all {
        let [x, y, z] = i.position;
        assert_eq!(field.height_at(x, z), Some(y));
        assert!((-20.0..=25.0).contains(&y), "{y}");
        let [nx, ny, nz] = field.normal_at(x, z).unwrap();
        let slope = gamecraft_math::rad_to_deg(((nx * nx + nz * nz).sqrt() / ny).patan());
        assert!(slope <= 12.0 + 1e-9, "{slope}");
    }
}

#[test]
fn editing_the_terrain_under_a_forest_moves_only_the_affected_trees() {
    let mut w = hilly_world();
    let mut g = forest("woods", [0.0, 0.0], 450.0, 80.0);
    g.slope_max_deg = Some(20.0);
    w.generators.push(g);
    regen(&mut w);
    let before = instances(&w, "woods");
    let candidates = w.generated().set("woods").unwrap().candidate_count();
    let (center, radius) = ([120.0, -80.0], 40.0);
    w.terrain.as_mut().unwrap().edits.push(TerrainEdit::Raise {
        center,
        radius_m: radius,
        strength_m: 30.0,
        falloff: 0.8,
    });
    assert!(!w.generated_is_current());
    regen(&mut w);
    assert!(w.generated_is_current());
    let after = instances(&w, "woods");
    assert_eq!(
        w.generated().set("woods").unwrap().candidate_count(),
        candidates
    );
    // A cell's border reaches one cell beyond the brush for the slope.
    let reach = radius + 8.0;
    let outside = |i: &Instance| {
        let (dx, dz) = (i.position[0] - center[0], i.position[2] - center[1]);
        dx * dx + dz * dz > reach * reach
    };
    let keep_a: Vec<&Instance> = before.iter().filter(|i| outside(i)).collect();
    let keep_b: Vec<&Instance> = after.iter().filter(|i| outside(i)).collect();
    assert_eq!(keep_a, keep_b, "every tree away from the edit is untouched");
    let moved = before
        .iter()
        .filter(|i| !outside(i))
        .filter(|i| after.iter().all(|j| j != *i))
        .count();
    assert!(moved > 0, "the edit moved or removed trees under it");
}

#[test]
fn excluding_a_spot_keeps_every_other_number() {
    let mut w = flat_world(1024.0);
    w.generators.push(forest("woods", [0.0, 0.0], 300.0, 100.0));
    regen(&mut w);
    let before = instances(&w, "woods");
    let spot = Area::Circle(Circle {
        center: [50.0, 50.0],
        radius_m: 30.0,
    });
    w.generators[0].exclude.push(spot.clone());
    regen(&mut w);
    let after = instances(&w, "woods");
    let expected: Vec<Instance> = before
        .iter()
        .filter(|i| !spot.contains([i.position[0], i.position[2]]))
        .copied()
        .collect();
    assert_eq!(after, expected);
    assert!(after.len() < before.len());
}

#[test]
fn instances_become_entities_with_their_plant_variant_and_growth() {
    let mut w = flat_world(512.0);
    let mut g = forest("woods", [0.0, 0.0], 100.0, 200.0);
    g.items[0].variants = Some(2);
    w.generators.push(g);
    regen(&mut w);
    let set = w.generated().set("woods").unwrap();
    let mut seeds = std::collections::BTreeSet::new();
    let mut growths = std::collections::BTreeSet::new();
    for i in set.instances() {
        let e = set.entity(i);
        assert_eq!(e.name, format!("woods#{}", i.number));
        e.validate().unwrap();
        let p = e.plant().unwrap();
        if i.item == 0 {
            seeds.insert(p.seed);
            growths.insert(p.growth.to_bits());
            assert_eq!(p.species, "quercus-robur");
        }
        let t = set.trunk(i).unwrap();
        assert!(t.radius_m > 0.0 && t.height_m > 0.0);
    }
    assert_eq!(seeds.into_iter().collect::<Vec<_>>(), vec![100, 101]);
    assert_eq!(growths.len(), 3, "three growth classes");
    let first = set.instances()[0];
    let (s, i) = w
        .generated()
        .find(&format!("woods#{}", first.number))
        .unwrap();
    assert_eq!((s.generator().id.as_str(), *i), ("woods", first));
    assert!(w.generated().find("woods#0").is_none());
    assert!(w.generated().find("nope#1").is_none());
}

#[test]
fn near_and_along_find_every_instance_they_should() {
    let mut w = flat_world(1024.0);
    w.generators.push(forest("woods", [0.0, 0.0], 200.0, 300.0));
    regen(&mut w);
    let set = w.generated().set("woods").unwrap();
    let near = set.near([30.0, -40.0], 10.0);
    let brute: Vec<&Instance> = set
        .instances()
        .iter()
        .filter(|i| {
            let (dx, dz) = (i.position[0] - 30.0, i.position[2] + 40.0);
            (dx * dx + dz * dz).sqrt() <= 10.0 + set.reach_m()
        })
        .collect();
    assert_eq!(near, brute);
    // A ray along x at z = 5: every instance within reach of the line is offered.
    let along = set.along([-500.0, 3.0, 5.0], [1.0, 0.0, 0.0], 1000.0);
    for i in set.instances() {
        if (i.position[2] - 5.0).abs() <= set.reach_m() {
            assert!(along.contains(&i), "{i:?}");
        }
    }
    assert!(along.len() < set.instances().len() / 4);
    assert!(
        set.along([0.0, 3.0, 900.0], [1.0, 0.0, 0.0], 50.0)
            .is_empty()
    );
}

#[test]
fn a_scatter_takes_shapes_and_prefabs() {
    let mut w = flat_world(512.0);
    w.prefabs.push(Prefab {
        id: "boulder".into(),
        name: "Boulder".into(),
        components: vec![rock()],
        children: Vec::new(),
    });
    let mut g = forest("rocks", [0.0, 0.0], 100.0, 20.0);
    g.kind = GeneratorKind::Scatter;
    g.items = vec![
        GeneratorItem {
            scale_range: Some([0.5, 2.0]),
            ..GeneratorItem::of(1.0, vec![rock()])
        },
        GeneratorItem {
            prefab: Some("boulder".into()),
            ..GeneratorItem::of(1.0, Vec::new())
        },
    ];
    w.generators.push(g);
    w.validate().unwrap();
    regen(&mut w);
    let set = w.generated().set("rocks").unwrap();
    assert!(set.instances().len() > 40);
    for i in set.instances() {
        assert!(set.trunk(i).is_none(), "rocks have no trunk");
        assert!(set.has_body(i));
        assert!(set.entity(i).shape().is_some());
        assert!((0.5..=2.0).contains(&i.scale) || i.item == 1);
    }
    assert!(set.reach_m() >= 0.5);
}

#[test]
fn without_terrain_nothing_is_placed() {
    let mut w = World::new("Void", 1);
    w.generators.push(forest("woods", [0.0, 0.0], 50.0, 100.0));
    w.regenerate(None);
    assert_eq!(w.generated().count(), 0);
    assert!(w.generated_is_current());
}

#[test]
fn the_file_keeps_the_recipe_and_not_the_instances() {
    let mut w = flat_world(512.0);
    let mut g = forest("woods", [0.0, 0.0], 100.0, 200.0);
    g.items[1].trunk = Some(Trunk {
        radius_m: 0.3,
        height_m: 8.0,
    });
    g.exclude.push(Area::Circle(Circle {
        center: [0.0, 0.0],
        radius_m: 10.0,
    }));
    w.generators.push(g);
    regen(&mut w);
    let text = w.to_json().unwrap();
    assert!(text.contains("\"generators\""));
    assert!(!text.contains("woods#"));
    let back = World::from_json(&text).unwrap();
    assert_eq!(back, w);
    assert_eq!(back.generated().count(), 0, "derived again by the session");
    // The JSON form of an area.
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        v["generators"][0]["area"],
        serde_json::json!({"circle": {"center": [0.0, 0.0], "radius_m": 100.0}})
    );
    // A world without generators writes no `generators`.
    assert!(!flat_world(64.0).to_json().unwrap().contains("generators"));
}

#[test]
fn bad_entries_are_errors_that_name_the_generator_and_the_field() {
    let check = |edit: &dyn Fn(&mut Generator), wanted: &str| {
        let mut w = flat_world(512.0);
        let mut g = forest("woods", [0.0, 0.0], 100.0, 100.0);
        edit(&mut g);
        w.generators.push(g);
        let e = w.validate().unwrap_err().to_string();
        assert!(e.contains(wanted), "{e} lacks {wanted}");
    };
    check(&|g| g.id = "a b".into(), "generators.id");
    check(&|g| g.id = "a#1".into(), "generators.id");
    check(&|g| g.density_per_ha = 0.0, "density_per_ha");
    check(&|g| g.density_per_ha = f64::NAN, "density_per_ha");
    check(
        &|g| {
            g.area = Area::Circle(Circle {
                center: [0.0, 0.0],
                radius_m: 50_000.0,
            });
            g.density_per_ha = 10.0;
        },
        &MAX_GENERATED_PER_GENERATOR.to_string(),
    );
    check(
        &|g| g.area = Area::Polygon(vec![[0.0, 0.0], [1.0, 1.0]]),
        "polygon",
    );
    check(
        &|g| g.area = Area::Polygon(vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]]),
        "no area",
    );
    check(&|g| g.items.clear(), "items");
    check(&|g| g.items[0].components = vec![rock()], "no `plant`");
    check(&|g| g.items[0].weight = -1.0, "items[0].weight");
    check(&|g| g.items[1].variants = Some(0), "items[1].variants");
    check(
        &|g| g.items[0].growth_range = Some([0.0, 1.0]),
        "growth_range",
    );
    check(
        &|g| g.items[0].prefab = Some("nope".into()),
        "both `prefab`",
    );
    check(
        &|g| {
            g.items[0].components.clear();
            g.items[0].prefab = Some("nope".into());
        },
        "no prefab has that id",
    );
    check(&|g| g.slope_max_deg = Some(91.0), "slope_max_deg");
    check(&|g| g.height_range_m = Some([5.0, 1.0]), "height_range_m");
    check(&|g| g.min_distance_m = Some(-1.0), "min_distance_m");
    check(
        &|g| {
            g.items[0].trunk = Some(Trunk {
                radius_m: f64::INFINITY,
                height_m: 1.0,
            })
        },
        "trunk",
    );
    check(
        &|g| {
            g.items[0].components = vec![
                plant("quercus-robur"),
                Component::Character(crate::Character::default()),
            ]
        },
        "character",
    );
    // Two generators with one id.
    let mut w = flat_world(512.0);
    w.generators.push(forest("woods", [0.0, 0.0], 10.0, 1.0));
    w.generators.push(forest("woods", [0.0, 0.0], 10.0, 1.0));
    assert!(matches!(w.validate(), Err(WorldError::Generator { .. })));
    // Unknown fields and a bad kind in the file.
    let text = r#"{"gamecraft": 1, "generators": [{"id": "a", "kind": "jungle", "area": {"circle": {"center": [0, 0], "radius_m": 5}}, "density_per_ha": 1, "items": []}]}"#;
    assert!(
        World::from_json(text)
            .unwrap_err()
            .to_string()
            .contains("jungle")
    );
    let text = r#"{"gamecraft": 1, "generators": [{"id": "a", "kind": "forest", "area": {"square": 5}, "density_per_ha": 1, "items": []}]}"#;
    assert!(World::from_json(text).is_err());
}
