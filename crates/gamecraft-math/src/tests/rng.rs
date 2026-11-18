use crate::Rng;

#[test]
fn seed_zero_matches_published_splitmix64_outputs() {
    // Reference outputs of Vigna's splitmix64.c for seed 0.
    let mut r = Rng::new(0);
    assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
    assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    assert_eq!(r.next_u64(), 0x06C4_5D18_8009_454F);
}

#[test]
fn seed_1234567_matches_published_splitmix64_outputs() {
    // Rosetta Code, "Pseudo-random numbers/Splitmix64".
    let mut r = Rng::new(1_234_567);
    let expected = [
        6_457_827_717_110_365_317_u64,
        3_203_168_211_198_807_973,
        9_817_491_932_198_370_423,
        4_593_380_528_125_082_431,
        16_408_922_859_458_223_821,
    ];
    for e in expected {
        assert_eq!(r.next_u64(), e);
    }
}

#[test]
fn next_u32_is_the_high_half() {
    let mut r = Rng::new(0);
    assert_eq!(r.next_u32(), 0xE220_A839);
}

#[test]
fn fork_is_pinned_independent_and_does_not_advance_the_parent() {
    let parent = Rng::new(7);
    let mut a = parent.fork(1);
    let mut b = parent.fork(1);
    let mut c = parent.fork(2);
    let mut p = parent.clone();
    assert_eq!(parent, Rng::new(7), "fork must not advance the parent");
    let a1 = a.next_u64();
    assert_eq!(a1, b.next_u64(), "the same label gives the same stream");
    assert_ne!(a1, c.next_u64(), "another label gives another stream");
    assert_ne!(a1, p.next_u64(), "a fork differs from its parent");
    // Pinned golden value of the fork.
    assert_eq!(a1, FORK_7_1_FIRST);
    // A fork of a fork is a stream too.
    let mut aa = parent.fork(1).fork(1);
    assert_ne!(aa.next_u64(), a1);
}

const FORK_7_1_FIRST: u64 = 10_849_586_449_113_067_221;

#[test]
fn unit_stays_in_the_half_open_unit_interval() {
    let mut r = Rng::new(42);
    let mut sum = 0.0;
    for _ in 0..100_000 {
        let u = r.unit();
        assert!((0.0..1.0).contains(&u), "{u}");
        sum += u;
    }
    let mean = sum / 100_000.0;
    assert!((mean - 0.5).abs() < 0.01, "mean {mean}");
}

#[test]
fn range_and_below() {
    let mut r = Rng::new(3);
    for _ in 0..10_000 {
        let x = r.range(-2.0, 5.0);
        assert!((-2.0..=5.0).contains(&x), "{x}");
        let n = r.below(6);
        assert!(n < 6);
    }
    assert_eq!(r.below(0), 0);
    assert_eq!(r.below(1), 0);
    let mut counts = [0_u32; 3];
    for _ in 0..30_000 {
        counts[r.below(3) as usize] += 1;
    }
    for c in counts {
        assert!((9_000..11_000).contains(&c), "{counts:?}");
    }
    // Huge bounds do not hang.
    for _ in 0..1000 {
        let _ = r.below(u64::MAX);
        let _ = r.below((1 << 63) + 1);
    }
}

#[test]
fn hostile_range_does_not_panic() {
    let mut r = Rng::new(9);
    let _ = r.range(f64::NAN, 1.0);
    let _ = r.range(f64::NEG_INFINITY, f64::INFINITY);
    let _ = r.range(5.0, -5.0);
}
