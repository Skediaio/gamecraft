use crate::StateHasher;

fn fnv(bytes: &[u8]) -> u64 {
    let mut h = StateHasher::new();
    h.write_bytes(bytes);
    h.finish()
}

#[test]
fn raw_bytes_match_published_fnv1a_64_vectors() {
    // Test vectors of the FNV reference (Fowler, Noll, Vo; draft-eastlake-fnv).
    assert_eq!(fnv(b""), 0xCBF2_9CE4_8422_2325);
    assert_eq!(fnv(b"a"), 0xAF63_DC4C_8601_EC8C);
    assert_eq!(fnv(b"foobar"), 0x8594_4171_F739_67E8);
}

#[test]
fn empty_hasher_is_the_offset_basis() {
    assert_eq!(StateHasher::new().finish(), 0xCBF2_9CE4_8422_2325);
    assert_eq!(StateHasher::default().finish(), 0xCBF2_9CE4_8422_2325);
}

#[test]
fn string_is_length_prefixed() {
    let mut h = StateHasher::new();
    h.write_str("foobar");
    let mut manual = StateHasher::new();
    manual.write_bytes(&6_u64.to_le_bytes());
    manual.write_bytes(b"foobar");
    assert_eq!(h.finish(), manual.finish());
    assert_eq!(h.finish(), GOLDEN_FOOBAR);
    // "ab" + "c" and "a" + "bc" must differ.
    let mut x = StateHasher::new();
    x.write_str("ab");
    x.write_str("c");
    let mut y = StateHasher::new();
    y.write_str("a");
    y.write_str("bc");
    assert_ne!(x.finish(), y.finish());
}

const GOLDEN_FOOBAR: u64 = 12_859_785_306_565_319_154;
const GOLDEN_MIXED: u64 = 987_864_485_036_194_471;

#[test]
fn mixed_sequence_is_pinned() {
    let mut h = StateHasher::new();
    h.write_u8(7);
    h.write_u32(0xDEAD_BEEF);
    h.write_u64(u64::MAX);
    h.write_i64(-5);
    h.write_f64(1.5);
    h.write_f32(-0.25);
    h.write_bool(true);
    h.write_bool(false);
    h.write_str("Valley");
    assert_eq!(h.finish(), GOLDEN_MIXED);

    let mut manual = StateHasher::new();
    manual.write_bytes(&[7]);
    manual.write_bytes(&0xDEAD_BEEF_u32.to_le_bytes());
    manual.write_bytes(&u64::MAX.to_le_bytes());
    manual.write_bytes(&(-5_i64).to_le_bytes());
    manual.write_bytes(&1.5_f64.to_bits().to_le_bytes());
    manual.write_bytes(&(-0.25_f32).to_bits().to_le_bytes());
    manual.write_bytes(&[1, 0]);
    manual.write_bytes(&6_u64.to_le_bytes());
    manual.write_bytes(b"Valley");
    assert_eq!(h.finish(), manual.finish());
}

#[test]
fn floats_hash_their_bit_patterns() {
    let mut a = StateHasher::new();
    a.write_f64(0.0);
    let mut b = StateHasher::new();
    b.write_f64(-0.0);
    assert_ne!(a.finish(), b.finish());
}
