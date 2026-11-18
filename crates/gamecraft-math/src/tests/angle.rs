use crate::{deg_to_rad, rad_to_deg, wrap_deg, wrap_deg_signed, yaw_forward};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-12
}

#[test]
fn degrees_and_radians() {
    assert!(close(deg_to_rad(180.0), core::f64::consts::PI));
    assert!(close(rad_to_deg(core::f64::consts::FRAC_PI_2), 90.0));
    assert!(close(rad_to_deg(deg_to_rad(37.5)), 37.5));
}

#[test]
fn wrap_into_ranges() {
    let cases = [
        (0.0, 0.0, 0.0),
        (360.0, 0.0, 0.0),
        (-90.0, 270.0, -90.0),
        (180.0, 180.0, 180.0),
        (-180.0, 180.0, 180.0),
        (540.0, 180.0, 180.0),
        (725.0, 5.0, 5.0),
        (-1e-20, 0.0, 0.0),
        (190.0, 190.0, -170.0),
    ];
    for (a, w, s) in cases {
        assert!(close(wrap_deg(a), w), "wrap_deg({a}) = {}", wrap_deg(a));
        assert!(
            close(wrap_deg_signed(a), s),
            "wrap_deg_signed({a}) = {}",
            wrap_deg_signed(a)
        );
    }
    let mut a = -2000.0;
    while a < 2000.0 {
        let w = wrap_deg(a);
        assert!((0.0..360.0).contains(&w), "{a} -> {w}");
        let s = wrap_deg_signed(a);
        assert!(s > -180.0 && s <= 180.0, "{a} -> {s}");
        a += 0.37;
    }
}

#[test]
fn hostile_angles_do_not_panic() {
    for a in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1e300,
        -1e300,
        f64::MAX,
    ] {
        let w = wrap_deg(a);
        assert!((0.0..360.0).contains(&w), "{a} -> {w}");
        let s = wrap_deg_signed(a);
        assert!(s > -180.0 && s <= 180.0, "{a} -> {s}");
        let _ = yaw_forward(a);
        let _ = deg_to_rad(a);
    }
}

#[test]
fn yaw_forward_follows_the_convention() {
    // Yaw 0 faces north (-Z); +90 turns counter-clockwise seen from above, to the west (-X).
    let f0 = yaw_forward(0.0);
    assert!(
        close(f0.x, 0.0) && close(f0.y, 0.0) && close(f0.z, -1.0),
        "{f0}"
    );
    let f90 = yaw_forward(90.0);
    assert!(close(f90.x, -1.0) && close(f90.z, 0.0), "{f90}");
    let f180 = yaw_forward(180.0);
    assert!(close(f180.z, 1.0), "{f180}");
    let f270 = yaw_forward(-90.0);
    assert!(close(f270.x, 1.0), "{f270}");
    assert!(close(yaw_forward(33.0).length(), 1.0));
}
