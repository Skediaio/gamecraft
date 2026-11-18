//! The environment as light: exposure, haze, clouds and wind, in plain numbers (no GPU).
//!
//! The exposure follows VisionCraft's studio (`apps/visioncraft/src/studio.rs`: `auto_ev`,
//! `look_grade`, `haze_extinction`, `MIST_EV`), and the daylight illuminance its sun module
//! (`crates/visioncraft-core/src/sun.rs`: `daylight_lux`, `usno_atmos`, after USNO Circular
//! 171), ported to the inputs GameCraft has (no moon, no lamps, the "natural" look).

use gamecraft_math::yaw_forward;
use gamecraft_world::Environment;

/// Cloud cover above which the layer is a closed grey overcast.
pub(crate) const OVERCAST_COVER: f64 = 0.85;
/// Stops a photographer opens up in a thick mist (scaled by the mist, 0 to 1). From
/// VisionCraft's `studio.rs` (`MIST_EV`).
const MIST_EV: f64 = 1.0;
/// The night sky's light on open ground (lux) the renderer always adds: its night floor plus
/// starlight. From VisionCraft's `studio.rs` (`NIGHT_SKY_LUX`) and `sun.rs` (`STARLIGHT_LUX`).
const NIGHT_SKY_LUX: f64 = 0.0575 + 0.0005;
/// The height over which the haze's aerosol thins by e, metres (`studio.rs`).
pub(crate) const AEROSOL_HEIGHT_M: f64 = 1200.0;
/// The haze's turbidity: a clear day (Preetham et al. 1999), as VisionCraft's default.
const HAZE_TURBIDITY: f64 = 2.5;
/// How gusty the wind is (VisionCraft's `Wind::gustiness`, 0 to 1).
pub(crate) const WIND_GUSTINESS: f64 = 0.5;
/// Gust patches travelling downwind (VisionCraft's `Wind::gusts`, 0 to 1).
pub(crate) const WIND_GUSTS: f64 = 0.4;

/// USNO Circular 171's clear-sky factor for an apparent altitude in degrees: direct light
/// through the air mass of a curved atmosphere plus scattered light, which carries on below
/// the horizon as twilight. Ported from VisionCraft's `crates/visioncraft-core/src/sun.rs`.
fn usno_atmos(altitude_deg: f64) -> f64 {
    const X: f64 = 753.66156;
    let h = altitude_deg.to_radians();
    let u = h.sin();
    let s = (X * h.cos() / (X + 1.0)).asin();
    let m = X * (s.cos() - u) + s.cos();
    ((-0.21 * m).exp() * u
        + 0.0289 * (-0.042 * m).exp() * (1.0 + (altitude_deg + 90.0) * u / 57.29577951))
        .max(0.0)
}

/// Illuminance (lux) on open level ground from the sun and the clear sky, for the sun's
/// elevation in degrees: about 100 klx at 60 degrees, 760 lux at sunset, 3 lux at -6.
/// Ported from VisionCraft's `crates/visioncraft-core/src/sun.rs`.
pub(crate) fn daylight_lux(elevation_deg: f64) -> f64 {
    133_775.0 * usno_atmos(elevation_deg)
}

/// Automatic exposure (EV100, lower is brighter) for the sun's elevation: noon 15, 13.5 with
/// the sun at 10 degrees; lower, it follows the light on open ground as an incident meter
/// reads it (EV = log2(E / 2.5)), kept darker than the meter's mid grey the less light there
/// is (1 stop at sunset, 2 at the end of civil twilight, about 3.4 on a moonless night), never
/// brighter than EV -2.5. Ported from VisionCraft's `apps/visioncraft/src/studio.rs`
/// (`auto_ev`) without the moon and the garden lamps.
pub(crate) fn auto_ev(elevation_deg: f64) -> f64 {
    let e = elevation_deg;
    if !e.is_finite() {
        return 15.0;
    }
    if e >= 45.0 {
        return 15.0;
    }
    if e >= 10.0 {
        return 13.5 + 1.5 * (e - 10.0) / 35.0;
    }
    let lux = daylight_lux(e) + NIGHT_SKY_LUX;
    let metered = (lux / 2.5).log2();
    // Stops darker than the meter, by log10(lux).
    const DARKER: [(f64, f64); 6] = [
        (-1.5, 3.6),
        (-0.5, 2.6),
        (0.48, 2.0),
        (1.39, 1.5),
        (2.88, 1.0),
        (4.045, 1.39),
    ];
    let (first, last) = (DARKER.first(), DARKER.last());
    let (Some(&(l_min, _)), Some(&(l_max, d_max))) = (first, last) else {
        return metered.max(-2.5);
    };
    let l = lux.log10().clamp(l_min, l_max);
    let darker = DARKER
        .windows(2)
        .find_map(|w| match *w {
            [(l0, d0), (l1, d1)] if l <= l1 => Some(d0 + (d1 - d0) * (l - l0) / (l1 - l0)),
            _ => None,
        })
        .unwrap_or(d_max);
    (metered + darker).max(-2.5)
}

/// The "natural" look's daylight lift in stops (a camera meters an average scene to mid grey,
/// brighter than the sky's saturation-based exposure), fading out below 20 degrees of sun.
/// From VisionCraft's `studio.rs` (`look_grade`, `Look::Natural`).
fn natural_lift(elevation_deg: f64) -> f64 {
    1.2 * (elevation_deg / 20.0).clamp(0.0, 1.0)
}

/// The natural look's contrast power and saturation (`studio.rs`, `look_grade`).
pub(crate) const NATURAL_GRADE: (f32, f32) = (1.1, 1.2);

/// The exposure (EV100) for an environment: the automatic exposure for the sun's elevation,
/// the natural look's lift by day, opened up in mist.
pub(crate) fn exposure_ev(env: &Environment) -> f64 {
    let elevation = env.sun().elevation_deg;
    auto_ev(elevation) - natural_lift(elevation) - MIST_EV * env.mist.clamp(0.0, 1.0)
}

/// How far below the horizon the sun is, as 0 (day, above 1 degree) to 1 (6 degrees under):
/// the haze thins and the sun's glow goes as it sinks (`studio.rs`).
pub(crate) fn night(elevation_deg: f64) -> f64 {
    ((1.0 - elevation_deg) / 7.0).clamp(0.0, 1.0)
}

/// The haze's extinction at the ground per km on a clear day: the aerosol of turbidity 2.5
/// spread over its scale height plus Rayleigh's at the ground, a visibility of about 30 km.
/// From VisionCraft's `studio.rs` (`haze_extinction`), thinned at night as the studio does.
pub(crate) fn haze_per_km(elevation_deg: f64) -> f64 {
    let aerosol = (HAZE_TURBIDITY - 1.0) * 0.097 / (AEROSOL_HEIGHT_M / 1000.0);
    (aerosol + 0.0116) * (1.0 - 0.7 * night(elevation_deg))
}

/// The direction the wind blows TO as a horizontal unit vector, from the compass direction
/// it comes FROM (degrees clockwise from north): from the north (0) it blows towards +Z
/// (south), from the east (90) towards -X (west).
pub fn wind_towards(from_deg: f64) -> [f64; 3] {
    // A compass bearing b clockwise from north points along (sin b, 0, -cos b); the wind
    // blows the opposite way. `yaw_forward` of -b is that bearing's vector.
    let bearing = yaw_forward(-from_deg);
    [-bearing.x, 0.0, -bearing.z]
}

/// Whether a cloud cover (0 to 1) draws as a closed overcast layer.
pub(crate) fn overcast(cloud_cover: f64) -> bool {
    cloud_cover > OVERCAST_COVER
}
