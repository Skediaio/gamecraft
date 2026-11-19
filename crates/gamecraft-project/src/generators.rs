//! `worlds/<world>/generators.json`: a world's generators (forests, scatters), one per line.
//!
//! A generator is a recipe (`gamecraft_world::Generator`), so it is text; what it places is
//! derived and never written. Packs layer generators by id: a later pack's entry with the id
//! of an earlier one replaces it in its place, a new id is added after the others, and
//! `{"id": "...", "removed": true}` removes the generator of an earlier pack. A save writes the
//! root pack's file as the difference from what the dependencies give.

use crate::ProjectError;
use gamecraft_world::Generator;
use serde_json::Value;

/// The version of the generators file format (`gamecraft_generators`).
pub const GENERATORS_FORMAT: u32 = 1;

/// One line of a generators file: an entry, or a removal of an earlier pack's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GeneratorEntry {
    /// An entry, added or replacing the one with its id.
    Entry(Generator),
    /// Removes the generator with this id.
    Removed(String),
}

impl GeneratorEntry {
    /// The id the line is about.
    pub(crate) fn id(&self) -> &str {
        match self {
            GeneratorEntry::Entry(g) => &g.id,
            GeneratorEntry::Removed(id) => id,
        }
    }
}

/// The path of a world's generators, relative to its pack.
pub(crate) fn generators_path(world: &str) -> String {
    format!("worlds/{world}/generators.json")
}

/// Reads a generators file (`file` names it in errors). Each entry is checked on its own
/// later, with the world (a prefab item needs the world's prefabs).
pub(crate) fn parse_generators(
    text: &str,
    file: &str,
) -> Result<Vec<GeneratorEntry>, ProjectError> {
    let shape = "a generators file is {\"gamecraft_generators\": 1, \"generators\": [...]}";
    let value: Value = serde_json::from_str(text)
        .map_err(|e| ProjectError::in_file(file, format!("not valid JSON: {e}; {shape}")))?;
    let Value::Object(map) = value else {
        return Err(ProjectError::in_file(file, shape));
    };
    if map.get("gamecraft_generators").and_then(Value::as_u64) != Some(u64::from(GENERATORS_FORMAT))
    {
        return Err(ProjectError::in_file(
            file,
            format!(
                "`gamecraft_generators` must be {GENERATORS_FORMAT}, the format this engine \
                 reads; {shape}"
            ),
        ));
    }
    if let Some(key) = map
        .keys()
        .find(|k| *k != "gamecraft_generators" && *k != "generators")
    {
        return Err(ProjectError::in_file(
            file,
            format!("unknown field `{key}`; {shape}"),
        ));
    }
    let Some(Value::Array(items)) = map.get("generators") else {
        return Err(ProjectError::in_file(
            file,
            format!("`generators` must be a list; {shape}"),
        ));
    };
    let max = gamecraft_world::generator::MAX_GENERATORS;
    if items.len() > max {
        return Err(ProjectError::in_file(
            file,
            format!("{} generators; a world has at most {max}", items.len()),
        ));
    }
    let mut out: Vec<GeneratorEntry> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let entry = entry_of(item)
            .map_err(|e| ProjectError::in_file(file, format!("generators[{i}]: {e}")))?;
        if let Some(earlier) = out.iter().position(|o| o.id() == entry.id()) {
            return Err(ProjectError::in_file(
                file,
                format!(
                    "generators[{i}] has the id `{}` of generators[{earlier}]; an id is used \
                     once in a file",
                    entry.id()
                ),
            ));
        }
        out.push(entry);
    }
    Ok(out)
}

/// One line: a removal is exactly `{"id", "removed": true}`.
fn entry_of(item: &Value) -> Result<GeneratorEntry, String> {
    if let Some(removed) = item.get("removed") {
        let id = item.get("id").and_then(Value::as_str);
        let only = item.as_object().is_some_and(|m| m.len() == 2);
        return match (removed, id, only) {
            (Value::Bool(true), Some(id), true) => Ok(GeneratorEntry::Removed(id.to_string())),
            _ => Err("a removal is exactly {\"id\": \"...\", \"removed\": true}".to_string()),
        };
    }
    let g: Generator = serde_json::from_value(item.clone()).map_err(|e| e.to_string())?;
    if !gamecraft_world::is_generator_id(&g.id) {
        return Err(format!(
            "the id {:?} is not a generator id: 1 to 64 ASCII letters, digits, `-` and `_`",
            g.id
        ));
    }
    Ok(GeneratorEntry::Entry(g))
}

/// The generators `layers` give, in order: each pack's lines applied over the earlier ones.
/// Returns them and, per id, the file that gave it last; removals of ids no earlier pack has
/// are warnings.
pub(crate) fn layered<'a>(
    layers: impl Iterator<Item = (&'a [GeneratorEntry], String)>,
    warnings: &mut Vec<String>,
) -> (Vec<Generator>, Vec<String>) {
    let mut out: Vec<Generator> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for (entries, file) in layers {
        for entry in entries {
            let at = out.iter().position(|g| g.id == entry.id());
            match (entry, at) {
                (GeneratorEntry::Entry(g), Some(i)) => {
                    if let (Some(slot), Some(f)) = (out.get_mut(i), files.get_mut(i)) {
                        *slot = g.clone();
                        f.clone_from(&file);
                    }
                }
                (GeneratorEntry::Entry(g), None) => {
                    out.push(g.clone());
                    files.push(file.clone());
                }
                (GeneratorEntry::Removed(_), Some(i)) => {
                    out.remove(i);
                    files.remove(i);
                }
                (GeneratorEntry::Removed(id), None) => warnings.push(format!(
                    "`{file}` removes the generator `{id}`, which no earlier pack has"
                )),
            }
        }
    }
    (out, files)
}

/// The lines that turn `base` (what the dependencies give) into `world`: every generator that
/// is new or differs, in the world's order, then a removal for each of `base` that is gone.
/// `None` when the lines would not give `world` back in its order (a reordering a file cannot
/// say): then every generator is written, and every one of `base` not in `world` removed.
pub(crate) fn difference(base: &[Generator], world: &[Generator]) -> Vec<GeneratorEntry> {
    let mut lines: Vec<GeneratorEntry> = world
        .iter()
        .filter(|g| !base.contains(g))
        .cloned()
        .map(GeneratorEntry::Entry)
        .collect();
    lines.extend(
        base.iter()
            .filter(|b| world.iter().all(|g| g.id != b.id))
            .map(|b| GeneratorEntry::Removed(b.id.clone())),
    );
    let mut ignored = Vec::new();
    let base_lines = as_entries(base);
    let (back, _) = layered(
        [
            (base_lines.as_slice(), String::new()),
            (lines.as_slice(), String::new()),
        ]
        .into_iter(),
        &mut ignored,
    );
    if back == world {
        return lines;
    }
    let mut all: Vec<GeneratorEntry> = world.iter().cloned().map(GeneratorEntry::Entry).collect();
    all.extend(
        base.iter()
            .filter(|b| world.iter().all(|g| g.id != b.id))
            .map(|b| GeneratorEntry::Removed(b.id.clone())),
    );
    all
}

fn as_entries(gs: &[Generator]) -> Vec<GeneratorEntry> {
    gs.iter().cloned().map(GeneratorEntry::Entry).collect()
}

/// The file's text: one generator per line, so a diff shows exactly what changed.
pub(crate) fn generators_text(entries: &[GeneratorEntry]) -> Result<String, ProjectError> {
    let mut out = format!("{{\n  \"gamecraft_generators\": {GENERATORS_FORMAT},\n");
    if entries.is_empty() {
        out.push_str("  \"generators\": []\n}\n");
        return Ok(out);
    }
    out.push_str("  \"generators\": [\n");
    for (i, e) in entries.iter().enumerate() {
        let line = match e {
            GeneratorEntry::Entry(g) => serde_json::to_string(g)
                .map_err(|e| ProjectError::new(format!("cannot write JSON: {e}")))?,
            GeneratorEntry::Removed(id) => {
                let id = serde_json::to_string(id)
                    .map_err(|e| ProjectError::new(format!("cannot write JSON: {e}")))?;
                format!("{{\"id\":{id},\"removed\":true}}")
            }
        };
        out.push_str("    ");
        out.push_str(&line);
        if i + 1 < entries.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    Ok(out)
}
