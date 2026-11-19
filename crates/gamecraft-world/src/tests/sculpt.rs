//! Sculpt and paint edits: each edit's closed form, order, rebuilding by tile, the tile cache.

use crate::limits::TERRAIN_EDIT_WORK_MAX;
use crate::tiles::tile_file_name;
use crate::{HeightField, PaintRule, Terrain, TerrainEdit, TerrainLayer, TerrainSource, World};
use gamecraft_math::Rng;
use std::collections::BTreeMap;

fn flat(size_m: f64, cell_m: f64) -> Terrain {
    Terrain::new(size_m, cell_m, TerrainSource::Flat { height_m: 0.0 })
}

fn hills(size_m: f64, cell_m: f64) -> Terrain {
    Terrain::new(
        size_m,
        cell_m,
        TerrainSource::Noise {
            seed: 3,
            relief_m: 30.0,
            feature_m: 60.0,
            octaves: 4,
            roughness: 0.5,
        },
    )
}

fn raise(center: [f64; 2], radius_m: f64, strength_m: f64) -> TerrainEdit {
    TerrainEdit::Raise {
        center,
        radius_m,
        strength_m,
        falloff: 0.5,
    }
}

fn layer(name: &str, color: [f64; 3]) -> TerrainLayer {
    TerrainLayer {
        name: name.into(),
        color,
        roughness: 0.9,
    }
}

fn with(mut t: Terrain, edits: Vec<TerrainEdit>) -> Terrain {
    t.edits = edits;
    t
}

/// The node of `field` nearest to world `x`, `z`.
fn node(field: &HeightField, x: f64, z: f64) -> (usize, usize) {
    let [x0, z0] = field.min();
    (
        ((x - x0) / field.cell_m()).round() as usize,
        ((z - z0) / field.cell_m()).round() as usize,
    )
}

fn at(field: &HeightField, x: f64, z: f64) -> f32 {
    let (c, r) = node(field, x, z);
    field.sample(c, r).unwrap()
}

/// Every sample farther than `radius` from `center`, as (col, row).
fn outside(field: &HeightField, center: [f64; 2], radius: f64) -> Vec<(usize, usize)> {
    let [x0, z0] = field.min();
    let mut out = Vec::new();
    for r in 0..field.rows() {
        for c in 0..field.cols() {
            let x = x0 + c as f64 * field.cell_m();
            let z = z0 + r as f64 * field.cell_m();
            if ((x - center[0]) * (x - center[0]) + (z - center[1]) * (z - center[1])).sqrt()
                >= radius
            {
                out.push((c, r));
            }
        }
    }
    out
}

fn assert_unchanged_outside(a: &HeightField, b: &HeightField, center: [f64; 2], radius: f64) {
    for (c, r) in outside(a, center, radius) {
        assert_eq!(
            a.sample(c, r).unwrap().to_bits(),
            b.sample(c, r).unwrap().to_bits(),
            "sample ({c}, {r}) outside the radius changed"
        );
    }
}

#[test]
fn raise_and_lower_have_their_closed_form() {
    let base = flat(64.0, 1.0).build().unwrap();
    let up = with(flat(64.0, 1.0), vec![raise([3.0, -4.0], 10.0, 3.0)])
        .build()
        .unwrap();
    // Full strength inside radius * (1 - falloff) = 5 m.
    assert_eq!(at(&up, 3.0, -4.0), 3.0);
    assert_eq!(at(&up, 7.0, -4.0), 3.0);
    // In the falloff (7 m out: 0.4 of the way) the weight is 1 - smoothstep(0.4) = 0.648.
    assert!((at(&up, 10.0, -4.0) - 3.0 * 0.648).abs() < 1e-6);
    assert_unchanged_outside(&base, &up, [3.0, -4.0], 10.0);
    let down = with(
        flat(64.0, 1.0),
        vec![TerrainEdit::Lower {
            center: [3.0, -4.0],
            radius_m: 10.0,
            strength_m: 2.0,
            falloff: 0.0,
        }],
    )
    .build()
    .unwrap();
    assert_eq!(at(&down, 3.0, -4.0), -2.0);
    assert_eq!(at(&down, 12.0, -4.0), -2.0);
    assert_unchanged_outside(&base, &down, [3.0, -4.0], 10.0);
}

#[test]
fn flatten_levels_the_centre_and_order_matters() {
    let base = hills(128.0, 1.0).build().unwrap();
    let level = TerrainEdit::Flatten {
        center: [10.0, 10.0],
        radius_m: 12.0,
        height_m: 4.0,
        strength: 1.0,
        falloff: 0.5,
    };
    let flat_then = with(hills(128.0, 1.0), vec![level.clone()])
        .build()
        .unwrap();
    for (x, z) in [(10.0, 10.0), (14.0, 10.0), (10.0, 5.0)] {
        assert!((at(&flat_then, x, z) - 4.0).abs() < 1e-5, "{x} {z}");
    }
    assert_unchanged_outside(&base, &flat_then, [10.0, 10.0], 12.0);
    // A raise after the flatten lifts the level; before it, the flatten erases it.
    let bump = raise([10.0, 10.0], 3.0, 2.0);
    let raised_after = with(hills(128.0, 1.0), vec![level.clone(), bump.clone()])
        .build()
        .unwrap();
    let raised_before = with(hills(128.0, 1.0), vec![bump, level]).build().unwrap();
    assert!((at(&raised_after, 10.0, 10.0) - 6.0).abs() < 1e-5);
    assert!((at(&raised_before, 10.0, 10.0) - 4.0).abs() < 1e-5);
    // Raises and lowers commute (to rounding).
    let a = raise([0.0, 0.0], 20.0, 3.0);
    let b = TerrainEdit::Lower {
        center: [8.0, 0.0],
        radius_m: 15.0,
        strength_m: 1.5,
        falloff: 0.7,
    };
    let ab = with(hills(128.0, 1.0), vec![a.clone(), b.clone()])
        .build()
        .unwrap();
    let ba = with(hills(128.0, 1.0), vec![b, a]).build().unwrap();
    for (x, y) in ab.heights().iter().zip(ba.heights()) {
        assert!((x - y).abs() < 1e-5);
    }
}

#[test]
fn smooth_flattens_bumps_inside_its_radius_only() {
    let base = hills(128.0, 1.0).build().unwrap();
    let smooth = TerrainEdit::Smooth {
        center: [0.0, 0.0],
        radius_m: 20.0,
        strength: 1.0,
        iterations: 4,
        falloff: 0.3,
    };
    let smoothed = with(hills(128.0, 1.0), vec![smooth]).build().unwrap();
    let roughness = |f: &HeightField| {
        let (c0, r0) = node(f, -10.0, -10.0);
        let mut sum = 0.0;
        for r in r0..r0 + 20 {
            for c in c0..c0 + 20 {
                let h = |c: usize, r: usize| f64::from(f.sample(c, r).unwrap());
                let lap = 4.0 * h(c, r) - h(c - 1, r) - h(c + 1, r) - h(c, r - 1) - h(c, r + 1);
                sum += lap * lap;
            }
        }
        sum
    };
    assert!(roughness(&smoothed) < 0.5 * roughness(&base));
    assert_unchanged_outside(&base, &smoothed, [0.0, 0.0], 20.0);
}

#[test]
fn a_ramp_runs_straight_between_its_ends() {
    let base = hills(128.0, 1.0).build().unwrap();
    let ramp = TerrainEdit::Ramp {
        from: [-30.0, 5.0],
        to: [30.0, 5.0],
        width_m: 8.0,
        from_height_m: 2.0,
        to_height_m: 8.0,
        strength: 1.0,
        falloff: 0.5,
    };
    let road = with(hills(128.0, 1.0), vec![ramp]).build().unwrap();
    for (x, want) in [(-30.0, 2.0), (0.0, 5.0), (15.0, 6.5), (30.0, 8.0)] {
        assert!((f64::from(at(&road, x, 5.0)) - want).abs() < 1e-5, "{x}");
        // Across the flat part of the road (half width 4, falloff 0.5: 2 m each side).
        assert!((f64::from(at(&road, x, 7.0)) - want).abs() < 1e-5, "{x}");
    }
    // Beyond the half width nothing changed.
    for x in [-20.0, 0.0, 20.0] {
        for z in [10.0, -1.0] {
            assert_eq!(at(&road, x, z).to_bits(), at(&base, x, z).to_bits());
        }
    }
}

#[test]
fn noise_adds_detail_in_its_circle_only() {
    let base = flat(64.0, 1.0).build().unwrap();
    let edit = TerrainEdit::Noise {
        center: [0.0, 0.0],
        radius_m: 16.0,
        strength_m: 2.0,
        feature_m: 4.0,
        seed: 9,
        falloff: 0.25,
    };
    let rough = with(flat(64.0, 1.0), vec![edit.clone()]).build().unwrap();
    let again = with(flat(64.0, 1.0), vec![edit]).build().unwrap();
    assert_eq!(rough, again);
    assert_unchanged_outside(&base, &rough, [0.0, 0.0], 16.0);
    let (lo, hi) = rough.height_range();
    assert!(
        lo < -0.2 && hi > 0.2 && lo >= -2.0 && hi <= 2.0,
        "{lo} {hi}"
    );
}

fn painted() -> Terrain {
    let mut t = hills(128.0, 1.0);
    t.layers = vec![
        layer("grass", [0.1, 0.2, 0.05]),
        layer("rock", [0.3, 0.3, 0.3]),
        layer("road", [0.2, 0.15, 0.1]),
    ];
    t.paint_rules = vec![PaintRule {
        layer: "rock".into(),
        slope_min_deg: Some(35.0),
        ..PaintRule::default()
    }];
    t.edits = vec![
        raise([-30.0, -30.0], 6.0, 12.0),
        TerrainEdit::Paint {
            layer: "road".into(),
            center: [20.0, 20.0],
            radius_m: 6.0,
            strength: 1.0,
            falloff: 0.5,
        },
    ];
    t
}

#[test]
fn paint_weights_sum_to_255_and_follow_the_rules() {
    let field = painted().build().unwrap();
    assert_eq!(field.layer_count(), 3);
    assert_eq!(field.weights().len(), field.heights().len());
    for w in field.weights() {
        assert_eq!(w.iter().map(|b| u32::from(*b)).sum::<u32>(), 255, "{w:?}");
        assert_eq!(w[3..], [0; 5]);
    }
    // The painted road is all road at its centre, nothing beyond its radius.
    assert_eq!(field.weight_at(20.0, 20.0).unwrap()[2], 255);
    assert_eq!(field.weight_at(27.0, 20.0).unwrap()[2], 0);
    // The steep flank of the raised cone is rock; the cone's flat top is not.
    let flank = field.weight_at(-30.0 + 4.5, -30.0).unwrap();
    assert!(flank[1] > 200, "{flank:?}");
    assert_eq!(field.weight_at(-30.0, -30.0).unwrap()[1], 0);
    // Without layers there are no weights.
    let bare = hills(64.0, 1.0).build().unwrap();
    assert_eq!(bare.layer_count(), 0);
    assert!(bare.weights().is_empty());
    assert_eq!(bare.weight(0, 0), None);
}

#[test]
fn edits_and_layers_are_validated_with_the_field_named() {
    let mut t = painted();
    t.edits.push(TerrainEdit::Paint {
        layer: "snow".into(),
        center: [0.0, 0.0],
        radius_m: 5.0,
        strength: 1.0,
        falloff: 0.5,
    });
    let e = t.validate().unwrap_err().to_string();
    assert!(
        e.contains("terrain.edits[2].layer") && e.contains("grass, rock, road"),
        "{e}"
    );
    let mut t = painted();
    t.layers[1].name = "bad name".into();
    let e = t.validate().unwrap_err().to_string();
    assert!(e.contains("terrain.layers[1].name"), "{e}");
    let mut t = painted();
    t.layers.push(layer("grass", [0.0; 3]));
    assert!(t.validate().unwrap_err().to_string().contains("unique"));
    let mut t = painted();
    t.edits[0] = raise([0.0, 0.0], f64::NAN, 1.0);
    assert!(
        t.validate()
            .unwrap_err()
            .to_string()
            .contains("terrain.edits[0].radius_m")
    );
    let mut t = painted();
    t.edits[0] = TerrainEdit::Smooth {
        center: [0.0, 0.0],
        radius_m: 5.0,
        strength: 0.5,
        iterations: 99,
        falloff: 0.5,
    };
    assert!(t.validate().unwrap_err().to_string().contains("iterations"));
    // Too much work: big smooths over a big grid.
    let mut t = flat(4096.0, 2.0);
    t.edits = (0..20)
        .map(|_| TerrainEdit::Smooth {
            center: [0.0, 0.0],
            radius_m: 4096.0,
            strength: 0.5,
            iterations: 8,
            falloff: 0.5,
        })
        .collect();
    let e = t.validate().unwrap_err().to_string();
    assert!(e.contains(&TERRAIN_EDIT_WORK_MAX.to_string()), "{e}");
    // Too many layers.
    let mut t = painted();
    t.layers = (0..9).map(|i| layer(&format!("l{i}"), [0.0; 3])).collect();
    t.paint_rules.clear();
    t.edits.clear();
    assert!(t.validate().unwrap_err().to_string().contains("at most 8"));
}

#[test]
fn a_world_without_edits_writes_the_same_file() {
    let mut w = World::new("w", 1);
    w.terrain = Some(hills(64.0, 2.0));
    let text = w.to_json().unwrap();
    assert!(
        !text.contains("edits") && !text.contains("layers"),
        "{text}"
    );
    w.terrain = Some(painted());
    let text = w.to_json().unwrap();
    assert!(text.contains("\"op\": \"raise\"") || text.contains("\"op\":\"raise\""));
    let back = World::from_json(&text).unwrap();
    assert_eq!(back, w);
    // An unknown op is refused with the known ones.
    let bad = text.replace("\"raise\"", "\"dig\"");
    let e = World::from_json(&bad).unwrap_err().to_string();
    assert!(e.contains("dig"), "{e}");
}

/// A random edit on a terrain of `half` metres half-side.
fn random_edit(rng: &mut Rng, half: f64) -> TerrainEdit {
    let p = |rng: &mut Rng| [rng.range(-half, half), rng.range(-half, half)];
    let radius = rng.range(2.0, 30.0);
    match rng.below(7) {
        0 => raise(p(rng), radius, rng.range(0.0, 5.0)),
        1 => TerrainEdit::Lower {
            center: p(rng),
            radius_m: radius,
            strength_m: rng.range(0.0, 5.0),
            falloff: rng.unit(),
        },
        2 => TerrainEdit::Flatten {
            center: p(rng),
            radius_m: radius,
            height_m: rng.range(-5.0, 5.0),
            strength: rng.unit(),
            falloff: rng.unit(),
        },
        3 => TerrainEdit::Smooth {
            center: p(rng),
            radius_m: radius,
            strength: rng.unit(),
            iterations: 1 + rng.below(4) as u32,
            falloff: rng.unit(),
        },
        4 => TerrainEdit::Ramp {
            from: p(rng),
            to: p(rng),
            width_m: rng.range(2.0, 10.0),
            from_height_m: rng.range(-5.0, 5.0),
            to_height_m: rng.range(-5.0, 5.0),
            strength: rng.unit(),
            falloff: rng.unit(),
        },
        5 => TerrainEdit::Noise {
            center: p(rng),
            radius_m: radius,
            strength_m: rng.range(0.0, 3.0),
            feature_m: rng.range(2.0, 20.0),
            seed: rng.next_u64(),
            falloff: rng.unit(),
        },
        _ => TerrainEdit::Paint {
            layer: if rng.below(2) == 0 { "rock" } else { "road" }.into(),
            center: p(rng),
            radius_m: radius,
            strength: rng.unit(),
            falloff: rng.unit(),
        },
    }
}

#[test]
fn a_rebuild_is_exact_and_touches_only_the_tiles_of_the_change() {
    // 512 m at 1 m: 513 samples, 4 x 4 tiles of 128 cells.
    let mut t = painted();
    t.size_m = 512.0;
    t.edits.clear();
    let mut field = t.build().unwrap();
    assert_eq!(field.tile_count(), [4, 4]);
    let mut rng = Rng::new(5);
    for round in 0..40 {
        let previous = t.clone();
        if t.edits.len() > 3 && rng.below(3) == 0 {
            let i = rng.below(t.edits.len() as u64) as usize;
            t.edits.remove(i);
        } else {
            let i = rng.below(t.edits.len() as u64 + 1) as usize;
            t.edits.insert(i, random_edit(&mut rng, 250.0));
        }
        let rebuilt = t.rebuild(&previous, &field).unwrap();
        let full = t.build().unwrap();
        assert!(rebuilt.field == full, "round {round}: the rebuild differs");
        assert_eq!(rebuilt.tiles_total, 16);
        field = rebuilt.field;
    }
    // A small crater in the middle of one tile rebuilds that tile only.
    let previous = t.clone();
    t.edits.push(TerrainEdit::Lower {
        center: [-192.0, -192.0],
        radius_m: 6.0,
        strength_m: 3.0,
        falloff: 0.5,
    });
    let rebuilt = t.rebuild(&previous, &field).unwrap();
    assert_eq!(rebuilt.tiles, vec![[0, 0]]);
    assert!(rebuilt.field == t.build().unwrap());
    // Undo: back to the previous recipe gives the previous field exactly.
    let undone = previous.rebuild(&t, &rebuilt.field).unwrap();
    assert!(undone.field == field);
    assert_eq!(undone.tiles, vec![[0, 0]]);
    // Another source rebuilds everything.
    let mut other = t.clone();
    other.source = TerrainSource::Flat { height_m: 1.0 };
    assert_eq!(other.rebuild(&t, &rebuilt.field).unwrap().tiles.len(), 16);
}

#[test]
fn the_tile_cache_round_trips_bit_exactly_and_stale_tiles_are_rebuilt() {
    let mut t = painted();
    t.size_m = 512.0;
    let field = t.build().unwrap();
    let keys = t.tile_keys().unwrap();
    assert_eq!(keys.len(), 16);
    let files: BTreeMap<[usize; 2], Vec<u8>> = keys
        .iter()
        .map(|(&tile, &key)| (tile, field.tile_bytes(tile, key).unwrap()))
        .collect();
    assert_eq!(tile_file_name([2, 3]), "tile_2_3.bin");
    // 36 header bytes, 129 x 129 f32 heights, 3 weights per sample.
    assert_eq!(files[&[0, 0]].len(), 36 + 129 * 129 * 4 + 129 * 129 * 3);
    let loaded = t.build_cached(&files).unwrap();
    assert!(loaded.field == field);
    assert!(loaded.rebuilt.is_empty());
    // A crater in one tile makes that tile's key (and only that one) stale.
    let mut cratered = t.clone();
    cratered.edits.push(TerrainEdit::Lower {
        center: [64.0, 64.0],
        radius_m: 5.0,
        strength_m: 2.0,
        falloff: 0.5,
    });
    let new_keys = cratered.tile_keys().unwrap();
    let stale: Vec<_> = keys
        .iter()
        .filter(|(tile, key)| new_keys[*tile] != **key)
        .map(|(tile, _)| *tile)
        .collect();
    assert_eq!(stale, vec![[2, 2]]);
    let loaded = cratered.build_cached(&files).unwrap();
    assert_eq!(loaded.rebuilt, vec![[2, 2]]);
    assert!(loaded.field == cratered.build().unwrap());
    // Broken files are stale, never a panic.
    let mut broken = files.clone();
    broken.get_mut(&[0, 0]).unwrap().truncate(100);
    broken.get_mut(&[1, 0]).unwrap()[0] = b'X';
    let nan = f32::NAN.to_le_bytes();
    broken.get_mut(&[2, 0]).unwrap()[36..40].copy_from_slice(&nan);
    broken.get_mut(&[3, 0]).unwrap().push(0);
    broken.remove(&[0, 1]);
    let loaded = t.build_cached(&broken).unwrap();
    assert_eq!(loaded.rebuilt, vec![[0, 0], [0, 1], [1, 0], [2, 0], [3, 0]]);
    assert!(loaded.field == field);
    // Every truncation of a tile file is refused.
    let one = &files[&[1, 1]];
    for cut in (0..one.len()).step_by(997) {
        let mut m = files.clone();
        m.insert([1, 1], one[..cut].to_vec());
        assert_eq!(t.build_cached(&m).unwrap().rebuilt, vec![[1, 1]]);
    }
}

#[test]
fn a_four_kilometre_terrain_builds() {
    // 4 km at 2 m: 2049 x 2049 samples, 16 x 16 tiles.
    let mut t = hills(4096.0, 2.0);
    t.layers = vec![layer("grass", [0.1; 3]), layer("rock", [0.3; 3])];
    t.paint_rules = vec![PaintRule {
        layer: "rock".into(),
        slope_min_deg: Some(35.0),
        ..PaintRule::default()
    }];
    t.edits = vec![
        raise([100.0, 100.0], 200.0, 30.0),
        TerrainEdit::Ramp {
            from: [-1500.0, 0.0],
            to: [1500.0, 200.0],
            width_m: 12.0,
            from_height_m: 0.0,
            to_height_m: 20.0,
            strength: 1.0,
            falloff: 0.5,
        },
    ];
    let clock = std::time::Instant::now();
    let field = t.build().unwrap();
    let built = clock.elapsed();
    assert_eq!((field.cols(), field.rows()), (2049, 2049));
    assert_eq!(field.tile_count(), [16, 16]);
    assert_eq!(field.weights().len(), 2049 * 2049);
    // A crater rebuilds the tiles it touches only.
    let mut cratered = t.clone();
    cratered.edits.push(TerrainEdit::Lower {
        center: [300.0, -700.0],
        radius_m: 15.0,
        strength_m: 4.0,
        falloff: 0.5,
    });
    let clock = std::time::Instant::now();
    let rebuilt = cratered.rebuild(&t, &field).unwrap();
    let crater = clock.elapsed();
    assert!(rebuilt.tiles.len() <= 4, "{:?}", rebuilt.tiles);
    // Loading from a full cache builds nothing.
    let keys = t.tile_keys().unwrap();
    let files: BTreeMap<[usize; 2], Vec<u8>> = keys
        .iter()
        .map(|(&tile, &key)| (tile, field.tile_bytes(tile, key).unwrap()))
        .collect();
    let bytes: usize = files.values().map(Vec::len).sum();
    let clock = std::time::Instant::now();
    let loaded = t.build_cached(&files).unwrap();
    let cached = clock.elapsed();
    assert!(loaded.rebuilt.is_empty());
    assert!(loaded.field == field);
    let peak = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM"))
                .map(str::to_string)
        })
        .unwrap_or_default();
    println!(
        "4 km at 2 m: build {built:?}, crater rebuild {crater:?} ({} tiles), cache load \
         {cached:?} ({bytes} bytes in {} files), {peak}",
        rebuilt.tiles.len(),
        files.len()
    );
}
