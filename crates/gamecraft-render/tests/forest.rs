//! The generators' forests on a real GPU: instanced, a bounded number of draws whatever the
//! number of trees, plants grown once and kept across rebuilds. Skips without an adapter
//! (fails with `GAMECRAFT_REQUIRE_GPU=1`). Writes `out/render-forest.png`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use gamecraft_render::{
    Camera, Picture, PictureRequest, Quality, WorldRenderer, encode_png, headless_gpu,
};
use gamecraft_world::{
    Area, Circle, Component, EntityDraft, Generator, GeneratorItem, GeneratorKind, HeightField,
    Plant, Shape, ShapeKind, Terrain, TerrainSource, World,
};
use std::sync::{Mutex, MutexGuard, OnceLock};

static SERIAL: Mutex<()> = Mutex::new(());

fn renderer() -> Option<(MutexGuard<'static, ()>, WorldRenderer)> {
    let (guard, device, queue) = gpu()?;
    Some((guard, WorldRenderer::new(device, queue)))
}

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

fn plant(species: &str) -> Component {
    Component::Plant(Plant {
        species: species.into(),
        seed: 7,
        growth: 1.0,
    })
}

/// A flat world with a forest of `density` trees per hectare in a circle of 200 m.
fn woods(density: f64) -> (World, HeightField) {
    let mut world = World::new("woods", 2);
    let terrain = Terrain {
        size_m: 512.0,
        cell_m: 4.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    };
    let hf = terrain.build().unwrap();
    world.terrain = Some(terrain);
    world.environment.time_of_day_h = 11.0;
    world.generators.push(Generator {
        id: "woods".into(),
        kind: GeneratorKind::Forest,
        area: Area::Circle(Circle {
            center: [0.0, 0.0],
            radius_m: 200.0,
        }),
        seed: 1,
        density_per_ha: density,
        min_distance_m: Some(1.5),
        slope_max_deg: None,
        height_range_m: None,
        exclude: Vec::new(),
        items: vec![
            GeneratorItem {
                growth_range: Some([0.6, 1.0]),
                ..GeneratorItem::of(2.0, vec![plant("picea-abies")])
            },
            GeneratorItem::of(1.0, vec![plant("betula-pendula")]),
        ],
    });
    world.regenerate(Some(&hf));
    (world, hf)
}

fn camera() -> Camera {
    Camera {
        position: [-260.0, 60.0, 260.0],
        target: [0.0, 0.0, 0.0],
        fov_y_deg: 45.0,
    }
}

fn picture(r: &mut WorldRenderer) -> Picture {
    r.render(&PictureRequest {
        width: 640,
        height: 360,
        camera: camera(),
        time_s: 1.0,
    })
    .unwrap()
}

#[test]
fn draws_stay_bounded_as_a_forest_grows_and_rebuilds_keep_the_grown_plants() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (small, hf) = woods(800.0);
    let problems = r.sync(&small, Some(&hf));
    assert!(problems.is_empty(), "{problems:?}");
    let few = picture(&mut r);
    let grown = r.generated_plants_grown();
    // Two species: spruces in three seeds and three growth classes, birches in three seeds.
    assert!(grown <= 12, "{grown} variants");
    let (big, hf) = woods(2400.0);
    let trees = big.generated().count();
    assert!(
        trees > 25_000 && trees > 2 * small.generated().count(),
        "{trees}"
    );
    let problems = r.sync(&big, Some(&hf));
    assert!(problems.is_empty(), "{problems:?}");
    let many = picture(&mut r);
    println!(
        "{} trees: {:?}; {trees} trees: {:?}",
        small.generated().count(),
        few.stats,
        many.stats
    );
    assert_eq!(
        r.generated_plants_grown(),
        grown,
        "the same variants, not grown again"
    );
    assert!(many.stats.instances > few.stats.instances);
    // Instanced: a draw is one part (bark, leaves) of one level of detail (or the impostor)
    // of one variant in one pass, so the draws are bounded by the variants, not by the
    // trees: three times the trees only fill in levels a smaller forest left empty.
    assert!(many.stats.instances > 2 * few.stats.instances);
    assert!(
        many.stats.draws <= few.stats.draws + few.stats.draws / 4,
        "{:?} vs {:?}",
        many.stats,
        few.stats
    );
    assert!(many.stats.draws < 60 * grown + 100, "{:?}", many.stats);
    // A rock added elsewhere rebuilds the scene but grows no plant again.
    let mut more = big.clone();
    more.add_entity(EntityDraft {
        position: [0.0, 0.0, 0.0],
        components: vec![Component::Shape(Shape {
            kind: ShapeKind::Box,
            size: [2.0, 2.0, 2.0],
            color: [0.5, 0.5, 0.5],
            solid: true,
        })],
        ..EntityDraft::default()
    })
    .unwrap();
    let before = r.rebuild_count();
    r.sync(&more, Some(&hf));
    assert_eq!(r.rebuild_count(), before + 1);
    assert_eq!(r.generated_plants_grown(), grown);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("render-forest.png"),
        encode_png(&many.rgba, many.width, many.height).unwrap(),
    )
    .unwrap();
    // A world handed over without its instances derived is derived by the renderer.
    let mut stale = big.clone();
    stale.generators[0].seed = 9;
    assert!(!stale.generated_is_current());
    r.sync(&stale, Some(&hf));
    assert!(picture(&mut r).stats.instances > 0);
}

/// Resident memory of this process, bytes (Linux; 0 elsewhere).
fn resident_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map_or(0, |pages| pages * 4096)
}

/// `examples/forest`: generation time and memory, draws and the time of a frame, printed for
/// `docs/design/generators.md`; writes `out/forest-measured.png`. Ignored by default:
/// `cargo test -p gamecraft-render --release --test forest -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement of 50 000 trees, minutes on a software adapter: run it with --ignored on a real GPU"]
fn the_forest_example_is_measured() {
    let Some((_gpu, device, queue)) = gpu() else {
        return;
    };
    let mut r = WorldRenderer::new(device.clone(), queue.clone());
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/forest/world.gamecraft");
    let Ok(text) = std::fs::read_to_string(&path) else {
        println!("skipping: no {}", path.display());
        return;
    };
    let mut world = World::from_json(&text).unwrap();
    let t = std::time::Instant::now();
    let hf = world.terrain.as_ref().unwrap().build().unwrap();
    let terrain_s = t.elapsed().as_secs_f64();
    let before = resident_bytes();
    let t = std::time::Instant::now();
    world.regenerate(Some(&hf));
    let generate_s = t.elapsed().as_secs_f64();
    let grown_bytes = resident_bytes().saturating_sub(before);
    let trees = world.generated().count();
    let t = std::time::Instant::now();
    let problems = r.sync(&world, Some(&hf));
    let sync_s = t.elapsed().as_secs_f64();
    assert!(problems.is_empty(), "{problems:?}");
    let camera = Camera {
        position: [-700.0, 140.0, 120.0],
        target: [100.0, 0.0, -150.0],
        fov_y_deg: 50.0,
    };
    let request = PictureRequest {
        width: 1280,
        height: 720,
        camera,
        time_s: 1.0,
    };
    let first = r.render(&request).unwrap();
    let frames = 10;
    let t = std::time::Instant::now();
    let mut last = first.clone();
    for _ in 0..frames {
        last = r.render(&request).unwrap();
    }
    let frame_ms = t.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
    println!(
        "forest: {trees} instances in {} generators; terrain built in {terrain_s:.3} s, \
         instances derived in {generate_s:.3} s, resident memory +{:.1} MB; first sync \
         (growing {} plant variants, baking their impostors) {sync_s:.2} s; a 1280x720 still \
         {frame_ms:.1} ms (with readback); {:?}",
        world.generators.len(),
        grown_bytes as f64 / 1e6,
        r.generated_plants_grown(),
        last.stats
    );
    assert!(trees > 40_000, "{trees}");
    // Instanced: far fewer draws than trees. VisionCraft draws an item's runs of neighbouring
    // 8 m cells that share a level of detail, so the draws follow the variants and the
    // forest's extent, not the number of trees.
    assert!(last.stats.draws < (trees as u64) / 4, "{:?}", last.stats);
    // The same valley without its generators, for the cost of the forest.
    let mut bare = world.clone();
    bare.generators.clear();
    bare.regenerate(Some(&hf));
    r.sync(&bare, Some(&hf));
    let _ = r.render(&request).unwrap();
    let t = std::time::Instant::now();
    let mut empty = first.clone();
    for _ in 0..frames {
        empty = r.render(&request).unwrap();
    }
    let bare_ms = t.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
    println!(
        "the valley without its forests: a still {bare_ms:.1} ms; {:?}",
        empty.stats
    );
    // Live frames at 1280 x 720 (the window's path), each waited for: with and without the
    // forests, at the quality LOW and MEDIUM.
    for quality in [Quality::Low, Quality::Medium] {
        r.set_quality(quality);
        r.resize(1280, 720);
        let with = live_ms(&mut r, &device, &queue, &world, &hf, &camera);
        let without = live_ms(&mut r, &device, &queue, &bare, &hf, &camera);
        println!(
            "live frame at {quality:?}: {with:.1} ms with the forests, {without:.1} ms without"
        );
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("forest-measured.png"),
        encode_png(&last.rgba, last.width, last.height).unwrap(),
    )
    .unwrap();
}

/// The mean time of a live frame of `world` (after a few to warm up), each waited for.
fn live_ms(
    r: &mut WorldRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    hf: &HeightField,
    camera: &Camera,
) -> f64 {
    r.sync(world, Some(hf));
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("forest target"),
        size: wgpu::Extent3d {
            width: 1280,
            height: 720,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut frame = |i: u32| {
        let mut encoder = device.create_command_encoder(&Default::default());
        r.draw(camera, f64::from(i) / 60.0, &view, &mut encoder)
            .unwrap();
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    };
    for i in 0..20 {
        frame(i);
    }
    let n = 60;
    let t = std::time::Instant::now();
    for i in 0..n {
        frame(20 + i);
    }
    t.elapsed().as_secs_f64() * 1000.0 / f64::from(n)
}
