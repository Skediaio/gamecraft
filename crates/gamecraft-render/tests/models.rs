//! Models: the generated demo models read on the CPU (no GPU needed), and on a real GPU one
//! upload per asset with an instance per entity, a crate drawn in its texture's colours, and a
//! missing or broken model drawn as a magenta box with a problem line. The GPU tests skip
//! without an adapter (and fail with `GAMECRAFT_REQUIRE_GPU=1`); `models_in_a_yard` writes
//! `out/render-models.png`. A character with a model is drawn as that model, which follows it
//! without a rebuild. A held model (a first-person weapon) is drawn at the aim of the hidden
//! character only, and follows the aim without a rebuild.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use gamecraft_render::model::{self, Alpha};
use gamecraft_render::{
    Camera, Picture, PictureRequest, ViewModelAim, WorldRenderer, encode_png, headless_gpu,
};
use gamecraft_world::{
    Character, Component, EntityDraft, HeightField, HeldAttachment, HeldModel, Model,
    ModelCollider, Reticle, Shape, ShapeKind, Terrain, TerrainSource, World,
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

/// A view from `eye` along the aim of `yaw_deg` (level), as a first-person camera looks.
fn eyes(eye: [f64; 3], yaw_deg: f64) -> Camera {
    let (s, c) = yaw_deg.to_radians().sin_cos();
    Camera {
        position: eye,
        target: [eye[0] - 10.0 * s, eye[1], eye[2] - 10.0 * c],
        fov_y_deg: 70.0,
    }
}

fn is_wood([red, green, blue]: [f64; 3]) -> bool {
    red > 1.3 * blue && green > 0.6 * blue
}

#[test]
fn a_held_model_is_drawn_at_the_aim_of_the_hidden_character_and_moves_without_a_rebuild() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    let player = world
        .add_entity(EntityDraft {
            name: "Player".into(),
            components: vec![
                Component::Character(Character::default()),
                Component::HeldModel(HeldModel {
                    asset: "models/crate.glb".into(),
                    offset_m: [0.0, -0.2, 1.0],
                    scale: 0.5,
                    ..HeldModel::of("models/crate.glb")
                }),
            ],
            ..EntityDraft::default()
        })
        .unwrap();
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    r.set_hidden_character(Some(player));
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    let eye_height = Character::default().eye_height_m;
    let eye = [0.0, eye_height, 0.0];
    // No aim yet: nothing in front of the eyes.
    let empty = centre(&picture(&mut r, eyes(eye, 0.0)), 6);
    assert!(!is_wood(empty), "no view model without an aim: {empty:?}");
    r.set_view_model(Some(ViewModelAim {
        eye,
        yaw_deg: 0.0,
        pitch_deg: 0.0,
        ads: 0.0,
        reload: None,
    }));
    let held = centre(&picture(&mut r, eyes(eye, 0.0)), 6);
    assert!(
        is_wood(held),
        "the crate is held in front of the eyes: {held:?}"
    );
    assert_eq!(r.model_uploads(), 1);
    let rebuilds = r.rebuild_count();
    // The player walks and turns; the aim follows every frame.
    for step in 1..=10 {
        let at = [f64::from(step), 0.0, -f64::from(step)];
        world.entity_mut(player).unwrap().position = at;
        world.entity_mut(player).unwrap().rotation_y_deg = 9.0 * f64::from(step);
        r.sync(&world, Some(&hf));
        r.set_view_model(Some(ViewModelAim {
            eye: [at[0], eye_height, at[2]],
            yaw_deg: 9.0 * f64::from(step),
            pitch_deg: 0.0,
            ads: 0.0,
            reload: None,
        }));
    }
    assert_eq!(
        r.rebuild_count(),
        rebuilds,
        "following the aim is no rebuild"
    );
    let moved = [10.0, eye_height, -10.0];
    let followed = centre(&picture(&mut r, eyes(moved, 90.0)), 6);
    assert!(
        is_wood(followed),
        "the crate followed the aim: {followed:?}"
    );
    // Without an aim (third person) it is gone again.
    r.set_view_model(None);
    r.sync(&world, Some(&hf));
    let gone = centre(&picture(&mut r, eyes(moved, 90.0)), 6);
    assert!(!is_wood(gone), "no aim, no view model: {gone:?}");
    assert_eq!(r.rebuild_count(), rebuilds);
}

/// A player stands with its face against a blue wall (as near as its radius lets it); its
/// held crate, a metre ahead of the eye where the wall is, is still seen: a view model is never
/// hidden by a wall in front of the camera. `out/render-view-model-wall.png` shows it.
#[test]
fn a_held_model_is_not_hidden_by_a_wall_in_front_of_the_eyes() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    let character = Character::default();
    let player = world
        .add_entity(EntityDraft {
            name: "Player".into(),
            components: vec![
                Component::Character(character.clone()),
                Component::HeldModel(HeldModel {
                    offset_m: [0.0, -0.2, 1.0],
                    scale: 0.5,
                    ..HeldModel::of("models/crate.glb")
                }),
            ],
            ..EntityDraft::default()
        })
        .unwrap();
    // The wall's face is the character's radius (plus 2 cm) north of the eye.
    let thickness = 0.1;
    world
        .add_entity(EntityDraft {
            name: "Wall".into(),
            position: [0.0, 0.0, -(character.radius_m + 0.02 + thickness / 2.0)],
            components: vec![Component::Shape(Shape {
                kind: ShapeKind::Box,
                size: [4.0, 3.0, thickness],
                color: [0.1, 0.2, 0.8],
                solid: true,
            })],
            ..EntityDraft::default()
        })
        .unwrap();
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    r.set_hidden_character(Some(player));
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    let eye = [0.0, character.eye_height_m, 0.0];
    let wall = centre(&picture(&mut r, eyes(eye, 0.0)), 6);
    assert!(!is_wood(wall), "the wall fills the view: {wall:?}");
    r.set_view_model(Some(ViewModelAim {
        eye,
        yaw_deg: 0.0,
        pitch_deg: 0.0,
        ads: 0.0,
        reload: None,
    }));
    let shown = picture(&mut r, eyes(eye, 0.0));
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("render-view-model-wall.png"),
        encode_png(&shown.rgba, shown.width, shown.height).unwrap(),
    )
    .unwrap();
    let held = centre(&shown, 6);
    assert!(
        is_wood(held),
        "the crate is held in front of the wall: {held:?}"
    );
}

/// The mean colour of the square of side `2 * half` pixels centred `dx` pixels right of the
/// picture's centre.
fn beside_centre(p: &Picture, dx: i64, half: usize) -> [f64; 3] {
    let (w, h) = (p.width as usize, p.height as usize);
    let cx = (w as i64 / 2 + dx) as usize;
    let mut sum = [0.0; 3];
    let mut n = 0.0;
    for y in h / 2 - half..h / 2 + half {
        for x in cx - half..cx + half {
            let i = (y * w + x) * 4;
            for k in 0..3 {
                sum[k] += f64::from(p.rgba[i + k]);
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

fn is_red([red, green, blue]: [f64; 3]) -> bool {
    red > 180.0 && green < 0.5 * red && blue < 0.5 * red
}

/// A crate held low, below the view, with a second crate attached on top of it (in the held
/// crate's own units) that reaches the middle of the view, and a red dot: the attachment is
/// drawn with the held model, and down the sights the dot marks where the aim points, also
/// when the aim (sway) leaves the camera's axis. `out/render-reticle.png` shows it.
#[test]
fn an_attachment_rides_on_the_held_model_and_the_dot_marks_the_aim() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    let held = HeldModel {
        offset_m: [0.0, -1.2, 1.0],
        scale: 0.5,
        ..HeldModel::of("models/crate.glb")
    };
    let player = world
        .add_entity(EntityDraft {
            name: "Player".into(),
            components: vec![
                Component::Character(Character::default()),
                Component::HeldModel(held.clone()),
            ],
            ..EntityDraft::default()
        })
        .unwrap();
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    r.set_hidden_character(Some(player));
    r.sync(&world, Some(&hf));
    let eye = [0.0, Character::default().eye_height_m, 0.0];
    let aim = |yaw_deg: f64, ads: f64| ViewModelAim {
        eye,
        yaw_deg,
        pitch_deg: 0.0,
        ads,
        reload: None,
    };
    r.set_view_model(Some(aim(0.0, 1.0)));
    let low = centre(&picture(&mut r, eyes(eye, 0.0)), 4);
    assert!(
        !is_wood(low) && !is_red(low),
        "the crate is held below: {low:?}"
    );
    // The attachment sits two crates up, in the middle of the view.
    world
        .entity_mut(player)
        .unwrap()
        .set_component(Component::HeldModel(HeldModel {
            attachments: vec![HeldAttachment {
                asset: "models/crate.glb".into(),
                offset_m: [0.0, 2.0, 0.0],
            }],
            ..held.clone()
        }));
    r.sync(&world, Some(&hf));
    let attached = centre(&picture(&mut r, eyes(eye, 0.0)), 4);
    assert!(is_wood(attached), "the attachment is drawn: {attached:?}");
    // A red dot: at the centre down the sights, not at the hip.
    world
        .entity_mut(player)
        .unwrap()
        .set_component(Component::HeldModel(HeldModel {
            reticle: Some(Reticle::default()),
            ..held
        }));
    r.sync(&world, Some(&hf));
    r.set_view_model(Some(aim(0.0, 0.5)));
    let hip = centre(&picture(&mut r, eyes(eye, 0.0)), 1);
    assert!(!is_red(hip), "no dot at the hip: {hip:?}");
    r.set_view_model(Some(aim(0.0, 1.0)));
    let dot = centre(&picture(&mut r, eyes(eye, 0.0)), 1);
    assert!(is_red(dot), "the dot at the centre: {dot:?}");
    // The barrel sways 3 degrees left of where the camera looks: the dot goes with it, by
    // tan(3 degrees) / tan(35 degrees) of half the picture's height (120 px): 9 px.
    r.set_view_model(Some(aim(3.0, 1.0)));
    let swayed = picture(&mut r, eyes(eye, 0.0));
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("render-reticle.png"),
        encode_png(&swayed.rgba, swayed.width, swayed.height).unwrap(),
    )
    .unwrap();
    let dx = -(3f64.to_radians().tan() / 35f64.to_radians().tan() * 120.0).round() as i64;
    assert_eq!(dx, -9);
    let moved = beside_centre(&swayed, dx, 1);
    assert!(is_red(moved), "the dot followed the barrel: {moved:?}");
    let left = centre(&swayed, 1);
    assert!(!is_red(left), "and left the centre: {left:?}");
}

/// The mean row of the wood-coloured pixels of a picture, `None` without any.
fn wood_row(p: &Picture) -> Option<f64> {
    let (w, h) = (p.width as usize, p.height as usize);
    let (mut sum, mut n) = (0.0, 0.0);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let px = &p.rgba[i..i + 3];
            if is_wood([f64::from(px[0]), f64::from(px[1]), f64::from(px[2])]) {
                sum += y as f64;
                n += 1.0;
            }
        }
    }
    (n > 0.0).then(|| sum / n)
}

#[test]
fn a_reloading_held_model_is_drawn_lower() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    let player = world
        .add_entity(EntityDraft {
            name: "Player".into(),
            components: vec![
                Component::Character(Character::default()),
                Component::HeldModel(HeldModel {
                    asset: "models/crate.glb".into(),
                    offset_m: [0.0, -0.1, 0.8],
                    scale: 0.3,
                    ..HeldModel::of("models/crate.glb")
                }),
            ],
            ..EntityDraft::default()
        })
        .unwrap();
    r.provide_asset("models/crate.glb", CRATE.to_vec());
    r.set_hidden_character(Some(player));
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    let eye = [0.0, Character::default().eye_height_m, 0.0];
    let aim = |reload: Option<f64>| ViewModelAim {
        eye,
        yaw_deg: 0.0,
        pitch_deg: 0.0,
        ads: 0.0,
        reload,
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    let mut rows = Vec::new();
    for (name, reload) in [("hip", None), ("reload", Some(0.5)), ("done", Some(1.0))] {
        r.set_view_model(Some(aim(reload)));
        let shown = picture(&mut r, eyes(eye, 0.0));
        std::fs::write(
            dir.join(format!("render-view-model-{name}.png")),
            encode_png(&shown.rgba, shown.width, shown.height).unwrap(),
        )
        .unwrap();
        rows.push(wood_row(&shown).expect("the crate is in view"));
    }
    assert!(
        rows[1] > rows[0] + 2.0,
        "mid-reload the weapon is lower: {rows:?}"
    );
    assert!((rows[2] - rows[0]).abs() < 0.5, "back at the end: {rows:?}");
}
