use crate::noise::{fbm2, value2};

#[test]
fn value2_is_pinned() {
    let got = [
        value2(0, 0.0, 0.0),
        value2(7, 0.5, 0.25),
        value2(7, -3.7, 12.9),
        value2(123, 1000.123, -2000.456),
    ];
    for (g, e) in got.iter().zip(GOLDEN_VALUE2) {
        assert_eq!(g.to_bits(), e.to_bits(), "{got:?}");
    }
}

#[test]
fn fbm2_is_pinned() {
    let got = [
        fbm2(7, 0.3, 0.7, 5, 2.0, 0.5),
        fbm2(7, -12.5, 3.25, 3, 2.0, 0.8),
    ];
    for (g, e) in got.iter().zip(GOLDEN_FBM2) {
        assert_eq!(g.to_bits(), e.to_bits(), "{got:?}");
    }
}

const GOLDEN_VALUE2: [f64; 4] = [
    0.317_487_817_584_199_75,
    0.022_194_507_372_939_015,
    -0.098_149_558_771_420_95,
    0.245_844_575_801_715_31,
];
const GOLDEN_FBM2: [f64; 2] = [-0.223_670_014_321_370_96, -0.062_601_990_886_937_49];

#[test]
fn value2_is_in_range_continuous_and_repeatable() {
    let mut x = -37.3;
    while x < 37.3 {
        let z = x * 0.731 + 5.0;
        let a = value2(11, x, z);
        assert!((-1.0..=1.0).contains(&a), "value2({x}, {z}) = {a}");
        let b = value2(11, x + 1e-6, z + 1e-6);
        assert!((a - b).abs() < 1e-4, "jump at {x}, {z}: {a} vs {b}");
        assert_eq!(a.to_bits(), value2(11, x, z).to_bits());
        x += 0.0173;
    }
}

#[test]
fn value2_matches_lattice_values_and_varies() {
    // Different seeds give different noise.
    let a: Vec<f64> = (0..50)
        .map(|i| value2(1, f64::from(i) * 0.37, 0.5))
        .collect();
    let b: Vec<f64> = (0..50)
        .map(|i| value2(2, f64::from(i) * 0.37, 0.5))
        .collect();
    assert_ne!(a, b);
    let spread = a.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
    assert!(spread > 0.3, "noise is too flat: {spread}");
}

#[test]
fn fbm2_is_in_range_continuous_and_repeatable() {
    for (octaves, gain) in [(1, 0.5), (5, 0.5), (8, 1.0), (10, 0.0), (4, 2.0)] {
        let mut x = -20.0;
        while x < 20.0 {
            let z = 3.0 - x * 0.41;
            let a = fbm2(5, x, z, octaves, 2.0, gain);
            assert!((-1.0..=1.0).contains(&a), "fbm2 = {a}");
            let b = fbm2(5, x + 1e-6, z, octaves, 2.0, gain);
            assert!((a - b).abs() < 1e-4, "jump at {x}: {a} vs {b}");
            assert_eq!(a.to_bits(), fbm2(5, x, z, octaves, 2.0, gain).to_bits());
            x += 0.0377;
        }
    }
}

#[test]
fn hostile_inputs_do_not_panic_and_stay_in_range() {
    let hostile = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1e300,
        -1e300,
        -12345.678,
        9.3e18,
        -9.3e18,
        f64::MAX,
        f64::MIN,
    ];
    for &x in &hostile {
        for &z in &hostile {
            let v = value2(u64::MAX, x, z);
            assert!((-1.0..=1.0).contains(&v), "value2({x}, {z}) = {v}");
            let f = fbm2(u64::MAX, x, z, 6, 2.0, 0.5);
            assert!((-1.0..=1.0).contains(&f), "fbm2({x}, {z}) = {f}");
        }
    }
    for (l, g) in [
        (f64::NAN, 0.5),
        (2.0, f64::NAN),
        (f64::INFINITY, 0.5),
        (2.0, -1.0),
    ] {
        let f = fbm2(1, 0.3, 0.4, 5, l, g);
        assert!(
            (-1.0..=1.0).contains(&f),
            "fbm2 lacunarity {l} gain {g} = {f}"
        );
    }
    // A hostile octave count is capped, not looped four billion times.
    let f = fbm2(1, 0.3, 0.4, u32::MAX, 2.0, 0.5);
    assert!((-1.0..=1.0).contains(&f));
    assert_eq!(fbm2(1, 0.3, 0.4, 0, 2.0, 0.5), 0.0);
}
