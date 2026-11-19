//! The window's frame path without a window: the example world played for 120 frames with W
//! held, drawn through `GameView` into a texture as into a window's surface. Writes
//! `out/window-third.png` and `out/window-first.png`, and a night walk with a lantern
//! `out/window-night.png`, at the workspace root to look at.
//!
//! Skips with one printed line when no GPU adapter exists, and fails instead when
//! `GAMECRAFT_REQUIRE_GPU=1`.

// Integration tests are compiled as their own crates, outside `cfg(test)`, so clippy.toml's
// test allowances do not reach them. Picture statistics are not part of the deterministic
// core, so the platform's powers are fine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use gamecraft::game::Game;
use gamecraft::input::Key;
use gamecraft::rig::ViewMode;
use gamecraft::view::GameView;
use gamecraft_engine::example_world;
use gamecraft_render::{Quality, encode_png, headless_gpu};
use gamecraft_sim::DT;
use gamecraft_world::{Component, Light};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn target(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("window stand-in"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let row = WIDTH * 4;
    assert_eq!(row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(row * HEIGHT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| ());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    buffer.slice(..).get_mapped_range().unwrap().to_vec()
}

fn save(name: &str, rgba: &[u8]) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, encode_png(rgba, WIDTH, HEIGHT).unwrap()).unwrap();
    println!("wrote {}", path.display());
}

/// Mean luminance (0 to 255) and its standard deviation over the picture.
fn luminance_stats(rgba: &[u8]) -> (f64, f64) {
    let lum: Vec<f64> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]))
        .collect();
    let mean = lum.iter().sum::<f64>() / lum.len() as f64;
    let var = lum.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / lum.len() as f64;
    (mean, var.sqrt())
}

/// Pixels of the character's safety orange in the rows and columns `from` to `to` thirds of
/// the picture (0 to 3).
fn orange(rgba: &[u8], from: usize, to: usize) -> usize {
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let mut n = 0;
    for y in from * h / 3..to * h / 3 {
        for x in from * w / 3..to * w / 3 {
            let i = (y * w + x) * 4;
            let (r, g, b) = (
                i32::from(rgba[i]),
                i32::from(rgba[i + 1]),
                i32::from(rgba[i + 2]),
            );
            if r > 120 && r > g + 40 && g > b {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn a_walk_through_the_example_valley_in_both_views() {
    let (device, queue) = match headless_gpu() {
        Ok(gpu) => gpu,
        Err(e) => {
            if std::env::var("GAMECRAFT_REQUIRE_GPU").as_deref() == Ok("1") {
                panic!("GAMECRAFT_REQUIRE_GPU=1 but there is no GPU: {e}");
            }
            println!("skipping GPU test: {e}");
            return;
        }
    };
    let texture = target(&device);
    let surface = texture.create_view(&Default::default());
    let mut view = GameView::new(
        device.clone(),
        queue.clone(),
        Quality::Medium,
        (WIDTH, HEIGHT),
    );
    let mut game = Game::new(example_world());
    let scout = game.player().unwrap();
    let start = game
        .session()
        .current_world()
        .entity(scout)
        .unwrap()
        .position;
    assert_eq!(game.view(), ViewMode::ThirdPerson);

    let draw = |game: &Game, view: &mut GameView| {
        let mut encoder = device.create_command_encoder(&Default::default());
        let problems = view.draw(game, &surface, &mut encoder).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        queue.submit([encoder.finish()]);
    };

    game.key(Key::Forward, true);
    let started = std::time::Instant::now();
    let mut ticks = 0;
    for _ in 0..120 {
        ticks += game.frame(DT);
        draw(&game, &mut view);
    }
    let elapsed = started.elapsed();
    let end = game
        .session()
        .current_world()
        .entity(scout)
        .unwrap()
        .position;
    let walked = start[2] - end[2];
    println!(
        "120 frames: {ticks} ticks, walked {walked:.2} m north, {:.1} ms per frame, {} \
         rebuilds",
        elapsed.as_secs_f64() * 1000.0 / 120.0,
        view.rebuild_count()
    );
    assert!((118..=122).contains(&ticks), "{ticks}");
    assert!(walked > 2.5, "Scout walked about 3.2 m north: {walked}");
    assert_eq!(
        view.rebuild_count(),
        1,
        "a walk moves the character, it does not rebuild the scene"
    );
    for line in game.take_notices() {
        println!("told: {line}");
    }

    let third = read_back(&device, &queue, &texture);
    save("window-third.png", &third);
    let (mean, sd) = luminance_stats(&third);
    println!("third person: mean {mean:.1}, sd {sd:.1}");
    assert!(mean > 30.0 && sd > 10.0, "a lit, varied picture");
    let middle = orange(&third, 1, 2);
    println!("third person: {middle} orange pixels in the middle");
    assert!(
        middle > 200,
        "the character is in the middle of the picture"
    );

    game.key(Key::ToggleView, true);
    game.key(Key::ToggleView, false);
    assert_eq!(game.view(), ViewMode::FirstPerson);
    for _ in 0..10 {
        game.frame(DT);
        draw(&game, &mut view);
    }
    let first = read_back(&device, &queue, &texture);
    save("window-first.png", &first);
    let (mean, sd) = luminance_stats(&first);
    println!("first person: mean {mean:.1}, sd {sd:.1}");
    assert!(mean > 30.0 && sd > 10.0, "a lit, varied picture");
    let anywhere = orange(&first, 0, 3);
    println!("first person: {anywhere} orange pixels");
    assert!(anywhere < 50, "the character's own body is not in its view");
}

#[test]
fn a_lantern_carried_at_night_lights_the_way() {
    let (device, queue) = match headless_gpu() {
        Ok(gpu) => gpu,
        Err(e) => {
            if std::env::var("GAMECRAFT_REQUIRE_GPU").as_deref() == Ok("1") {
                panic!("GAMECRAFT_REQUIRE_GPU=1 but there is no GPU: {e}");
            }
            println!("skipping GPU test: {e}");
            return;
        }
    };
    let mut world = example_world();
    world.environment.time_of_day_h = 0.0;
    let scout = world
        .entities()
        .find(|e| e.character().is_some())
        .unwrap()
        .id;
    world
        .entity_mut(scout)
        .unwrap()
        .set_component(Component::Light(Light {
            color: [1.0, 0.8, 0.55],
            intensity_cd: 60.0,
            range_m: 10.0,
            on: true,
        }));
    let texture = target(&device);
    let surface = texture.create_view(&Default::default());
    let mut view = GameView::new(
        device.clone(),
        queue.clone(),
        Quality::Medium,
        (WIDTH, HEIGHT),
    );
    let mut game = Game::new(world);
    game.key(Key::Forward, true);
    for _ in 0..60 {
        game.frame(DT);
        let mut encoder = device.create_command_encoder(&Default::default());
        view.draw(&game, &surface, &mut encoder).unwrap();
        queue.submit([encoder.finish()]);
    }
    assert_eq!(
        view.rebuild_count(),
        1,
        "the lantern moves without a rebuild"
    );
    let night = read_back(&device, &queue, &texture);
    save("window-night.png", &night);
    // The lit ground round the character against the dark ground far off (the top of the
    // land, just under the horizon, at the left edge).
    let mean_in = |x0: usize, y0: usize, x1: usize, y1: usize| {
        let mut sum = 0.0;
        let mut n = 0.0;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * WIDTH as usize + x) * 4;
                sum += 0.2126 * f64::from(night[i])
                    + 0.7152 * f64::from(night[i + 1])
                    + 0.0722 * f64::from(night[i + 2]);
                n += 1.0;
            }
        }
        sum / n
    };
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let near = mean_in(w / 3, 3 * h / 4, 2 * w / 3, h);
    let far = mean_in(0, h / 3, w / 8, h / 2);
    println!("night: near the lantern {near:.1}, far off {far:.1}");
    assert!(
        near > 2.0 * far + 10.0,
        "the lantern lights the ground round it"
    );
}

#[test]
fn firing_in_first_person_shows_a_crosshair_and_tracers() {
    let (device, queue) = match headless_gpu() {
        Ok(gpu) => gpu,
        Err(e) => {
            if std::env::var("GAMECRAFT_REQUIRE_GPU").as_deref() == Ok("1") {
                panic!("GAMECRAFT_REQUIRE_GPU=1 but there is no GPU: {e}");
            }
            println!("skipping GPU test: {e}");
            return;
        }
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/skirmish/world.gamecraft");
    let world = gamecraft::app::load_world_file(&path).unwrap();
    let texture = target(&device);
    let surface = texture.create_view(&Default::default());
    let mut view = GameView::new(
        device.clone(),
        queue.clone(),
        Quality::Medium,
        (WIDTH, HEIGHT),
    );
    let mut game = Game::new(world);
    game.key(Key::ToggleView, true);
    game.key(Key::ToggleView, false);
    assert_eq!(game.view(), ViewMode::FirstPerson);
    game.fire(true);
    let mut flying = 0;
    for _ in 0..4 {
        game.frame(DT);
        flying = flying.max(game.effects().tracers.len());
        let mut encoder = device.create_command_encoder(&Default::default());
        let problems = view.draw(&game, &surface, &mut encoder).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        queue.submit([encoder.finish()]);
    }
    assert!(flying >= 1, "the first shot is in the air");
    let picture = read_back(&device, &queue, &texture);
    save("window-fire.png", &picture);
    let (cx, cy) = (WIDTH as usize / 2, HEIGHT as usize / 2);
    for (dx, dy) in [(9, 0), (-9, 0), (0, 9), (0, -9)] {
        let (x, y) = ((cx as i64 + dx) as usize, (cy as i64 + dy) as usize);
        let i = (y * WIDTH as usize + x) * 4;
        let [r, g, b] = [picture[i], picture[i + 1], picture[i + 2]];
        assert!(
            r.min(g).min(b) >= 225,
            "an arm of the crosshair at ({dx}, {dy}) from the centre: {r} {g} {b}"
        );
    }
}
