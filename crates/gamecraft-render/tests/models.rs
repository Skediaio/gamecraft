//! Models: the generated demo models read on the CPU (no GPU needed), and on a real GPU one
//! upload per asset with an instance per entity, a crate drawn in its texture's colours, and a
//! missing or broken model drawn as a magenta box with a problem line. The GPU tests skip
//! without an adapter (and fail with `GAMECRAFT_REQUIRE_GPU=1`); `models_in_a_yard` writes
//! `out/render-models.png`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use gamecraft_render::model::{self, Alpha};
use gamecraft_render::{Camera, Picture, PictureRequest, WorldRenderer, encode_png, headless_gpu};
use gamecraft_world::{
    Component, EntityDraft, HeightField, Model, ModelCollider, Terrain, TerrainSource, World,
};
use std::sync::{Mutex, MutexGuard, OnceLock};

const CRATE: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/crate.glb");
const FENCE: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/fence.glb");

#[test]
fn the_crate_reads_with_its_material_and_texture() {
    let m = model::parse(CRATE, &|_: &str| None).unwrap();
    assert_eq!(m.triangles(), 12);
    assert_eq!(m.prims.len(), 1);
    let mat = &m.materials[m.prims[0].material];
    assert_eq!(mat.alpha, Alpha::Opaque);
    assert!((mat.roughness - 0.8).abs() < 1e-6 && mat.metallic == 0.0);
    let base = mat.base_image.unwrap();
    assert!(!base.repeat, "the crate's sampler clamps");
    let img = model::decode_image(&m.images[base.image]).unwrap();
    assert_eq!((img.width, img.height), (64, 64));
    // Wood: more red than blue.
    let (mut r, mut b) = (0u64, 0u64);
    for px in img.pixels.as_chunks::<4>().0 {
        r += u64::from(px[0]);
        b += u64::from(px[2]);
    }
    assert!(r > 2 * b, "{r} {b}");
    let f = model::parse(FENCE, &|_: &str| None).unwrap();
    assert_eq!(f.triangles(), 48);
    assert!(f.materials[f.prims[0].material].base_image.unwrap().repeat);
}

#[test]
fn a_broken_model_or_image_is_an_error_never_a_panic() {
    for bytes in [CRATE, FENCE] {
        for cut in 0..bytes.len() {
            let _ = model::parse(&bytes[..cut], &|_: &str| None);
        }
        for at in (0..bytes.len()).step_by(3) {
            let mut bad = bytes.to_vec();
            bad[at] ^= 0xA5;
            if let Ok(m) = model::parse(&bad, &|_: &str| None) {
                for img in &m.images {
                    let _ = model::decode_image(img);
                }
            }
        }
    }
    let m = model::parse(CRATE, &|_: &str| None).unwrap();
    let png = &m.images[0];
    for cut in 0..png.len() {
        let _ = model::decode_image(&png[..cut]);
    }
    assert!(model::decode_image(&png[..png.len() / 2]).is_err());
}

#[test]
fn a_metal_rough_image_becomes_an_orm_image() {
    let img = model::Rgba {
        width: 1,
        height: 1,
        pixels: vec![0, 128, 255, 255],
    };
    assert_eq!(
        model::orm_from_metal_rough(img.clone(), false).pixels,
        [255, 128, 255, 255]
    );
    assert_eq!(
        model::orm_from_metal_rough(img, true).pixels,
        [0, 128, 255, 255]
    );
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

fn flat() -> (World, HeightField) {
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

fn add_model(world: &mut World, name: &str, asset: &str, at: [f64; 3], yaw: f64, scale: f64) {
    world
        .add_entity(EntityDraft {
            name: name.into(),
            position: at,
            rotation_y_deg: yaw,
            components: vec![Component::Model(Model {
                asset: asset.into(),
                scale,
                solid: true,
                collider: ModelCollider::Box,
            })],
            ..EntityDraft::default()
        })
        .unwrap();
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

/// Mean (r, g, b) of a square of `half` pixels around the picture's centre.
fn centre(p: &Picture, half: usize) -> [f64; 3] {
    let (w, h) = (p.width as usize, p.height as usize);
    let mut sum = [0.0; 3];
    let mut n = 0.0;
    for y in h / 2 - half..h / 2 + half {
        for x in w / 2 - half..w / 2 + half {
            let i = (y * w + x) * 4;
            for k in 0..3 {
                sum[k] += f64::from(p.rgba[i + k]);
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

#[test]
fn a_hundred_crates_are_one_upload_and_a_hundred_instances() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    for i in 0..100 {
        let (x, z) = (f64::from(i % 10) * 2.0 - 9.0, f64::from(i / 10) * 2.0 - 9.0);
        add_model(
            &mut world,
            &format!("Crate {i}"),
            "models/crate.glb",
            [x, 0.0, z],
            0.0,
            1.0,
        );
    }
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    assert_eq!(r.model_uploads(), 1);
    let base = picture(
        &mut r,
        Camera {
            position: [0.0, 25.0, 25.0],
            target: [0.0, 0.0, 0.0],
            fov_y_deg: 60.0,
        },
    );
    // A rebuild (a crate moved) reuses the upload.
    let id = world.entities().next().unwrap().id;
    world.entity_mut(id).unwrap().position[0] += 0.5;
    r.sync(&world, Some(&hf));
    assert_eq!(r.model_uploads(), 1);
    // The same bytes again change nothing; new bytes upload once more.
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    r.sync(&world, Some(&hf));
    assert_eq!(r.model_uploads(), 1);
    r.provide_asset("models/crate.glb", FENCE.to_vec());
    r.sync(&world, Some(&hf));
    assert_eq!(r.model_uploads(), 2);
    // The crates' instances: at least 100 drawn in the forward pass (shadows draw more).
    assert!(base.stats.instances >= 100, "{:?}", base.stats);
}

#[test]
fn models_in_a_yard() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    add_model(
        &mut world,
        "Crate",
        "models/crate.glb",
        [0.0, 0.0, 0.0],
        25.0,
        3.0,
    );
    add_model(
        &mut world,
        "Fence 1",
        "models/fence.glb",
        [-3.0, 0.0, -2.5],
        0.0,
        1.0,
    );
    add_model(
        &mut world,
        "Fence 2",
        "models/fence.glb",
        [3.0, 0.0, -2.5],
        0.0,
        1.0,
    );
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    r.provide_asset("models/fence.glb", FENCE.to_vec());
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    let p = picture(
        &mut r,
        Camera {
            position: [0.0, 2.2, 6.0],
            target: [0.0, 1.2, 0.0],
            fov_y_deg: 50.0,
        },
    );
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("render-models.png"),
        encode_png(&p.rgba, p.width, p.height).unwrap(),
    )
    .unwrap();
    // The crate fills the centre in its wood: red over blue, not magenta (green low too).
    let [red, green, blue] = centre(&p, 6);
    println!("centre {red} {green} {blue}");
    assert!(
        red > 1.3 * blue && green > 0.6 * blue,
        "wood, not magenta: {red} {green} {blue}"
    );
}

#[test]
fn a_missing_or_broken_model_is_a_magenta_box_and_a_problem_line() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    add_model(
        &mut world,
        "Ghost",
        "models/ghost.glb",
        [0.0, 0.0, 0.0],
        0.0,
        6.0,
    );
    let problems = r.sync(&world, Some(&hf));
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("models/ghost.glb")
            && problems[0].contains("\"Ghost\"")
            && problems[0].contains("magenta"),
        "{problems:?}"
    );
    let camera = Camera {
        position: [0.0, 2.0, 8.0],
        target: [0.0, 1.5, 0.0],
        fov_y_deg: 40.0,
    };
    let [red, green, blue] = centre(&picture(&mut r, camera), 5);
    assert!(
        red > 1.5 * green && blue > 1.5 * green,
        "magenta: {red} {green} {blue}"
    );
    // A broken file: the same, with why.
    r.provide_asset("models/ghost.glb", CRATE[..CRATE.len() / 2].to_vec());
    let problems = r.sync(&world, Some(&hf));
    assert!(
        problems
            .iter()
            .any(|p| p.contains("models/ghost.glb") && p.contains("cut short")),
        "{problems:?}"
    );
    // Once the file is whole, it is drawn and the problem is gone.
    r.provide_asset("models/ghost.glb", CRATE.to_vec());
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    assert_eq!(r.model_uploads(), 1);
}
