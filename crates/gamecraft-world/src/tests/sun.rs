use crate::Environment;

fn env(latitude_deg: f64, day_of_year: u32, time_of_day_h: f64) -> Environment {
    Environment {
        latitude_deg,
        day_of_year,
        time_of_day_h,
        ..Environment::default()
    }
}

fn unit(d: [f64; 3]) -> bool {
    ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - 1.0).abs() < 1e-12
}

#[test]
fn equator_at_equinox_noon_is_overhead() {
    // Day 81: (284 + 81) / 365 is a whole turn, so the declination is 0.
    let s = env(0.0, 81, 12.0).sun();
    assert!((s.elevation_deg - 90.0).abs() < 0.5, "{s:?}");
    assert!(s.direction[1] > 0.9999, "{s:?}");
}

#[test]
fn brussels_at_midsummer_noon_is_high_in_the_south() {
    let s = env(50.85, 172, 12.0).sun();
    assert!((s.elevation_deg - 62.6).abs() < 0.1, "{s:?}");
    assert!((s.azimuth_deg - 180.0).abs() < 1e-6, "{s:?}");
    assert!(s.direction[2] > 0.0, "south is +Z: {s:?}");
    assert!(s.direction[0].abs() < 1e-9, "{s:?}");
    assert!(unit(s.direction));
}

#[test]
fn midnight_is_below_the_horizon() {
    let s = env(50.85, 172, 0.0).sun();
    assert!(s.elevation_deg < 0.0, "{s:?}");
    assert!(s.direction[1] < 0.0);
    assert!(unit(s.direction));
}

#[test]
fn morning_sun_is_east_of_south_and_afternoon_west() {
    let m = env(50.85, 172, 9.0).sun();
    assert!(m.azimuth_deg > 0.0 && m.azimuth_deg < 180.0, "{m:?}");
    assert!(m.direction[0] > 0.0, "east is +X: {m:?}");
    let a = env(50.85, 172, 15.0).sun();
    assert!(a.azimuth_deg > 180.0 && a.azimuth_deg < 360.0, "{a:?}");
    assert!(a.direction[0] < 0.0);
    // Symmetric about noon.
    assert!((m.elevation_deg - a.elevation_deg).abs() < 1e-9);
    assert!((m.azimuth_deg + a.azimuth_deg - 360.0).abs() < 1e-9);
}

#[test]
fn direction_is_always_unit_and_matches_the_angles() {
    for lat in [-90.0, -45.0, 0.0, 23.0, 50.85, 89.9, 90.0] {
        for day in [1, 81, 172, 266, 355, 366] {
            for i in 0..48 {
                let s = env(lat, day, f64::from(i) * 0.5).sun();
                assert!(unit(s.direction), "{s:?}");
                assert!((0.0..360.0).contains(&s.azimuth_deg), "{s:?}");
                assert!((-90.0..=90.0).contains(&s.elevation_deg), "{s:?}");
                let up =
                    gamecraft_math::Portable::psin(gamecraft_math::deg_to_rad(s.elevation_deg));
                assert!((up - s.direction[1]).abs() < 1e-9, "{s:?}");
            }
        }
    }
}

#[test]
fn hostile_environment_does_not_panic() {
    let s = env(f64::NAN, 0, f64::INFINITY).sun();
    assert!(unit(s.direction), "{s:?}");
}
