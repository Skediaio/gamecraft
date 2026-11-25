//! Levels of detail of models: how a file says them (on the CPU), and on a real GPU the
//! finest level near the camera, the coarsest farther away and nothing beyond the draw
//! distance; the distances from the component, the file and the rule from the size; a field
//! of 400 crates over a kilometre that draws far fewer triangles than it holds; the
//! renderer's own frustum culling; a broken set; and a character the camera follows. All
//! asserted on the numbers the renderer reports (`Picture::stats`), which are checked against
//! what the renderer submitted. The GPU tests skip without an adapter (and fail with
//! `GAMECRAFT_REQUIRE_GPU=1`). The models are built in the test: meshes of unit cubes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use gamecraft_project::textures::pack_glb;
use gamecraft_render::model;
use gamecraft_render::{Camera, LevelCount, Picture, PictureRequest, WorldRenderer, headless_gpu};
use gamecraft_world::{Character, Component, EntityDraft, Model, World};
use serde_json::{Value, json};
use std::sync::{Mutex, MutexGuard, OnceLock};

const CRATE: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/crate.glb");
const ASSET: &str = "models/thing.glb";

// ------------------------------------------------------------------------ the file, on the CPU

/// A node of the test models.
struct Node {
    name: &'static str,
    mesh: Option<usize>,
    extensions: Value,
    extras: Value,
}

fn node(name: &'static str, mesh: Option<usize>) -> Node {
    Node {
        name,
        mesh,
        extensions: json!({}),
        extras: Value::Null,
    }
}

/// A GLB with a mesh per count of `boxes` (unit cubes side by side along x, 12 triangles
/// each, no material), the `nodes` over them showing `roots`, and `extras` on its asset.
fn glb(boxes: &[usize], nodes: &[Node], roots: &[usize], extras: Value) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    for &count in boxes {
        let offset = bin.len();
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for b in 0..count {
            for face in 0..6 {
                for corner in 0..6 {
                    let p = corner_of(face, corner, b);
                    for k in 0..3 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                        bin.extend_from_slice(&p[k].to_le_bytes());
                    }
                }
            }
        }
        views.push(json!({"buffer": 0, "byteOffset": offset, "byteLength": bin.len() - offset}));
        accessors.push(json!({
            "bufferView": views.len() - 1,
            "componentType": 5126,
            "count": count * 36,
            "type": "VEC3",
            "min": lo,
            "max": hi
        }));
        meshes.push(json!({"primitives": [{"attributes": {"POSITION": accessors.len() - 1}}]}));
    }
    let nodes_json: Vec<Value> = nodes
        .iter()
        .map(|n| {
            let mut value = json!({"name": n.name});
            if let Some(mesh) = n.mesh {
                value["mesh"] = json!(mesh);
            }
            if n.extensions.as_object().is_some_and(|o| !o.is_empty()) {
                value["extensions"] = n.extensions.clone();
            }
            if !n.extras.is_null() {
                value["extras"] = n.extras.clone();
            }
            value
        })
        .collect();
    let mut doc = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": roots}],
        "nodes": nodes_json,
        "meshes": meshes,
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": accessors,
    });
    if !extras.is_null() {
        doc["asset"]["extras"] = extras;
    }
    if nodes
        .iter()
        .any(|n| !n.extensions.as_object().is_some_and(|o| o.is_empty()))
    {
        doc["extensionsUsed"] = json!(["MSFT_lod"]);
    }
    pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap()
}

/// Corner `corner` (0 to 5) of side `face` of a unit cube at x = `b`, standing on y = 0,
/// counter-clockwise from outside.
fn corner_of(face: usize, corner: usize, b: usize) -> [f32; 3] {
    let (axis, sign) = [
        (0, 0.0f32),
        (0, 1.0),
        (1, 0.0),
        (1, 1.0),
        (2, 0.0),
        (2, 1.0),
    ][face];
    let (a, c) = match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let (u, v) = [
        (0.0, 0.0),
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 0.0),
        (0.0, 1.0),
        (1.0, 1.0),
    ][corner];
    let mut p = [0.0; 3];
    p[axis] = sign;
    p[a] = u;
    p[c] = v;
    p[0] += b as f32;
    p
}

/// A thing of three levels named Unity style: 3, 2 and 1 cubes.
fn named() -> Vec<u8> {
    glb(
        &[3, 2, 1],
        &[
            node("Thing_LOD0", Some(0)),
            node("Thing_LOD1", Some(1)),
            node("Thing_LOD2", Some(2)),
        ],
        &[0, 1, 2],
        Value::Null,
    )
}

/// The same by `MSFT_lod`: the finest node carries the extension, the others stand outside
/// the scene.
fn msft() -> Vec<u8> {
    let mut high = node("High", Some(0));
    high.extensions = json!({"MSFT_lod": {"ids": [1, 2]}});
    glb(
        &[3, 2, 1],
        &[high, node("Medium", Some(1)), node("Low", Some(2))],
        &[0],
        Value::Null,
    )
}

/// The same levels backwards: each coarser level has more triangles than the one before.
fn backwards() -> Vec<u8> {
    glb(
        &[1, 2, 3],
        &[
            node("Thing_LOD0", Some(0)),
            node("Thing_LOD1", Some(1)),
            node("Thing_LOD2", Some(2)),
        ],
        &[0, 1, 2],
        Value::Null,
    )
}

/// The named thing with `asset.extras`.
fn named_with(extras: Value) -> Vec<u8> {
    glb(
        &[3, 2, 1],
        &[
            node("Thing_LOD0", Some(0)),
            node("Thing_LOD1", Some(1)),
            node("Thing_LOD2", Some(2)),
        ],
        &[0, 1, 2],
        extras,
    )
}

fn level(list: Vec<LevelCount>, k: usize) -> LevelCount {
    list.get(k).copied().unwrap_or_default()
}

#[test]
fn a_file_says_its_levels_by_names_or_by_the_extension() {
    for bytes in [named(), msft()] {
        let m = model::parse(&bytes, &|_: &str| None).unwrap();
        assert_eq!(m.lods.levels, 3);
        assert_eq!(m.triangles(), 36);
        assert_eq!((m.triangles_at(1), m.triangles_at(2)), (Some(24), Some(12)));
        assert_eq!(m.lods.triangles, vec![36, 24, 12]);
        // One prim per level, in the order the scene is walked: the finest level at the
        // group's node, then the others standing in (the last one pushed is popped first).
        assert_eq!(m.lods.drawn_at, vec![1, 4, 2], "one prim per level");
        assert!(m.lods.file.is_none() && m.lods.warnings.is_empty());
        assert!((m.lods.size_m - 11f64.sqrt()).abs() < 1e-12);
        assert!(m.lods.source.is_some());
    }
    // Shared geometry belongs to every level; a crate has one level as before.
    let shared = glb(
        &[3, 1, 1],
        &[
            node("Slab", Some(1)),
            node("Thing_LOD0", Some(0)),
            node("Thing_LOD1", Some(2)),
        ],
        &[0, 1, 2],
        Value::Null,
    );
    let m = model::parse(&shared, &|_: &str| None).unwrap();
    // The finest level at the group's node, its coarser level standing in, then the slab.
    assert_eq!(m.lods.drawn_at, vec![1, 2, 3], "{:?}", m.lods);
    assert_eq!(m.lods.triangles, vec![48, 24]);
    assert_eq!(m.triangles(), 48);
    let crate_ = model::parse(CRATE, &|_: &str| None).unwrap();
    assert_eq!((crate_.lods.levels, crate_.triangles()), (1, 12));
    assert_eq!(crate_.lods.drawn_at, vec![1]);
    // The file's distances and a broken set come back as said.
    let m = model::parse(
        &named_with(json!({"gamecraft": {"draw_distance_m": 300}})),
        &|_: &str| None,
    )
    .unwrap();
    assert_eq!(m.lods.file.as_ref().unwrap().draw_m, Some(300.0));
    let m = model::parse(&backwards(), &|_: &str| None).unwrap();
    assert_eq!(
        m.lods.warnings,
        vec![
            "level 1 has more triangles (24) than level 0 (12); the levels should get coarser, \
             check their order (the model is drawn as it is)",
            "level 2 has more triangles (36) than level 1 (24); the levels should get coarser, \
             check their order (the model is drawn as it is)",
        ]
    );
}

#[test]
fn every_truncation_and_mutation_of_a_file_with_levels_is_an_error_never_a_panic() {
    for bytes in [named(), msft(), backwards()] {
        for cut in 0..bytes.len() {
            let _ = model::parse(&bytes[..cut], &|_: &str| None);
        }
        for at in (0..bytes.len()).step_by(3) {
            let mut bad = bytes.clone();
            bad[at] ^= 0xA5;
            if let Ok(m) = model::parse(&bad, &|_: &str| None) {
                for k in 0..m.lods.levels {
                    let _ = m.triangles_at(k);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------- on a GPU

static SERIAL: Mutex<()> = Mutex::new(());

fn renderer() -> Option<(MutexGuard<'static, ()>, WorldRenderer)> {
    static GPU: OnceLock<Result<(wgpu::Device, wgpu::Queue), String>> = OnceLock::new();
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    match GPU.get_or_init(|| headless_gpu().map_err(|e| e.to_string())) {
        Ok((device, queue)) => Some((guard, WorldRenderer::new(device.clone(), queue.clone()))),
        Err(e) => {
            if std::env::var("GAMECRAFT_REQUIRE_GPU").as_deref() == Ok("1") {
                panic!("GAMECRAFT_REQUIRE_GPU=1 but there is no GPU: {e}");
            }
            println!("skipping GPU test: {e}");
            None
        }
    }
}

/// A world without terrain (the ground plane is not drawn: the counts are the models' own).
fn world() -> World {
    World::new("levels", 1)
}

/// A model of `asset` at `at` on the ground, its component changed by `set`.
fn add(world: &mut World, name: &str, asset: &str, at: [f64; 3], set: impl FnOnce(&mut Model)) {
    let mut m = Model::of(asset);
    set(&mut m);
    world
        .add_entity(EntityDraft {
            name: name.into(),
            position: at,
            components: vec![Component::Model(m)],
            ..EntityDraft::default()
        })
        .unwrap();
}

/// A camera at `position` looking at `target`, 60 degrees.
fn view(position: [f64; 3], target: [f64; 3]) -> Camera {
    Camera {
        position,
        target,
        fov_y_deg: 60.0,
    }
}

fn picture(r: &mut WorldRenderer, camera: Camera) -> Picture {
    r.render(&PictureRequest {
        width: 320,
        height: 240,
        camera,
        time_s: 0.0,
    })
    .unwrap()
}

/// Triangles the view pass submits with nothing but the sky: the base every model adds to.
fn baseline(r: &mut WorldRenderer, camera: Camera) -> u64 {
    r.sync(&world(), None);
    picture(r, camera).stats.forward_triangles
}

/// The one model row of a picture.
fn one(p: &Picture) -> gamecraft_render::ModelStats {
    assert_eq!(p.stats.models.len(), 1, "{:?}", p.stats.models);
    p.stats.models[0].clone()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

#[test]
fn the_finest_level_is_near_the_coarsest_far_and_nothing_beyond_the_draw_distance() {
    for bytes in [named(), msft()] {
        let Some((_gpu, mut r)) = renderer() else {
            return;
        };
        let mut w = world();
        for (name, z) in [("Near", -10.0), ("Far", -100.0), ("Too far", -1000.0)] {
            add(&mut w, name, ASSET, [0.0, 0.0, z], |m| {
                m.lod_distances_m = Some(vec![20.0, 60.0]);
                m.draw_distance_m = Some(150.0);
            });
        }
        r.provide_asset(ASSET, bytes);
        let camera = view([0.0, 0.5, 0.0], [0.0, 0.5, -10.0]);
        let base = baseline(&mut r, camera);
        assert_eq!(r.sync(&w, None), Vec::<String>::new());
        let p = picture(&mut r, camera);
        let m = one(&p);
        assert_eq!(m.asset, ASSET);
        assert_eq!((m.instances, m.drawn, m.culled), (3, 2, 1), "{m:?}");
        assert_eq!(
            level(m.levels.clone(), 0),
            LevelCount {
                instances: 1,
                triangles: 36
            }
        );
        assert_eq!(
            level(m.levels.clone(), 1),
            LevelCount {
                instances: 0,
                triangles: 0
            }
        );
        assert_eq!(
            level(m.levels.clone(), 2),
            LevelCount {
                instances: 1,
                triangles: 12
            }
        );
        assert_eq!(m.until_m, vec![20.0, 60.0, 150.0]);
        // What the renderer submitted in the view: the two levels that drew, no more.
        assert_eq!(p.stats.forward_triangles, base + 36 + 12, "{:?}", p.stats);
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        // The camera 50 m further back: the levels change by the same distances (79.5 m is
        // past the second change).
        let far = view([0.0, 0.5, 40.0], [0.0, 0.5, 0.0]);
        let p = picture(&mut r, far);
        let m = one(&p);
        assert_eq!((m.drawn, m.culled), (2, 1), "{m:?}");
        assert_eq!(
            level(m.levels.clone(), 0),
            LevelCount {
                instances: 0,
                triangles: 0
            }
        );
        assert_eq!(
            level(m.levels.clone(), 1),
            LevelCount {
                instances: 1,
                triangles: 24
            }
        );
        assert_eq!(
            level(m.levels.clone(), 2),
            LevelCount {
                instances: 1,
                triangles: 12
            }
        );
        assert_eq!(p.stats.forward_triangles, base + 24 + 12, "{:?}", p.stats);
    }
}

#[test]
fn a_model_without_levels_is_drawn_as_before_and_culled_by_its_size() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let mut w = world();
    add(
        &mut w,
        "Near",
        "models/crate.glb",
        [0.0, 0.0, -10.0],
        |_| {},
    );
    add(
        &mut w,
        "Far",
        "models/crate.glb",
        [0.0, 0.0, -5000.0],
        |_| {},
    );
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    let camera = view([0.0, 0.5, 0.0], [0.0, 0.5, -10.0]);
    let base = baseline(&mut r, camera);
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    let p = picture(&mut r, camera);
    let m = one(&p);
    assert_eq!((m.instances, m.drawn, m.culled), (2, 1, 1), "{m:?}");
    assert_eq!(
        m.levels,
        vec![LevelCount {
            instances: 1,
            triangles: 12
        }]
    );
    // 0.9 m on a side, 1.56 m across: drawn to about 364 m (the rule).
    assert!(
        m.until_m.len() == 1 && (364.0..366.0).contains(&m.until_m[0]),
        "{m:?}"
    );
    assert_eq!(p.stats.forward_triangles, base + 12, "{:?}", p.stats);
    assert_eq!(r.model_uploads(), 1);
    // Scaled ten times, it counts ten times the size: 9 m crates are drawn to 3.6 km.
    let mut big = world();
    add(
        &mut big,
        "Big",
        "models/crate.glb",
        [0.0, 0.0, -3000.0],
        |m| m.scale = 10.0,
    );
    assert_eq!(r.sync(&big, None), Vec::<String>::new());
    let m = one(&picture(&mut r, camera));
    assert_eq!(m.drawn, 1, "{m:?}");
    assert!(m.until_m[0] > 3600.0, "{m:?}");
}

#[test]
fn the_component_wins_over_the_file_which_wins_over_the_rule() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    r.provide_asset(
        ASSET,
        named_with(json!({"gamecraft": {"draw_distance_m": 300}})),
    );
    let mut w = world();
    add(&mut w, "Rule", ASSET, [0.0, 0.0, -10.0], |_| {});
    add(&mut w, "Component", ASSET, [20.0, 0.0, -10.0], |m| {
        m.draw_distance_m = Some(250.0)
    });
    add(&mut w, "Both", ASSET, [40.0, 0.0, -10.0], |m| {
        m.lod_distances_m = Some(vec![30.0]);
        m.draw_distance_m = Some(400.0);
    });
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    let m = r.count_models(&view([0.0, 0.5, 0.0], [20.0, 0.5, -10.0]), 4.0 / 3.0);
    assert_eq!(m.len(), 3, "{m:?}");
    // The rule: 3.32 m across, changes at 11.5 m and 46 m; the file: drawn to 300 m.
    let size = 11f64.sqrt();
    let first = size / (2.0 * 0.5773502691896257 * 0.25);
    let second = size / (2.0 * 0.5773502691896257 * 0.0625);
    assert!(
        close(m[0].until_m[0], first) && close(m[0].until_m[1], second),
        "{m:?}"
    );
    assert_eq!(m[0].until_m[2], 300.0);
    // The component: its draw distance; the rest as the file and the rule say.
    assert_eq!(m[1].until_m, vec![m[0].until_m[0], m[0].until_m[1], 250.0]);
    // Both: the first change from the component, the second from the rule, the end from the
    // component.
    assert_eq!(m[2].until_m, vec![30.0, m[0].until_m[1], 400.0]);
    assert!(m.iter().all(|row| row.instances == 1), "{m:?}");
    // A picture from this camera carries the same rows.
    let p = picture(&mut r, view([0.0, 0.5, 0.0], [20.0, 0.5, -10.0]));
    assert_eq!(p.stats.models.len(), 3);
    assert_eq!(p.stats.models[1].until_m[2], 250.0);
    // Another distance is other geometry: the scene is built again.
    let rebuilds = r.rebuild_count();
    let both = w.find("Both").unwrap();
    w.entity_mut(both)
        .unwrap()
        .set_component(Component::Model(Model {
            draw_distance_m: Some(450.0),
            ..Model::of(ASSET)
        }));
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    assert!(r.rebuild_count() > rebuilds, "a new draw distance rebuilds");
    let p = picture(&mut r, view([0.0, 0.5, 0.0], [20.0, 0.5, -10.0]));
    assert_eq!(p.stats.models[2].until_m[2], 450.0);
    // Too many distances for three levels: said, and the extra one is not used.
    let rule = w.find("Rule").unwrap();
    w.entity_mut(rule)
        .unwrap()
        .set_component(Component::Model(Model {
            lod_distances_m: Some(vec![5.0, 9.0, 50.0]),
            ..Model::of(ASSET)
        }));
    let problems = r.sync(&w, None);
    assert!(
        problems
            .iter()
            .any(|p| p.contains(ASSET) && p.contains("1 switch distance(s) more")),
        "{problems:?}"
    );
    let p = picture(&mut r, view([0.0, 0.5, 0.0], [20.0, 0.5, -10.0]));
    assert_eq!(p.stats.models[0].until_m, vec![5.0, 9.0, 300.0]);
}

#[test]
fn four_hundred_crates_over_a_kilometre_draw_far_fewer_triangles_than_they_hold() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let mut w = world();
    for i in 0..400 {
        let (x, z) = (f64::from(i % 20) * 50.0, f64::from(i / 20) * 50.0);
        add(
            &mut w,
            &format!("Crate {i}"),
            "models/crate.glb",
            [x, 0.0, z],
            |_| {},
        );
    }
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    let camera = view([-40.0, 25.0, -40.0], [475.0, 0.0, 475.0]);
    let base = baseline(&mut r, camera);
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    assert_eq!(r.model_uploads(), 1, "one upload for 400 crates");
    let p = picture(&mut r, camera);
    let m = one(&p);
    assert_eq!(m.instances, 400);
    let drawn: u64 = m.levels.iter().map(|l| l.triangles).sum();
    assert!(m.drawn < 100, "drawn {} of 400", m.drawn);
    assert_eq!(drawn, m.drawn as u64 * 12);
    assert!(drawn * 4 < 400 * 12, "{drawn} triangles of 4800");
    // The renderer submitted exactly what the count says.
    assert_eq!(p.stats.forward_triangles, base + drawn, "{:?}", p.stats);
    // From 300 m above the centre, the crates within about 200 m horizontally are within
    // the draw distance; the far side of the field is not.
    let above = view([475.0, 300.0, 475.0], [475.0, 0.0, 474.9]);
    let p = picture(&mut r, above);
    let m = one(&p);
    assert!(
        m.drawn > 20 && m.drawn < 200,
        "drawn {} of 400 from above",
        m.drawn
    );
}

#[test]
fn the_renderer_culls_what_is_outside_the_view() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let camera = view([0.0, 0.5, 0.0], [0.0, 0.5, -10.0]);
    let base = baseline(&mut r, camera);
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    let mut w = world();
    add(
        &mut w,
        "Front",
        "models/crate.glb",
        [0.0, 0.0, -10.0],
        |_| {},
    );
    add(&mut w, "Back", "models/crate.glb", [0.0, 0.0, 10.0], |_| {});
    add(
        &mut w,
        "Side",
        "models/crate.glb",
        [-200.0, 0.0, -10.0],
        |_| {},
    );
    add(
        &mut w,
        "Up",
        "models/crate.glb",
        [0.0, 3000.0, -30.0],
        |_| {},
    );
    // Out of the view by itself, but in the 8 m cell of the crate in front: the renderer
    // decides per cell, on the ground, so it is drawn with it.
    add(
        &mut w,
        "Above",
        "models/crate.glb",
        [0.0, 20.0, -10.0],
        |_| {},
    );
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    let p = picture(&mut r, camera);
    let m = one(&p);
    assert_eq!((m.instances, m.drawn, m.culled), (5, 2, 3), "{m:?}");
    // Only the cell in the view was submitted: the renderer's own frustum culling, per cell.
    assert_eq!(p.stats.forward_triangles, base + 24, "{:?}", p.stats);
    // Turned around, it is the other way round.
    let back = view([0.0, 0.5, 0.0], [0.0, 0.5, 10.0]);
    let p = picture(&mut r, back);
    assert_eq!(
        p.stats.forward_triangles,
        baseline(&mut r, back) + 12,
        "{:?}",
        p.stats
    );
}

#[test]
fn a_broken_set_of_levels_warns_and_is_drawn() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let mut w = world();
    add(&mut w, "Near", ASSET, [0.0, 0.0, -10.0], |_| {});
    add(&mut w, "Far", ASSET, [0.0, 0.0, -100.0], |_| {});
    r.provide_asset(ASSET, backwards());
    let problems = r.sync(&w, None);
    assert!(
        problems.iter().any(|p| p.contains(ASSET)
            && p.contains("level 1 has more triangles (24) than level 0 (12)")),
        "{problems:?}"
    );
    let p = picture(&mut r, view([0.0, 0.5, 0.0], [0.0, 0.5, -10.0]));
    // Both hand-overs run the wrong way; each is said once.
    assert_eq!(
        p.problems
            .iter()
            .filter(|p| p.contains("more triangles"))
            .count(),
        2,
        "{:?}",
        p.problems
    );
    let m = one(&p);
    assert_eq!((m.drawn, m.culled), (2, 0), "{m:?}");
    assert_eq!(
        m.levels,
        vec![
            LevelCount {
                instances: 1,
                triangles: 12
            },
            LevelCount {
                instances: 0,
                triangles: 0
            },
            LevelCount {
                instances: 1,
                triangles: 36
            }
        ]
    );
}

#[test]
fn a_character_the_camera_follows_is_never_culled_by_distance() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let mut w = world();
    let walker = w
        .add_entity(EntityDraft {
            name: "Walker".into(),
            position: [0.0, 0.0, -2000.0],
            components: vec![
                Component::Character(Character::default()),
                Component::Model(Model::of("models/crate.glb")),
            ],
            ..EntityDraft::default()
        })
        .unwrap();
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    let camera = view([0.0, 2.0, -1995.0], [0.0, 1.0, -2000.0]);
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    let m = one(&picture(&mut r, camera));
    assert_eq!((m.drawn, m.culled), (1, 0), "five metres from the camera");
    // Far beyond the draw distance: culled like any model.
    w.entity_mut(walker).unwrap().position = [0.0, 0.0, -5000.0];
    r.sync(&w, None);
    let m = one(&picture(&mut r, camera));
    assert_eq!((m.drawn, m.culled), (0, 1), "3 km from the camera: {m:?}");
    // Followed by the camera: drawn at every distance; the count says so.
    r.set_followed_character(Some(walker));
    let rebuilds = r.rebuild_count();
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    assert!(
        r.rebuild_count() > rebuilds,
        "the scene is built again for the followed model"
    );
    let p = picture(&mut r, camera);
    let m = one(&p);
    assert_eq!((m.drawn, m.culled), (1, 0), "{m:?}");
    assert_eq!(m.until_m, vec![f64::INFINITY]);
    assert_eq!(p.stats.instances, 1, "the renderer drew it: {:?}", p.stats);
    // Let go: culled again.
    r.set_followed_character(None);
    r.sync(&w, None);
    assert_eq!(one(&picture(&mut r, camera)).culled, 1);
}
