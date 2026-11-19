//! GPU-free tests: meshes, cameras, requests, PNG, the light and wind mappings.

use crate::mesh::{self, MeshData};
use crate::{
    Camera, PictureRequest, Quality, RenderError, encode_png, light, plants, wind_towards,
};
use gamecraft_math::{Rng, yaw_forward};
use gamecraft_world::{
    Character, Component, EntityDraft, HeightField, Light, Shape, ShapeKind, Terrain,
    TerrainSource, World,
};
use std::collections::BTreeMap;

fn noise_terrain(size_m: f64, cell_m: f64) -> Terrain {
    Terrain {
        size_m,
        cell_m,
        source: TerrainSource::Noise {
            seed: 7,
            relief_m: 30.0,
            feature_m: 40.0,
            octaves: 4,
            roughness: 0.5,
        },
        ..Default::default()
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f64; 3] {
    [0, 1, 2].map(|k| f64::from(a[k]) - f64::from(b[k]))
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The height at (x, z) of the mesh triangle whose xz projection holds the point, if any.
fn height_on_triangle(p: [[f32; 3]; 3], x: f64, z: f64) -> Option<f64> {
    let [a, b, c] = p.map(|v| v.map(f64::from));
    let det = (b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2]);
    if det.abs() < 1e-12 {
        return None;
    }
    let u = ((x - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (z - a[2])) / det;
    let v = ((b[0] - a[0]) * (z - a[2]) - (x - a[0]) * (b[2] - a[2])) / det;
    let eps = 1e-9;
    (u >= -eps && v >= -eps && u + v <= 1.0 + eps)
        .then(|| a[1] + u * (b[1] - a[1]) + v * (c[1] - a[1]))
}

#[test]
fn terrain_mesh_agrees_with_the_height_field() {
    // 130 cells per side: chunks of 64, 64 and 2 cells, so there are borders and a thin edge.
    let hf = noise_terrain(130.0, 1.0).build().unwrap();
    let chunks = mesh::terrain_chunks(&hf);
    assert_eq!(chunks.len(), 9);
    let [x0, z0] = hf.min();
    let [x1, z1] = hf.max();

    // Indices in range, triangles facing up, weights summing to 255.
    for chunk in &chunks {
        let m = &chunk.mesh;
        assert_eq!(m.positions.len(), m.normals.len());
        assert_eq!(m.positions.len(), chunk.weights.len());
        for w in &chunk.weights {
            assert_eq!(w.iter().map(|&v| u32::from(v)).sum::<u32>(), 255);
        }
        for [a, b, c] in m.triangles() {
            for i in [a, b, c] {
                assert!((i as usize) < m.positions.len(), "index {i} out of range");
            }
            let (pa, pb, pc) = (
                m.positions[a as usize],
                m.positions[b as usize],
                m.positions[c as usize],
            );
            let n = cross(sub(pb, pa), sub(pc, pa));
            assert!(n[1] > 0.0, "a triangle faces down: {n:?}");
        }
        for n in &m.normals {
            let len = n.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
            assert!((len - 1.0).abs() < 1e-5 && n[1] > 0.0);
        }
    }

    // Every sample is a vertex, and every position shared by chunks has one normal and one
    // set of weights there (the borders match).
    let mut seen: BTreeMap<[u32; 3], ([u32; 3], [u8; 8])> = BTreeMap::new();
    for chunk in &chunks {
        for ((p, n), w) in chunk
            .mesh
            .positions
            .iter()
            .zip(&chunk.mesh.normals)
            .zip(&chunk.weights)
        {
            let entry = (n.map(f32::to_bits), *w);
            let previous = seen.insert(p.map(f32::to_bits), entry);
            if let Some(previous) = previous {
                assert_eq!(previous, entry, "chunks disagree at {p:?}");
            }
        }
    }
    assert_eq!(seen.len(), hf.cols() * hf.rows());
    for row in 0..hf.rows() {
        for col in 0..hf.cols() {
            let p = [
                (x0 + col as f64 * hf.cell_m()) as f32,
                hf.sample(col, row).unwrap(),
                (z0 + row as f64 * hf.cell_m()) as f32,
            ];
            assert!(
                seen.contains_key(&p.map(f32::to_bits)),
                "sample ({col}, {row}) is no vertex"
            );
        }
    }
    let total: usize = chunks.iter().map(|c| c.mesh.positions.len()).sum();
    assert!(
        total > seen.len(),
        "neighbouring chunks share their borders"
    );

    // The mesh under 1000 random points has the height field's height.
    let mut rng = Rng::new(42);
    for _ in 0..1000 {
        let (x, z) = (rng.range(x0, x1), rng.range(z0, z1));
        let expected = hf.height_at(x, z).unwrap();
        let mut found = None;
        'chunks: for chunk in &chunks {
            let (lo, hi) = chunk.mesh.bounds().unwrap();
            if x < f64::from(lo[0]) - 1e-6
                || x > f64::from(hi[0]) + 1e-6
                || z < f64::from(lo[2]) - 1e-6
                || z > f64::from(hi[2]) + 1e-6
            {
                continue;
            }
            for [a, b, c] in chunk.mesh.triangles() {
                let m = &chunk.mesh.positions;
                if let Some(h) =
                    height_on_triangle([m[a as usize], m[b as usize], m[c as usize]], x, z)
                {
                    found = Some(h);
                    break 'chunks;
                }
            }
        }
        let h = found.unwrap_or_else(|| panic!("no triangle under ({x}, {z})"));
        assert!(
            (h - expected).abs() < 1e-4,
            "at ({x}, {z}) the mesh is at {h}, height_at says {expected}"
        );
    }
}

#[test]
fn layer_weights_follow_the_slope() {
    let flat = mesh::layer_weights(1.0);
    assert_eq!(flat[mesh::LAYER_GRASS], 255);
    let steep = mesh::layer_weights(60f64.to_radians().cos());
    assert_eq!(steep[mesh::LAYER_ROCK], 255);
    let middle = mesh::layer_weights(32f64.to_radians().cos());
    assert!(middle[mesh::LAYER_EARTH] > 0);
    for deg in 0..=90 {
        let w = mesh::layer_weights(f64::from(deg).to_radians().cos());
        assert_eq!(w.iter().map(|&v| u32::from(v)).sum::<u32>(), 255, "{deg}");
    }
}

/// Every undirected edge (by position) is used by exactly two triangles, once each way.
fn assert_closed(m: &MeshData, what: &str) {
    let key = |i: u32| m.positions[i as usize].map(|v| (v * 1e5).round() as i64);
    let mut directed: BTreeMap<([i64; 3], [i64; 3]), usize> = BTreeMap::new();
    for [a, b, c] in m.triangles() {
        for (p, q) in [(a, b), (b, c), (c, a)] {
            *directed.entry((key(p), key(q))).or_default() += 1;
        }
    }
    for ((p, q), n) in &directed {
        assert_eq!(*n, 1, "{what}: edge {p:?}->{q:?} used {n} times one way");
        assert_eq!(
            directed.get(&(*q, *p)),
            Some(&1),
            "{what}: edge {p:?}-{q:?} is not shared by two triangles"
        );
    }
}

/// Unit normals pointing away from `centre`, triangles wound outward.
fn assert_outward(m: &MeshData, centre: [f64; 3], what: &str) {
    for (p, n) in m.positions.iter().zip(&m.normals) {
        let len = n.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
        assert!((len - 1.0).abs() < 1e-5, "{what}: normal {n:?} not unit");
        let out = [0, 1, 2].map(|k| f64::from(p[k]) - centre[k]);
        let dot: f64 = (0..3).map(|k| out[k] * f64::from(n[k])).sum();
        assert!(dot > 0.0, "{what}: normal {n:?} at {p:?} points inwards");
    }
    for [a, b, c] in m.triangles() {
        let (pa, pb, pc) = (
            m.positions[a as usize],
            m.positions[b as usize],
            m.positions[c as usize],
        );
        let n = cross(sub(pb, pa), sub(pc, pa));
        let mid = [0, 1, 2]
            .map(|k| (f64::from(pa[k]) + f64::from(pb[k]) + f64::from(pc[k])) / 3.0 - centre[k]);
        let dot: f64 = (0..3).map(|k| n[k] * mid[k]).sum();
        assert!(dot > 0.0, "{what}: a triangle is wound inwards");
    }
}

fn assert_bounds(m: &MeshData, lo: [f32; 3], hi: [f32; 3], what: &str) {
    let (l, h) = m.bounds().unwrap();
    for k in 0..3 {
        assert!(
            (l[k] - lo[k]).abs() < 1e-5,
            "{what}: min {l:?}, want {lo:?}"
        );
        assert!(
            (h[k] - hi[k]).abs() < 1e-5,
            "{what}: max {h:?}, want {hi:?}"
        );
    }
}

#[test]
fn shape_meshes_are_closed_outward_and_unit_sized_on_the_ground() {
    let centre = [0.0, 0.5, 0.0];
    let (lo, hi) = ([-0.5, 0.0, -0.5], [0.5, 1.0, 0.5]);
    for (what, m) in [
        ("box", mesh::unit_box()),
        ("cylinder", mesh::unit_cylinder()),
        ("sphere", mesh::unit_sphere()),
    ] {
        assert_closed(&m, what);
        assert_outward(&m, centre, what);
        assert_bounds(&m, lo, hi, what);
        assert_eq!(m.bounds().unwrap().0[1], 0.0, "{what}: base at y = 0");
    }
    assert_eq!(mesh::unit_box().triangle_count(), 12);
}

#[test]
fn a_character_is_as_tall_as_its_component_and_faces_north() {
    let c = Character::default();
    let m = mesh::character(&c);
    let (lo, hi) = m.body.bounds().unwrap();
    assert!(lo[1].abs() < 1e-5 && (f64::from(hi[1]) - c.height_m).abs() < 1e-5);
    assert!((f64::from(hi[0]) - c.radius_m).abs() < 1e-5);
    let (vlo, vhi) = m.visor.bounds().unwrap();
    let eye = c.eye_height_m as f32;
    assert!(
        vlo[1] < eye && vhi[1] > eye,
        "the visor spans the eye height"
    );
    assert!(
        vlo[2] < -(c.radius_m as f32) * 0.5,
        "the visor is on the -Z side"
    );
    assert!(vhi[2] < 0.0);
}

fn valley() -> (World, HeightField) {
    let mut world = World::new("valley", 3);
    let terrain = Terrain {
        size_m: 512.0,
        cell_m: 4.0,
        source: TerrainSource::Noise {
            seed: 3,
            relief_m: 60.0,
            feature_m: 200.0,
            octaves: 5,
            roughness: 0.5,
        },
        ..Default::default()
    };
    let hf = terrain.build().unwrap();
    world.terrain = Some(terrain);
    (world, hf)
}

fn assert_frames(camera: &Camera, lo: [f64; 3], hi: [f64; 3]) {
    for aspect in [16.0 / 9.0, 21.0 / 9.0] {
        for k in 0..8 {
            let corner = [
                if k & 1 == 0 { lo[0] } else { hi[0] },
                if k & 2 == 0 { lo[1] } else { hi[1] },
                if k & 4 == 0 { lo[2] } else { hi[2] },
            ];
            let [x, y] = camera
                .project(corner, aspect)
                .unwrap_or_else(|| panic!("corner {corner:?} behind the camera"));
            assert!(
                x.abs() <= 1.0 && y.abs() <= 1.0,
                "corner {corner:?} at {x}, {y} is outside the frame (aspect {aspect})"
            );
        }
    }
}

#[test]
fn framing_sees_every_corner_of_the_terrain() {
    let (world, hf) = valley();
    let (y0, y1) = hf.height_range();
    let [x0, z0] = hf.min();
    let [x1, z1] = hf.max();
    let camera = Camera::framing(&world, Some(&hf), 16.0 / 9.0);
    camera.validate().unwrap();
    assert_frames(&camera, [x0, f64::from(y0), z0], [x1, f64::from(y1), z1]);
    for aspect in [1.0, 16.0 / 9.0, 21.0 / 9.0] {
        let camera = Camera::framing(&world, Some(&hf), aspect);
        let mut xs = Vec::new();
        for k in 0..8 {
            let corner = [
                if k & 1 == 0 { x0 } else { x1 },
                f64::from(if k & 2 == 0 { y0 } else { y1 }),
                if k & 4 == 0 { z0 } else { z1 },
            ];
            let [x, y] = camera.project(corner, aspect).unwrap();
            assert!(
                x.abs() <= 1.0 && y.abs() <= 1.0,
                "{corner:?} {x} {y} at {aspect}"
            );
            xs.push(x);
        }
        if (aspect - 16.0 / 9.0).abs() < 1e-9 {
            let span = xs.iter().fold(f64::MIN, |a, b| a.max(*b))
                - xs.iter().fold(f64::MAX, |a, b| a.min(*b));
            assert!(
                span / 2.0 >= 0.70,
                "terrain spans {span} of 2 at 16:9: {xs:?}"
            );
        }
    }
    // A bad aspect falls back to 16:9.
    assert_eq!(
        Camera::framing(&world, Some(&hf), f64::NAN),
        Camera::framing(&world, Some(&hf), 16.0 / 9.0)
    );
    assert_eq!(
        Camera::framing(&world, Some(&hf), -2.0),
        Camera::framing(&world, Some(&hf), 16.0 / 9.0)
    );
    // From above, to the south-east of the centre.
    let [cx, cy, cz] = camera.position;
    assert!(
        cx > 0.0 && cz > 0.0 && cy > f64::from(y1),
        "{:?}",
        camera.position
    );
    // Not absurdly far: the terrain fills a good share of the frame.
    let distance = (cx * cx + cz * cz).sqrt();
    assert!(distance < 1000.0, "{distance}");

    // Without the height field the recipe's box is framed.
    let camera = Camera::framing(&world, None, 16.0 / 9.0);
    assert_frames(&camera, [-256.0, -30.0, -256.0], [256.0, 30.0, 256.0]);

    // An empty world frames the origin; a world of entities frames them.
    let mut empty = World::new("empty", 0);
    let camera = Camera::framing(&empty, None, 16.0 / 9.0);
    assert_frames(&camera, [-10.0, 0.0, -10.0], [10.0, 2.0, 10.0]);
    empty
        .add_entity(EntityDraft {
            position: [100.0, 0.0, -40.0],
            components: vec![Component::Shape(Shape {
                kind: ShapeKind::Box,
                size: [2.0, 6.0, 2.0],
                color: [1.0, 0.0, 0.0],
                solid: true,
            })],
            ..EntityDraft::default()
        })
        .unwrap();
    let camera = Camera::framing(&empty, None, 16.0 / 9.0);
    assert_frames(&camera, [100.0, 0.0, -40.0], [100.0, 6.0, -40.0]);
}

#[test]
fn first_person_sits_at_eye_height_and_looks_along_the_facing() {
    let mut world = World::new("walk", 0);
    for yaw in [0.0, 90.0, -135.0, 33.0] {
        let id = world
            .add_entity(EntityDraft {
                position: [3.0, 2.0, -7.0],
                rotation_y_deg: yaw,
                components: vec![Component::Character(Character::default())],
                ..EntityDraft::default()
            })
            .unwrap();
        let e = world.entity(id).unwrap();
        let cam = Camera::first_person(e).unwrap();
        assert!((cam.position[1] - (2.0 + Character::default().eye_height_m)).abs() < 1e-12);
        assert_eq!([cam.position[0], cam.position[2]], [3.0, -7.0]);
        let d = [0, 1, 2].map(|k| cam.target[k] - cam.position[k]);
        let len = d.iter().map(|v| v * v).sum::<f64>().sqrt();
        let f = yaw_forward(yaw);
        assert!((d[0] / len - f.x).abs() < 1e-12 && d[1].abs() < 1e-12);
        assert!((d[2] / len - f.z).abs() < 1e-12);

        let third = Camera::third_person(e, 6.0);
        third.validate().unwrap();
        let back = [0, 1, 2].map(|k| third.position[k] - third.target[k]);
        assert!(
            back[0] * f.x + back[2] * f.z < 0.0,
            "third person is behind"
        );
        assert!(back[1] > 0.0, "third person is above");
        let len = back.iter().map(|v| v * v).sum::<f64>().sqrt();
        assert!((len - 6.0).abs() < 1e-9);
    }
    let id = world.add_entity(EntityDraft::default()).unwrap();
    assert_eq!(Camera::first_person(world.entity(id).unwrap()), None);
}

fn request() -> PictureRequest {
    PictureRequest {
        width: 640,
        height: 360,
        camera: Camera {
            position: [0.0, 10.0, 20.0],
            target: [0.0, 0.0, 0.0],
            fov_y_deg: 60.0,
        },
        time_s: 0.0,
    }
}

fn refused(r: PictureRequest, field: &str) {
    match r.validate() {
        Err(RenderError::BadRequest(m)) => {
            assert!(m.contains(field), "{m:?} should name {field}");
            assert!(RenderError::BadRequest(m).to_string().ends_with('.'));
        }
        other => panic!("{field}: expected a bad request, got {other:?}"),
    }
}

#[test]
fn bad_requests_are_refused_with_the_field_named() {
    request().validate().unwrap();
    let mut r = request();
    r.width = 15;
    refused(r, "request.width");
    r.width = 3841;
    refused(r, "request.width");
    let mut r = request();
    r.height = 2161;
    refused(r, "request.height");
    let mut r = request();
    r.time_s = f64::NAN;
    refused(r, "request.time_s");
    let mut r = request();
    r.camera.position[1] = f64::NAN;
    refused(r, "camera.position[1]");
    let mut r = request();
    r.camera.target[2] = f64::INFINITY;
    refused(r, "camera.target[2]");
    for fov in [4.9, 150.1, f64::NAN] {
        let mut r = request();
        r.camera.fov_y_deg = fov;
        refused(r, "camera.fov_y_deg");
    }
    let mut r = request();
    r.camera.target = r.camera.position;
    refused(r, "camera.position");
    // The limits themselves are allowed.
    let mut r = request();
    (r.width, r.height, r.camera.fov_y_deg) = (16, 2160, 150.0);
    r.validate().unwrap();
    (r.width, r.height, r.camera.fov_y_deg) = (3840, 16, 5.0);
    r.validate().unwrap();
}

#[test]
fn encode_png_round_trips_and_refuses_a_wrong_length() {
    let (w, h) = (7u32, 5u32);
    let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i * 37 % 256) as u8).collect();
    let file = encode_png(&rgba, w, h).unwrap();
    let decoder = png::Decoder::new(file.as_slice());
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (w, h));
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(&buf[..info.buffer_size()], rgba.as_slice());

    for (len, w, h) in [(rgba.len() - 1, 7, 5), (rgba.len(), 5, 7 + 1), (0, 0, 0)] {
        match encode_png(&rgba[..len], w, h) {
            Err(RenderError::BadRequest(m)) => assert!(m.contains("bytes"), "{m}"),
            other => panic!("expected a bad request, got {other:?}"),
        }
    }
}

#[test]
fn wind_from_the_north_blows_south_and_from_the_east_blows_west() {
    let close = |a: [f64; 3], b: [f64; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-12);
    assert!(
        close(wind_towards(0.0), [0.0, 0.0, 1.0]),
        "{:?}",
        wind_towards(0.0)
    );
    assert!(
        close(wind_towards(90.0), [-1.0, 0.0, 0.0]),
        "{:?}",
        wind_towards(90.0)
    );
    assert!(close(wind_towards(180.0), [0.0, 0.0, -1.0]));
    assert!(close(wind_towards(270.0), [1.0, 0.0, 0.0]));
}

#[test]
fn exposure_follows_the_light() {
    assert_eq!(light::auto_ev(60.0), 15.0);
    assert!((light::auto_ev(10.0) - 13.5).abs() < 1e-9);
    // Continuous where the day curve meets the metered one, and falling into the night.
    assert!(
        (light::auto_ev(9.999) - 13.5).abs() < 0.2,
        "{}",
        light::auto_ev(9.999)
    );
    let mut previous = f64::INFINITY;
    for e in (-30..=60).rev() {
        let ev = light::auto_ev(f64::from(e));
        assert!(ev <= previous + 1e-9, "EV rises as the sun sinks at {e}");
        assert!(ev >= -2.5);
        previous = ev;
    }
    assert!(light::auto_ev(-30.0) < 0.0);
    assert!((light::daylight_lux(0.0) - 760.0).abs() < 60.0);

    let mut env = gamecraft_world::Environment {
        time_of_day_h: 12.0,
        ..Default::default()
    };
    let noon = light::exposure_ev(&env, false);
    env.time_of_day_h = 0.0;
    let midnight = light::exposure_ev(&env, false);
    assert!(
        noon > 12.0 && midnight < 2.0,
        "noon {noon}, midnight {midnight}"
    );
    env.time_of_day_h = 12.0;
    env.mist = 1.0;
    assert!(light::exposure_ev(&env, false) < noon);
}

#[test]
fn an_unknown_species_has_a_nearest_known_id() {
    assert!(plants::known("quercus-robur"));
    assert!(!plants::known("quercus-robr"));
    assert_eq!(
        plants::nearest_species("quercus-robr"),
        Some("quercus-robur")
    );
    assert_eq!(
        plants::nearest_species("Betula Pendula"),
        Some("betula-pendula")
    );
    assert_eq!(plants::edit_distance("kitten", "sitting"), 3);
    assert_eq!(plants::edit_distance("", "abc"), 3);
    assert!(plants::mature_height_m("quercus-robur").unwrap() > 5.0);
}

#[test]
fn a_plant_generates_parts_with_wind_weights() {
    let plant = plants::generate("betula-pendula", 1, 1.0).unwrap();
    assert!(!plant.parts.is_empty());
    for p in &plant.parts {
        assert_eq!(p.positions.len(), p.wind.len());
        assert_eq!(p.positions.len(), p.uvs.len());
        assert!(p.indices.iter().all(|&i| (i as usize) < p.positions.len()));
    }
    assert!(plants::generate("no-such-plant", 1, 1.0).is_none());
}

#[test]
fn the_apron_joins_the_terrain_and_ends_at_the_plane() {
    let (_world, hf) = valley();
    let plane = 3.5;
    let apron = mesh::terrain_apron(&hf, plane);
    let width = mesh::apron_width_m(&hf);
    assert_eq!(width, 76.8);
    let [x0, z0] = hf.min();
    let [x1, z1] = hf.max();
    let (lc, lr) = (hf.cols() - 1, hf.rows() - 1);
    let per = 2 * (lc + lr);
    let m = &apron.mesh;
    assert_eq!(m.positions.len(), per * (mesh::APRON_RINGS + 1));
    assert_eq!(m.normals.len(), m.positions.len());
    assert_eq!(apron.weights.len(), m.positions.len());
    // Inner ring: the terrain's border vertices, bit for bit.
    let terrain: Vec<[f32; 3]> = mesh::terrain_chunks(&hf)
        .into_iter()
        .flat_map(|c| c.mesh.positions)
        .collect();
    for p in m.positions.iter().take(per) {
        assert!(terrain.contains(p), "{p:?} is not a terrain vertex");
    }
    // Outer ring at the plane height and the full width away.
    for p in m.positions.iter().skip(per * mesh::APRON_RINGS) {
        assert_eq!(p[1], plane as f32);
        let (x, z) = (f64::from(p[0]), f64::from(p[2]));
        let outside = (x0 - x).max(x - x1).max(z0 - z).max(z - z1);
        assert!((outside - width).abs() < 1e-3, "{p:?}");
    }
    // Beyond the inner ring nothing lies inside or on the square.
    for p in m.positions.iter().skip(per) {
        let (x, z) = (f64::from(p[0]), f64::from(p[2]));
        assert!(
            x < x0 || x > x1 || z < z0 || z > z1,
            "{p:?} is over the terrain"
        );
    }
    for p in m.positions.iter() {
        let (x, z) = (f64::from(p[0]), f64::from(p[2]));
        assert!(
            !(x > x0 && x < x1 && z > z0 && z < z1),
            "{p:?} is inside the square"
        );
    }
    // Indices in range, triangles face up, normals unit and upward, weights sum to 255.
    assert!(m.indices.iter().all(|&i| (i as usize) < m.positions.len()));
    assert_eq!(m.triangle_count(), 2 * per * mesh::APRON_RINGS);
    for [a, b, c] in m.triangles() {
        let p = [a, b, c].map(|i| m.positions[i as usize]);
        let n = cross(sub(p[1], p[0]), sub(p[2], p[0]));
        assert!(n[1] > 0.0, "triangle {p:?} faces down");
    }
    for n in &m.normals {
        let len = n.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
        assert!((len - 1.0).abs() < 1e-5 && n[1] > 0.0, "{n:?}");
    }
    for w in &apron.weights {
        assert_eq!(w.iter().map(|v| u32::from(*v)).sum::<u32>(), 255);
    }
    let outer = apron.weights.iter().skip(per * mesh::APRON_RINGS);
    for w in outer {
        assert_eq!(w[mesh::LAYER_GRASS], 255);
    }
}

#[test]
fn lamps_keep_the_blue_hour_and_the_night_darker() {
    // VisionCraft's studio: with lamps the blue hour is exposed 2 EV darker, the night 0.8 EV,
    // the day not at all.
    for e in [20.0, 10.0] {
        assert_eq!(light::lamp_ev(e), 0.0, "no lamp allowance by day at {e}");
    }
    assert!(
        (light::lamp_ev(-2.0) - 2.0).abs() < 1e-9,
        "{}",
        light::lamp_ev(-2.0)
    );
    assert!(
        (light::lamp_ev(-20.0) - 0.8).abs() < 1e-9,
        "{}",
        light::lamp_ev(-20.0)
    );
    for e in -30..2 {
        assert!(
            light::lamp_ev(f64::from(e)) > 0.0,
            "a lamp allowance at {e}"
        );
    }
    let env = gamecraft_world::Environment {
        time_of_day_h: 0.0,
        ..Default::default()
    };
    let without = light::exposure_ev(&env, false);
    let with = light::exposure_ev(&env, true);
    assert!(with > without && with - without < 1.0, "{without} {with}");
}

fn lit_world() -> World {
    let mut world = World::new("lights", 1);
    let mut add = |position: [f64; 3], yaw: f64, components: Vec<Component>| {
        world
            .add_entity(EntityDraft {
                position,
                rotation_y_deg: yaw,
                components,
                ..EntityDraft::default()
            })
            .unwrap()
    };
    let lantern = Light {
        color: [1.0, 0.8, 0.5],
        intensity_cd: 50.0,
        range_m: 8.0,
        on: true,
    };
    // 1: a character carrying a lantern, facing east (yaw -90).
    add(
        [10.0, 2.0, 0.0],
        -90.0,
        vec![
            Component::Character(Character::default()),
            Component::Light(lantern.clone()),
        ],
    );
    // 2: a box of 1 x 3 x 1 standing at a height of 1, with a light on top.
    add(
        [0.0, 1.0, 5.0],
        0.0,
        vec![
            Component::Shape(Shape {
                kind: ShapeKind::Box,
                size: [1.0, 3.0, 1.0],
                color: [0.5; 3],
                solid: true,
            }),
            Component::Light(Light::default()),
        ],
    );
    // 3: a light that is off.
    add(
        [-5.0, 0.0, 0.0],
        0.0,
        vec![Component::Light(Light {
            on: false,
            ..Light::default()
        })],
    );
    // 4: a light on its own.
    add([-9.0, 0.5, 3.0], 0.0, vec![Component::Light(lantern)]);
    // 5: a box without a light.
    add(
        [3.0, 0.0, 3.0],
        0.0,
        vec![Component::Shape(Shape {
            kind: ShapeKind::Sphere,
            size: [1.0; 3],
            color: [0.5; 3],
            solid: false,
        })],
    );
    world
}

#[test]
fn every_light_that_is_on_becomes_a_spot_at_a_sensible_height() {
    let mut world = lit_world();
    let spots = light::spots(&world);
    assert_eq!(spots.len(), 3, "{spots:?}");
    let [carried, beacon, alone] = [&spots[0], &spots[1], &spots[2]];

    // A lantern at hand height, beside the body (outside its radius), on its right hand side:
    // facing east, right is south (+Z).
    let c = Character::default();
    let [x, y, z] = carried.position;
    assert!(
        (y - (2.0 + c.height_m * light::HAND_HEIGHT_SHARE)).abs() < 1e-9,
        "{y}"
    );
    let side = ((x - 10.0).powi(2) + z.powi(2)).sqrt();
    assert!(side > c.radius_m, "the lantern is outside the body: {side}");
    assert!(z > 0.0 && (x - 10.0).abs() < 1e-9, "{:?}", carried.position);
    assert_eq!(carried.color, [1.0, 0.8, 0.5]);
    assert!((carried.lumens - 50.0 * 4.0 * std::f64::consts::PI).abs() < 1e-9);
    assert_eq!(carried.range_m, 8.0);

    // On a box: over its top (base 1, 3 m tall).
    assert_eq!(beacon.position[0], 0.0);
    assert_eq!(beacon.position[2], 5.0);
    let top = 1.0 + 3.0 * world.entities().nth(1).unwrap().scale;
    assert!(
        beacon.position[1] > top && beacon.position[1] < top + 0.5,
        "{:?} over the top {top}",
        beacon.position
    );
    // On its own: a little over its base.
    assert!(alone.position[1] > 0.5 && alone.position[1] <= 0.5 + 1.5);

    // The carried light moves with its character.
    let id = world.entities().next().unwrap().id;
    world.entity_mut(id).unwrap().position = [20.0, 2.0, 0.0];
    let moved = light::spots(&world);
    assert!((moved[0].position[0] - 20.0).abs() < 1e-9);
}

#[test]
fn quality_names_parse_and_a_wrong_one_names_the_choices() {
    assert_eq!(Quality::parse("low"), Ok(Quality::Low));
    assert_eq!(Quality::parse("medium"), Ok(Quality::Medium));
    assert_eq!(Quality::parse("high"), Ok(Quality::High));
    assert_eq!(Quality::default(), Quality::Medium);
    for q in [Quality::Low, Quality::Medium, Quality::High] {
        assert_eq!(Quality::parse(q.name()), Ok(q));
    }
    let e = Quality::parse("ultra").unwrap_err();
    assert!(
        e.contains("ultra") && e.contains("low, medium, high"),
        "{e}"
    );
}

// ---- combat effects ----

mod combat {
    use crate::Camera;
    use crate::effects::{
        self, CROSSHAIR_INNER_PX, CROSSHAIR_OUTER_PX, IMPACT_LIFE_S, IMPACT_RADIUS_M, Impact,
        Tracer,
    };

    fn tracer(age_s: f64) -> Tracer {
        Tracer {
            from: [0.0, 1.0, 0.0],
            to: [0.0, 1.0, -20.0],
            age_s,
        }
    }

    #[test]
    fn a_tracer_is_a_wide_dim_glow_under_a_thin_bright_core() {
        let segments = effects::tracer_segments(&tracer(0.0));
        assert_eq!(segments.len(), 2);
        let (glow, core) = (segments[0], segments[1]);
        assert!(glow.width_px > core.width_px, "the glow is wider");
        assert!(glow.rgba[3] < core.rgba[3], "the glow is dimmer");
        assert_eq!((core.from, core.to), ([0.0, 1.0, 0.0], [0.0, 1.0, -20.0]));
        let old = effects::tracer_segments(&tracer(10.0));
        assert!(old[1].rgba[3] < core.rgba[3], "an old tracer is dimmer");
        assert!(old[1].rgba[3] > 0, "but still there");
    }

    #[test]
    fn a_tracer_that_is_not_finite_is_left_out() {
        let mut t = tracer(0.0);
        t.to[1] = f64::NAN;
        assert!(effects::tracer_segments(&t).is_empty());
    }

    fn impact(age_s: f64) -> Impact {
        Impact {
            point: [1.0, 2.0, 3.0],
            normal: [0.0, 1.0, 0.0],
            age_s,
        }
    }

    #[test]
    fn a_puff_swells_then_shrinks_and_is_gone_after_a_second() {
        let radius = |age| effects::impact_puff(&impact(age)).map(|(_, r)| r);
        assert_eq!(radius(0.0), Some(0.0));
        let swelling = radius(0.06).unwrap();
        let full = radius(0.12).unwrap();
        let fading = radius(0.6).unwrap();
        assert!(swelling < full && fading < full && fading > 0.0);
        assert!((full - IMPACT_RADIUS_M).abs() < 1e-9);
        assert_eq!(radius(IMPACT_LIFE_S), None);
        assert_eq!(radius(-0.1), None);
        assert_eq!(radius(f64::NAN), None);
    }

    #[test]
    fn a_puff_rises_off_the_surface_along_its_normal() {
        let (centre, radius) = effects::impact_puff(&impact(0.12)).unwrap();
        assert_eq!((centre[0], centre[2]), (1.0, 3.0));
        assert!((centre[1] - (2.0 + radius * 0.5)).abs() < 1e-9);
    }

    #[test]
    fn the_crosshair_is_four_arms_of_a_fixed_pixel_length_round_the_view_centre() {
        let camera = Camera {
            position: [0.0, 1.6, 0.0],
            target: [0.0, 1.6, -10.0],
            fov_y_deg: 70.0,
        };
        let height = 720;
        let segments = effects::crosshair_segments(&camera, height);
        assert_eq!(segments.len(), 8, "four outlines and four arms");
        let metres_per_px = 2.0 * (35.0_f64).to_radians().tan() / f64::from(height);
        for s in &segments[4..] {
            // One metre ahead of the eye, along the view.
            assert!((s.from[2] + 1.0).abs() < 1e-9 && (s.to[2] + 1.0).abs() < 1e-9);
            let d = ((s.to[0] - s.from[0]).powi(2) + (s.to[1] - s.from[1]).powi(2)).sqrt();
            let want = (CROSSHAIR_OUTER_PX - CROSSHAIR_INNER_PX) * metres_per_px;
            assert!((d - want).abs() < 1e-9, "{d} against {want}");
        }
        assert!(
            segments[..4].iter().all(|s| s.rgba[0] == 0),
            "dark outlines first"
        );
        assert!(effects::crosshair_segments(&camera, 0).is_empty());
        let degenerate = Camera {
            target: camera.position,
            ..camera
        };
        assert!(effects::crosshair_segments(&degenerate, height).is_empty());
    }
}

#[test]
fn painted_layers_reach_the_chunks_and_the_apron() {
    let mut t = noise_terrain(256.0, 2.0);
    t.layers = ["grass", "rock", "road"]
        .iter()
        .map(|n| gamecraft_world::TerrainLayer {
            name: (*n).into(),
            color: [0.2, 0.2, 0.2],
            roughness: 0.9,
        })
        .collect();
    t.edits = vec![gamecraft_world::TerrainEdit::Paint {
        layer: "road".into(),
        center: [0.0, 0.0],
        radius_m: 20.0,
        strength: 1.0,
        falloff: 0.2,
    }];
    let h = t.build().unwrap();
    // Every vertex carries its sample's weights from the height field.
    for chunk in mesh::terrain_chunks(&h) {
        let nx = chunk.mesh.positions.len();
        assert_eq!(chunk.weights.len(), nx);
        let cols = 1 + (h.cols() - 1 - chunk.first_col).min(mesh::TERRAIN_CHUNK_CELLS);
        for (i, w) in chunk.weights.iter().enumerate() {
            let (c, r) = (chunk.first_col + i % cols, chunk.first_row + i / cols);
            assert_eq!(Some(*w), h.weight(c, r), "({c}, {r})");
        }
    }
    let centre = mesh::terrain_chunks(&h)
        .into_iter()
        .flat_map(|c| c.mesh.positions.into_iter().zip(c.weights))
        .find(|(p, _)| p[0] == 0.0 && p[2] == 0.0)
        .unwrap();
    assert_eq!(centre.1[2], 255);
    // The apron's border ring is the terrain's weights; its outer ring the first layer.
    let apron = mesh::terrain_apron(&h, 0.0);
    let per = apron.weights.len() / (mesh::APRON_RINGS + 1);
    assert_eq!(apron.weights[0], h.weight(0, 0).unwrap());
    for w in apron.weights.iter().skip(per * mesh::APRON_RINGS) {
        assert_eq!(w[0], 255, "{w:?}");
    }
    for w in &apron.weights {
        assert_eq!(w.iter().map(|v| u32::from(*v)).sum::<u32>(), 255);
    }
}
