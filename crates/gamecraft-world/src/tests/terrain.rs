use crate::{HeightField, Terrain, TerrainSource, WorldError};
use gamecraft_math::{Rng, StateHasher};

fn noise_terrain(size_m: f64, cell_m: f64) -> Terrain {
    Terrain {
        size_m,
        cell_m,
        source: TerrainSource::Noise {
            seed: 7,
            relief_m: 40.0,
            feature_m: 30.0,
            octaves: 5,
            roughness: 0.5,
        },
    }
}

fn hilly() -> HeightField {
    noise_terrain(128.0, 2.0).build().unwrap()
}

fn node_x(hf: &HeightField, col: usize) -> f64 {
    hf.min()[0] + col as f64 * hf.cell_m()
}

fn node_z(hf: &HeightField, row: usize) -> f64 {
    hf.min()[1] + row as f64 * hf.cell_m()
}

#[test]
fn geometry_of_the_grid() {
    let hf = noise_terrain(64.0, 2.0).build().unwrap();
    assert_eq!((hf.cols(), hf.rows()), (33, 33));
    assert_eq!(hf.heights().len(), 33 * 33);
    assert_eq!(hf.min(), [-32.0, -32.0]);
    assert_eq!(hf.max(), [32.0, 32.0]);
    assert_eq!(hf.cell_m(), 2.0);
    assert_eq!(hf.sample(1, 2), Some(hf.heights()[2 * 33 + 1]));
    assert_eq!(hf.sample(33, 0), None);
    assert_eq!(hf.sample(0, 33), None);
    assert_eq!(hf.sample(usize::MAX, usize::MAX), None);
}

#[test]
fn flat_terrain_is_constant() {
    let t = Terrain {
        size_m: 64.0,
        cell_m: 4.0,
        source: TerrainSource::Flat { height_m: 3.5 },
    };
    let hf = t.build().unwrap();
    assert!(hf.heights().iter().all(|&h| h == 3.5));
    assert_eq!(hf.height_range(), (3.5, 3.5));
    let mut r = Rng::new(1);
    for _ in 0..1000 {
        let x = r.range(-32.0, 32.0);
        let z = r.range(-32.0, 32.0);
        assert_eq!(hf.height_at(x, z), Some(3.5));
        assert_eq!(hf.normal_at(x, z), Some([0.0, 1.0, 0.0]));
    }
}

#[test]
fn noise_terrain_is_deterministic_and_pinned() {
    let t = noise_terrain(16.0, 1.0);
    let a = t.build().unwrap();
    let b = t.build().unwrap();
    assert_eq!(a, b);
    let mut h = StateHasher::new();
    h.write_u64(a.cols() as u64);
    h.write_u64(a.rows() as u64);
    for &v in a.heights() {
        h.write_f32(v);
    }
    assert_eq!(h.finish(), GOLDEN_NOISE_16_HASH, "{a:?}");
}

const GOLDEN_NOISE_16_HASH: u64 = 2_193_978_194_858_708_058;

#[test]
fn noise_terrain_stays_within_half_the_relief() {
    let hf = noise_terrain(256.0, 2.0).build().unwrap();
    let (lo, hi) = hf.height_range();
    assert!(lo >= -20.0 && hi <= 20.0, "{lo}..{hi}");
    assert!(hi - lo > 5.0, "terrain is too flat: {lo}..{hi}");
}

#[test]
fn a_larger_terrain_extends_the_same_land() {
    let small = noise_terrain(64.0, 2.0).build().unwrap();
    let large = noise_terrain(128.0, 2.0).build().unwrap();
    for row in 0..small.rows() {
        for col in 0..small.cols() {
            let x = node_x(&small, col);
            let z = node_z(&small, row);
            let s = small.sample(col, row).unwrap();
            let l = large.height_at(x, z).unwrap();
            assert_eq!(f64::from(s).to_bits(), l.to_bits(), "at {x}, {z}");
        }
    }
}

#[test]
fn height_at_equals_the_samples_at_nodes() {
    let hf = hilly();
    for row in 0..hf.rows() {
        for col in 0..hf.cols() {
            let h = hf.height_at(node_x(&hf, col), node_z(&hf, row)).unwrap();
            assert_eq!(
                h,
                f64::from(hf.sample(col, row).unwrap()),
                "node {col}, {row}"
            );
        }
    }
}

/// The documented split: the diagonal runs from (col, row) to (col + 1, row + 1).
fn reference_height(hf: &HeightField, x: f64, z: f64) -> f64 {
    let u = (x - hf.min()[0]) / hf.cell_m();
    let v = (z - hf.min()[1]) / hf.cell_m();
    let col = (u.floor() as usize).min(hf.cols() - 2);
    let row = (v.floor() as usize).min(hf.rows() - 2);
    let fu = u - col as f64;
    let fv = v - row as f64;
    let h = |c, r| f64::from(hf.sample(c, r).unwrap());
    let (h00, h10, h01, h11) = (
        h(col, row),
        h(col + 1, row),
        h(col, row + 1),
        h(col + 1, row + 1),
    );
    if fu >= fv {
        h00 + fu * (h10 - h00) + fv * (h11 - h10)
    } else {
        h00 + fu * (h11 - h01) + fv * (h01 - h00)
    }
}

#[test]
fn height_at_is_the_plane_of_the_triangle() {
    let hf = hilly();
    let mut r = Rng::new(5);
    for _ in 0..20_000 {
        let x = r.range(-64.0, 64.0);
        let z = r.range(-64.0, 64.0);
        let got = hf.height_at(x, z).unwrap();
        let want = reference_height(&hf, x, z);
        assert!((got - want).abs() < 1e-9, "at {x}, {z}: {got} vs {want}");
    }
}

#[test]
fn height_at_is_continuous_across_borders_and_the_diagonal() {
    let hf = hilly();
    let eps = 1e-7;
    let mut r = Rng::new(6);
    for _ in 0..5000 {
        let col = r.below(63) as usize + 1;
        let row = r.below(63) as usize + 1;
        let t = r.unit();
        let x0 = node_x(&hf, col);
        let z0 = node_z(&hf, row);
        let c = hf.cell_m();
        // A vertical border, a horizontal border, the diagonal.
        let points = [(x0, z0 + t * c), (x0 + t * c, z0), (x0 + t * c, z0 + t * c)];
        for (x, z) in points {
            let here = hf.height_at(x, z).unwrap();
            for (dx, dz) in [
                (eps, 0.0),
                (-eps, 0.0),
                (0.0, eps),
                (0.0, -eps),
                (eps, -eps),
                (-eps, eps),
            ] {
                let near = hf.height_at(x + dx, z + dz).unwrap();
                assert!(
                    (here - near).abs() < 1e-4,
                    "jump at {x}, {z}: {here} vs {near}"
                );
            }
        }
    }
}

#[test]
fn height_at_is_none_outside_and_for_bad_input() {
    let hf = hilly();
    assert!(hf.height_at(64.0, 64.0).is_some());
    assert!(hf.height_at(-64.0, -64.0).is_some());
    for (x, z) in [
        (64.001, 0.0),
        (0.0, -64.001),
        (1e300, 0.0),
        (f64::NAN, 0.0),
        (0.0, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    ] {
        assert_eq!(hf.height_at(x, z), None, "{x}, {z}");
        assert_eq!(hf.normal_at(x, z), None, "{x}, {z}");
    }
}

#[test]
fn normal_is_unit_upward_and_the_triangle_normal() {
    let hf = hilly();
    let mut r = Rng::new(8);
    for _ in 0..5000 {
        let x = r.range(-63.0, 63.0);
        let z = r.range(-63.0, 63.0);
        let n = hf.normal_at(x, z).unwrap();
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-12, "{n:?}");
        assert!(n[1] > 0.0);
        // The plane through (x, z) with this normal predicts the height a small step away
        // within the same triangle (steps of 1e-4 m rarely cross an edge; allow those).
        let h = hf.height_at(x, z).unwrap();
        let step = 1e-4;
        let predicted = h - (n[0] * step + n[2] * step) / n[1];
        let actual = hf.height_at(x + step, z + step).unwrap();
        assert!((predicted - actual).abs() < 1e-3, "{predicted} vs {actual}");
    }
}

#[test]
fn raycast_straight_down_hits_at_height_at() {
    let hf = hilly();
    let mut r = Rng::new(9);
    for _ in 0..2000 {
        let x = r.range(-64.0, 64.0);
        let z = r.range(-64.0, 64.0);
        let ground = hf.height_at(x, z).unwrap();
        let top = 100.0;
        let d = hf.raycast([x, top, z], [0.0, -1.0, 0.0], 500.0).unwrap();
        assert!((top - d - ground).abs() < 1e-9, "{x}, {z}: {d}");
        // A non-unit direction is normalised: the distance is in metres.
        let d2 = hf.raycast([x, top, z], [0.0, -7.0, 0.0], 500.0).unwrap();
        assert!((d - d2).abs() < 1e-9);
        // Too short a ray misses.
        assert_eq!(hf.raycast([x, top, z], [0.0, -1.0, 0.0], d * 0.5), None);
        // Pointing up misses.
        assert_eq!(hf.raycast([x, top, z], [0.0, 1.0, 0.0], 500.0), None);
    }
}

#[test]
fn grazing_rays_along_slopes_hit_the_surface() {
    let hf = hilly();
    let mut r = Rng::new(10);
    let mut tested = 0;
    for _ in 0..3000 {
        let x = r.range(-50.0, 50.0);
        let z = r.range(-50.0, 50.0);
        let ground = hf.height_at(x, z).unwrap();
        let n = hf.normal_at(x, z).unwrap();
        // Horizontal heading, then follow the slope along it, a little steeper downwards.
        let yaw = r.range(0.0, core::f64::consts::TAU);
        let (hx, hz) = (
            gamecraft_math::Portable::pcos(yaw),
            gamecraft_math::Portable::psin(yaw),
        );
        let slope = -(n[0] * hx + n[2] * hz) / n[1];
        let dir = [hx, slope - 0.01, hz];
        // Aim through the surface point (x, ground, z) from 3 m back along the ray, so the
        // ray must hit at or before it.
        let back = 3.0;
        let origin = [x - hx * back, ground - (slope - 0.01) * back, z - hz * back];
        match hf.height_at(origin[0], origin[2]) {
            Some(g) if origin[1] > g + 1e-3 => {}
            _ => continue,
        }
        tested += 1;
        let t = hf.raycast(origin, dir, 50.0).unwrap();
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        assert!(t <= back * len + 1e-9, "hit at {t}, beyond the aimed point");
        let p = [
            origin[0] + dir[0] / len * t,
            origin[1] + dir[1] / len * t,
            origin[2] + dir[2] / len * t,
        ];
        let g = hf.height_at(p[0], p[2]).unwrap();
        assert!(
            (p[1] - g).abs() < 0.01,
            "hit {p:?} is {} m off the ground",
            p[1] - g
        );
    }
    assert!(tested > 500, "only {tested} grazing rays tested");
}

#[test]
fn raycast_from_outside_the_terrain_and_bad_input() {
    let hf = hilly();
    // From outside, sloping down into the terrain.
    let t = hf
        .raycast([-100.0, 60.0, 0.0], [1.0, -0.5, 0.0], 1000.0)
        .unwrap();
    let len = (1.25_f64).sqrt();
    let p = [-100.0 + t / len, 60.0 - 0.5 * t / len];
    let g = hf.height_at(p[0], 0.0).unwrap();
    assert!((p[1] - g).abs() < 1e-6);
    // Passing beside the terrain misses.
    assert_eq!(
        hf.raycast([-100.0, 60.0, 100.0], [1.0, -0.5, 0.0], 1000.0),
        None
    );
    for (o, d, m) in [
        ([0.0, 50.0, 0.0], [0.0, 0.0, 0.0], 100.0),
        ([f64::NAN, 50.0, 0.0], [0.0, -1.0, 0.0], 100.0),
        ([0.0, 50.0, 0.0], [0.0, f64::INFINITY, 0.0], 100.0),
        ([0.0, 50.0, 0.0], [0.0, -1.0, 0.0], f64::NAN),
        ([0.0, 50.0, 0.0], [0.0, -1.0, 0.0], -1.0),
        ([0.0, 50.0, 0.0], [0.0, -1.0, 0.0], f64::INFINITY),
        ([0.0, -500.0, 0.0], [0.0, -1.0, 0.0], 100.0),
    ] {
        assert_eq!(hf.raycast(o, d, m), None, "{o:?} {d:?} {m}");
    }
    // Extreme but finite rays do not panic.
    let _ = hf.raycast([0.0, 1e300, 0.0], [1e-300, -1.0, 0.0], 1e300);
    let _ = hf.raycast([-1e300, 1e300, 1e300], [1.0, -1.0, -1.0], 1e300);
    let _ = hf.raycast([0.0, 10.0, 0.0], [f64::MIN_POSITIVE, -1.0, 0.0], 1e300);
}

#[test]
fn oversized_terrain_is_an_error_not_an_allocation() {
    let t = Terrain {
        size_m: 16384.0,
        cell_m: 0.25,
        source: TerrainSource::Flat { height_m: 0.0 },
    };
    match t.build() {
        Err(WorldError::TerrainTooLarge { fitting_cell_m, .. }) => assert_eq!(fitting_cell_m, 8.0),
        other => panic!("{other:?}"),
    }
    // The largest allowed grid builds.
    let t = Terrain {
        size_m: 4096.0,
        cell_m: 2.0,
        source: TerrainSource::Flat { height_m: 0.0 },
    };
    let hf = t.build().unwrap();
    assert_eq!(hf.cols(), 2049);
}
