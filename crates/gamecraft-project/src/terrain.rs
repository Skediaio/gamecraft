//! `worlds/<world>/terrain/edits.json`: a world's terrain edits, one per line.
//!
//! The edits are recipes (`gamecraft_world::sculpt`), so they are text; they live beside
//! `world.json` rather than in it so that sculpting a terrain changes one small file, one line
//! per edit. `world.json` keeps the rest of the terrain recipe (size, cell, source, layers,
//! paint rules). The file of a later pack replaces the edits of earlier packs as a whole.
//! The resolved heights are a cache the host keeps in `terrain/cache/` (ignored by git).

use crate::ProjectError;
use gamecraft_world::TerrainEdit;
use serde_json::Value;

/// The version of the terrain edits file format (`gamecraft_terrain_edits`).
pub const TERRAIN_EDITS_FORMAT: u32 = 1;

/// The path of a world's terrain edits, relative to its pack.
pub(crate) fn edits_path(world: &str) -> String {
    format!("worlds/{world}/terrain/edits.json")
}

/// Reads a terrain edits file (`file` names it in errors). The numbers are checked with the
/// world (a paint edit needs the terrain's layers).
pub(crate) fn parse_edits(text: &str, file: &str) -> Result<Vec<TerrainEdit>, ProjectError> {
    let shape = "a terrain edits file is {\"gamecraft_terrain_edits\": 1, \"edits\": [...]}";
    let value: Value = serde_json::from_str(text)
        .map_err(|e| ProjectError::in_file(file, format!("not valid JSON: {e}; {shape}")))?;
    let Value::Object(map) = value else {
        return Err(ProjectError::in_file(file, shape));
    };
    if map.get("gamecraft_terrain_edits").and_then(Value::as_u64)
        != Some(u64::from(TERRAIN_EDITS_FORMAT))
    {
        return Err(ProjectError::in_file(
            file,
            format!(
                "`gamecraft_terrain_edits` must be {TERRAIN_EDITS_FORMAT}, the format this \
                 engine reads; {shape}"
            ),
        ));
    }
    if let Some(key) = map
        .keys()
        .find(|k| *k != "gamecraft_terrain_edits" && *k != "edits")
    {
        return Err(ProjectError::in_file(
            file,
            format!("unknown field `{key}`; {shape}"),
        ));
    }
    let Some(Value::Array(items)) = map.get("edits") else {
        return Err(ProjectError::in_file(
            file,
            format!("`edits` must be a list; {shape}"),
        ));
    };
    let max = gamecraft_world::limits::MAX_TERRAIN_EDITS;
    if items.len() > max {
        return Err(ProjectError::in_file(
            file,
            format!("{} edits; a terrain has at most {max}", items.len()),
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            serde_json::from_value(item.clone()).map_err(|e| {
                ProjectError::in_file(
                    file,
                    format!(
                        "edits[{i}]: {e}; the ops are {}",
                        gamecraft_world::sculpt::EDIT_OPS.join(", ")
                    ),
                )
            })
        })
        .collect()
}

/// The file's text: one edit per line, so a diff shows exactly what changed.
pub(crate) fn edits_text(edits: &[TerrainEdit]) -> Result<String, ProjectError> {
    let mut out = format!("{{\n  \"gamecraft_terrain_edits\": {TERRAIN_EDITS_FORMAT},\n");
    if edits.is_empty() {
        out.push_str("  \"edits\": []\n}\n");
        return Ok(out);
    }
    out.push_str("  \"edits\": [\n");
    for (i, e) in edits.iter().enumerate() {
        let line = serde_json::to_string(e)
            .map_err(|e| ProjectError::new(format!("cannot write JSON: {e}")))?;
        out.push_str("    ");
        out.push_str(&line);
        if i + 1 < edits.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    Ok(out)
}
