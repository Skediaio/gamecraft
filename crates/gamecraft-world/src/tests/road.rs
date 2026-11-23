use crate::limits::{ROAD_TIER_STEP_M, TERRAIN_CELL_MIN_M};
use crate::{
    Component, Entity, EntityDraft, EntityId, HeightField, Road, RoadEnds, Terrain, TerrainSource,
    World, WorldError, free_road_tier, raycast_roads, retier_roads,
};

/// A terrain of 128 m at 2 m cells: a slope of 1 in 10 along x and 1 in 20 along z with
/// rolling noise on top, so no cell is flat and no two cells are coplanar.
fn hilly() -> Terrain {
    let cols = 9u32;
    let heights: Vec<f64> = (0..cols * cols)
        .map(|i| {
            let (c, r) = (f64::from(i % cols), f64::from(i / cols));
            c * 1.6 + r * 0.8 + if (i * 7) % 3 == 0 { 1.1 } else { -0.4 }
        })
        .collect();
    Terrain::new(
        128.0,
        2.0,
        TerrainSource::Grid {
            cols,
            rows: cols,
            heights: heights.into(),
        },
    )
}

fn world_with(terrain: Terrain, roads: Vec<Road>) -> (World, HeightField, Vec<EntityId>) {
    let mut world = World::new("roads", 1);
    let field = terrain.build().unwrap();
    world.terrain = Some(terrain);
    let ids = roads
        .into_iter()
        .map(|r| {
            world
                .add_entity(EntityDraft {
                    components: vec![Component::Road(r)],
                    ..EntityDraft::default()
                })
                .unwrap()
        })
        .collect();
    world.validate().unwrap();
    (world, field, ids)
}

fn road_of(world: &World, id: EntityId) -> (&Entity, &Road) {
    let e = world.entity(id).unwrap();
    (e, e.road().unwrap())
}

#[test]
fn a_minimal_road_takes_its_defaults_and_round_trips() {
    let c: Component =
        serde_json::from_str(r#"{"type": "road", "points": [[0, 0], [10, 0]]}"#).unwrap();
    let Component::Road(r) = &c else {
        panic!("not a road: {c:?}")
    };
    assert_eq!(r, &Road::new(vec![[0.0, 0.0], [10.0, 0.0]]));
    assert_eq!(
        (r.width_m, r.lift_m, r.tier, r.ends),
        (6.0, 0.03, 0, RoadEnds::Round)
    );
    let text = serde_json::to_string(&c).unwrap();
    assert_eq!(serde_json::from_str::<Component>(&text).unwrap(), c);
    let err = serde_json::from_str::<Component>(
        r#"{"type": "road", "points": [[0, 0], [1, 0]], "wide": 3}"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("wide"), "{err}");
}

#[test]
fn bad_roads_are_refused_naming_the_field() {
    let base = Road::new(vec![[0.0, 0.0], [10.0, 0.0], [20.0, 5.0]]);
    let cases: Vec<(Road, &str)> = vec![
        (Road::new(vec![[0.0, 0.0]]), "road.points"),
        (
            Road::new(vec![[0.0, 0.0], [f64::NAN, 1.0]]),
            "road.points[1][0]",
        ),
        (Road::new(vec![[0.0, 0.0], [0.0, 0.001]]), "road.points[1]"),
        (Road::new(vec![[0.0, 0.0], [2e6, 0.0]]), "road.points[1][0]"),
        (
            Road {
                width_m: 0.0,
                ..base.clone()
            },
            "road.width_m",
        ),
        (
            Road {
                widths_m: vec![4.0, 5.0],
                ..base.clone()
            },
            "road.widths_m",
        ),
        (
            Road {
                widths_m: vec![4.0, 500.0, 4.0],
                ..base.clone()
            },
            "road.widths_m[1]",
        ),
        (
            Road {
                lift_m: 0.0,
                ..base.clone()
            },
            "road.lift_m",
        ),
        (
            Road {
                tier: 8,
                ..base.clone()
            },
            "road.tier",
        ),
        (
            Road {
                color: [0.1, 2.0, 0.1],
                ..base.clone()
            },
            "road.color[1]",
        ),
    ];
    for (road, field) in cases {
        let mut world = World::new("bad", 1);
        let err = world
            .add_entity(EntityDraft {
                components: vec![Component::Road(road)],
                ..EntityDraft::default()
            })
            .unwrap_err();
        match &err {
            WorldError::Invalid { field: f, .. } => assert_eq!(f, field, "{err}"),
            other => panic!("{other}"),
        }
    }
}

#[test]
fn the_surface_follows_a_hilly_terrain_exactly() {
    let road = Road {
        widths_m: vec![5.0, 7.0, 6.0, 4.0],
        ..Road::new(vec![
            [-50.0, -40.0],
            [-10.0, -35.0],
            [20.0, 10.0],
            [45.0, 50.0],
        ])
    };
    let (world, field, ids) = world_with(hilly(), vec![road]);
    let (e, r) = road_of(&world, ids[0]);
    let mesh = r.mesh(e, &field).unwrap();
    assert!(mesh.stats.triangles > 200, "{:?}", mesh.stats);
    assert_eq!(mesh.indices.len() as u64, mesh.stats.triangles * 3);
    let offset = r.surface_offset_m();
    let above = |p: [f64; 3]| p[1] - field.height_at(p[0], p[2]).unwrap();
    for p in &mesh.positions {
        assert!((above(*p) - offset).abs() < 1e-9, "{p:?}: {}", above(*p));
    }
    // Between the vertices too: every triangle's centroid and edge midpoints lie on the
    // ground plus the offset, because no triangle spans two terrain triangles.
    for [a, b, c] in mesh.triangles() {
        let mid = |p: [f64; 3], q: [f64; 3]| {
            [
                (p[0] + q[0]) / 2.0,
                (p[1] + q[1]) / 2.0,
                (p[2] + q[2]) / 2.0,
            ]
        };
        let centroid = [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ];
        for p in [centroid, mid(a, b), mid(b, c), mid(c, a)] {
            assert!((above(p) - offset).abs() < 1e-9, "{p:?}: {}", above(p));
        }
        // Counter-clockwise seen from above: the face normal points up.
        let (e1, e2) = ([b[0] - a[0], b[2] - a[2]], [c[0] - a[0], c[2] - a[2]]);
        assert!(e1[1] * e2[0] - e1[0] * e2[1] >= 0.0);
    }
    for n in &mesh.normals {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-12 && n[1] > 0.5, "{n:?}");
    }
}

#[test]
fn the_surface_covers_the_road_and_nothing_else() {
    let road = Road::new(vec![[-40.0, 0.0], [0.0, 0.0], [30.0, 30.0]]);
    let (world, field, ids) = world_with(hilly(), vec![road]);
    let (e, r) = road_of(&world, ids[0]);
    let mesh = r.mesh(e, &field).unwrap();
    let area: f64 = mesh
        .triangles()
        .map(|[a, b, c]| {
            ((b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2])).abs() / 2.0
        })
        .sum();
    // The union: 40 m and 42.43 m long, 6 m wide, a 45-degree join sector and two half discs
    // (polygons of 8 edges per half turn) over each other at the bend's inner side.
    let length = 40.0 + 30.0 * 2f64.sqrt();
    let exact = length * 6.0;
    assert!(
        area > exact && area < exact + 40.0,
        "{area} against {exact}"
    );
    // On the road and beside it.
    assert!(r.contains(e, -20.0, 2.9) && r.contains(e, 15.0, 15.0));
    assert!(!r.contains(e, -20.0, 3.1) && !r.contains(e, 0.0, 10.0));
    // The round end reaches 3 m beyond the first point; the outer side of the bend is closed.
    assert!(r.contains(e, -42.9, 0.0) && !r.contains(e, -43.1, 0.0));
    assert!(r.contains(e, 1.0, -2.6));
    let flat = Road {
        ends: RoadEnds::Flat,
        ..r.clone()
    };
    assert!(!flat.contains(e, -40.5, 0.0) && flat.contains(e, -39.5, 0.0));
}

#[test]
fn the_same_road_gives_the_same_mesh_bytes() {
    let build = || {
        let road = Road::new(vec![[-30.0, 12.0], [5.0, -7.5], [33.3, 20.1]]);
        let (world, field, ids) = world_with(hilly(), vec![road]);
        let (e, r) = road_of(&world, ids[0]);
        r.mesh(e, &field).unwrap()
    };
    let (a, b) = (build(), build());
    let bits = |m: &crate::RoadMesh| -> Vec<u64> {
        m.positions
            .iter()
            .chain(&m.normals)
            .flatten()
            .map(|v| v.to_bits())
            .collect()
    };
    assert_eq!(bits(&a), bits(&b));
    assert_eq!(a.indices, b.indices);
    assert_eq!(a.stats, b.stats);
}

#[test]
fn crossing_roads_get_different_tiers_and_are_never_coplanar() {
    let a = Road::new(vec![[-50.0, 0.0], [50.0, 3.0]]);
    let b = Road::new(vec![[0.0, -50.0], [4.0, 50.0]]);
    let (mut world, field, ids) = world_with(hilly(), vec![a, b.clone()]);
    // As road.add does: the new road takes the lowest tier its neighbours leave free.
    let (eb, rb) = road_of(&world, ids[1]);
    let tier = free_road_tier(&world, eb, rb);
    assert_eq!(tier, 1);
    world.entity_mut(ids[1]).unwrap().road_mut().unwrap().tier = tier;
    let (ea, ra) = road_of(&world, ids[0]);
    let (eb, rb) = road_of(&world, ids[1]);
    let (ma, mb) = (ra.mesh(ea, &field).unwrap(), rb.mesh(eb, &field).unwrap());
    let mut shared = 0;
    for (mine, te, tr) in [(&mb, ea, ra), (&ma, eb, rb)] {
        for p in &mine.positions {
            if tr.contains(te, p[0], p[2]) {
                shared += 1;
                let other = field.height_at(p[0], p[2]).unwrap() + tr.surface_offset_m();
                let gap = (p[1] - other).abs();
                assert!(gap > ROAD_TIER_STEP_M - 1e-9, "{p:?}: {gap}");
            }
        }
    }
    assert!(shared > 20, "{shared} vertices in the crossing");
    // A ray down at the crossing hits the upper road, beside it the lower one.
    let roads = [(ea, ra), (eb, rb)];
    let down = [0.0, -1.0, 0.0];
    let (t, id) = raycast_roads(&field, &roads, [2.0, 50.0, 1.0], down, 100.0).unwrap();
    assert_eq!(id, ids[1]);
    let ground = field.height_at(2.0, 1.0).unwrap();
    assert!((50.0 - t - ground - rb.surface_offset_m()).abs() < 1e-9);
    let (t, id) = raycast_roads(&field, &roads, [-30.0, 50.0, 0.5], down, 100.0).unwrap();
    assert_eq!(id, ids[0]);
    let ground = field.height_at(-30.0, 0.5).unwrap();
    assert!((50.0 - t - ground - ra.surface_offset_m()).abs() < 1e-9);
    assert_eq!(
        raycast_roads(&field, &roads, [-30.0, 50.0, 20.0], down, 100.0),
        None
    );
}

#[test]
fn a_slanted_ray_meets_the_road_where_the_mesh_is() {
    let road = Road::new(vec![[-50.0, 0.0], [50.0, 0.0]]);
    let (world, field, ids) = world_with(hilly(), vec![road]);
    let (e, r) = road_of(&world, ids[0]);
    // Down onto the road at a slant: about y 16 over its near edge, 4 over its far one.
    let origin = [-20.0, 30.0, -10.0];
    let dir = [0.3, -1.0, 0.5];
    let (t, id) = raycast_roads(&field, &[(e, r)], origin, dir, 500.0).unwrap();
    assert_eq!(id, ids[0]);
    let len = (0.09f64 + 1.0 + 0.25).sqrt();
    let p = [
        origin[0] + dir[0] / len * t,
        origin[1] + dir[1] / len * t,
        origin[2] + dir[2] / len * t,
    ];
    assert!(r.contains(e, p[0], p[2]));
    let above = p[1] - field.height_at(p[0], p[2]).unwrap();
    assert!((above - r.surface_offset_m()).abs() < 1e-9, "{above}");
    // Before the terrain: the ground hit lies farther along.
    let ground = field.raycast(origin, dir, 500.0).unwrap();
    assert!(t < ground);
    // Hostile rays miss without a panic.
    for (o, d, m) in [
        ([f64::NAN, 0.0, 0.0], dir, 10.0),
        (origin, [0.0, 0.0, 0.0], 10.0),
        (origin, dir, f64::INFINITY),
        (origin, [f64::MAX, -f64::MAX, 1.0], 1e300),
    ] {
        let _ = raycast_roads(&field, &[(e, r)], o, d, m);
    }
}

#[test]
fn retiering_gives_crossing_roads_different_tiers_with_few_tiers() {
    // A main road crossed by five side streets that do not meet each other.
    let mut roads = vec![Road::new(vec![[-60.0, 0.0], [60.0, 0.0]])];
    for k in 0..5 {
        let x = -50.0 + 25.0 * f64::from(k);
        roads.push(Road::new(vec![[x, -30.0], [x + 2.0, 30.0]]));
    }
    // And one that crosses the main road and the first side street.
    roads.push(Road::new(vec![[-60.0, -20.0], [-30.0, 20.0]]));
    let (mut world, _, ids) = world_with(hilly(), roads);
    let report = retier_roads(&mut world);
    assert_eq!(report.roads, 7);
    let tier = |i: usize| world.entity(ids[i]).unwrap().road().unwrap().tier;
    assert_eq!(tier(0), 0);
    for i in 1..6 {
        assert_eq!(tier(i), 1, "side street {i}");
    }
    assert_eq!(tier(6), 2);
    assert_eq!(report.highest, 2);
    assert_eq!(report.overlaps, 7);
    assert_eq!(report.changed, 6);
    // Again: nothing changes.
    assert_eq!(retier_roads(&mut world).changed, 0);
}

#[test]
fn a_road_too_big_for_the_terrain_s_cells_is_an_error() {
    let terrain = Terrain::new(
        2048.0,
        TERRAIN_CELL_MIN_M * 4.0,
        TerrainSource::Flat { height_m: 0.0 },
    );
    let road = Road {
        width_m: 200.0,
        ..Road::new(vec![
            [-1000.0, -1000.0],
            [1000.0, 1000.0],
            [-1000.0, 1000.0],
        ])
    };
    let (world, field, ids) = world_with(terrain, vec![road]);
    let (e, r) = road_of(&world, ids[0]);
    let mut emitted = 0;
    let err = r.drape(e, &field, &mut |_| emitted += 1).unwrap_err();
    assert!(
        err.to_string().contains("split it into shorter roads"),
        "{err}"
    );
    assert_eq!(emitted, 0, "nothing is emitted before the check");
}

#[test]
fn beyond_the_terrain_nothing_is_drawn_and_hostile_roads_do_not_panic() {
    let road = Road::new(vec![[100.0, 100.0], [200.0, 120.0]]);
    let (world, field, ids) = world_with(hilly(), vec![road]);
    let (e, r) = road_of(&world, ids[0]);
    let mesh = r.mesh(e, &field).unwrap();
    assert!(mesh.positions.is_empty());
    // Numbers no valid document holds.
    let mut e = e.clone();
    for (points, scale, yaw) in [
        (vec![[f64::NAN, 0.0], [1.0, 1.0]], 1.0, 0.0),
        (vec![[0.0, 0.0], [0.0, 0.0], [0.0, 0.0]], 1.0, 0.0),
        (vec![[1e300, -1e300], [-1e300, 1e300]], 1e300, f64::INFINITY),
        (vec![], 1.0, 0.0),
    ] {
        e.scale = scale;
        e.rotation_y_deg = yaw;
        let bad = Road::new(points);
        let _ = bad.mesh(&e, &field);
        let _ = bad.contains(&e, 0.0, 0.0);
        let _ = bad.bounds(&e);
        let _ = bad.overlaps(&e, r, &e);
        let _ = raycast_roads(
            &field,
            &[(&e, &bad)],
            [0.0, 10.0, 0.0],
            [0.0, -1.0, 0.0],
            20.0,
        );
    }
}

#[test]
fn the_entity_places_turns_and_scales_the_road() {
    let road = Road::new(vec![[0.0, 0.0], [10.0, 0.0]]);
    let mut world = World::new("placed", 1);
    let id = world
        .add_entity(EntityDraft {
            position: [5.0, 0.0, 5.0],
            rotation_y_deg: 90.0,
            scale: 2.0,
            components: vec![Component::Road(road)],
            ..EntityDraft::default()
        })
        .unwrap();
    let (e, r) = road_of(&world, id);
    // Turned 90 degrees counter-clockwise seen from above, +x goes to -z.
    let line = r.centre_line(e);
    assert!(
        (line[1][0] - 5.0).abs() < 1e-9 && (line[1][1] - -15.0).abs() < 1e-9,
        "{line:?}"
    );
    assert!((r.length_m(e) - 20.0).abs() < 1e-9);
    assert!(r.contains(e, 5.0 + 5.9, -10.0) && !r.contains(e, 5.0 + 6.1, -10.0));
}

#[test]
fn a_road_survives_the_cooked_cache() {
    let road = Road {
        widths_m: vec![3.0, 4.5],
        tier: 2,
        ends: RoadEnds::Flat,
        ..Road::new(vec![[0.0, 0.0], [10.0, 0.0]])
    };
    let (world, _, _) = world_with(hilly(), vec![road]);
    let bytes = world.to_cooked(7).unwrap();
    let back = World::from_cooked(&bytes, 7).unwrap();
    assert_eq!(back.to_json().unwrap(), world.to_json().unwrap());
}
