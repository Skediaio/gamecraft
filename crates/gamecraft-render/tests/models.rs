//! Models: the generated demo models read on the CPU (no GPU needed), and on a real GPU one
//! upload per asset with an instance per entity, a crate drawn in its texture's colours, and a
//! missing or broken model drawn as a magenta box with a problem line. The GPU tests skip
//! without an adapter (and fail with `GAMECRAFT_REQUIRE_GPU=1`); `models_in_a_yard` writes
//! `out/render-models.png`. A character with a model is drawn as that model, which follows it
//! without a rebuild.

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
    Character, Component, EntityDraft, HeightField, Model, ModelCollider, Terrain, TerrainSource,
    World,
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
                lod_distances_m: None,
                draw_distance_m: None,
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

/// A character entity wearing `asset` at `scale`, at the origin facing north.
fn add_walker(world: &mut World, asset: &str, scale: f64) -> gamecraft_world::EntityId {
    world
        .add_entity(EntityDraft {
            name: "Walker".into(),
            components: vec![
                Component::Character(Character::default()),
                Component::Model(Model {
                    asset: asset.into(),
                    scale,
                    solid: false,
                    collider: ModelCollider::Box,
                    lod_distances_m: None,
                    draw_distance_m: None,
                }),
            ],
            ..EntityDraft::default()
        })
        .unwrap()
}

/// A view of the model at `x` on the ground, from the south.
fn looking_at(x: f64) -> Camera {
    Camera {
        position: [x, 2.2, 6.0],
        target: [x, 1.2, 0.0],
        fov_y_deg: 50.0,
    }
}

#[test]
fn a_character_with_a_model_is_drawn_as_the_model_and_walks_without_a_rebuild() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    r.sync(&world, Some(&hf));
    let ground_items = r.item_count();
    let walker = add_walker(&mut world, "models/crate.glb", 3.0);
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    // One item, the model's: no capsule beside it.
    assert_eq!(r.item_count(), ground_items + 1, "the model alone");
    assert_eq!(r.model_uploads(), 1);
    let [red, green, blue] = centre(&picture(&mut r, looking_at(0.0)), 6);
    assert!(
        red > 1.3 * blue && green > 0.6 * blue,
        "wood at the start: {red} {green} {blue}"
    );
    let rebuilds = r.rebuild_count();
    for step in 1..=10 {
        let e = world.entity_mut(walker).unwrap();
        e.position = [f64::from(step), 0.0, 0.0];
        e.rotation_y_deg = 25.0;
        r.sync(&world, Some(&hf));
    }
    assert_eq!(r.rebuild_count(), rebuilds, "walking is no rebuild");
    assert_eq!(r.item_count(), ground_items + 1);
    let [red, green, blue] = centre(&picture(&mut r, looking_at(10.0)), 6);
    assert!(
        red > 1.3 * blue && green > 0.6 * blue,
        "the model followed: {red} {green} {blue}"
    );
}

#[test]
fn a_character_with_a_missing_model_is_a_magenta_box_that_follows_it() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    let walker = add_walker(&mut world, "models/ghost.glb", 6.0);
    let problems = r.sync(&world, Some(&hf));
    assert!(
        problems.len() == 1
            && problems[0].contains("models/ghost.glb")
            && problems[0].contains("\"Walker\"")
            && problems[0].contains("magenta"),
        "{problems:?}"
    );
    let rebuilds = r.rebuild_count();
    world.entity_mut(walker).unwrap().position = [10.0, 0.0, 0.0];
    r.sync(&world, Some(&hf));
    assert_eq!(r.rebuild_count(), rebuilds, "walking is no rebuild");
    let camera = Camera {
        position: [10.0, 2.0, 8.0],
        target: [10.0, 1.5, 0.0],
        fov_y_deg: 40.0,
    };
    let [red, green, blue] = centre(&picture(&mut r, camera), 5);
    assert!(
        red > 1.5 * green && blue > 1.5 * green,
        "magenta where the character went: {red} {green} {blue}"
    );
}

// ------------------------------------------------------------------------ the texture store

/// The bytes of the first image embedded in the demo GLB `glb`.
fn embedded_image(glb: &[u8]) -> Vec<u8> {
    let word = |at: usize| -> usize {
        u32::from_le_bytes([glb[at], glb[at + 1], glb[at + 2], glb[at + 3]]) as usize
    };
    let json_len = word(12);
    let doc: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    let view = &doc["bufferViews"][doc["images"][0]["bufferView"].as_u64().unwrap() as usize];
    let at = view["byteOffset"].as_u64().unwrap() as usize;
    let len = view["byteLength"].as_u64().unwrap() as usize;
    let bin = 20 + json_len + 8;
    glb[bin + at..bin + at + len].to_vec()
}

/// The crate made ready for the texture store as the model `path` of a pack: its file and its
/// texture, as a host hands them to the renderer (paths inside `assets/`).
fn stored_as(path: &str) -> (Vec<u8>, (String, Vec<u8>)) {
    let imported = gamecraft_project::textures::import(CRATE, &|_: &str| None, path).unwrap();
    assert_eq!(imported.textures.len(), 1);
    let texture = imported.textures[0].clone();
    (imported.model, (texture.path, texture.bytes))
}

/// The crate read by the renderer's rule: a URI against the model's folder, then a lookup
/// among the handed files.
fn read(
    path: &str,
    files: &std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<model::ModelData, String> {
    let folder = gamecraft_project::model::folder_of(path);
    let bytes = files.get(path).cloned().ok_or("the model was not handed")?;
    model::parse(&bytes, &|f: &str| {
        gamecraft_project::model::resolve_uri(folder, f)
            .ok()
            .and_then(|inside| files.get(&inside).cloned())
    })
}

#[test]
fn a_model_that_refers_to_a_texture_of_the_store_reads_exactly_as_before() {
    let (model, (texture_path, texture_bytes)) = stored_as("models/crate.glb");
    assert!(texture_path.starts_with("textures/") && texture_path.ends_with(".png"));
    // The stored model refers to the texture by a relative URI, and its binary chunk has no image.
    let image = embedded_image(CRATE);
    let (json, bin) = gamecraft_project::model::split_glb(&model).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(json).unwrap();
    assert_eq!(
        doc["images"],
        serde_json::json!([{"uri": format!("../{texture_path}")}])
    );
    assert!(!bin.unwrap().windows(8).any(|w| w == &image[..8]));
    // With the texture handed over, the model is the one it was.
    let files: std::collections::BTreeMap<String, Vec<u8>> = [
        ("models/crate.glb".to_string(), model.clone()),
        (texture_path.clone(), texture_bytes.clone()),
    ]
    .into_iter()
    .collect();
    let m = read("models/crate.glb", &files).unwrap();
    assert_eq!(m, model::parse(CRATE, &|_: &str| None).unwrap());
    // A model deeper in the pack refers to it by a longer path; a host that hands the path
    // as written (`models/../textures/...`) is understood too.
    let (deep, _) = stored_as("models/houses/deep.glb");
    let files: std::collections::BTreeMap<String, Vec<u8>> = [
        ("models/houses/deep.glb".to_string(), deep),
        (texture_path.clone(), texture_bytes.clone()),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        read("models/houses/deep.glb", &files).unwrap().triangles(),
        12
    );
    // Without the texture: an error that names it.
    let files: std::collections::BTreeMap<String, Vec<u8>> =
        [("models/crate.glb".to_string(), model)]
            .into_iter()
            .collect();
    let e = read("models/crate.glb", &files).unwrap_err();
    assert!(e.contains("../textures/") && e.contains("missing"), "{e}");
    // A reference that climbs out of assets/ is refused, never followed.
    let bad = {
        let (json, _) = gamecraft_project::model::split_glb(CRATE).unwrap();
        let mut doc: serde_json::Value = serde_json::from_slice(json).unwrap();
        doc["images"] = serde_json::json!([{"uri": "../../outside.png"}]);
        gamecraft_project::textures::pack_glb(&serde_json::to_vec(&doc).unwrap(), None).unwrap()
    };
    let files: std::collections::BTreeMap<String, Vec<u8>> =
        [("models/crate.glb".to_string(), bad)]
            .into_iter()
            .collect();
    let e = read("models/crate.glb", &files).unwrap_err();
    assert!(e.contains("../../outside.png"), "{e}");
}

#[test]
fn models_that_share_a_texture_draw_with_it_uploaded_once() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    add_model(&mut world, "A", "models/a.glb", [-3.0, 0.0, 0.0], 20.0, 2.0);
    add_model(
        &mut world,
        "B",
        "models/houses/b.glb",
        [3.0, 0.0, 0.0],
        -20.0,
        2.0,
    );
    let (a, (texture_path, texture_bytes)) = stored_as("models/a.glb");
    let (b, (again, _)) = stored_as("models/houses/b.glb");
    assert_eq!(again, texture_path);
    r.provide_asset("models/a.glb", a);
    r.provide_asset("models/houses/b.glb", b);
    // The texture is missing: both are magenta boxes, and the problem names the texture.
    let problems = r.sync(&world, Some(&hf));
    assert_eq!(problems.len(), 2, "{problems:?}");
    for p in &problems {
        assert!(p.contains(&texture_path) && p.contains("not handed"), "{p}");
    }
    // Handed once (by its path, as a host that writes `..` would too): drawn in wood.
    r.provide_asset(
        &format!("models/houses/../../{texture_path}"),
        texture_bytes.clone(),
    );
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    assert_eq!(r.model_uploads(), 2);
    for x in [-3.0, 3.0] {
        let [red, green, blue] = centre(&picture(&mut r, looking_at(x)), 6);
        assert!(
            red > 1.3 * blue && green > 0.6 * blue,
            "wood at {x}: {red} {green} {blue}"
        );
    }
    // The same bytes again change nothing.
    r.provide_asset(&texture_path, texture_bytes.clone());
    r.sync(&world, Some(&hf));
    assert_eq!(r.model_uploads(), 2);
    // Another texture: both models are read and uploaded again.
    r.provide_asset(&texture_path, embedded_image(FENCE));
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    assert_eq!(r.model_uploads(), 4);
}
