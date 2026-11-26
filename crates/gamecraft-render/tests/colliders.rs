//! Collision helpers are not drawn, on a real GPU: a glTF node named `*_collider` or
//! `*-collider` adds no
//! item and no pixels (the picture equals the one of the model without it), and a shape with
//! `visible: false` adds no item, no pixels and no merged far level, where it still collides.
//! The GPU tests skip without an adapter (and fail with `GAMECRAFT_REQUIRE_GPU=1`). The models
//! are built in the test: unit cubes, scaled and moved by their nodes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use gamecraft_render::{Camera, Picture, PictureRequest, WorldRenderer, encode_png, headless_gpu};
use gamecraft_world::{Component, EntityDraft, Model, Shape, Terrain, TerrainSource, World};
use std::sync::{Mutex, MutexGuard, OnceLock};

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

fn flat() -> (World, gamecraft_world::HeightField) {
    let mut world = World::new("yard", 1);
    let terrain = Terrain {
        size_m: 64.0,
        cell_m: 1.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    };
    let hf = terrain.build().unwrap();
    world.terrain = Some(terrain);
    world.environment.time_of_day_h = 14.0;
    (world, hf)
}

/// A GLB of unit cubes: one mesh, one node per `(name, translation, scale)`, all in the scene.
fn glb(nodes: &[(&str, [f64; 3], [f64; 3])]) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    for p in gamecraft_render::mesh::unit_box().positions {
        for v in p {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    let count = (bin.len() / 12) as u64;
    let nodes: Vec<serde_json::Value> = nodes
        .iter()
        .map(|(name, t, s)| {
            serde_json::json!({
                "name": name, "mesh": 0,
                "translation": t, "scale": s,
            })
        })
        .collect();
    let doc = serde_json::json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": (0..nodes.len()).collect::<Vec<_>>()}],
        "nodes": nodes,
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": bin.len()}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3",
                       "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 1.0]}],
    });
    gamecraft_project::textures::pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap()
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

/// The mean colour of a rectangle of the picture (x0, y0, x1, y1 in pixels).
fn patch(p: &Picture, x0: usize, y0: usize, x1: usize, y1: usize) -> [f64; 3] {
    let w = p.width as usize;
    let mut sum = [0.0; 3];
    let mut n = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * 4;
            for (k, s) in sum.iter_mut().enumerate() {
                *s += f64::from(p.rgba[i + k]);
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

/// How many pixels are red (far more red than green): the test box's colour, nowhere else.
fn red_pixels(p: &Picture) -> usize {
    p.rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| f64::from(px[0]) > 1.6 * f64::from(px[1]) + 25.0)
        .count()
}

fn close(a: [f64; 3], b: [f64; 3], tolerance: f64) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tolerance)
}

const CAMERA: Camera = Camera {
    position: [7.0, 4.0, 9.0],
    target: [2.0, 1.0, 0.0],
    fov_y_deg: 55.0,
};

#[test]
fn a_collider_node_is_not_drawn() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    world
        .add_entity(EntityDraft {
            name: "Thing".into(),
            components: vec![Component::Model(Model::of("models/thing.glb"))],
            ..EntityDraft::default()
        })
        .unwrap();
    // Without the collider node: the picture of the cube alone.
    r.provide_asset("models/thing.glb", glb(&[("Cube", [0.0; 3], [1.0; 3])]));
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    let items = r.item_count();
    let alone = picture(&mut r, CAMERA);
    // With a collider node three times the size, four metres to the right, where the camera
    // looks: not one item more, not one pixel different.
    r.provide_asset(
        "models/thing.glb",
        glb(&[
            ("Cube", [0.0; 3], [1.0; 3]),
            ("Cube-collider", [4.0, 0.0, -1.0], [3.0, 3.0, 3.0]),
        ]),
    );
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    assert_eq!(r.item_count(), items, "the collider node adds no item");
    let with = picture(&mut r, CAMERA);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("render-collider.png"),
        encode_png(&with.rgba, with.width, with.height).unwrap(),
    )
    .unwrap();
    // Every pixel: the same picture (the same scene, the same time).
    let mut worst = 0.0f64;
    for (a, b) in alone.rgba.iter().zip(&with.rgba) {
        worst = worst
            .max(f64::from(*a) - f64::from(*b))
            .max(f64::from(*b) - f64::from(*a));
    }
    assert!(worst <= 2.0, "a pixel differs by {worst}");
    // And the collider's place shows the meadow, not a cube: the right third of the picture is
    // grass and sky, as in the picture without any model.
    let only = world.entities().next().unwrap().id;
    world.remove_entities(&[only]);
    r.sync(&world, Some(&hf));
    let empty = picture(&mut r, CAMERA);
    let right = patch(&with, 214, 0, 320, 240);
    let right_empty = patch(&empty, 214, 0, 320, 240);
    assert!(
        close(right, right_empty, 3.0),
        "{right:?} vs {right_empty:?}"
    );
}

#[test]
fn a_hidden_shape_adds_no_item_no_pixels_and_no_far_level() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    let items = r.item_count();
    let empty = picture(&mut r, CAMERA);
    assert_eq!(empty.stats.hlod.cells, 0);
    let shape = |visible| {
        Component::Shape(Shape {
            kind: gamecraft_world::ShapeKind::Box,
            size: [2.0, 2.0, 2.0],
            color: [0.9, 0.1, 0.1],
            solid: true,
            visible,
        })
    };
    // A visible box in front of the camera: one item more, red at the centre.
    let id = world
        .add_entity(EntityDraft {
            name: "Box".into(),
            components: vec![shape(true)],
            ..EntityDraft::default()
        })
        .unwrap();
    r.sync(&world, Some(&hf));
    assert!(r.item_count() > items);
    assert_eq!(red_pixels(&empty), 0, "no red in the meadow");
    let shown = picture(&mut r, CAMERA);
    let red = red_pixels(&shown);
    assert!(red > 200, "the red box is in the picture: {red} pixels");
    // Hidden: as if it were not there, to the pixel.
    world.entity_mut(id).unwrap().components[0] = shape(false);
    r.sync(&world, Some(&hf));
    assert_eq!(r.item_count(), items, "a hidden shape adds no item");
    let hidden = picture(&mut r, CAMERA);
    assert_eq!(red_pixels(&hidden), 0, "no red when hidden");
    assert!(close(
        patch(&hidden, 150, 110, 170, 130),
        patch(&empty, 150, 110, 170, 130),
        2.0
    ));
    let mut worst = 0.0f64;
    for (a, b) in empty.rgba.iter().zip(&hidden.rgba) {
        worst = worst
            .max(f64::from(*a) - f64::from(*b))
            .max(f64::from(*b) - f64::from(*a));
    }
    assert!(worst <= 2.0, "a pixel differs by {worst}");
    // Inside a far cell: a visible shape has a merged far level, a hidden one none.
    world.entity_mut(id).unwrap().position = [400.0, 0.0, 400.0];
    world.entity_mut(id).unwrap().components[0] = shape(true);
    r.sync(&world, Some(&hf));
    let far = picture(
        &mut r,
        Camera {
            position: [400.0, 60.0, 470.0],
            target: [400.0, 0.0, 400.0],
            fov_y_deg: 50.0,
        },
    );
    assert_eq!(far.stats.hlod.cells, 1, "{:?}", far.stats.hlod);
    world.entity_mut(id).unwrap().components[0] = shape(false);
    r.sync(&world, Some(&hf));
    let far = picture(
        &mut r,
        Camera {
            position: [400.0, 60.0, 470.0],
            target: [400.0, 0.0, 400.0],
            fov_y_deg: 50.0,
        },
    );
    assert_eq!(far.stats.hlod.cells, 0, "{:?}", far.stats.hlod);
}
