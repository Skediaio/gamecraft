//! Tests on a real GPU. They skip with one printed line when `headless_gpu()` finds no
//! adapter, and fail instead when `GAMECRAFT_REQUIRE_GPU=1`.
//!
//! `pictures_of_a_valley_and_a_walker` writes `out/render-valley.png` and
//! `out/render-walker.png` at the workspace root for a person (or agent) to look at.

// Integration tests are compiled as their own crates, outside `cfg(test)`, so clippy.toml's
// test allowances do not reach their helper functions. Picture statistics are not part of the
// deterministic core, so the platform's powers are fine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use gamecraft_math::Rng;
use gamecraft_render::{Camera, Picture, PictureRequest, WorldRenderer, encode_png, headless_gpu};
use gamecraft_world::{
    Character, Component, EntityDraft, HeightField, Plant, Shape, ShapeKind, Terrain,
    TerrainSource, World,
};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// One test at a time on the GPU: renderers are large and stills share the device.
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

fn valley(time_of_day_h: f64, cell_m: f64) -> (World, HeightField) {
    let mut world = World::new("valley", 11);
    let terrain = Terrain {
        size_m: 512.0,
        cell_m,
        source: TerrainSource::Noise {
            seed: 11,
            relief_m: 70.0,
            feature_m: 220.0,
            octaves: 5,
            roughness: 0.5,
        },
    };
    let hf = terrain.build().unwrap();
    world.terrain = Some(terrain);
    world.environment.time_of_day_h = time_of_day_h;
    (world, hf)
}

fn flat(size_m: f64) -> (World, HeightField) {
    let mut world = World::new("flat", 1);
    let terrain = Terrain {
        size_m,
        cell_m: 1.0,
        source: TerrainSource::Flat { height_m: 0.0 },
    };
    let hf = terrain.build().unwrap();
    world.terrain = Some(terrain);
    world.environment.time_of_day_h = 12.0;
    (world, hf)
}

fn shape(kind: ShapeKind, size: [f64; 3], color: [f32; 3]) -> Component {
    Component::Shape(Shape {
        kind,
        size,
        color,
        solid: true,
    })
}

fn add(world: &mut World, position: [f64; 3], yaw: f64, component: Component) {
    world
        .add_entity(EntityDraft {
            position,
            rotation_y_deg: yaw,
            components: vec![component],
            ..EntityDraft::default()
        })
        .unwrap();
}

fn plant(species: &str, seed: u64) -> Component {
    Component::Plant(Plant {
        species: species.to_string(),
        seed,
        growth: 1.0,
    })
}

fn request(width: u32, height: u32, camera: Camera) -> PictureRequest {
    PictureRequest {
        width,
        height,
        camera,
        time_s: 1.0,
    }
}

/// Mean (r, g, b) of rows `rows` of a picture, 0 to 255.
fn mean_rgb(p: &Picture, rows: std::ops::Range<u32>) -> [f64; 3] {
    let row_bytes = p.width as usize * 4;
    let mut sum = [0.0; 3];
    let mut n = 0.0;
    for row in rows {
        let start = row as usize * row_bytes;
        for px in p.rgba[start..start + row_bytes].as_chunks::<4>().0.iter() {
            for k in 0..3 {
                sum[k] += f64::from(px[k]);
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

fn luminance(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

fn save(name: &str, p: &Picture) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, encode_png(&p.rgba, p.width, p.height).unwrap()).unwrap();
    println!("wrote {}", path.display());
}

#[test]
fn a_valley_at_noon_has_sky_above_and_land_below() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (world, hf) = valley(12.0, 4.0);
    let problems = r.sync(&world, Some(&hf));
    assert!(problems.is_empty(), "{problems:?}");
    let camera = Camera::framing(&world, Some(&hf));
    let p = r.render(&request(480, 270, camera)).unwrap();
    assert!(p.problems.is_empty());
    assert!(p.stats.triangles > 0 && p.stats.draws > 0, "{:?}", p.stats);
    let top = mean_rgb(&p, 0..6);
    let bottom = mean_rgb(&p, 264..270);
    println!("top {top:?}, bottom {bottom:?}, stats {:?}", p.stats);
    assert!(
        top[2] > top[0],
        "the top rows are sky, blue above red: {top:?}"
    );
    assert!(
        !(bottom[2] > bottom[0] && bottom[2] > bottom[1]),
        "the bottom rows are land, not sky: {bottom:?}"
    );
    let lum: Vec<f64> = p
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|px| luminance([px[0], px[1], px[2]].map(f64::from)))
        .collect();
    let mean = lum.iter().sum::<f64>() / lum.len() as f64;
    let var = lum.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / lum.len() as f64;
    assert!(
        var.sqrt() > 5.0,
        "the picture is not uniform: sd {}",
        var.sqrt()
    );
}

#[test]
fn a_large_red_box_in_the_middle_is_red() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat(64.0);
    add(
        &mut world,
        [0.0, 0.0, 0.0],
        20.0,
        shape(ShapeKind::Box, [4.0, 4.0, 4.0], [1.0, 0.0, 0.0]),
    );
    assert!(r.sync(&world, Some(&hf)).is_empty());
    let camera = Camera {
        position: [0.0, 3.0, 12.0],
        target: [0.0, 2.0, 0.0],
        fov_y_deg: 50.0,
    };
    let p = r.render(&request(320, 240, camera)).unwrap();
    let (w, h) = (p.width as usize, p.height as usize);
    let mut sum = [0.0; 3];
    for y in h / 2 - 5..h / 2 + 5 {
        for x in w / 2 - 5..w / 2 + 5 {
            let i = (y * w + x) * 4;
            for k in 0..3 {
                sum[k] += f64::from(p.rgba[i + k]);
            }
        }
    }
    println!("centre {sum:?}");
    assert!(
        sum[0] > 1.5 * sum[1] && sum[0] > 1.5 * sum[2],
        "the centre is red-dominant: {sum:?}"
    );
}

#[test]
fn midnight_is_much_darker_than_noon() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let mut means = Vec::new();
    for hour in [12.0, 0.0] {
        let (world, hf) = valley(hour, 4.0);
        r.sync(&world, Some(&hf));
        let p = r
            .render(&request(320, 180, Camera::framing(&world, Some(&hf))))
            .unwrap();
        means.push(luminance(mean_rgb(&p, 0..p.height)));
    }
    println!("noon {}, midnight {}", means[0], means[1]);
    assert!(means[1] < 0.5 * means[0], "{means:?}");
    assert!(means[1] > 0.0 || means[0] > 0.0);
}

/// Bounding box (x0, y0, x1, y1) and count of the pixels whose channels differ by more than
/// `threshold`.
fn difference(a: &Picture, b: &Picture, threshold: u8) -> Option<([usize; 4], usize)> {
    let w = a.width as usize;
    let mut bbox = [usize::MAX, usize::MAX, 0, 0];
    let mut count = 0;
    for (i, (pa, pb)) in a
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.rgba.as_chunks::<4>().0.iter())
        .enumerate()
    {
        if (0..3).any(|k| pa[k].abs_diff(pb[k]) > threshold) {
            let (x, y) = (i % w, i / w);
            bbox = [
                bbox[0].min(x),
                bbox[1].min(y),
                bbox[2].max(x),
                bbox[3].max(y),
            ];
            count += 1;
        }
    }
    (count > 0).then_some((bbox, count))
}

#[test]
fn a_tree_renders_in_a_compact_region_and_an_unknown_species_is_a_problem() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat(128.0);
    let camera = Camera {
        position: [0.0, 8.0, 45.0],
        target: [0.0, 8.0, 0.0],
        fov_y_deg: 60.0,
    };
    let req = request(320, 240, camera);
    r.sync(&world, Some(&hf));
    let without = r.render(&req).unwrap();
    add(&mut world, [0.0, 0.0, 0.0], 0.0, plant("quercus-robur", 3));
    let problems = r.sync(&world, Some(&hf));
    assert!(problems.is_empty(), "{problems:?}");
    let with = r.render(&req).unwrap();
    assert!(with.stats.triangles > without.stats.triangles);
    let (bbox, count) = difference(&without, &with, 24).expect("the tree changes the picture");
    let area = (bbox[2] - bbox[0] + 1) * (bbox[3] - bbox[1] + 1);
    println!("tree: {count} pixels differ in {bbox:?}");
    assert!(count > 1000, "the tree covers a good patch: {count}");
    assert!(
        area < (320 * 240) * 3 / 4,
        "the difference is compact, not the whole picture: {bbox:?}"
    );

    add(&mut world, [10.0, 0.0, 0.0], 0.0, plant("quercus-robr", 3));
    let problems = r.sync(&world, Some(&hf));
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("quercus-robr") && problems[0].contains("\"quercus-robur\""));
    assert!(problems[0].contains("Entity 2"), "{}", problems[0]);
    let p = r.render(&req).unwrap();
    assert_eq!(p.problems, problems);
}

#[test]
fn a_second_sync_leaves_no_stale_items() {
    let Some((gpu, mut r)) = renderer() else {
        return;
    };
    let (mut busy, hf) = flat(64.0);
    for (i, color) in [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]]
        .into_iter()
        .enumerate()
    {
        add(
            &mut busy,
            [i as f64 * 3.0, 0.0, 0.0],
            0.0,
            shape(ShapeKind::Box, [1.0, 1.0, 1.0], color),
        );
    }
    add(
        &mut busy,
        [0.0, 0.0, 5.0],
        0.0,
        shape(ShapeKind::Sphere, [1.0; 3], [0.0, 1.0, 0.0]),
    );
    add(
        &mut busy,
        [5.0, 0.0, 5.0],
        0.0,
        Component::Character(Character::default()),
    );
    add(&mut busy, [-5.0, 0.0, 5.0], 0.0, plant("betula-pendula", 1));
    let (mut quiet, _) = flat(64.0);
    add(
        &mut quiet,
        [0.0, 0.0, 0.0],
        0.0,
        shape(ShapeKind::Cylinder, [1.0, 2.0, 1.0], [0.5, 0.5, 0.5]),
    );

    assert_eq!(r.item_count(), 0);
    r.sync(&busy, Some(&hf));
    // One terrain chunk, red boxes, the blue box, the sphere, the character, the birch.
    assert_eq!(r.item_count(), 6);
    r.sync(&quiet, Some(&hf));
    // One terrain chunk and the cylinder.
    assert_eq!(r.item_count(), 2);
    r.sync(&busy, Some(&hf));
    assert_eq!(r.item_count(), 6);
    // An unchanged world is not rebuilt; an environment change is no rebuild either.
    busy.environment.time_of_day_h = 18.0;
    r.sync(&busy, Some(&hf));
    assert_eq!(r.item_count(), 6);
    r.sync(&World::new("nothing", 0), None);
    assert_eq!(r.item_count(), 0);
    drop(gpu);
}

#[test]
fn pictures_of_a_valley_and_a_walker() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = valley(16.0, 2.0);
    let ground = |x: f64, z: f64| hf.height_at(x, z).unwrap();
    let mut rng = Rng::new(2026);
    let species = ["quercus-robur", "betula-pendula", "pinus-sylvestris"];
    for i in 0..40 {
        let (x, z) = (rng.range(-110.0, 110.0), rng.range(-110.0, 110.0));
        let seed = rng.below(3);
        let yaw = rng.range(0.0, 360.0);
        add(
            &mut world,
            [x, ground(x, z), z],
            yaw,
            plant(species[i % species.len()], seed),
        );
    }
    let walker = [12.0, 0.0, 40.0];
    let boxes = [
        ([4.0, 22.0], [0.8, 0.1, 0.05], [2.0, 2.0, 2.0]),
        ([22.0, 26.0], [0.05, 0.2, 0.8], [1.5, 3.0, 1.5]),
        ([0.0, 34.0], [0.9, 0.75, 0.1], [3.0, 1.0, 3.0]),
        ([26.0, 38.0], [0.9, 0.9, 0.9], [1.0, 1.0, 1.0]),
    ];
    for ([x, z], color, size) in boxes {
        add(
            &mut world,
            [x, ground(x, z), z],
            15.0,
            shape(ShapeKind::Box, size, color),
        );
    }
    let at = [walker[0], ground(walker[0], walker[2]), walker[2]];
    add(
        &mut world,
        at,
        20.0,
        Component::Character(Character::default()),
    );

    let start = std::time::Instant::now();
    let problems = r.sync(&world, Some(&hf));
    println!("sync: {:?}, problems {problems:?}", start.elapsed());
    assert!(problems.is_empty(), "{problems:?}");

    let framing = Camera::framing(&world, Some(&hf));
    let p = r.render(&request(1280, 720, framing)).unwrap();
    println!("valley: {:?}", p.stats);
    save("render-valley.png", &p);

    let character = world.entities().find(|e| e.character().is_some()).unwrap();
    let third = Camera::third_person(character, 7.0);
    let p = r.render(&request(1280, 720, third)).unwrap();
    println!("walker: {:?}", p.stats);
    save("render-walker.png", &p);
}
