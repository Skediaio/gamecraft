//! The cooked form of a world: the same world back, checked against its source, never a panic.

use super::instance::lots;
use crate::cooked::source_hash;
use crate::{EntityId, World};

#[test]
fn a_cooked_world_reads_back_exactly_and_is_smaller() {
    let mut w = lots(300);
    w.entity_mut(EntityId(3)).unwrap().name = "edited".into();
    w.remove_entity(EntityId(10)).unwrap();
    let text = w.to_json().unwrap();
    let hash = source_hash(text.as_bytes());
    let bytes = w.to_cooked(hash).unwrap();
    let back = World::from_cooked(&bytes, hash).unwrap();
    assert_eq!(back, w);
    assert_eq!(back.to_json().unwrap(), text);
    // The same world cooks to the same bytes.
    assert_eq!(w.to_cooked(hash).unwrap(), bytes);
    println!("text {} bytes, cooked {} bytes", text.len(), bytes.len());
    assert!(bytes.len() < text.len());
}

#[test]
fn a_cooked_world_of_another_source_or_damaged_is_an_error() {
    let w = lots(5);
    let text = w.to_json().unwrap();
    let hash = source_hash(text.as_bytes());
    let bytes = w.to_cooked(hash).unwrap();
    let err = World::from_cooked(&bytes, hash ^ 1)
        .unwrap_err()
        .to_string();
    assert!(err.contains("another source"), "{err}");
    assert_ne!(source_hash(b"a"), source_hash(b"b"));
    for cut in (0..bytes.len()).step_by(7) {
        assert!(
            World::from_cooked(&bytes[..cut], hash).is_err(),
            "cut at {cut}"
        );
    }
    // Every single flipped byte: an error or (rarely) a valid world, never a panic.
    for i in 0..bytes.len() {
        let mut bad = bytes.clone();
        bad[i] ^= 0x5a;
        let _ = World::from_cooked(&bad, hash);
    }
    assert!(World::from_cooked(b"", hash).is_err());
    assert!(World::from_cooked(&[0xff; 64], hash).is_err());
}
