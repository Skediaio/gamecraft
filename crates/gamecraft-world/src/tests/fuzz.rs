//! `from_json` never panics, whatever it is given.

use super::{EXAMPLE_COMPACT, EXAMPLE_PRETTY};
use crate::World;
use gamecraft_math::Rng;

const TOKENS: [&str; 22] = [
    "{",
    "}",
    "[",
    "]",
    ",",
    ":",
    "\"",
    "null",
    "true",
    "-",
    "0",
    "1e999",
    "-1e999",
    "18446744073709551616",
    "-1",
    "\"type\"",
    "\"shape\"",
    "\"id\": 1",
    "NaN",
    "\\u0000",
    "1.7976931348623157e309",
    "\u{1F600}",
];

fn mutate(rng: &mut Rng, text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    let rounds = 1 + rng.below(6);
    for _ in 0..rounds {
        let len = bytes.len() as u64;
        let at = rng.below(len + 1) as usize;
        match rng.below(6) {
            0 if !bytes.is_empty() => {
                let i = at.min(bytes.len() - 1);
                bytes[i] = rng.next_u32() as u8;
            }
            1 => bytes.truncate(at),
            2 => {
                let token = TOKENS[rng.below(TOKENS.len() as u64) as usize];
                bytes.splice(at..at, token.bytes());
            }
            3 if !bytes.is_empty() => {
                let end = (at + 1 + rng.below(20) as usize).min(bytes.len());
                let start = at.min(end);
                bytes.drain(start..end);
            }
            4 => {
                // Duplicate a slice somewhere else.
                let a = rng.below(len + 1) as usize;
                let b = (a + rng.below(40) as usize).min(bytes.len());
                let piece: Vec<u8> = bytes[a.min(b)..b].to_vec();
                bytes.splice(at..at, piece);
            }
            _ => {
                // Replace a number-looking digit by another.
                if let Some(i) = bytes.iter().skip(at).position(u8::is_ascii_digit) {
                    bytes[at + i] = b'0' + rng.below(10) as u8;
                }
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn mutated_documents_never_panic() {
    let mut rng = Rng::new(0xF022);
    let mut accepted = 0;
    for i in 0..20_000 {
        let base = if i % 2 == 0 {
            EXAMPLE_PRETTY
        } else {
            EXAMPLE_COMPACT
        };
        let text = mutate(&mut rng, base);
        if let Ok(w) = World::from_json(&text) {
            accepted += 1;
            // Whatever is accepted is valid and round-trips.
            w.validate().unwrap();
            let again = World::from_json(&w.to_json().unwrap()).unwrap();
            assert_eq!(again, w);
        }
    }
    assert!(
        accepted > 0,
        "the mutations are too destructive to test anything"
    );
}

#[test]
fn truncations_never_panic() {
    for end in 0..EXAMPLE_PRETTY.len() {
        if let Some(prefix) = EXAMPLE_PRETTY.get(..end) {
            assert!(World::from_json(prefix).is_err());
        }
    }
}

#[test]
fn deep_nesting_and_wrong_types_never_panic() {
    let deep = "[".repeat(100_000);
    assert!(World::from_json(&deep).is_err());
    let deep_field = format!(r#"{{"gamecraft": 1, "name": {}}}"#, "[".repeat(100_000));
    assert!(World::from_json(&deep_field).is_err());
    let deep_obj = format!(r#"{{"x": {}}}"#, "{\"a\":".repeat(50_000));
    assert!(World::from_json(&deep_obj).is_err());
    for text in [
        "",
        "null",
        "[]",
        "1",
        "\"gamecraft\"",
        r#"{"gamecraft": 1, "entities": {}}"#,
        r#"{"gamecraft": 1, "entities": [1, 2]}"#,
        r#"{"gamecraft": 1, "entities": [{"id": -1}]}"#,
        r#"{"gamecraft": 1, "entities": [{"id": 1.5}]}"#,
        r#"{"gamecraft": 1, "entities": [{"id": 1, "position": [1, 2]}]}"#,
        r#"{"gamecraft": 1, "entities": [{"id": 1, "position": "here"}]}"#,
        r#"{"gamecraft": 1, "environment": {"day_of_year": 4294967296}}"#,
        r#"{"gamecraft": 1, "terrain": {"size_m": 1e308, "cell_m": 1e-308, "source": {"kind": "flat"}}}"#,
        r#"{"gamecraft": 1, "terrain": {"size_m": 16384, "cell_m": 0.25, "source": {"kind": "noise"}}}"#,
        r#"{"gamecraft": 1, "next_entity_id": 18446744073709551615, "entities": [{"id": 18446744073709551614}]}"#,
        r#"{"gamecraft": 18446744073709551616}"#,
        r#"{"gamecraft": 1, "gamecraft": 1}"#,
    ] {
        let _ = World::from_json(text);
    }
}

#[test]
fn the_last_id_cannot_be_exceeded() {
    let mut w = World::from_json(
        r#"{"gamecraft": 1, "next_entity_id": 18446744073709551615, "entities": [{"id": 18446744073709551614}]}"#,
    )
    .unwrap();
    let e = w.add_entity(crate::EntityDraft::default()).unwrap_err();
    assert_eq!(e, crate::WorldError::IdsExhausted);
}
