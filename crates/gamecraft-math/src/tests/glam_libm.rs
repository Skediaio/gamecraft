//! glam must compute its sines and cosines with libm (the `libm` feature), or the simulation's
//! rotations would differ between platforms.

use crate::{DQuat, DVec2};

fn angles() -> impl Iterator<Item = f64> {
    (0..10_000).map(|i| f64::from(i) * 0.012_345_678_9 - 61.7)
}

#[test]
fn dvec2_from_angle_uses_libm() {
    for a in angles() {
        let v = DVec2::from_angle(a);
        assert_eq!(v.x.to_bits(), libm::cos(a).to_bits(), "cos({a})");
        assert_eq!(v.y.to_bits(), libm::sin(a).to_bits(), "sin({a})");
    }
}

#[test]
fn dquat_from_rotation_y_uses_libm_half_angles() {
    for a in angles() {
        let q = DQuat::from_rotation_y(a);
        let half = a * 0.5;
        assert_eq!(q.x.to_bits(), 0.0_f64.to_bits());
        assert_eq!(q.y.to_bits(), libm::sin(half).to_bits(), "sin({half})");
        assert_eq!(q.z.to_bits(), 0.0_f64.to_bits());
        assert_eq!(q.w.to_bits(), libm::cos(half).to_bits(), "cos({half})");
    }
}
