//! Tests on a real GPU. They skip with one printed line when `headless_gpu()` finds no
//! adapter, and fail instead when `GAMECRAFT_REQUIRE_GPU=1`.
//!
//! `pictures_of_a_valley_and_a_walker` writes `out/render-valley.png` and
//! `out/render-walker.png`, `a_night_valley_is_lit_where_the_lights_are_on` writes
//! `out/render-night.png`, at the workspace root for a person (or agent) to look at. The combat
//! tests write `out/skirmish-fight.png` (the skirmish world mid-fight: two tracers in the air,
//! one impact, one enemy down) and `out/lantern-night.png` (the Lantern world at night with
//! three of five beacons lit).

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
use gamecraft_render::{
    Camera, Effects, Impact, Picture, PictureRequest, Quality, Tracer, WorldRenderer, encode_png,
    headless_gpu,
};
use gamecraft_world::{
    Character, Component, EntityDraft, HeightField, Light, Plant, Shape, ShapeKind, Terrain,
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
        ..Default::default()
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
        ..Default::default()
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
    let camera = Camera::framing(&world, Some(&hf), 16.0 / 9.0);
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
        var.sqrt() > 2.0,
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
            .render(&request(
                320,
                180,
                Camera::framing(&world, Some(&hf), 16.0 / 9.0),
            ))
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
    // One terrain chunk, the apron, red boxes, the blue box, the sphere, the character, the birch.
    assert_eq!(r.item_count(), 7);
    r.sync(&quiet, Some(&hf));
    // One terrain chunk, the apron and the cylinder.
    assert_eq!(r.item_count(), 3);
    r.sync(&busy, Some(&hf));
    assert_eq!(r.item_count(), 7);
    // An unchanged world is not rebuilt; an environment change is no rebuild either.
    busy.environment.time_of_day_h = 18.0;
    r.sync(&busy, Some(&hf));
    assert_eq!(r.item_count(), 7);
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

    let framing = Camera::framing(&world, Some(&hf), 16.0 / 9.0);
    let p = r.render(&request(1280, 720, framing)).unwrap();
    println!("valley: {:?}", p.stats);
    save("render-valley.png", &p);

    let character = world.entities().find(|e| e.character().is_some()).unwrap();
    let third = Camera::third_person(character, 7.0);
    let p = r.render(&request(1280, 720, third)).unwrap();
    println!("walker: {:?}", p.stats);
    save("render-walker.png", &p);
}

/// Mean luminance (0 to 255) of the `half`-pixel square round the pixel where `point` appears.
fn luminance_at(p: &Picture, camera: &Camera, point: [f64; 3], half: i64) -> f64 {
    let aspect = f64::from(p.width) / f64::from(p.height);
    let [x, y] = camera.project(point, aspect).expect("the point is in view");
    let px = ((x * 0.5 + 0.5) * f64::from(p.width)) as i64;
    let py = ((0.5 - y * 0.5) * f64::from(p.height)) as i64;
    let mut sum = 0.0;
    let mut n = 0.0;
    for yy in py - half..=py + half {
        for xx in px - half..=px + half {
            assert!(
                (0..i64::from(p.width)).contains(&xx) && (0..i64::from(p.height)).contains(&yy),
                "{point:?} is inside the picture"
            );
            let i = (yy as usize * p.width as usize + xx as usize) * 4;
            sum += luminance([p.rgba[i], p.rgba[i + 1], p.rgba[i + 2]].map(f64::from));
            n += 1.0;
        }
    }
    sum / n
}

#[test]
fn a_night_valley_is_lit_where_the_lights_are_on() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let mut world = World::new("night valley", 3);
    let terrain = Terrain {
        size_m: 160.0,
        cell_m: 2.0,
        source: TerrainSource::Noise {
            seed: 3,
            relief_m: 8.0,
            feature_m: 120.0,
            octaves: 4,
            roughness: 0.5,
        },
        ..Default::default()
    };
    let hf = terrain.build().unwrap();
    world.terrain = Some(terrain);
    world.environment.time_of_day_h = 0.0;
    let ground = |x: f64, z: f64| [x, hf.height_at(x, z).unwrap(), z];
    // Three lantern posts: west and east on, the middle one off.
    let posts = [(-20.0, true), (0.0, false), (20.0, true)];
    for (x, on) in posts {
        world
            .add_entity(EntityDraft {
                position: ground(x, 0.0),
                components: vec![
                    shape(ShapeKind::Cylinder, [0.3, 2.5, 0.3], [0.3, 0.3, 0.3]),
                    Component::Light(Light {
                        color: [1.0, 0.8, 0.55],
                        intensity_cd: 150.0,
                        range_m: 9.0,
                        on,
                    }),
                ],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    let problems = r.sync(&world, Some(&hf));
    assert!(problems.is_empty(), "{problems:?}");
    let camera = Camera {
        position: [0.0, ground(0.0, 30.0)[1] + 22.0, 30.0],
        target: ground(0.0, 0.0),
        fov_y_deg: 70.0,
    };
    let p = r.render(&request(960, 540, camera)).unwrap();
    save("render-night.png", &p);
    let spot = |x: f64| luminance_at(&p, &camera, ground(x, 2.0), 4);
    let (west, middle, east) = (spot(-20.0), spot(0.0), spot(20.0));
    let dark = luminance_at(&p, &camera, ground(0.0, -12.0), 4);
    println!("west {west:.1}, middle {middle:.1}, east {east:.1}, dark {dark:.1}");
    for (name, lit) in [("west", west), ("east", east)] {
        assert!(
            lit > 3.0 * dark + 10.0,
            "the {name} light's spot ({lit:.1}) is much brighter than the dark ground ({dark:.1})"
        );
    }
    assert!(
        middle < 1.5 * dark + 6.0,
        "the switched-off light leaves its spot dark: {middle:.1} against {dark:.1}"
    );
}

/// Draws `frames` live frames into a new texture of `format` and reads it back as RGBA.
fn draw_into(
    r: &mut WorldRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    camera: &Camera,
    (width, height): (u32, u32),
    frames: u32,
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    for frame in 0..frames {
        let mut encoder = device.create_command_encoder(&Default::default());
        r.draw(camera, f64::from(frame) / 60.0, &view, &mut encoder)
            .unwrap();
        queue.submit([encoder.finish()]);
    }
    let row = width * 4;
    assert_eq!(row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| ());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mut rgba = buffer.slice(..).get_mapped_range().unwrap().to_vec();
    if matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    ) {
        for px in rgba.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
    }
    rgba
}

#[test]
fn live_frames_look_the_same_in_an_srgb_and_a_plain_target() {
    let Some((guard, device, queue)) = gpu() else {
        return;
    };
    let mut r = WorldRenderer::new(device.clone(), queue.clone());
    r.set_quality(Quality::Low);
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
    let size = (320, 192);
    r.resize(size.0, size.1);
    let mut pictures = Vec::new();
    for format in [
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8Unorm,
    ] {
        let rgba = draw_into(&mut r, &device, &queue, format, &camera, size, 8);
        pictures.push(Picture {
            width: size.0,
            height: size.1,
            rgba,
            stats: Default::default(),
            problems: Vec::new(),
        });
    }
    save("render-live.png", &pictures[0]);
    let means: Vec<f64> = pictures
        .iter()
        .map(|p| luminance(mean_rgb(p, 0..p.height)))
        .collect();
    println!("means {means:?}");
    assert!(means[0] > 20.0, "the live frame is not black: {means:?}");
    assert!(
        (means[0] - means[1]).abs() < 4.0,
        "an sRGB and a plain target get the same picture: {means:?}"
    );
    let p = &pictures[1];
    let mut centre = [0.0; 3];
    for y in 90..102 {
        for x in 150..170 {
            let i = (y * p.width as usize + x) * 4;
            for k in 0..3 {
                centre[k] += f64::from(p.rgba[i + k]);
            }
        }
    }
    assert!(
        centre[0] > 1.5 * centre[1] && centre[0] > 1.5 * centre[2],
        "the box is red: {centre:?}"
    );
    // A size the renderer refuses to draw is an error, not a panic.
    let bad = Camera {
        fov_y_deg: f64::NAN,
        ..camera
    };
    let view = device
        .create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(r.draw(&bad, 0.0, &view, &mut encoder).is_err());
    drop(guard);
}

#[test]
fn a_walking_character_moves_without_a_rebuild() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat(64.0);
    add(
        &mut world,
        [0.0, 0.0, 0.0],
        0.0,
        Component::Character(Character::default()),
    );
    add(
        &mut world,
        [5.0, 0.0, 0.0],
        0.0,
        shape(ShapeKind::Box, [1.0; 3], [0.5; 3]),
    );
    r.sync(&world, Some(&hf));
    let rebuilds = r.rebuild_count();
    let items = r.item_count();
    let walker = world.entities().next().unwrap().id;
    for step in 1..=10 {
        let e = world.entity_mut(walker).unwrap();
        e.position = [0.0, 0.0, -0.1 * f64::from(step)];
        e.rotation_y_deg = f64::from(step);
        e.set_component(Component::Light(Light::default()));
        r.sync(&world, Some(&hf));
    }
    assert_eq!(
        r.rebuild_count(),
        rebuilds,
        "moving and lighting is no rebuild"
    );
    assert_eq!(r.item_count(), items);
    // A first-person view leaves its own body out, and back in, without a rebuild either.
    let camera = Camera {
        position: [0.0, 1.0, 6.0],
        target: [0.0, 1.0, -1.0],
        fov_y_deg: 50.0,
    };
    let seen = r.render(&request(160, 90, camera)).unwrap();
    r.set_hidden_character(Some(walker));
    r.sync(&world, Some(&hf));
    let hidden = r.render(&request(160, 90, camera)).unwrap();
    assert!(
        difference(&seen, &hidden, 24).is_some(),
        "the hidden character is gone from the picture"
    );
    r.set_hidden_character(None);
    r.sync(&world, Some(&hf));
    let back = r.render(&request(160, 90, camera)).unwrap();
    assert!(difference(&seen, &back, 24).is_none(), "and back");
    assert_eq!(r.rebuild_count(), rebuilds);
    let block = world.entities().nth(1).unwrap().id;
    world.entity_mut(block).unwrap().position = [6.0, 0.0, 0.0];
    r.sync(&world, Some(&hf));
    assert_eq!(r.rebuild_count(), rebuilds + 1, "a moved box is rebuilt");
}

// ---- combat ----

fn orange_at(p: &Picture, camera: &Camera, point: [f64; 3]) -> bool {
    let aspect = f64::from(p.width) / f64::from(p.height);
    let Some([x, y]) = camera.project(point, aspect) else {
        return false;
    };
    let px = ((x * 0.5 + 0.5) * f64::from(p.width)) as i64;
    let py = ((0.5 - y * 0.5) * f64::from(p.height)) as i64;
    let mut hits = 0;
    for yy in py - 2..=py + 2 {
        for xx in px - 2..=px + 2 {
            let (xx, yy) = (
                xx.clamp(0, i64::from(p.width) - 1),
                yy.clamp(0, i64::from(p.height) - 1),
            );
            let i = (yy as usize * p.width as usize + xx as usize) * 4;
            let [r, g, b] = [p.rgba[i], p.rgba[i + 1], p.rgba[i + 2]].map(f64::from);
            if r > 1.25 * g && r > 1.8 * b && r > 100.0 {
                hits += 1;
            }
        }
    }
    hits >= 8
}

#[test]
fn tracers_and_puffs_are_drawn_and_cleared() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat(80.0);
    world.environment.time_of_day_h = 14.0;
    add(
        &mut world,
        [0.0, 0.0, -30.0],
        0.0,
        shape(ShapeKind::Box, [20.0, 8.0, 1.0], [0.5, 0.5, 0.5]),
    );
    r.sync(&world, Some(&hf));
    let camera = Camera {
        position: [12.0, 2.0, 8.0],
        target: [0.0, 1.5, -10.0],
        fov_y_deg: 60.0,
    };
    let req = request(640, 360, camera);
    let plain = r.render(&req).unwrap();
    r.set_effects(&Effects {
        tracers: vec![Tracer {
            from: [-3.0, 1.5, 5.0],
            to: [-3.0, 1.5, -15.0],
            age_s: 0.1,
        }],
        impacts: vec![Impact {
            point: [3.0, 2.0, -29.5],
            normal: [0.0, 0.0, 1.0],
            age_s: 0.12,
        }],
    });
    let with = r.render(&req).unwrap();
    let (bbox, count) = difference(&plain, &with, 12).expect("the effects change the picture");
    println!("effects: {count} pixels differ, {bbox:?}");
    let aspect = 640.0 / 360.0;
    let [tx, ty] = camera.project([-3.0, 1.5, -5.0], aspect).unwrap();
    let (tpx, tpy) = (
        ((tx * 0.5 + 0.5) * 640.0) as usize,
        ((0.5 - ty * 0.5) * 360.0) as usize,
    );
    let lum_at = |p: &Picture, x: usize, y: usize| {
        let i = (y * 640 + x) * 4;
        luminance([p.rgba[i], p.rgba[i + 1], p.rgba[i + 2]].map(f64::from))
    };
    assert!(
        lum_at(&with, tpx, tpy) > lum_at(&plain, tpx, tpy) + 40.0,
        "the tracer is bright against what it crosses"
    );
    // The puff alone: the picture with and without the impact differs round the point hit.
    let tracers_only = {
        r.set_effects(&Effects {
            tracers: vec![Tracer {
                from: [-3.0, 1.5, 5.0],
                to: [-3.0, 1.5, -15.0],
                age_s: 0.1,
            }],
            impacts: Vec::new(),
        });
        r.render(&req).unwrap()
    };
    let [ix, iy] = camera.project([3.0, 2.0, -29.5], aspect).unwrap();
    let (ipx, ipy) = (
        ((ix * 0.5 + 0.5) * 640.0) as usize,
        ((0.5 - iy * 0.5) * 360.0) as usize,
    );
    let (bbox, count) = difference(&tracers_only, &with, 20).expect("the puff is drawn");
    println!("puff: {count} pixels differ, {bbox:?}, at ({ipx}, {ipy})");
    assert!(
        (bbox[0]..=bbox[2]).contains(&ipx) && (bbox[1]..=bbox[3]).contains(&ipy) && count > 30,
        "the impact shows where it hit"
    );
    // An impact that is over draws nothing, and clearing the effects restores the picture.
    r.set_effects(&Effects::default());
    let cleared = r.render(&req).unwrap();
    assert_eq!(difference(&plain, &cleared, 2), None);
}

#[test]
fn a_downed_character_lies_on_its_side() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat(60.0);
    world.environment.time_of_day_h = 13.0;
    add(
        &mut world,
        [0.0, 0.0, 0.0],
        0.0,
        Component::Character(Character::default()),
    );
    let id = world.entities().next().unwrap().id;
    // Seen from the south, the character faces away from the camera, up the picture.
    let camera = Camera {
        position: [0.0, 1.2, 7.0],
        target: [-0.5, 0.9, 0.0],
        fov_y_deg: 50.0,
    };
    let standing_head = [0.0, 1.6, 0.0];
    let lying_body = [-1.0, 0.3, 0.0];
    r.sync(&world, Some(&hf));
    let up = r.render(&request(640, 360, camera)).unwrap();
    save("combat-standing.png", &up);
    assert!(orange_at(&up, &camera, standing_head), "it stands");
    assert!(!orange_at(&up, &camera, lying_body));
    world
        .entity_mut(id)
        .unwrap()
        .set_component(Component::Health(gamecraft_world::Health {
            max: 100.0,
            current: 0.0,
            armor: 0.0,
        }));
    let rebuilds = r.rebuild_count();
    r.sync(&world, Some(&hf));
    assert_eq!(
        rebuilds,
        r.rebuild_count(),
        "going down moves the instance, no rebuild"
    );
    let down = r.render(&request(640, 360, camera)).unwrap();
    save("combat-down.png", &down);
    assert!(
        !orange_at(&down, &camera, standing_head),
        "it no longer stands"
    );
    assert!(
        orange_at(&down, &camera, lying_body),
        "it lies to the left, low"
    );
}

fn id_of(world: &World, name: &str) -> gamecraft_world::EntityId {
    world.entities().find(|e| e.name == name).unwrap().id
}

fn example(name: &str) -> World {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .join("world.gamecraft");
    World::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn the_skirmish_mid_fight_and_the_lantern_at_night() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    // ---- the skirmish ----
    let mut world = example("skirmish");
    let hf = world.terrain.as_ref().unwrap().build().unwrap();
    let ground = |x: f64, z: f64| hf.height_at(x, z).unwrap();
    // Stage the fight round the wall line (z 48): the rifleman behind it, two enemies in front.
    let stage = |world: &mut World, name: &str, x: f64, z: f64, yaw: f64| {
        let e = world.entity_mut(id_of(world, name)).unwrap();
        e.position = [x, ground(x, z), z];
        e.rotation_y_deg = yaw;
    };
    stage(&mut world, "Player", -6.0, 55.0, 0.0);
    stage(&mut world, "Patrol Alpha", -3.0, 34.0, 180.0);
    stage(&mut world, "Patrol Bravo", 5.5, 38.0, 150.0);
    let bravo = id_of(&world, "Patrol Bravo");
    world
        .entity_mut(bravo)
        .unwrap()
        .health_mut()
        .unwrap()
        .current = 0.0;
    let problems = r.sync(&world, Some(&hf));
    assert!(problems.is_empty(), "{problems:?}");
    let g = ground(-4.0, 40.0);
    // The top of the wall at x = -4.5: where the enemy's shot lands.
    let top = ground(-4.5, 48.0) + 1.4;
    // Muzzle and chest heights above the ground at the point.
    let at = |x: f64, z: f64, h: f64| [x, ground(x, z) + h, z];
    r.set_effects(&Effects {
        tracers: vec![
            // The rifleman's shot, going up the picture at the standing enemy.
            Tracer {
                from: at(-6.0, 53.5, 1.45),
                to: at(-3.1, 36.0, 1.4),
                age_s: 0.03,
            },
            // The enemy's answer, coming down on the wall.
            Tracer {
                from: at(-3.4, 37.0, 1.6),
                to: [-4.5, top, 47.9],
                age_s: 0.05,
            },
        ],
        impacts: vec![Impact {
            point: [-4.5, top, 47.9],
            normal: [0.0, 1.0, 0.0],
            age_s: 0.12,
        }],
    });
    let camera = Camera {
        position: [3.5, ground(3.5, 64.0) + 4.0, 64.0],
        target: [-3.0, g + 0.8, 40.0],
        fov_y_deg: 42.0,
    };
    let p = r.render(&request(1280, 720, camera)).unwrap();
    assert!(p.problems.is_empty(), "{:?}", p.problems);
    save("skirmish-fight.png", &p);

    // ---- the lantern at night, three of five beacons lit ----
    let mut world = example("lantern");
    let lit = ["Beacon Meadow", "Beacon Pines", "Beacon Ridge"];
    for name in lit {
        let id = id_of(&world, name);
        let e = world.entity_mut(id).unwrap();
        let mut light = e.light().unwrap().clone();
        light.on = true;
        e.set_component(Component::Light(light));
    }
    let hf = world.terrain.as_ref().unwrap().build().unwrap();
    let problems = r.sync(&world, Some(&hf));
    assert!(problems.is_empty(), "{problems:?}");
    r.set_effects(&Effects::default());
    // From the east and high up, the lit beacons of the west side are three pools in the dark.
    let camera = Camera {
        position: [125.0, 55.0, 20.0],
        target: [-20.0, -5.0, -10.0],
        fov_y_deg: 75.0,
    };
    let p = r.render(&request(1280, 720, camera)).unwrap();
    assert!(p.problems.is_empty(), "{:?}", p.problems);
    save("lantern-night.png", &p);
    assert!(luminance(mean_rgb(&p, 0..p.height)) < 90.0, "it is night");
}

/// Mean (r, g, b) of a square of `half` pixels round where `point` is seen.
fn rgb_at(p: &Picture, camera: &Camera, point: [f64; 3], half: i64) -> [f64; 3] {
    let aspect = f64::from(p.width) / f64::from(p.height);
    let [x, y] = camera.project(point, aspect).expect("the point is in view");
    let px = ((x * 0.5 + 0.5) * f64::from(p.width)) as i64;
    let py = ((0.5 - y * 0.5) * f64::from(p.height)) as i64;
    let mut sum = [0.0; 3];
    let mut n = 0.0;
    for yy in py - half..=py + half {
        for xx in px - half..=px + half {
            let i = (yy as usize * p.width as usize + xx as usize) * 4;
            for k in 0..3 {
                sum[k] += f64::from(p.rgba[i + k]);
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

#[test]
fn painted_layers_are_drawn_in_their_colours() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, _) = flat(64.0);
    let terrain = world.terrain.as_mut().unwrap();
    terrain.layers = vec![
        gamecraft_world::TerrainLayer {
            name: "grass".into(),
            color: [0.05, 0.25, 0.05],
            roughness: 0.9,
        },
        gamecraft_world::TerrainLayer {
            name: "paint".into(),
            color: [0.6, 0.05, 0.05],
            roughness: 0.9,
        },
    ];
    terrain.edits = vec![gamecraft_world::TerrainEdit::Paint {
        layer: "paint".into(),
        center: [0.0, 0.0],
        radius_m: 8.0,
        strength: 1.0,
        falloff: 0.2,
    }];
    let hf = world.terrain.as_ref().unwrap().build().unwrap();
    assert!(r.sync(&world, Some(&hf)).is_empty());
    let camera = Camera {
        position: [0.0, 30.0, 20.0],
        target: [0.0, 0.0, 0.0],
        fov_y_deg: 50.0,
    };
    let p = r.render(&request(480, 320, camera)).unwrap();
    save("render-painted.png", &p);
    let painted = rgb_at(&p, &camera, [0.0, 0.0, 0.0], 4);
    let grass = rgb_at(&p, &camera, [16.0, 0.0, 0.0], 4);
    println!("painted {painted:?}, grass {grass:?}");
    assert!(
        painted[0] > 1.5 * painted[1],
        "the painted circle is red: {painted:?}"
    );
    assert!(grass[1] > 1.3 * grass[0], "the rest is green: {grass:?}");
    assert!(painted[0] > 1.8 * grass[0], "{painted:?} against {grass:?}");
    // Without layers the same ground is shaded by slope again (all grass on the flat).
    world.terrain.as_mut().unwrap().layers.clear();
    world.terrain.as_mut().unwrap().edits.clear();
    let hf = world.terrain.as_ref().unwrap().build().unwrap();
    assert!(r.sync(&world, Some(&hf)).is_empty());
    let p = r.render(&request(480, 320, camera)).unwrap();
    let plain = rgb_at(&p, &camera, [0.0, 0.0, 0.0], 4);
    assert!(plain[1] > plain[0], "{plain:?}");
}
