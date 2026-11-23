//! The paths of a pack: which file is which, and the names that may appear in them.

use crate::ProjectError;
use crate::cells::Cell;

/// Most characters in a pack id, a world name or a layer name.
pub const MAX_PLAIN_NAME_CHARS: usize = 64;
/// Most characters in a prefab id that is saved to a file of its own.
pub const MAX_PREFAB_FILE_CHARS: usize = 200;
/// Most characters in the string id of an entity in a layer file.
pub const MAX_ENTITY_KEY_CHARS: usize = 200;

/// Whether `name` is 1 to `max` ASCII letters, digits, `-` and `_`.
pub(crate) fn is_plain(name: &str, max: usize) -> bool {
    !name.is_empty()
        && name.len() <= max
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Whether `name` can be a pack id or a world name (a folder): 1 to
/// [`MAX_PLAIN_NAME_CHARS`] ASCII letters, digits, `-` and `_`.
pub fn is_plain_name(name: &str) -> bool {
    is_plain(name, MAX_PLAIN_NAME_CHARS)
}

/// Whether a prefab id can be the name of its file (`prefabs/<id>.json`): 1 to
/// [`MAX_PREFAB_FILE_CHARS`] ASCII letters, digits, `-`, `_` and `.`, not starting with `.`.
pub fn is_prefab_file_id(id: &str) -> bool {
    !id.starts_with('.')
        && !id.contains("..")
        && !id.is_empty()
        && id.len() <= MAX_PREFAB_FILE_CHARS
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// Whether `key` can be the string id of an entity in a layer file: 1 to
/// [`MAX_ENTITY_KEY_CHARS`] ASCII letters, digits, `-`, `_`, `.`, `/` and `:`, without `..`.
pub fn is_entity_key(key: &str) -> bool {
    !key.is_empty()
        && !key.contains("..")
        && key.len() <= MAX_ENTITY_KEY_CHARS
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/' | b':'))
}

/// What a file of a pack is, by its path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FileKind {
    /// `pack.json`.
    Manifest,
    /// `prefabs/<id>.json`.
    Prefab(String),
    /// `worlds/<world>/world.json`.
    World(String),
    /// `worlds/<world>/layers/<layer>.json`.
    Layer(String, String),
    /// `worlds/<world>/layers/<layer>/<cx>_<cz>.json`: one cell of a layer split into cells.
    LayerCell(String, String, Cell),
    /// `worlds/<world>/terrain/edits.json`.
    TerrainEdits(String),
    /// Anything else (assets, notes): not read.
    Other,
}

/// What the file at `path` (relative to its pack, `/` between parts) is. A path that could
/// lead out of the pack (`..`, `.`, an empty part, a leading `/`, a `\`) is refused, and so
/// is a world, layer or prefab file whose name is not a plain name.
pub(crate) fn classify(path: &str, location: &str) -> Result<FileKind, ProjectError> {
    let shown = format!("{location}/{path}");
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(ProjectError::in_file(
            &shown,
            "the path leads out of its pack or is not a plain relative path; every file of a \
             pack lies inside its folder, without `..`, `.` or empty parts",
        ));
    }
    let parts: Vec<&str> = path.split('/').collect();
    let stem = |file: &str| file.strip_suffix(".json").map(str::to_string);
    let name_rule = || {
        format!(
            "a world or layer name is 1 to {MAX_PLAIN_NAME_CHARS} ASCII letters, digits, `-` \
             and `_`"
        )
    };
    Ok(match parts.as_slice() {
        ["pack.json"] => FileKind::Manifest,
        ["prefabs", file] => match stem(file) {
            Some(id) if is_prefab_file_id(&id) => FileKind::Prefab(id),
            Some(_) => {
                return Err(ProjectError::in_file(
                    &shown,
                    "a prefab file is named after its prefab's id: ASCII letters, digits, `-`, \
                     `_` and `.`",
                ));
            }
            None => FileKind::Other,
        },
        ["worlds", world, "world.json"] => {
            if !is_plain_name(world) {
                return Err(ProjectError::in_file(&shown, name_rule()));
            }
            FileKind::World((*world).to_string())
        }
        ["worlds", world, "terrain", "edits.json"] => {
            if !is_plain_name(world) {
                return Err(ProjectError::in_file(&shown, name_rule()));
            }
            FileKind::TerrainEdits((*world).to_string())
        }
        ["worlds", world, "layers", file] => match stem(file) {
            Some(layer) => {
                if !is_plain_name(world) || !is_plain_name(&layer) {
                    return Err(ProjectError::in_file(&shown, name_rule()));
                }
                FileKind::Layer((*world).to_string(), layer)
            }
            None => FileKind::Other,
        },
        ["worlds", world, "layers", layer, file] => match stem(file) {
            Some(stem) => {
                if !is_plain_name(world) || !is_plain_name(layer) {
                    return Err(ProjectError::in_file(&shown, name_rule()));
                }
                let cell = Cell::parse(&stem).ok_or_else(|| {
                    ProjectError::in_file(
                        &shown,
                        "a cell file of a layer is named after its cell, `<cx>_<cz>.json` with \
                         whole numbers as written plainly (`3_-2.json`, not `+3_02.json`)",
                    )
                })?;
                FileKind::LayerCell((*world).to_string(), (*layer).to_string(), cell)
            }
            None => FileKind::Other,
        },
        _ => FileKind::Other,
    })
}

/// The path of a world's `world.json`, relative to its pack.
pub(crate) fn world_path(world: &str) -> String {
    format!("worlds/{world}/world.json")
}

/// The path of a layer file, relative to its pack.
pub(crate) fn layer_path(world: &str, layer: &str) -> String {
    format!("worlds/{world}/layers/{layer}.json")
}

/// The path of a prefab file, relative to its pack.
pub(crate) fn prefab_path(id: &str) -> String {
    format!("prefabs/{id}.json")
}
