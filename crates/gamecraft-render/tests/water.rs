//! Water on a real GPU: a pond is drawn as water (blue where the grass was) with the second
//! area of the world named as not drawn; the picture is written to `out/render-water*.png`.
//! The GPU tests skip without an adapter (and fail with `GAMECRAFT_REQUIRE_GPU=1`). The wave
//! rules and the choice of the surface are tested without a GPU in `src/water.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use gamecraft_render::{Camera, Picture, PictureRequest, WorldRenderer, encode_png, headless_gpu};
use gamecraft_world::{Component, EntityDraft, Terrain, TerrainSource, Water, World};
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
    let mut world = World::new("meadow", 1);
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

fn pond(name: &str, at: [f64; 3], half: f64) -> EntityDraft {
    EntityDraft {
        name: name.into(),
        position: at,
        components: vec![Component::Water(Water::new(
            0.0,
            vec![[-half, -half], [half, -half], [half, half], [-half, half]],
        ))],
        ..EntityDraft::default()
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

/// Mean (r, g, b) of a square of `half` pixels around the picture's centre.
fn centre(p: &Picture, half: usize) -> [f64; 3] {
    let (w, h) = (p.width as usize, p.height as usize);
    let mut sum = [0.0; 3];
    let mut n = 0.0;
    for y in h / 2 - half..h / 2 + half {
        for x in w / 2 - half..w / 2 + half {
            let i = (y * w + x) * 4;
            for (k, s) in sum.iter_mut().enumerate() {
                *s += f64::from(p.rgba[i + k]);
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

#[test]
fn a_pond_is_drawn_as_water_and_the_other_areas_are_named() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    let (mut world, hf) = flat();
    let camera = Camera {
        position: [0.0, 14.0, 18.0],
        target: [0.0, 0.0, 0.0],
        fov_y_deg: 50.0,
    };
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    let dry = picture(&mut r, camera);
    // A pond 0.6 m deep over the middle, and a small one the renderer cannot draw as well.
    world
        .add_entity(pond("Pond", [0.0, 0.6, 0.0], 6.0))
        .unwrap();
    world
        .add_entity(pond("Tarn", [22.0, 1.0, 0.0], 1.5))
        .unwrap();
    let problems = r.sync(&world, Some(&hf));
    let said: Vec<&String> = problems
        .iter()
        .filter(|p| p.contains("is not drawn"))
        .collect();
    assert_eq!(said.len(), 1, "{problems:?}");
    assert!(
        said[0].contains("\"Tarn\"") && said[0].contains("\"Pond\""),
        "{said:?}"
    );
    let wet = picture(&mut r, camera);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("render-water-dry.png"),
        encode_png(&dry.rgba, dry.width, dry.height).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.join("render-water.png"),
        encode_png(&wet.rgba, wet.width, wet.height).unwrap(),
    )
    .unwrap();
    let dry_c = centre(&dry, 8);
    let wet_c = centre(&wet, 8);
    println!("dry {:?} wet {:?}", dry_c, wet_c);
    // Grass is green; the pond over it is water: shallow, so turquoise (the sky reflected
    // and the bed through it), far bluer and less red than the grass.
    assert!(dry_c[1] > dry_c[2], "grass: {dry_c:?}");
    assert!(
        wet_c[2] > wet_c[0] + 30.0 && wet_c[0] < dry_c[0] - 30.0 && wet_c[2] > dry_c[2] + 30.0,
        "water: {wet_c:?} over grass {dry_c:?}"
    );
    // Without water again, the surface goes and the picture is the meadow's.
    let ids: Vec<_> = world.entities().map(|e| e.id).collect();
    world.remove_entities(&ids);
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    let again = picture(&mut r, camera);
    let again_c = centre(&again, 8);
    for (a, b) in again_c.iter().zip(dry_c) {
        assert!((a - b).abs() < 0.5, "{again_c:?} vs {dry_c:?}");
    }
}
