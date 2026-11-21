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
    // The project, so the village's crates come from its pack as the window hands them over.
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/skirmish/project");
    let texture = target(&device);
    let surface = texture.create_view(&Default::default());
    let mut view = GameView::new(
        device.clone(),
        queue.clone(),
        Quality::Medium,
        (WIDTH, HEIGHT),
    );
    let mut game = Game::from_project(&path).unwrap();
    game.key(Key::ToggleView, true);
    game.key(Key::ToggleView, false);
    assert_eq!(game.view(), ViewMode::FirstPerson);
    game.fire(true);
    let mut flying = 0;
    for _ in 0..4 {
        game.frame(DT);
        flying = flying.max(game.effects().tracers.len());
        view.provide_assets(game.take_assets());
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

/// Draws one frame of `game` and reads the picture back.
fn shoot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &mut GameView,
    texture: &wgpu::Texture,
    game: &mut Game,
) -> Vec<u8> {
    let surface = texture.create_view(&Default::default());
    view.provide_assets(game.take_assets());
    let mut encoder = device.create_command_encoder(&Default::default());
    let problems = view.draw(game, &surface, &mut encoder).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    queue.submit([encoder.finish()]);
    read_back(device, queue, texture)
}

/// The mean of each colour channel over the picture.
fn mean_rgb(rgba: &[u8]) -> [f64; 3] {
    let n = (rgba.len() / 4) as f64;
    let mut sum = [0.0; 3];
    for p in rgba.as_chunks::<4>().0 {
        for (s, c) in sum.iter_mut().zip(p) {
            *s += f64::from(*c);
        }
    }
    sum.map(|s| s / n)
}

/// How many pixels of the bottom right corner (a 360 x 140 block) differ between two pictures.
fn corner_changes(a: &[u8], b: &[u8]) -> usize {
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let mut n = 0;
    for y in h - 140..h {
        for x in w - 360..w {
            let i = (y * w + x) * 4;
            let d: i32 = (0..3)
                .map(|k| (i32::from(a[i + k]) - i32::from(b[i + k])).abs())
                .sum();
            if d > 90 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn the_hud_shows_the_ammunition_briefly_and_the_death_screen_is_unmistakable() {
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
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/skirmish/project");
    let texture = target(&device);
    let mut view = GameView::new(
        device.clone(),
        queue.clone(),
        Quality::Medium,
        (WIDTH, HEIGHT),
    );
    let mut game = Game::from_project(&path).unwrap();
    game.key(Key::ToggleView, true);
    game.key(Key::ToggleView, false);
    assert_eq!(game.view(), ViewMode::FirstPerson);
    for _ in 0..5 {
        game.frame(DT);
    }
    // The ammunition: hidden, then up after a check (T), then gone after three seconds.
    let hidden = shoot(&device, &queue, &mut view, &texture, &mut game);
    game.key(Key::CheckMagazine, true);
    game.key(Key::CheckMagazine, false);
    game.frame(DT);
    let shown = shoot(&device, &queue, &mut view, &texture, &mut game);
    save("hud-ammo.png", &shown);
    let changed = corner_changes(&hidden, &shown);
    assert!(
        changed > 400,
        "the ammunition changes the corner: {changed}"
    );
    for _ in 0..200 {
        game.frame(DT);
    }
    let gone = shoot(&device, &queue, &mut view, &texture, &mut game);
    assert!(
        corner_changes(&hidden, &gone) < changed / 4,
        "the ammunition is hidden again"
    );
    // A sniper 25 m north shoots the player down.
    let player = game.player().unwrap();
    let [px, _, pz] = game
        .session()
        .current_world()
        .entity(player)
        .unwrap()
        .position;
    let add = serde_json::json!({"id": 1, "method": "engine.execute", "params": {
        "command": "entity.add", "params": {"name": "Sniper", "at": [px, pz - 25.0],
        "components": [{"type": "launcher", "dispersion_mrad": 0, "max_flight_s": 1}]}}});
    let reply = game.control(&add.to_string()).unwrap();
    assert!(reply.contains("\"ok\":true"), "{reply}");
    let sniper = game
        .session()
        .current_world()
        .entities()
        .find(|e| e.name == "Sniper")
        .unwrap()
        .id;
    let fire = serde_json::json!({"id": 2, "method": "engine.execute",
        "params": {"command": "sim.fire", "params": {"entity": sniper.0, "yaw_deg": 180}}});
    let alive = shoot(&device, &queue, &mut view, &texture, &mut game);
    let reply = game.control(&fire.to_string()).unwrap();
    assert!(reply.contains("\"ok\":true"), "{reply}");
    let mut flash = None;
    for _ in 0..60 {
        game.frame(DT);
        if flash.is_none() && game.hud().vignette.is_some() && !game.is_dead() {
            flash = Some(shoot(&device, &queue, &mut view, &texture, &mut game));
        }
        if game.is_dead() {
            break;
        }
    }
    assert!(game.is_dead(), "the sniper's shot downs the player");
    if let Some(flash) = flash {
        save("hud-hit.png", &flash);
    }
    for _ in 0..120 {
        game.frame(DT);
    }
    let dead = shoot(&device, &queue, &mut view, &texture, &mut game);
    save("hud-death.png", &dead);
    let (before, after) = (luminance_stats(&alive).0, luminance_stats(&dead).0);
    assert!(
        after < before * 0.5,
        "the screen is dark: {before} -> {after}"
    );
    let [r, g, b] = mean_rgb(&dead);
    assert!(r > g * 1.3 && r > b * 1.3, "and red: {r} {g} {b}");
    // Enter: the world starts again from its document.
    game.key(Key::Confirm, true);
    game.key(Key::Confirm, false);
    game.frame(DT);
    assert!(!game.is_dead());
    let restarted = shoot(&device, &queue, &mut view, &texture, &mut game);
    assert!(luminance_stats(&restarted).0 > before * 0.7);
}

/// A project of your own seen down its player's sights, to look at: with
/// `GAMECRAFT_SIGHTS_PROJECT` set to a project folder, the player is put
/// `GAMECRAFT_SIGHTS_RANGE_M` (default 75) metres from the nearest other character (towards
/// `GAMECRAFT_SIGHTS_BEARING_DEG` from it, if given), turns to it
/// and is drawn from the hip (`out/window-sights-hip.png`) and fully down the sights
/// (`out/window-sights-ads.png`), its held model's attachments and reticle included. Skipped
/// without the variable.
#[test]
fn a_project_seen_down_the_sights() {
    let Ok(dir) = std::env::var("GAMECRAFT_SIGHTS_PROJECT") else {
        println!("skipping: GAMECRAFT_SIGHTS_PROJECT names no project");
        return;
    };
    let range_m: f64 = std::env::var("GAMECRAFT_SIGHTS_RANGE_M")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(75.0);
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
    let mut game = Game::from_project(std::path::Path::new(&dir)).unwrap();
    let player = game.player().unwrap();
    let world = game.session().current_world().clone();
    let me = world.entity(player).unwrap().position;
    let foe = world
        .entities()
        .filter(|e| e.id != player && e.character().is_some())
        .min_by(|a, b| {
            let d = |p: [f64; 3]| (p[0] - me[0]).powi(2) + (p[2] - me[2]).powi(2);
            d(a.position).total_cmp(&d(b.position))
        })
        .unwrap()
        .id;
    let there = world.entity(foe).unwrap().position;
    // From the target towards the player, or towards a compass bearing (degrees clockwise from
    // north) when `GAMECRAFT_SIGHTS_BEARING_DEG` gives one.
    let (dx, dz) = match std::env::var("GAMECRAFT_SIGHTS_BEARING_DEG")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        Some(b) => (b.to_radians().sin(), -b.to_radians().cos()),
        None => (me[0] - there[0], me[2] - there[2]),
    };
    let len = dx.hypot(dz).max(1e-9);
    let at = [there[0] + dx / len * range_m, there[2] + dz / len * range_m];
    game.session_mut()
        .run("entity.set", &serde_json::json!({"id": player.0, "at": at}))
        .unwrap();
    if game.view() != ViewMode::FirstPerson {
        game.key(Key::ToggleView, true);
        game.key(Key::ToggleView, false);
    }
    let texture = target(&device);
    let surface = texture.create_view(&Default::default());
    let mut view = GameView::new(
        device.clone(),
        queue.clone(),
        Quality::High,
        (WIDTH, HEIGHT),
    );
    // Each frame the mouse turns the view to the target's chest.
    let frame = |game: &mut Game, view: &mut GameView| {
        let session = game.session();
        let w = session.current_world();
        let (Some(t), Some(p)) = (w.entity(foe), w.entity(player)) else {
            return;
        };
        let eye_h = p.character().map_or(1.6, |c| c.eye_height_m);
        let chest = t.character().map_or(1.3, |c| c.height_m * 0.72);
        let d = [
            t.position[0] - p.position[0],
            t.position[1] + chest - (p.position[1] + eye_h),
            t.position[2] - p.position[2],
        ];
        let yaw = (-d[0]).atan2(-d[2]).to_degrees();
        let pitch = d[1].atan2(d[0].hypot(d[2])).to_degrees();
        let c = game.camera(16.0 / 9.0);
        let f = [
            c.target[0] - c.position[0],
            c.target[1] - c.position[1],
            c.target[2] - c.position[2],
        ];
        let now_yaw = (-f[0]).atan2(-f[2]).to_degrees();
        let now_pitch = f[1].atan2(f[0].hypot(f[2])).to_degrees();
        let turn = (yaw - now_yaw + 540.0).rem_euclid(360.0) - 180.0;
        let px = gamecraft::input::LOOK_DEG_PER_PIXEL;
        game.look(-turn / px, -(pitch - now_pitch) / px);
        game.frame(DT);
        view.provide_assets(game.take_assets());
        let mut encoder = device.create_command_encoder(&Default::default());
        view.draw(game, &surface, &mut encoder).unwrap();
        queue.submit([encoder.finish()]);
    };
    for _ in 0..30 {
        frame(&mut game, &mut view);
    }
    save(
        "window-sights-hip.png",
        &read_back(&device, &queue, &texture),
    );
    game.aim_down_sights(true);
    for _ in 0..40 {
        frame(&mut game, &mut view);
    }
    assert!(game.ads_blend() > 0.99, "{}", game.ads_blend());
    save(
        "window-sights-ads.png",
        &read_back(&device, &queue, &texture),
    );
}

/// A world that turns the crosshair off draws none: writes `out/window-hip.png` (with one)
/// and `out/window-no-crosshair.png`; the reload runs through its progress. (The lowered
/// weapon itself is drawn in `gamecraft-render`'s `models` test.)
#[test]
fn a_world_can_turn_the_crosshair_off() {
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
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/skirmish/project");
    let texture = target(&device);
    let mut view = GameView::new(
        device.clone(),
        queue.clone(),
        Quality::Medium,
        (WIDTH, HEIGHT),
    );
    let mut game = Game::from_project(&path).unwrap();
    game.key(Key::ToggleView, true);
    game.key(Key::ToggleView, false);
    assert_eq!(game.view(), ViewMode::FirstPerson);
    for _ in 0..3 {
        game.frame(DT);
    }
    let hip = shoot(&device, &queue, &mut view, &texture, &mut game);
    save("window-hip.png", &hip);
    let arm = |picture: &[u8]| {
        let i = ((HEIGHT as usize / 2) * WIDTH as usize + WIDTH as usize / 2 + 9) * 4;
        picture[i..i + 3].iter().copied().min().unwrap()
    };
    assert!(arm(&hip) >= 225, "a crosshair by default");
    game.session_mut()
        .run("view.set", &serde_json::json!({"crosshair": false}))
        .unwrap();
    game.frame(DT);
    let bare = shoot(&device, &queue, &mut view, &texture, &mut game);
    save("window-no-crosshair.png", &bare);
    assert!(arm(&bare) < 225, "no crosshair when the rules say none");
    game.fire(true);
    game.frame(DT);
    game.fire(false);
    game.key(Key::Reload, true);
    game.key(Key::Reload, false);
    let mut frames = 0;
    while game.reload_progress().is_none_or(|p| p < 0.5) && frames < 600 {
        game.frame(DT);
        frames += 1;
    }
    let progress = game.reload_progress().expect("reloading");
    assert!((0.5..0.75).contains(&progress), "{progress}");
}
