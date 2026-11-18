//! The environment: time of day, date, place on Earth and weather; and the sun they give.

use crate::WorldError;
use crate::error::Ctx;
use crate::limits::{
    DAY_OF_YEAR_MAX, DAY_OF_YEAR_MIN, LATITUDE_MAX_DEG, SEA_LEVEL_MAX_M, SEA_LEVEL_MIN_M,
    TEMPERATURE_MAX_C, TEMPERATURE_MIN_C, TIME_OF_DAY_MAX_H, WIND_SPEED_MAX_MPS,
};
use gamecraft_math::{Portable, deg_to_rad, rad_to_deg, wrap_deg};
use serde::{Deserialize, Serialize};

/// Time, place and weather of a world. Every field has a default (see [`Default`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Environment {
    /// Local solar time in hours, [0, 24); 12 is the sun's highest point. Default 14.
    pub time_of_day_h: f64,
    /// Day of the year, 1 to 366. Default 172 (around the June solstice).
    pub day_of_year: u32,
    /// Latitude in degrees, -90 (south pole) to 90 (north pole). Default 50.85.
    pub latitude_deg: f64,
    /// Compass direction the wind comes from, degrees clockwise from north (0 = from the
    /// north, 90 = from the east). Default 250.
    pub wind_direction_deg: f64,
    /// Wind speed, metres per second, 0 to 100. Default 3.
    pub wind_speed_mps: f64,
    /// Fraction of the sky covered by cloud, 0 to 1. Default 0.3.
    pub cloud_cover: f64,
    /// Mist density, 0 to 1. Default 0.
    pub mist: f64,
    /// Rain intensity, 0 to 1. Default 0.
    pub rain: f64,
    /// Air temperature at world y = 0, degrees Celsius, -90 to 60; it falls by 6.5 degrees per
    /// kilometre upwards. With `sea_level_m` it gives the density of the air projectiles fly
    /// through. Default 15 (the standard atmosphere). Written only when not the default.
    #[serde(skip_serializing_if = "is_default_temperature")]
    pub temperature_c: f64,
    /// Height of world y = 0 above sea level, metres, -500 to 11000. Default 0. Written only
    /// when not the default.
    #[serde(skip_serializing_if = "is_default_sea_level")]
    pub sea_level_m: f64,
}

/// The temperature of the standard atmosphere at sea level, degrees Celsius.
pub const STANDARD_TEMPERATURE_C: f64 = 15.0;

// Bit equality: only the exact default is left out, so every other value reads back the same.
fn is_default_temperature(v: &f64) -> bool {
    v.to_bits() == STANDARD_TEMPERATURE_C.to_bits()
}

fn is_default_sea_level(v: &f64) -> bool {
    v.to_bits() == 0f64.to_bits()
}

impl Default for Environment {
    fn default() -> Self {
        Environment {
            time_of_day_h: 14.0,
            day_of_year: 172,
            latitude_deg: 50.85,
            wind_direction_deg: 250.0,
            wind_speed_mps: 3.0,
            cloud_cover: 0.3,
            mist: 0.0,
            rain: 0.0,
            temperature_c: STANDARD_TEMPERATURE_C,
            sea_level_m: 0.0,
        }
    }
}

/// Where the sun stands in the sky.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunPosition {
    /// Compass direction of the sun, degrees clockwise from north, [0, 360).
    pub azimuth_deg: f64,
    /// Angle above the horizon in degrees; negative at night.
    pub elevation_deg: f64,
    /// Unit vector from the ground towards the sun in world axes (east +X, up +Y, north -Z).
    pub direction: [f64; 3],
}

/// Maximum declination of the sun (the obliquity of the ecliptic), degrees.
const OBLIQUITY_DEG: f64 = 23.44;

/// Day offset of Cooper's declination formula: `(284 + N) / 365` of a turn.
const DECLINATION_DAY_OFFSET: f64 = 284.0;

/// Days in the year of the declination formula.
const DAYS_PER_YEAR: f64 = 365.0;

/// Degrees the sun's hour angle turns per hour.
const DEG_PER_HOUR: f64 = 15.0;

impl Environment {
    /// The sun's position from the date, the local solar time and the latitude.
    ///
    /// Standard solar geometry (Cooper's declination, the hour angle and the spherical
    /// triangle of the observer, the pole and the sun):
    ///
    /// - declination `dec = 23.44 deg * sin(2 pi (284 + day_of_year) / 365)`
    /// - hour angle `h = 15 deg * (time_of_day_h - 12)`, so noon is the highest point
    /// - `sin(elevation) = sin(lat) sin(dec) + cos(lat) cos(dec) cos(h)`
    /// - the east, north and up components of the unit vector to the sun are
    ///   `-cos(dec) sin(h)`, `cos(lat) sin(dec) - sin(lat) cos(dec) cos(h)` and
    ///   `sin(elevation)`; the azimuth is `atan2(east, north)`, clockwise from north.
    ///
    /// In world axes the direction is `[east, up, -north]`. All trigonometry goes through
    /// [`Portable`], so the result is the same on every platform. Out-of-range input never
    /// panics; non-finite input gives the zenith `[0, 1, 0]` as direction.
    pub fn sun(&self) -> SunPosition {
        let day = f64::from(self.day_of_year);
        let turn = 2.0 * core::f64::consts::PI * (DECLINATION_DAY_OFFSET + day) / DAYS_PER_YEAR;
        let declination = deg_to_rad(OBLIQUITY_DEG * turn.psin());
        let hour_angle = deg_to_rad(DEG_PER_HOUR * (self.time_of_day_h - 12.0));
        let (sin_lat, cos_lat) = deg_to_rad(self.latitude_deg).psin_cos();
        let (sin_dec, cos_dec) = declination.psin_cos();
        let (sin_h, cos_h) = hour_angle.psin_cos();

        let east = -cos_dec * sin_h;
        let north = cos_lat * sin_dec - sin_lat * cos_dec * cos_h;
        let up = sin_lat * sin_dec + cos_lat * cos_dec * cos_h;

        let len = (east * east + north * north + up * up).sqrt();
        let direction = if len.is_finite() && len > 0.0 {
            [east / len, up / len, -north / len]
        } else {
            [0.0, 1.0, 0.0]
        };
        let elevation_deg = rad_to_deg(direction[1].clamp(-1.0, 1.0).pasin());
        let azimuth_deg = wrap_deg(rad_to_deg(direction[0].patan2(-direction[2])));
        SunPosition {
            azimuth_deg,
            elevation_deg,
            direction,
        }
    }

    /// Checks every field: finite numbers within the ranges of [`crate::limits`].
    pub(crate) fn validate(&self) -> Result<(), WorldError> {
        let ctx = Ctx::WORLD;
        let t = self.time_of_day_h;
        if !(t.is_finite() && (0.0..TIME_OF_DAY_MAX_H).contains(&t)) {
            return Err(ctx.invalid(
                "environment.time_of_day_h",
                t,
                format!("it must be at least 0 and below {TIME_OF_DAY_MAX_H} hours"),
            ));
        }
        let d = self.day_of_year;
        if !(DAY_OF_YEAR_MIN..=DAY_OF_YEAR_MAX).contains(&d) {
            return Err(ctx.invalid(
                "environment.day_of_year",
                d,
                format!("it must be from {DAY_OF_YEAR_MIN} to {DAY_OF_YEAR_MAX}"),
            ));
        }
        ctx.range(
            "environment.latitude_deg",
            self.latitude_deg,
            -LATITUDE_MAX_DEG,
            LATITUDE_MAX_DEG,
            " degrees",
        )?;
        ctx.finite("environment.wind_direction_deg", self.wind_direction_deg)?;
        ctx.range(
            "environment.wind_speed_mps",
            self.wind_speed_mps,
            0.0,
            WIND_SPEED_MAX_MPS,
            " metres per second",
        )?;
        ctx.range("environment.cloud_cover", self.cloud_cover, 0.0, 1.0, "")?;
        ctx.range("environment.mist", self.mist, 0.0, 1.0, "")?;
        ctx.range("environment.rain", self.rain, 0.0, 1.0, "")?;
        ctx.range(
            "environment.temperature_c",
            self.temperature_c,
            TEMPERATURE_MIN_C,
            TEMPERATURE_MAX_C,
            " degrees Celsius",
        )?;
        ctx.range(
            "environment.sea_level_m",
            self.sea_level_m,
            SEA_LEVEL_MIN_M,
            SEA_LEVEL_MAX_M,
            " metres",
        )?;
        Ok(())
    }
}
