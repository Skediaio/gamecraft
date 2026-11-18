//! Oracles of the projectile flight: the vacuum parabola, a published G7 trajectory table, the
//! standard atmosphere, wind and air density (`docs/design/ballistics.md`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use gamecraft_sim::ballistics::{
    Air, Flight, SUBSTEP_S, advance, air_at, air_density_kg_m3, g7_drag_coefficient,
    speed_of_sound_mps, wind_at,
};
use gamecraft_sim::{DT, GRAVITY_MPS2, PROJECTILE_SUBSTEPS};
use gamecraft_world::Environment;

#[test]
fn substeps_cut_the_tick() {
    assert!((SUBSTEP_S * f64::from(PROJECTILE_SUBSTEPS) - DT).abs() < 1e-15);
    // A shot at 900 m/s moves at most about 2 m per substep.
    const { assert!(900.0 * SUBSTEP_S <= 2.0) };
}

/// In a vacuum the flight is the closed-form parabola within 1e-6 m over 2 s.
#[test]
fn vacuum_flight_is_the_parabola() {
    let vacuum = Air {
        density_kg_m3: 0.0,
        speed_of_sound_mps: 340.0,
        wind_mps: [0.0; 3],
    };
    let p0 = [3.0, 10.0, -4.0];
    let v0 = [120.0, 250.0, -310.0];
    let mut f = Flight {
        position: p0,
        velocity: v0,
    };
    let steps = (2.0 / SUBSTEP_S).round() as u32;
    let mut worst: f64 = 0.0;
    for n in 1..=steps {
        f = advance(f, 200.0, &vacuum);
        let t = f64::from(n) * SUBSTEP_S;
        let want = [
            p0[0] + v0[0] * t,
            p0[1] + v0[1] * t - 0.5 * GRAVITY_MPS2 * t * t,
            p0[2] + v0[2] * t,
        ];
        for (got, want) in f.position.iter().zip(want) {
            worst = worst.max((got - want).abs());
        }
    }
    assert!(worst < 1e-6, "off the parabola by {worst} m");
    assert!(
        (f.velocity[1] - (v0[1] - GRAVITY_MPS2 * 2.0)).abs() < 1e-9,
        "{:?}",
        f.velocity
    );
}

#[test]
fn the_g7_function_is_the_published_table() {
    // Points straight from the table, and a linear interpolation between two.
    assert_eq!(g7_drag_coefficient(0.0), 0.1198);
    assert_eq!(g7_drag_coefficient(1.0), 0.3803);
    assert_eq!(g7_drag_coefficient(2.0), 0.2980);
    assert!((g7_drag_coefficient(1.0125) - (0.3803 + 0.4015) / 2.0).abs() < 1e-12);
    // Beyond the table it holds its ends; nonsense gives the subsonic value, never a panic.
    assert_eq!(g7_drag_coefficient(9.0), 0.1618);
    assert_eq!(g7_drag_coefficient(-1.0), 0.1198);
    assert_eq!(g7_drag_coefficient(f64::NAN), 0.1198);
}

/// The international standard atmosphere: 1.225 kg/m3 at sea level and 15 degrees, 1.1117 at
/// 1000 m, 0.9093 at 3000 m (with the temperature at y = 0 falling 6.5 degrees per km);
/// colder air is denser. The speed of sound at 15 degrees is 340.3
/// m/s.
#[test]
fn the_standard_atmosphere() {
    assert!((air_density_kg_m3(15.0, 0.0, 0.0) - 1.2250).abs() < 5e-4);
    // At 1000 m above sea level the standard atmosphere is 8.5 degrees warm.
    assert!((air_density_kg_m3(8.5, 1000.0, 0.0) - 1.1117).abs() < 5e-4);
    assert!((air_density_kg_m3(15.0, 0.0, 3000.0) - 0.9093).abs() < 5e-4);
    assert!(air_density_kg_m3(-20.0, 0.0, 0.0) > air_density_kg_m3(40.0, 0.0, 0.0));
    assert!((speed_of_sound_mps(15.0, 0.0) - 340.3).abs() < 0.1);
    // Out of range or absurd heights never give a negative or non-finite density.
    for (t, s, y) in [(15.0, 0.0, 1e6), (15.0, 0.0, -1e6), (-90.0, 11000.0, 1e5)] {
        let rho = air_density_kg_m3(t, s, y);
        assert!(rho.is_finite() && rho >= 0.0, "{rho}");
    }
}

/// The wind blows from `wind_direction_deg`, at `wind_speed_mps` 10 m above the ground and
/// less below (a logarithmic profile); none at the ground.
#[test]
fn wind_direction_and_profile() {
    let env = Environment {
        wind_direction_deg: 270.0, // from the west
        wind_speed_mps: 5.0,
        ..Environment::default()
    };
    let w = wind_at(&env, 10.0);
    assert!(
        (w[0] - 5.0).abs() < 1e-9 && w[1] == 0.0 && w[2].abs() < 1e-9,
        "{w:?}"
    );
    let low = wind_at(&env, 1.5);
    assert!(low[0] > 0.0 && low[0] < 5.0, "{low:?}");
    assert_eq!(wind_at(&env, 0.0), [0.0; 3]);
    let north = Environment {
        wind_direction_deg: 0.0,
        ..env.clone()
    };
    let n = wind_at(&north, 10.0);
    assert!(
        n[2] > 4.99 && n[0].abs() < 1e-9,
        "from the north blows south (+z): {n:?}"
    );
    let air = air_at(&env, Some(0.0), [0.0, 10.0, 0.0]);
    assert!((air.wind_mps[0] - 5.0).abs() < 1e-9);
}

/// One row of the published trajectory table.
struct Row {
    yards: f64,
    velocity_fps: f64,
    time_s: f64,
    path_in: f64,
}

/// The reference projectile: a .308 (7.62 mm) 168 grain bullet with a G7 ballistic coefficient
/// of 0.223 lb/in2 at 2750 ft/s, sight 2 in over the bore, bore raised 4.221 MOA (zero at 100
/// yd), the ICAO standard atmosphere. The rows are the published G7 trajectory of the open
/// py-ballisticcalc test suite (`tests/test_trajectory.py`, `test_path_g7`, LGPL-3.0, values
/// checked by its authors against JBM Ballistics):
/// <https://github.com/o-murphy/py-ballisticcalc/blob/master/tests/test_trajectory.py>.
///
/// Tolerances: time of flight 0.5 %, velocity 1 %, path 0.5 in at 100 yd, 1.5 in at 500 yd and
/// 6 in (0.6 MOA) at 1000 yd. They are not tighter because the reference was computed with a
/// 5 mph quartering wind (we fly without wind), with another integrator and its own constants
/// for the speed of sound and the unit conversions; the drag function is the same table. (Seen
/// when written: 0.3 %, 0.6 % and 1.9 in at 1000 yd.) A wrong drag function, density or BC
/// conversion misses by far more: an error of 10 % in drag moves the 1000 yd path by about
/// 50 in.
const REFERENCE: [Row; 3] = [
    Row {
        yards: 100.0,
        velocity_fps: 2545.0,
        time_s: 0.113,
        path_in: 0.0,
    },
    Row {
        yards: 500.0,
        velocity_fps: 1814.0,
        time_s: 0.672,
        path_in: -56.2,
    },
    Row {
        yards: 1000.0,
        velocity_fps: 1086.0,
        time_s: 1.748,
        path_in: -399.9,
    },
];

const M_PER_YARD: f64 = 0.9144;
const M_PER_INCH: f64 = 0.0254;
const MPS_PER_FPS: f64 = 0.3048;
/// G7 ballistic coefficient lb/in2 to the kg/m2 form of the launcher: 703.0696 * 4 / pi.
const BC_LB_IN2_TO_KG_M2: f64 = 703.069_6 * 4.0 / core::f64::consts::PI;

#[test]
fn drag_matches_a_published_g7_table() {
    use gamecraft_math::{Portable, deg_to_rad};
    let bc = 0.223 * BC_LB_IN2_TO_KG_M2;
    let mv = 2750.0 * MPS_PER_FPS;
    let elevation = deg_to_rad(4.221 / 60.0);
    let (s, c) = elevation.psin_cos();
    let env = Environment {
        wind_speed_mps: 0.0,
        ..Environment::default()
    };
    // Downrange along -z (north), the bore at y = 0, the sight line level at y = 2 in.
    let mut f = Flight {
        position: [0.0, 0.0, 0.0],
        velocity: [0.0, mv * s, -mv * c],
    };
    let mut t = 0.0;
    for row in REFERENCE {
        let range = row.yards * M_PER_YARD;
        loop {
            let air = air_at(&env, None, f.position);
            let next = advance(f, bc, &air);
            if -next.position[2] >= range {
                // Interpolate linearly inside the substep.
                let k = (range + f.position[2]) / (f.position[2] - next.position[2]);
                let time = t + k * SUBSTEP_S;
                let y = f.position[1] + k * (next.position[1] - f.position[1]);
                let speed = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                let v = speed(f.velocity) + k * (speed(next.velocity) - speed(f.velocity));
                let path_in = (y - 2.0 * M_PER_INCH) / M_PER_INCH;
                let v_fps = v / MPS_PER_FPS;
                let path_tol = match row.yards as u32 {
                    100 => 0.5,
                    500 => 1.5,
                    _ => 6.0,
                };
                println!(
                    "{} yd: t {time:.4} s (table {}), v {v_fps:.1} ft/s (table {}), path \
                     {path_in:.2} in (table {})",
                    row.yards, row.time_s, row.velocity_fps, row.path_in
                );
                assert!(
                    (time - row.time_s).abs() <= 0.005 * row.time_s,
                    "{} yd: time {time} vs {}",
                    row.yards,
                    row.time_s
                );
                assert!(
                    (v_fps - row.velocity_fps).abs() <= 0.01 * row.velocity_fps,
                    "{} yd: velocity {v_fps} vs {}",
                    row.yards,
                    row.velocity_fps
                );
                assert!(
                    (path_in - row.path_in).abs() <= path_tol,
                    "{} yd: path {path_in} in vs {}",
                    row.yards,
                    row.path_in
                );
                break;
            }
            f = next;
            t += SUBSTEP_S;
            assert!(t < 5.0, "the reference shot never reached {} yd", row.yards);
        }
    }
}
