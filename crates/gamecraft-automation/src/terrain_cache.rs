//! The binary tile cache of a project's terrain: `worlds/<world>/terrain/cache/tile_<x>_<z>.bin`
//! in the root pack's folder (`gamecraft_world::tiles` has the format), with a `.gitignore`
//! that keeps it out of git.
//!
//! The cache is derived data: each tile carries a key of what it follows from, a tile whose
//! key does not match is built again, and deleting the folder loses nothing. Opening a project
//! loads the tiles that are current and builds the rest; a save (and an open) writes the tiles
//! whose key changed when the folder is inside the write root.

use crate::files::write_whole;
use gamecraft_world::tiles::{TILE_HEADER_BYTES, tile_file_name};
use gamecraft_world::{HeightField, Terrain};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The largest tile file read: 129 x 129 samples of a height and eight weights, and the header.
const MAX_TILE_BYTES: u64 = (TILE_HEADER_BYTES + 129 * 129 * (4 + 8)) as u64;

/// The tile keys a cache folder holds, as this host last wrote or checked them.
pub(crate) type Keys = BTreeMap<[usize; 2], u64>;

/// The cache folder of `world` in the pack folder `dir`.
pub(crate) fn cache_dir(dir: &Path, world: &str) -> PathBuf {
    dir.join("worlds").join(world).join("terrain").join("cache")
}

/// The tile files of `cache` for the tiles of `keys` that exist, are plain files inside `pack`
/// (no link out) and are not too large.
fn read_tiles(pack: &Path, cache: &Path, keys: &Keys) -> BTreeMap<[usize; 2], Vec<u8>> {
    let mut out = BTreeMap::new();
    for tile in keys.keys() {
        let path = cache.join(tile_file_name(*tile));
        let Ok(real) = fs::canonicalize(&path) else {
            continue;
        };
        let small_file =
            fs::metadata(&real).is_ok_and(|m| m.is_file() && m.len() <= MAX_TILE_BYTES);
        if !real.starts_with(pack) || !small_file {
            continue;
        }
        if let Ok(bytes) = fs::read(&real) {
            out.insert(*tile, bytes);
        }
    }
    out
}

/// Writes the tiles of `field` whose key in `keys` differs from `on_disk`, and the folder's
/// `.gitignore`. Returns how many tiles were written; `on_disk` becomes `keys`. A tile that
/// cannot be written is left out (it is rebuilt on the next open).
pub(crate) fn write_tiles(
    pack: &Path,
    cache: &Path,
    field: &HeightField,
    keys: &Keys,
    on_disk: &mut Keys,
) -> Result<usize, String> {
    let stale: Vec<([usize; 2], u64)> = keys
        .iter()
        .filter(|(tile, key)| on_disk.get(*tile) != Some(*key))
        .map(|(t, k)| (*t, *k))
        .collect();
    if stale.is_empty() {
        return Ok(0);
    }
    fs::create_dir_all(cache)
        .map_err(|e| format!("cannot create the folder `{}`: {e}", cache.display()))?;
    let real = fs::canonicalize(cache)
        .map_err(|e| format!("cannot use the folder `{}`: {e}", cache.display()))?;
    if !real.starts_with(pack) {
        return Err(format!(
            "`{}` is a link that leads out of the project; the terrain cache is not written",
            cache.display()
        ));
    }
    let ignore = real.join(".gitignore");
    if !ignore.exists() {
        write_whole(
            &ignore,
            b"# The terrain tile cache: derived data, rebuilt when missing or stale.\n*\n",
        )?;
    }
    let mut written = 0;
    for (tile, key) in stale {
        let Some(bytes) = field.tile_bytes(tile, key) else {
            continue;
        };
        let target = real.join(tile_file_name(tile));
        if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
            continue;
        }
        if write_whole(&target, &bytes).is_ok() {
            on_disk.insert(tile, key);
            written += 1;
        }
    }
    on_disk.retain(|tile, _| keys.contains_key(tile));
    Ok(written)
}

/// What opening a project's terrain gives: its height field, and the keys now on disk.
pub(crate) struct Loaded {
    pub field: HeightField,
    pub on_disk: Keys,
    /// `{tiles, from_cache, built, written, cache, ms}` for the answer of `project.open`.
    pub report: Value,
}

/// Builds `terrain` with the cache of `world` in the pack folder `pack`, and writes the tiles
/// it had to build when `writable`.
pub(crate) fn load(
    pack: &Path,
    world: &str,
    terrain: &Terrain,
    writable: bool,
) -> Result<Loaded, String> {
    let clock = crate::commands::Stopwatch::start();
    let cache = cache_dir(pack, world);
    let keys = terrain.tile_keys().map_err(|e| e.to_string())?;
    let files = read_tiles(pack, &cache, &keys);
    let built = terrain.build_cached(&files).map_err(|e| e.to_string())?;
    let mut on_disk: Keys = keys
        .iter()
        .filter(|(tile, _)| !built.rebuilt.contains(tile))
        .map(|(t, k)| (*t, *k))
        .collect();
    let mut problem = Value::Null;
    let written = if writable {
        write_tiles(pack, &cache, &built.field, &keys, &mut on_disk).unwrap_or_else(|e| {
            problem = json!(e);
            0
        })
    } else {
        0
    };
    let report = json!({
        "tiles": keys.len(),
        "from_cache": keys.len() - built.rebuilt.len(),
        "built": built.rebuilt.len(),
        "written": written,
        "cache": writable.then(|| cache.display().to_string()),
        "problem": problem,
        "ms": clock.ms(),
    });
    Ok(Loaded {
        field: built.field,
        on_disk,
        report,
    })
}
