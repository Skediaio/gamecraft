//! What a bake left in a project, so that the next bake of the same script replaces it and
//! nothing else: `gamecraft-cli project bake SCRIPT --into DIR` runs a script of commands and
//! saves the result as a project of cells, and writes this record beside the world's
//! `world.json`. A later bake removes what the record names before it runs the script again:
//! the entities of the layers the script owns, and the terrain edits it added between two
//! places of the list. Everything else in the project (a layer edited by hand, a prefab, an
//! asset) stays as it is.
//!
//! Text, like every file of a project: stable and pretty, so it diffs and never holds a clock
//! reading.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The name of the record's file, in the folder of the world (`worlds/<world>/bake.json`).
pub const BAKE_FILE: &str = "bake.json";
/// The `gamecraft_bake` field of the record's file.
pub const BAKE_FORMAT: u64 = 1;

/// What a bake left in a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeRecord {
    /// The layers the script owns, sorted and without doubles: the project's default layer
    /// (where its commands add entities that name no layer) and every layer a step names.
    /// The next bake removes every entity of these layers before it runs.
    pub layers: Vec<String>,
    /// The places of the terrain edits the bake added, as `before..after` in the list of
    /// edits: the next bake removes the edits `before..after` when the list still ends at
    /// `after`, and refuses when something was sculpted since (a re-run would stack them).
    pub terrain_edits: [usize; 2],
}

impl BakeRecord {
    /// The record of a bake that added the terrain edits `before..after`, over the layers
    /// `layers` (sorted, without doubles).
    pub fn new(layers: Vec<String>, before: usize, after: usize) -> BakeRecord {
        let mut layers = layers;
        layers.sort();
        layers.dedup();
        BakeRecord {
            layers,
            terrain_edits: [before, after],
        }
    }

    /// The record's file in the folder of the world `world_dir` (`worlds/<world>`).
    pub fn path(world_dir: &Path) -> PathBuf {
        world_dir.join(BAKE_FILE)
    }

    /// The record as the text of its file: pretty JSON with a stable order.
    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(&json!({
            "gamecraft_bake": BAKE_FORMAT,
            "layers": self.layers,
            "terrain_edits": self.terrain_edits,
        }))
        .unwrap_or_default();
        text.push('\n');
        text
    }

    /// The record in the text of its file. An error, never a panic, for a file of another
    /// format or with a field that is not what it should be.
    pub fn parse(text: &[u8]) -> Result<BakeRecord, String> {
        let bad = |why: &str| format!("the bake record is not one: {why}");
        let value: Value =
            serde_json::from_slice(text).map_err(|e| bad(&format!("not JSON ({e})")))?;
        let Value::Object(map) = value else {
            return Err(bad("not an object"));
        };
        if map.get("gamecraft_bake").and_then(Value::as_u64) != Some(BAKE_FORMAT) {
            return Err(bad("`gamecraft_bake` is not 1"));
        }
        let mut layers = Vec::new();
        match map.get("layers") {
            Some(Value::Array(list)) => {
                for layer in list {
                    let Some(name) = layer.as_str().filter(|s| !s.is_empty()) else {
                        return Err(bad("`layers` holds something else than layer names"));
                    };
                    layers.push(name.to_string());
                }
            }
            _ => return Err(bad("`layers` is missing or not a list")),
        }
        let [before, after]: [u64; 2] = match map.get("terrain_edits") {
            Some(Value::Array(list)) if list.len() == 2 => {
                let first = list.first().and_then(Value::as_u64);
                let second = list.get(1).and_then(Value::as_u64);
                match (first, second) {
                    (Some(before), Some(after)) => [before, after],
                    _ => return Err(bad("`terrain_edits` holds something else than two places")),
                }
            }
            _ => return Err(bad("`terrain_edits` is missing or not a pair")),
        };
        let (Ok(before), Ok(after)) = (usize::try_from(before), usize::try_from(after)) else {
            return Err(bad("`terrain_edits` is too large for this platform"));
        };
        if after < before {
            return Err(bad("`terrain_edits` ends before it starts"));
        }
        Ok(BakeRecord {
            layers,
            terrain_edits: [before, after],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_round_trips_in_a_stable_text() {
        let record = BakeRecord::new(vec!["roads".into(), "main".into(), "main".into()], 0, 12);
        let text = record.to_text();
        assert_eq!(
            text,
            "{\n  \"gamecraft_bake\": 1,\n  \"layers\": [\n    \"main\",\n    \"roads\"\n  ],\n  \
             \"terrain_edits\": [\n    0,\n    12\n  ]\n}\n"
        );
        assert_eq!(BakeRecord::parse(text.as_bytes()).unwrap(), record);
        let empty = BakeRecord::new(Vec::new(), 3, 3);
        assert_eq!(
            BakeRecord::parse(empty.to_text().as_bytes()).unwrap(),
            empty
        );
        assert_eq!(BakeRecord::path(Path::new("w")), Path::new("w/bake.json"));
    }

    #[test]
    fn a_record_that_is_not_one_says_what_is_wrong() {
        for (text, why) in [
            ("{}", "`gamecraft_bake` is not 1"),
            ("[]", "not an object"),
            ("not json", "not JSON"),
            (
                "{\"gamecraft_bake\": 1}",
                "`layers` is missing or not a list",
            ),
            (
                "{\"gamecraft_bake\": 1, \"layers\": [1]}",
                "something else than layer names",
            ),
            (
                "{\"gamecraft_bake\": 1, \"layers\": [], \"terrain_edits\": [3]}",
                "not a pair",
            ),
            (
                "{\"gamecraft_bake\": 1, \"layers\": [], \"terrain_edits\": [4, 2]}",
                "ends before it starts",
            ),
        ] {
            let error = BakeRecord::parse(text.as_bytes()).err().unwrap_or_default();
            assert!(error.contains(why), "{text}: {error}");
        }
    }
}
