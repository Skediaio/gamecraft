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
use gamecraft_render::{
    Camera, HlodStats, LevelCount, Picture, PictureRequest, Quality, WorldRenderer, headless_gpu,
};
use gamecraft_world::{Character, Component, EntityDraft, Model, Shape, ShapeKind, World};
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
    glb_modes(boxes, &[], nodes, roots, extras)
}

/// [`glb`] with the glTF `mode` of each mesh (4 triangles when left out; 5 a strip, 6 a fan).
fn glb_modes(
    boxes: &[usize],
    modes: &[u16],
    nodes: &[Node],
    roots: &[usize],
    extras: Value,
) -> Vec<u8> {
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
        let mut primitive = json!({"attributes": {"POSITION": accessors.len() - 1}});
        if let Some(&mode) = modes.get(meshes.len()) {
            primitive["mode"] = json!(mode);
        }
        meshes.push(json!({"primitives": [primitive]}));
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

/// What the row of the file's level `k` says (nothing drawn when the row is missing).
fn level(list: Vec<LevelCount>, k: usize) -> LevelCount {
    list.iter()
        .find(|l| l.level == k)
        .copied()
        .unwrap_or(lc(k, 0, 0))
}

/// The row of the file's level `level`.
fn lc(level: usize, instances: usize, triangles: u64) -> LevelCount {
    LevelCount {
        level,
        instances,
        triangles,
    }
}

#[test]
fn a_file_says_its_levels_by_names_or_by_the_extension() {
    for bytes in [named(), msft()] {
        let m = model::parse(&bytes, &|_: &str| None).unwrap();
        assert_eq!(m.lods.read.levels, 3);
        assert_eq!(m.triangles(), 36);
        assert_eq!((m.triangles_at(1), m.triangles_at(2)), (Some(24), Some(12)));
        assert_eq!(m.lods.read.triangles, vec![36, 24, 12]);
        // One prim per level, in the order the scene is walked: the finest level at the
        // group's node, then the others standing in (the last one pushed is popped first).
        assert_eq!(m.lods.drawn_at, vec![1, 4, 2], "one prim per level");
        assert!(m.lods.read.file.is_none() && m.lods.read.warnings.is_empty());
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
    assert_eq!(m.lods.read.triangles, vec![48, 24]);
    assert_eq!(m.triangles(), 48);
    let crate_ = model::parse(CRATE, &|_: &str| None).unwrap();
    assert_eq!((crate_.lods.read.levels, crate_.triangles()), (1, 12));
    assert_eq!(crate_.lods.drawn_at, vec![1]);
    // The file's distances and a broken set come back as said.
    let m = model::parse(
        &named_with(json!({"gamecraft": {"draw_distance_m": 300}})),
        &|_: &str| None,
    )
    .unwrap();
    assert_eq!(m.lods.read.file.as_ref().unwrap().draw_m, Some(300.0));
    let m = model::parse(&backwards(), &|_: &str| None).unwrap();
    assert_eq!(
        m.lods.read.warnings,
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
                for k in 0..m.lods.read.levels {
                    let _ = m.triangles_at(k);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------- on a GPU

static SERIAL: Mutex<()> = Mutex::new(());

fn gpu() -> Option<(MutexGuard<'static, ()>, wgpu::Device, wgpu::Queue)> {
    static GPU: OnceLock<Result<(wgpu::Device, wgpu::Queue), String>> = OnceLock::new();
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    match GPU.get_or_init(|| headless_gpu().map_err(|e| e.to_string())) {
        Ok((device, queue)) => Some((guard, device.clone(), queue.clone())),
        Err(e) => {
            if std::env::var("GAMECRAFT_REQUIRE_GPU").as_deref() == Ok("1") {
                panic!("GAMECRAFT_REQUIRE_GPU=1 but there is no GPU: {e}");
            }
            println!("skipping GPU test: {e}");
            None
        }
    }
}

fn renderer() -> Option<(MutexGuard<'static, ()>, WorldRenderer)> {
    let (guard, device, queue) = gpu()?;
    Some((guard, WorldRenderer::new(device, queue)))
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
        assert_eq!(level(m.levels.clone(), 0), lc(0, 1, 36));
        assert_eq!(level(m.levels.clone(), 1), lc(1, 0, 0));
        assert_eq!(level(m.levels.clone(), 2), lc(2, 1, 12));
        // The distances are given in metres: nothing beyond the draw distance, far level or
        // not (the cell's merged mesh shows the model again from 300 m).
        assert_eq!(m.own_until_m, vec![20.0, 60.0, 150.0]);
        assert_eq!(m.until_m, vec![20.0, 60.0, 150.0], "{m:?}");
        // What the renderer submitted in the view: the two levels that drew, no more.
        assert_eq!(
            p.stats.forward_triangles,
            base + 36 + 12 + p.stats.hlod.triangles,
            "{:?}",
            p.stats
        );
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        // The camera 50 m further back: the levels change by the same distances (79.5 m is
        // past the second change).
        let far = view([0.0, 0.5, 40.0], [0.0, 0.5, 0.0]);
        let p = picture(&mut r, far);
        let m = one(&p);
        assert_eq!((m.drawn, m.culled), (2, 1), "{m:?}");
        assert_eq!(level(m.levels.clone(), 0), lc(0, 0, 0));
        assert_eq!(level(m.levels.clone(), 1), lc(1, 1, 24));
        assert_eq!(level(m.levels.clone(), 2), lc(2, 1, 12));
        assert_eq!(
            p.stats.forward_triangles,
            base + 24 + 12 + p.stats.hlod.triangles,
            "{:?}",
            p.stats
        );
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
    assert_eq!(m.levels, vec![lc(0, 1, 12)]);
    // 0.9 m on a side, 1.56 m across: drawn to about 364 m (the rule).
    assert!(
        m.own_until_m.len() == 1 && (364.0..366.0).contains(&m.own_until_m[0]),
        "{m:?}"
    );
    assert_eq!(
        p.stats.forward_triangles,
        base + 12 + p.stats.hlod.triangles,
        "{:?}",
        p.stats
    );
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
    assert!(m.own_until_m[0] > 3600.0, "{m:?}");
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
        close(m[0].own_until_m[0], first) && close(m[0].own_until_m[1], second),
        "{m:?}"
    );
    assert_eq!(m[0].own_until_m[2], 300.0);
    // The component: its draw distance; the rest as the file and the rule say.
    assert_eq!(
        m[1].own_until_m,
        vec![m[0].own_until_m[0], m[0].own_until_m[1], 250.0]
    );
    // Both: the first change from the component, the second from the rule, the end from the
    // component.
    assert_eq!(m[2].own_until_m, vec![30.0, m[0].own_until_m[1], 400.0]);
    assert!(m.iter().all(|row| row.instances == 1), "{m:?}");
    // A picture from this camera carries the same rows.
    let p = picture(&mut r, view([0.0, 0.5, 0.0], [20.0, 0.5, -10.0]));
    assert_eq!(p.stats.models.len(), 3);
    assert_eq!(p.stats.models[1].own_until_m[2], 250.0);
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
    assert_eq!(p.stats.models[2].own_until_m[2], 450.0);
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
    assert_eq!(p.stats.models[0].own_until_m, vec![5.0, 9.0, 300.0]);
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
    assert_eq!(
        p.stats.forward_triangles,
        base + drawn + p.stats.hlod.triangles,
        "{:?}",
        p.stats
    );
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
    assert_eq!(
        p.stats.forward_triangles,
        base + 24 + p.stats.hlod.triangles,
        "{:?}",
        p.stats
    );
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
    // The size is the finest level's (one cube), so its first change is at 6 m: the model
    // 10 m away is already at the second level, the one 100 m away at the third.
    assert_eq!(m.own_until_m[0], 6.0, "{m:?}");
    assert_eq!(m.levels, vec![lc(0, 0, 0), lc(1, 1, 24), lc(2, 1, 36)]);
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

// ------------------------------------------------------------------ review round 1

#[test]
fn the_size_is_the_host_s_strips_and_fans_included() {
    // A strip beside the levels: the host measures the model over its vertices too, so the
    // renderer must as well, or its levels would end at other distances than `asset.list`
    // says.
    let bytes = glb_modes(
        &[3, 1, 5],
        &[4, 4, 5],
        &[
            node("Thing_LOD0", Some(0)),
            node("Thing_LOD1", Some(1)),
            node("Slab", Some(2)),
        ],
        &[0, 1, 2],
        Value::Null,
    );
    let m = model::parse(&bytes, &|_: &str| None).unwrap();
    let s = gamecraft_project::inspect(&bytes, &|_: &str| None).unwrap();
    assert_eq!(m.lods.size_m, s.size_m(), "the same size to the bit");
    assert!(
        (m.lods.size_m - 27f64.sqrt()).abs() < 1e-9,
        "{}",
        m.lods.size_m
    );
    // Both readers count the strip's triangles: the renderer draws a strip as the triangles
    // it stands for.
    assert_eq!(
        (m.lods.read.triangles.clone(), s.lod.triangles.clone()),
        (vec![36 + 178, 12 + 178], vec![36 + 178, 12 + 178])
    );
}

#[test]
fn strips_and_fans_are_drawn_as_their_triangles() {
    // A strip of four points is two triangles, the second wound the other way (2, 1, 3 for
    // the points 1, 2, 3); a fan of four is two around its first point.
    let strip = glb_modes(&[1], &[5], &[node("Ribbon", Some(0))], &[0], Value::Null);
    let m = model::parse(&strip, &|_: &str| None).unwrap();
    assert_eq!((m.lods.read.levels, m.triangles()), (1, 34));
    let first: Vec<u32> = m.prims[0].indices.iter().take(9).copied().collect();
    assert_eq!(first, vec![0, 1, 2, 2, 1, 3, 2, 3, 4]);
    assert_eq!(
        gamecraft_project::inspect(&strip, &|_: &str| None)
            .unwrap()
            .triangles,
        34
    );
    let fan = glb_modes(&[1], &[6], &[node("Fan", Some(0))], &[0], Value::Null);
    let m = model::parse(&fan, &|_: &str| None).unwrap();
    let first: Vec<u32> = m.prims[0].indices.iter().take(9).copied().collect();
    assert_eq!(first, vec![0, 1, 2, 0, 2, 3, 0, 3, 4]);
    assert_eq!(m.triangles(), 34);
}

/// The one model row of a count.
fn only(rows: &[gamecraft_render::ModelStats]) -> gamecraft_render::ModelStats {
    assert_eq!(rows.len(), 1, "{rows:?}");
    rows[0].clone()
}

#[test]
fn a_thousand_scattered_copies_with_their_own_scales_are_a_handful_of_items() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let mut rng = gamecraft_math::Rng::new(7);
    let mut w = world();
    let mut fixed = world();
    for i in 0..1000 {
        let (x, z) = (f64::from(i % 40) * 6.0, f64::from(i / 40) * 6.0);
        let scale = rng.range(0.5, 2.0);
        add(
            &mut w,
            &format!("Crate {i}"),
            "models/crate.glb",
            [x, 0.0, z],
            |m| m.scale = scale,
        );
        add(
            &mut fixed,
            &format!("Crate {i}"),
            "models/crate.glb",
            [x, 0.0, z],
            |m| {
                m.scale = scale;
                m.draw_distance_m = Some(600.0);
            },
        );
    }
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    let camera = view([120.0, 90.0, 140.0], [117.0, 0.0, 72.0]);
    let base = baseline(&mut r, camera);
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    let p = picture(&mut r, camera);
    let rows = &p.stats.models;
    // A handful: one item per bucket of scales and per take-over distance of the cells the
    // bucket's copies lie in.
    assert!(
        (6..=18).contains(&rows.len()),
        "{} items for 1000 copies of their own size",
        rows.len()
    );
    assert_eq!(rows.iter().map(|m| m.instances).sum::<usize>(), 1000);
    let counted: u64 = rows
        .iter()
        .flat_map(|m| m.levels.iter().map(|l| l.triangles))
        .sum();
    // The copies' far levels (their merged meshes) draw where the copies are culled.
    assert!(p.stats.hlod.cells > 0, "{:?}", p.stats.hlod);
    assert_eq!(
        p.stats.forward_triangles,
        base + counted + p.stats.hlod.triangles,
        "{:?}",
        p.stats
    );
    assert!(p.problems.is_empty(), "{:?}", p.problems);
    // A bucket's distances are its upper scale's: a copy is never culled early.
    for row in rows {
        let biggest = row.own_until_m.last().copied().unwrap_or(0.0)
            / (0.9 * 3f64.sqrt() * 270.0 / (2.0 * 0.5773502691896257));
        let smallest = biggest / 1.25;
        assert!(
            row.instances > 0 && biggest >= 0.5 && smallest < 2.0,
            "the bucket {:?} of scales {:.3}..{:.3}",
            row.until_m,
            smallest,
            biggest
        );
    }
    // Distances given in metres do not follow the scale: every copy shares one item.
    assert_eq!(r.sync(&fixed, None), Vec::<String>::new());
    let p = picture(&mut r, camera);
    assert_eq!(p.stats.models.len(), 1, "{:?}", p.stats.models);
    assert_eq!(p.stats.models[0].own_until_m, vec![600.0]);
    let counted: u64 = p
        .stats
        .models
        .iter()
        .flat_map(|m| m.levels.iter().map(|l| l.triangles))
        .sum();
    assert_eq!(
        p.stats.forward_triangles,
        base + counted + p.stats.hlod.triangles,
        "{:?}",
        p.stats
    );
}

#[test]
fn a_live_view_counts_at_its_quality_s_distances_and_a_picture_at_the_written_ones() {
    let Some((_guard, device, queue)) = gpu() else {
        return;
    };
    let mut r = WorldRenderer::new(device.clone(), queue.clone());
    let mut w = world();
    add(
        &mut w,
        "Near",
        "models/crate.glb",
        [0.0, 0.0, -100.0],
        |_| {},
    );
    add(
        &mut w,
        "Middle",
        "models/crate.glb",
        [0.0, 0.0, -300.0],
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
    let camera = view([0.0, 40.0, 20.0], [0.0, 0.0, -100.0]);
    // A live frame of an empty world: what the sky alone submits in the view pass.
    r.resize(320, 240);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 320,
            height: 240,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target = target.create_view(&Default::default());
    assert_eq!(r.sync(&world(), None), Vec::<String>::new());
    let mut encoder = device.create_command_encoder(&Default::default());
    r.draw(&camera, 0.0, &target, &mut encoder).unwrap();
    queue.submit([encoder.finish()]);
    let live_base = r.frame_stats().forward_triangles;
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    // High (the quality of every picture): drawn to 364 m, so two of the three.
    r.set_quality(Quality::High);
    let m = only(&r.count_models(&camera, 4.0 / 3.0));
    assert_eq!((m.drawn, m.culled), (2, 1), "{m:?}");
    // The crate's own end is the size rule's (364 m); where its cell's merged mesh takes
    // over, a cell's diagonal past it (545 m) is where the crate's own item ends.
    let m = only(&r.count_models(&camera, 4.0 / 3.0));
    assert!(
        close(m.own_until_m[0], 364.5) && close(m.until_m[0], 364.5 + 128.0 * 2f64.sqrt()),
        "{m:?}"
    );
    // Medium: the renderer halves every distance, the cells' take-over included (the item
    // ends at 273 m), so the crate at 300 m is beyond its own item: the merged mesh of its
    // cell draws it instead.
    r.set_quality(Quality::Medium);
    let m = only(&r.count_models(&camera, 4.0 / 3.0));
    assert_eq!((m.drawn, m.culled), (1, 2), "{m:?}");
    // The live frame the renderer draws agrees with the count...
    let mut encoder = device.create_command_encoder(&Default::default());
    r.draw(&camera, 0.0, &target, &mut encoder).unwrap();
    queue.submit([encoder.finish()]);
    let live = r.frame_stats();
    // The crate near the camera, the merged meshes of the two cells beyond it, and the near
    // cell's empty level.
    assert_eq!(live.instances, 4, "the live view: {live:?}");
    assert_eq!(live.forward_triangles, live_base + 36, "{live:?}");
    // ...and a picture from the same camera is still drawn at the highest quality.
    let p = picture(&mut r, camera);
    assert_eq!(
        (p.stats.models[0].drawn, p.stats.models[0].culled),
        (2, 1),
        "{:?}",
        p.stats
    );
    // The two crates and the three cells' merged meshes (the near cell's at its empty level).
    assert_eq!(p.stats.instances, 5, "{:?}", p.stats);
    assert_eq!(p.stats.hlod.cells, 3, "{:?}", p.stats);
    // The picture went through the renderer's frame path too; the live counts are their own.
    assert_eq!(
        r.frame_stats(),
        live,
        "a picture does not change the live counts"
    );
    // Low: 40 percent: the item ends at 218 m; the crate at 100 m is still drawn.
    r.set_quality(Quality::Low);
    let m = only(&r.count_models(&camera, 4.0 / 3.0));
    assert_eq!((m.drawn, m.culled), (1, 2), "{m:?}");
}

#[test]
fn a_level_without_geometry_ends_the_model_or_is_left_out() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    // `MSFT_lod` whose last id is a node without a mesh: "draw nothing from here".
    let mut high = node("High", Some(0));
    high.extensions = json!({"MSFT_lod": {"ids": [1, 2]}});
    let last_empty = glb(
        &[3, 2, 1],
        &[high, node("Medium", Some(1)), node("Low", None)],
        &[0],
        Value::Null,
    );
    // Names with the level in the middle missing its mesh.
    let middle_empty = glb(
        &[3, 1, 1],
        &[
            node("Thing_LOD0", Some(0)),
            node("Thing_LOD1", None),
            node("Thing_LOD2", Some(2)),
        ],
        &[0, 1, 2],
        Value::Null,
    );
    for (bytes, missing, until, far_triangles, at_50) in [
        (last_empty, 2, vec![20.0, 60.0], 0, 24),
        (middle_empty, 1, vec![20.0, 150.0], 12, 12),
    ] {
        let mut w = world();
        for (name, z) in [("Near", -10.0), ("Middle", -50.0), ("Far", -100.0)] {
            add(&mut w, name, ASSET, [0.0, 0.0, z], |m| {
                m.lod_distances_m = Some(vec![20.0, 60.0]);
                m.draw_distance_m = Some(150.0);
            });
        }
        r.provide_asset(ASSET, bytes);
        let camera = view([0.0, 0.5, 0.0], [0.0, 0.5, -10.0]);
        let base = baseline(&mut r, camera);
        let problems = r.sync(&w, None);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains(ASSET) && problems[0].contains(&format!("level {missing}")),
            "{problems:?}"
        );
        let p = picture(&mut r, camera);
        let m = one(&p);
        assert_eq!(m.own_until_m, until, "{m:?}");
        assert_eq!(m.levels.len(), 2, "{m:?}");
        // Near: the finest level; Middle: the coarse one that draws; Far: beyond 60 m only
        // where the empty level was the last.
        let (drawn, far) = if missing == 2 {
            (2, 0)
        } else {
            (3, far_triangles)
        };
        assert_eq!((m.drawn, m.culled), (drawn, 3 - drawn), "{m:?}");
        assert_eq!(
            p.stats.forward_triangles,
            base + 36 + at_50 + far,
            "{:?}",
            p.stats
        );
    }
}

#[test]
fn screen_coverage_follows_the_model_s_scale_in_buckets() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let mut w = world();
    add(&mut w, "One", ASSET, [-20.0, 0.0, 0.0], |_| {});
    add(&mut w, "Ten", ASSET, [20.0, 0.0, 0.0], |m| m.scale = 10.0);
    let mut high = node("High", Some(0));
    high.extensions = json!({"MSFT_lod": {"ids": [1, 2]}});
    high.extras = json!({"MSFT_screencoverage": [1.0, 0.25, 0.02]});
    let bytes = glb(
        &[3, 2, 1],
        &[high, node("Medium", Some(1)), node("Low", Some(2))],
        &[0],
        Value::Null,
    );
    r.provide_asset(ASSET, bytes);
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    let rows = r.count_models(&view([0.0, 5.0, 60.0], [0.0, 5.0, 0.0]), 4.0 / 3.0);
    assert_eq!(rows.len(), 2, "two buckets of scale: {rows:?}");
    let (one, ten) = (&rows[0], &rows[1]);
    assert_eq!((one.instances, ten.instances), (1, 1));
    // The coverage is turned into metres for the size the model has in the world: the
    // bucket's upper scale (11.64 for 10) times the extent.
    let half_tan = 0.5773502691896257;
    for k in 0..3 {
        let share = [1.0, 0.25, 0.02][k];
        assert!(
            (one.own_until_m[k] - 11f64.sqrt() / (2.0 * half_tan * share)).abs() < 1e-9,
            "{}",
            one.own_until_m[k]
        );
        let ratio = ten.own_until_m[k] / one.own_until_m[k];
        assert!((ratio - 11.641_532_182_653_137).abs() < 1e-9, "{ratio}");
    }
}

#[test]
fn what_the_distances_say_of_an_asset_is_said_once() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    // Thirty copies of their own size, all with one switch distance more than the model has
    // changes of level: several items, one line.
    let mut rng = gamecraft_math::Rng::new(11);
    let mut w = world();
    for i in 0..30 {
        add(
            &mut w,
            &format!("Thing {i}"),
            ASSET,
            [
                f64::from(i % 6) * 8.0 - 20.0,
                0.0,
                -f64::from(i / 6) * 8.0 - 10.0,
            ],
            |m| {
                m.scale = rng.range(0.5, 2.0);
                m.lod_distances_m = Some(vec![20.0, 60.0, 90.0]);
            },
        );
    }
    r.provide_asset(ASSET, named());
    let problems = r.sync(&w, None);
    let rows = r.count_models(&view([0.0, 20.0, 30.0], [0.0, 0.0, -20.0]), 4.0 / 3.0);
    assert!(rows.len() > 1, "{} items", rows.len());
    let lines: Vec<&String> = problems
        .iter()
        .filter(|p| p.contains("switch distance"))
        .collect();
    assert_eq!(lines.len(), 1, "{problems:?}");
    assert!(
        lines[0].contains(ASSET) && lines[0].contains("1 switch distance(s) more"),
        "{lines:?}"
    );
    let p = picture(&mut r, view([0.0, 20.0, 30.0], [0.0, 0.0, -20.0]));
    assert_eq!(
        p.problems
            .iter()
            .filter(|p| p.contains("switch distance"))
            .count(),
        1,
        "{:?}",
        p.problems
    );
}

#[test]
fn a_model_made_of_strips_and_fans_is_drawn() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    // A ribbon that is one strip (34 triangles) and a fan beside it (34), as two assets.
    let strip = glb_modes(&[1], &[5], &[node("Ribbon", Some(0))], &[0], Value::Null);
    let fan = glb_modes(&[1], &[6], &[node("Fan", Some(0))], &[0], Value::Null);
    let mut w = world();
    add(
        &mut w,
        "Ribbon",
        "models/ribbon.glb",
        [-3.0, 0.0, -10.0],
        |_| {},
    );
    add(&mut w, "Fan", "models/fan.glb", [3.0, 0.0, -10.0], |_| {});
    r.provide_asset("models/ribbon.glb", strip);
    r.provide_asset("models/fan.glb", fan);
    let camera = view([0.0, 0.5, 0.0], [0.0, 0.5, -10.0]);
    let base = baseline(&mut r, camera);
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    let p = picture(&mut r, camera);
    let triangles: u64 = p
        .stats
        .models
        .iter()
        .map(|m| m.levels.iter().map(|l| l.triangles).sum::<u64>())
        .sum();
    assert_eq!(triangles, 2 * 34, "{:?}", p.stats.models);
    assert_eq!(p.stats.forward_triangles, base + 2 * 34, "{:?}", p.stats);
    // A level made only of a strip draws like any other: a strip's triangles are its geometry
    // (three cubes in the fine level, one cube as a strip in the coarse one).
    let mixed = glb_modes(
        &[3, 1],
        &[4, 5],
        &[node("Thing_LOD0", Some(0)), node("Thing_LOD1", Some(1))],
        &[0, 1],
        Value::Null,
    );
    let mut w = world();
    add(&mut w, "Near", ASSET, [10.0, 0.0, -10.0], |m| {
        m.lod_distances_m = Some(vec![20.0])
    });
    add(&mut w, "Far", ASSET, [10.0, 0.0, -100.0], |m| {
        m.lod_distances_m = Some(vec![20.0])
    });
    r.provide_asset(ASSET, mixed);
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    // From above and behind: 16 m to the near one, within its first change of level, and
    // a hundred to the far one.
    let p = picture(&mut r, view([10.0, 2.0, 6.0], [10.0, 0.0, -50.0]));
    let rows = &p.stats.models;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0].levels,
        vec![lc(0, 1, 36), lc(1, 1, 34)],
        "the strip is the coarse level's geometry: {rows:?}"
    );
}

// ---------------------------------------------------------- far levels of detail (cells)

/// A world of `n` solid boxes, `spacing` metres apart over a square, all one colour.
fn boxes(n: usize, spacing: f64) -> World {
    let mut w = world();
    let side = (n as f64).sqrt().round() as usize;
    for i in 0..n {
        let (x, z) = ((i % side) as f64 * spacing, (i / side) as f64 * spacing);
        w.add_entity(EntityDraft {
            name: format!("Box {i}"),
            position: [x, 0.0, z],
            components: vec![Component::Shape(Shape {
                kind: ShapeKind::Box,
                size: [4.0, 3.0, 4.0],
                color: [0.6, 0.5, 0.4],
                solid: true,
            })],
            ..EntityDraft::default()
        })
        .unwrap();
    }
    w
}

#[test]
fn a_kilometre_of_boxes_seen_from_far_draws_its_cells_not_its_boxes() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    // 64 cells of 128 m over a kilometre, each with its merged mesh.
    let w = boxes(1024, 32.0);
    let camera = view([500.0, 500.0, 2500.0], [500.0, 0.0, 500.0]);
    let base = baseline(&mut r, camera);
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    assert_eq!(r.hlod_builds(), 64, "one merged mesh per cell");
    let p = picture(&mut r, camera);
    assert_eq!(p.stats.hlod.cells, 64, "{:?}", p.stats.hlod);
    assert_eq!(
        p.stats.hlod.drawn, 64,
        "every cell is far: {:?}",
        p.stats.hlod
    );
    assert_eq!(p.stats.hlod.triangles, 1024 * 12, "{:?}", p.stats.hlod);
    // Nothing else: the boxes' own items are culled (every cluster is more than 481 m away).
    assert_eq!(p.stats.forward_triangles, base + 1024 * 12, "{:?}", p.stats);
    // A few items: one for the boxes' colour, one per cell, and the puff.
    assert!(r.item_count() <= 1 + 64, "{}", r.item_count());
    // Walking the camera does not build anything. From nearby the cells around the
    // camera are near (their own boxes draw) and only the far ones their merged meshes.
    let p2 = picture(&mut r, view([16.0, 40.0, 16.0], [500.0, 0.0, 500.0]));
    assert_eq!(r.hlod_builds(), 64);
    assert!(
        p2.stats.hlod.drawn > 5 && p2.stats.hlod.drawn < 64,
        "seen from a corner: {:?}",
        p2.stats.hlod
    );
}

#[test]
fn a_near_cell_draws_its_own_items_and_only_a_far_one_its_merged_mesh() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let w = boxes(1024, 32.0);
    let camera = view([500.0, 2.0, 500.0], [500.0, 2.0, 1000.0]);
    let base = baseline(&mut r, camera);
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    let p = picture(&mut r, camera);
    let stats = p.stats;
    assert!(
        stats.hlod.drawn > 0 && stats.hlod.drawn < 64,
        "{:?}",
        stats.hlod
    );
    // Sixteen boxes of twelve triangles in a cell.
    assert_eq!(stats.hlod.triangles, stats.hlod.drawn as u64 * 192);
    // The boxes of the near cells draw themselves: what the view submitted is the merged
    // meshes plus whole boxes.
    let rest = stats.forward_triangles - base - stats.hlod.triangles;
    assert!(rest >= 12 && rest.is_multiple_of(12), "{stats:?}");
}

#[test]
fn an_edit_rebuilds_only_the_cells_it_touches() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let mut w = boxes(400, 50.0);
    r.sync(&w, None);
    let first = r.hlod_builds();
    assert_eq!(first, 64, "20 by 20 boxes of 50 m: 8 by 8 cells of 128 m");
    // A box moved inside its cell: that cell's mesh is built again, no other.
    let moved = w.entities().next().unwrap().id;
    w.entity_mut(moved).unwrap().position[0] += 1.0;
    r.sync(&w, None);
    assert_eq!(r.hlod_builds(), first + 1);
    // A box moved into another cell: both cells change.
    w.entity_mut(moved).unwrap().position[0] = 300.0;
    r.sync(&w, None);
    assert_eq!(r.hlod_builds(), first + 3, "{:?}", r.hlod_builds());
    // A character walking is no change to any cell.
    let walker = w
        .add_entity(EntityDraft {
            name: "Walker".into(),
            position: [10.0, 0.0, 10.0],
            components: vec![Component::Character(Character::default())],
            ..EntityDraft::default()
        })
        .unwrap();
    r.sync(&w, None);
    let before_walk = r.hlod_builds();
    w.entity_mut(walker).unwrap().position = [11.0, 0.0, 10.0];
    r.sync(&w, None);
    assert_eq!(r.hlod_builds(), before_walk, "a character takes no part");
    // Every box turned: every cell again.
    let shaped: Vec<_> = w
        .entities()
        .filter(|e| e.shape().is_some())
        .map(|e| e.id)
        .collect();
    for id in shaped {
        w.entity_mut(id).unwrap().rotation_y_deg = 10.0;
    }
    r.sync(&w, None);
    assert_eq!(r.hlod_builds(), before_walk + 64);
    // No boxes: no cells, and nothing left behind.
    let ids: Vec<_> = w
        .entities()
        .filter(|e| e.shape().is_some())
        .map(|e| e.id)
        .collect();
    for id in ids {
        w.remove_entity(id);
    }
    r.sync(&w, None);
    let p = picture(&mut r, view([500.0, 500.0, 2500.0], [500.0, 0.0, 500.0]));
    assert_eq!(p.stats.hlod, HlodStats::default(), "{:?}", p.stats.hlod);
    assert_eq!(r.hlod_builds(), before_walk + 64, "nothing new to build");
    assert_eq!(
        r.item_count(),
        1,
        "the walker's capsule and nothing of the cells: {}",
        r.item_count()
    );
}

#[test]
fn a_model_is_merged_at_its_coarsest_level_and_a_reupload_builds_its_cells_again() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let mut w = world();
    // Ten things of three levels in one cell (36, 24 and 12 triangles) and ten in another.
    for i in 0..20 {
        let (x, z) = if i < 10 {
            (i as f64 * 4.0, 4.0)
        } else {
            ((i - 10) as f64 * 4.0, 900.0)
        };
        add(&mut w, &format!("Thing {i}"), ASSET, [x, 0.0, z], |m| {
            m.lod_distances_m = Some(vec![20.0, 60.0]);
            m.draw_distance_m = Some(150.0);
        });
    }
    r.provide_asset(ASSET, named());
    assert_eq!(r.sync(&w, None), Vec::<String>::new());
    assert_eq!(r.hlod_builds(), 2, "one merged mesh per cell");
    // A model with a draw distance of its own (150 m): the merged mesh takes over at the least
    // distance, 300 m, beyond it.
    let near = view([20.0, 40.0, 500.0], [20.0, 0.0, 4.0]);
    let p = picture(&mut r, near);
    // The cell at z = 900 is behind the camera; the cell at z = 4 is 500 m away: drawn.
    assert_eq!(
        (p.stats.hlod.cells, p.stats.hlod.drawn),
        (2, 1),
        "{:?}",
        p.stats.hlod
    );
    assert_eq!(
        p.stats.hlod.triangles,
        10 * 12,
        "the coarsest level: {:?}",
        p.stats.hlod
    );
    assert_eq!(
        p.stats.models[0].drawn, 0,
        "its own items are culled: {:?}",
        p.stats.models
    );
    // Another version of the asset: both cells again.
    r.provide_asset(
        ASSET,
        named_with(json!({"gamecraft": {"draw_distance_m": 900}})),
    );
    r.sync(&w, None);
    assert_eq!(r.hlod_builds(), 4, "the cells show the new model");
    // A shape beside the models joins the cell's merged mesh under its own colour.
    w.add_entity(EntityDraft {
        name: "Slab".into(),
        position: [0.0, 0.0, 300.0],
        components: vec![Component::Shape(Shape {
            kind: ShapeKind::Box,
            size: [8.0, 1.0, 8.0],
            color: [0.9, 0.1, 0.1],
            solid: false,
        })],
        ..EntityDraft::default()
    })
    .unwrap();
    r.sync(&w, None);
    assert_eq!(r.hlod_builds(), 5, "the cell of the slab, and no other");
    let p = picture(&mut r, view([0.0, 30.0, 900.0], [0.0, 0.0, 300.0]));
    assert!(p.stats.hlod.drawn >= 1, "{:?}", p.stats.hlod);
    assert!(p.problems.is_empty(), "{:?}", p.problems);
}
