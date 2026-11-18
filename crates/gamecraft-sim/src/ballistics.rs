//! The flight of a projectile: the air it flies through (density, speed of sound, wind),
//! the G7 reference drag function, and one substep of flight.
//!
//! # The model
//!
//! A projectile is a point mass. Per substep of [`SUBSTEP_S`] its acceleration is gravity plus
//! drag,
//!
//! ```text
//! a = -g y  -  rho * Cd_G7(|v_rel| / c) / (2 * BC) * |v_rel| * v_rel,     v_rel = v - wind
//! ```
//!
//! with `rho` the air density, `c` the speed of sound, `Cd_G7` the G7 reference drag function
//! and `BC` the launcher's ballistic coefficient in its kg/m2 form, mass over (form factor
//! times reference area): `F = rho/2 v^2 Cd_G7 i A` and `a = F / m`. It is integrated with
//! Heun's method (explicit trapezoid, second order), which is exact for a constant
//! acceleration, so a flight without air is the parabola. The air is taken at the start of a
//! substep.
//!
//! # The air
//!
//! - **Density** from the international standard atmosphere's barometric formula for the
//!   troposphere, `p = p0 (1 - L h / T0)^(g M / (R L))` at the height `h` above sea level
//!   (`environment.sea_level_m + y`), with the temperature `environment.temperature_c` at
//!   world y = 0 falling by `L` = 6.5 K per km, and `rho = p / (R_air T)`. At 15 degrees and
//!   sea level that is 1.225 kg/m3.
//! - **Speed of sound** `c = sqrt(gamma R_air T)`.
//! - **Wind** blows from `environment.wind_direction_deg` (clockwise from north) at
//!   `wind_speed_mps` measured [`WIND_REFERENCE_HEIGHT_M`] above the ground, and with a
//!   logarithmic profile over a roughness length of [`ROUGHNESS_LENGTH_M`]:
//!   `u(h) = u_ref ln(h / z0) / ln(h_ref / z0)`, none below `z0`. The height is taken above the
//!   terrain under the projectile, or above y = 0 without terrain.
//!
//! Rain and mist do not move a projectile.

use gamecraft_math::{Portable, deg_to_rad};
use gamecraft_world::Environment;

use crate::{DT, GRAVITY_MPS2, PROJECTILE_SUBSTEPS};

/// Seconds per substep of a projectile's flight: a tick cut into [`PROJECTILE_SUBSTEPS`].
pub const SUBSTEP_S: f64 = DT / PROJECTILE_SUBSTEPS as f64;

/// Height above the ground at which `environment.wind_speed_mps` is measured, metres (the
/// meteorological standard).
pub const WIND_REFERENCE_HEIGHT_M: f64 = 10.0;
/// Roughness length of the wind profile, metres: open, flat country with low grass.
pub const ROUGHNESS_LENGTH_M: f64 = 0.03;

/// Sea-level pressure of the standard atmosphere, pascal.
const P0_PA: f64 = 101_325.0;
/// Sea-level temperature of the standard atmosphere, kelvin.
const T0_K: f64 = 288.15;
/// Temperature lapse rate of the troposphere, kelvin per metre.
const LAPSE_K_PER_M: f64 = 0.0065;
/// Specific gas constant of dry air, J/(kg K).
const R_AIR: f64 = 287.052_87;
/// Exponent of the barometric formula, `g M / (R L)` with g 9.80665 m/s2, M 0.028 964 4
/// kg/mol, R 8.314 462 J/(mol K).
const BAROMETRIC_EXPONENT: f64 = 5.255_88;
/// Heat capacity ratio of air.
const GAMMA_AIR: f64 = 1.4;
/// Zero degrees Celsius in kelvin.
const KELVIN_OFFSET: f64 = 273.15;
/// The coldest air the formulas take, kelvin, so nothing divides by zero high up.
const MIN_TEMPERATURE_K: f64 = 150.0;

/// The G7 reference drag function: drag coefficient by Mach number.
///
/// The standard table of the G7 projectile (a long boat-tail with a tangent ogive) of the US
/// Army Ballistic Research Laboratory, as published with the open ballistics solver
/// py-ballisticcalc (`py_ballisticcalc/drag_tables.py`, `TableG7`,
/// <https://github.com/o-murphy/py-ballisticcalc>), the same values JBM Ballistics and other
/// solvers use. Interpolated linearly; beyond its ends the end values hold.
// 0.3010 at Mach 1.95 is a table value, not log10(2).
#[allow(clippy::approx_constant)]
pub const G7: [(f64, f64); 84] = [
    (0.00, 0.1198),
    (0.05, 0.1197),
    (0.10, 0.1196),
    (0.15, 0.1194),
    (0.20, 0.1193),
    (0.25, 0.1194),
    (0.30, 0.1194),
    (0.35, 0.1194),
    (0.40, 0.1193),
    (0.45, 0.1193),
    (0.50, 0.1194),
    (0.55, 0.1193),
    (0.60, 0.1194),
    (0.65, 0.1197),
    (0.70, 0.1202),
    (0.725, 0.1207),
    (0.75, 0.1215),
    (0.775, 0.1226),
    (0.80, 0.1242),
    (0.825, 0.1266),
    (0.85, 0.1306),
    (0.875, 0.1368),
    (0.90, 0.1464),
    (0.925, 0.1660),
    (0.95, 0.2054),
    (0.975, 0.2993),
    (1.0, 0.3803),
    (1.025, 0.4015),
    (1.05, 0.4043),
    (1.075, 0.4034),
    (1.10, 0.4014),
    (1.125, 0.3987),
    (1.15, 0.3955),
    (1.20, 0.3884),
    (1.25, 0.3810),
    (1.30, 0.3732),
    (1.35, 0.3657),
    (1.40, 0.3580),
    (1.50, 0.3440),
    (1.55, 0.3376),
    (1.60, 0.3315),
    (1.65, 0.3260),
    (1.70, 0.3209),
    (1.75, 0.3160),
    (1.80, 0.3117),
    (1.85, 0.3078),
    (1.90, 0.3042),
    (1.95, 0.3010),
    (2.00, 0.2980),
    (2.05, 0.2951),
    (2.10, 0.2922),
    (2.15, 0.2892),
    (2.20, 0.2864),
    (2.25, 0.2835),
    (2.30, 0.2807),
    (2.35, 0.2779),
    (2.40, 0.2752),
    (2.45, 0.2725),
    (2.50, 0.2697),
    (2.55, 0.2670),
    (2.60, 0.2643),
    (2.65, 0.2615),
    (2.70, 0.2588),
    (2.75, 0.2561),
    (2.80, 0.2533),
    (2.85, 0.2506),
    (2.90, 0.2479),
    (2.95, 0.2451),
    (3.00, 0.2424),
    (3.10, 0.2368),
    (3.20, 0.2313),
    (3.30, 0.2258),
    (3.40, 0.2205),
    (3.50, 0.2154),
    (3.60, 0.2106),
    (3.70, 0.2060),
    (3.80, 0.2017),
    (3.90, 0.1975),
    (4.00, 0.1935),
    (4.20, 0.1861),
    (4.40, 0.1793),
    (4.60, 0.1730),
    (4.80, 0.1672),
    (5.00, 0.1618),
];

/// The G7 drag coefficient at `mach`, linearly interpolated in [`G7`]; the first value below
/// the table or for a non-finite Mach number, the last above it.
pub fn g7_drag_coefficient(mach: f64) -> f64 {
    let (first, last) = ((0.0, 0.1198), (5.0, 0.1618));
    let (m0, c0) = G7.first().copied().unwrap_or(first);
    if !(mach.is_finite() && mach > m0) {
        return c0;
    }
    let mut previous = (m0, c0);
    for &(m, c) in G7.iter().skip(1) {
        if mach <= m {
            let (pm, pc) = previous;
            let span = m - pm;
            return if span > 0.0 {
                pc + (c - pc) * (mach - pm) / span
            } else {
                c
            };
        }
        previous = (m, c);
    }
    G7.last().copied().unwrap_or(last).1
}

/// The air at one point: what drag needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Air {
    /// Density, kilograms per cubic metre; 0 is a vacuum.
    pub density_kg_m3: f64,
    /// Speed of sound, metres per second.
    pub speed_of_sound_mps: f64,
    /// The wind's velocity, metres per second, world axes.
    pub wind_mps: [f64; 3],
}

/// The temperature, kelvin, at world height `y` when it is `temperature_c` at y = 0.
fn temperature_k(temperature_c: f64, y: f64) -> f64 {
    let t = temperature_c + KELVIN_OFFSET - LAPSE_K_PER_M * y;
    if t.is_finite() {
        t.max(MIN_TEMPERATURE_K)
    } else {
        T0_K
    }
}

/// The air density, kg/m3, at world height `y` when the temperature at y = 0 is
/// `temperature_c` and y = 0 lies `sea_level_m` above sea level (see the module docs). Never
/// negative or non-finite; 0 above the standard atmosphere's reach.
pub fn air_density_kg_m3(temperature_c: f64, sea_level_m: f64, y: f64) -> f64 {
    let h = sea_level_m + y;
    let base = 1.0 - LAPSE_K_PER_M * h / T0_K;
    if !(base.is_finite() && base > 0.0) {
        return 0.0;
    }
    let pressure = P0_PA * base.ppowf(BAROMETRIC_EXPONENT);
    let rho = pressure / (R_AIR * temperature_k(temperature_c, y));
    if rho.is_finite() && rho > 0.0 {
        rho
    } else {
        0.0
    }
}

/// The speed of sound, m/s, at world height `y` when the temperature at y = 0 is
/// `temperature_c`.
pub fn speed_of_sound_mps(temperature_c: f64, y: f64) -> f64 {
    (GAMMA_AIR * R_AIR * temperature_k(temperature_c, y)).sqrt()
}

/// The wind's velocity, m/s, `height_m` above the ground (see the module docs).
pub fn wind_at(env: &Environment, height_m: f64) -> [f64; 3] {
    let speed = env.wind_speed_mps;
    if !(speed.is_finite() && speed > 0.0 && height_m.is_finite() && height_m > ROUGHNESS_LENGTH_M)
    {
        return [0.0; 3];
    }
    let profile = (height_m / ROUGHNESS_LENGTH_M).pln()
        / (WIND_REFERENCE_HEIGHT_M / ROUGHNESS_LENGTH_M).pln();
    let u = speed * profile;
    // From bearing b (clockwise from north) it blows towards b + 180: east -sin b, north -cos b,
    // and world z is south.
    let (sin, cos) = deg_to_rad(env.wind_direction_deg).psin_cos();
    let w = [-sin * u, 0.0, cos * u];
    if w.iter().all(|c| c.is_finite()) {
        w
    } else {
        [0.0; 3]
    }
}

/// The air at `position`, with `ground_y` the terrain height under it (`None`: height above
/// y = 0 counts).
pub fn air_at(env: &Environment, ground_y: Option<f64>, position: [f64; 3]) -> Air {
    let [_, y, _] = position;
    Air {
        density_kg_m3: air_density_kg_m3(env.temperature_c, env.sea_level_m, y),
        speed_of_sound_mps: speed_of_sound_mps(env.temperature_c, y),
        wind_mps: wind_at(env, y - ground_y.unwrap_or(0.0)),
    }
}

/// Where a projectile is and how it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    /// World position, metres.
    pub position: [f64; 3],
    /// Velocity, metres per second, world axes.
    pub velocity: [f64; 3],
}

/// The acceleration of a projectile moving at `v` through `air` (see the module docs).
fn acceleration(v: [f64; 3], bc: f64, air: &Air) -> [f64; 3] {
    let [vx, vy, vz] = v;
    let [wx, wy, wz] = air.wind_mps;
    let rel = [vx - wx, vy - wy, vz - wz];
    let [rx, ry, rz] = rel;
    let speed = (rx * rx + ry * ry + rz * rz).sqrt();
    let k = if air.density_kg_m3 > 0.0 && bc > 0.0 && speed > 0.0 && air.speed_of_sound_mps > 0.0 {
        air.density_kg_m3 * g7_drag_coefficient(speed / air.speed_of_sound_mps) / (2.0 * bc) * speed
    } else {
        0.0
    };
    let k = if k.is_finite() { k } else { 0.0 };
    [-k * rx, -GRAVITY_MPS2 - k * ry, -k * rz]
}

/// One substep of [`SUBSTEP_S`] of flight with ballistic coefficient `bc` (kg/m2) through
/// `air`, by Heun's method.
pub fn advance(flight: Flight, bc: f64, air: &Air) -> Flight {
    let dt = SUBSTEP_S;
    let [vx, vy, vz] = flight.velocity;
    let [px, py, pz] = flight.position;
    let [ax, ay, az] = acceleration(flight.velocity, bc, air);
    let predicted = [vx + ax * dt, vy + ay * dt, vz + az * dt];
    let [bx, by, bz] = acceleration(predicted, bc, air);
    let (nx, ny, nz) = (
        vx + 0.5 * (ax + bx) * dt,
        vy + 0.5 * (ay + by) * dt,
        vz + 0.5 * (az + bz) * dt,
    );
    Flight {
        position: [
            px + 0.5 * (vx + nx) * dt,
            py + 0.5 * (vy + ny) * dt,
            pz + 0.5 * (vz + nz) * dt,
        ],
        velocity: [nx, ny, nz],
    }
}
