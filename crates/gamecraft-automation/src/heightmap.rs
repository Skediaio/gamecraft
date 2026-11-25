//! The heightmap of a project's world on disk (not in the browser): the sample tiles under
//! `worlds/<world>/terrain/heightmap/` that `world.json` names by shape and content, read
//! when a project opens and written when the heightmap changes; and `terrain.import`, which
//! reads an elevation file into one (`docs/design/projects.md`).
//!
//! The tiles are a source, committed with the project (unlike the derived field's cache
//! beside them). A tile that is missing, cut short, a link out of the pack or of another
//! content than the document names is an error that says which; the project still opens,
//! without ground until the heightmap is imported again.

use crate::commands::{host_spec, strip_owner};
use crate::fields::Fields;
use crate::files::{Direction, mb, resolve, write_whole};
use crate::host::Host;
use crate::projects::writable;
use gamecraft_engine::{CommandSpec, ParamType as T, param};
use gamecraft_project::{
    ElevationFormat, HEIGHTMAP_DIR, MAX_ELEVATION_BYTES, TILE_BYTES, decode_tile, encode_tiles,
    read_elevation, tile_file_name,
};
use gamecraft_world::{Heightmap, Terrain, TerrainSource, World};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The folder of the heightmap's tiles in the pack folder `pack`, world `world`.
pub(crate) fn dir(pack: &Path, world: &str) -> PathBuf {
    pack.join("worlds")
        .join(world)
        .join("terrain")
        .join(HEIGHTMAP_DIR)
}

/// Reads the tiles the terrain names in the pack folder `pack` (resolved) for the world
/// `world`: every tile a plain file inside the pack, of the right size, and together what
/// the document names. An error, never a panic, for anything else.
pub(crate) fn read(pack: &Path, world: &str, terrain: &Terrain) -> Result<Heightmap, String> {
    let TerrainSource::Heightmap { cols, rows, .. } = &terrain.source else {
        return Err("the terrain's source is not a heightmap".into());
    };
    let folder = dir(pack, world);
    let (tx, tz) = Heightmap::tiles_of(*cols, *rows);
    let mut tiles = BTreeMap::new();
    for tz_i in 0..tz {
        for tx_i in 0..tx {
            let (px, pz) = (tx_i as i32, tz_i as i32);
            let path = folder.join(tile_file_name(px, pz));
            let missing = || format!("tile ({px}, {pz}) is missing from `{}`", folder.display());
            let Ok(real) = fs::canonicalize(&path) else {
                return Err(missing());
            };
            let Ok(meta) = fs::metadata(&real) else {
                return Err(missing());
            };
            if !real.starts_with(pack) || !meta.is_file() {
                return Err(missing());
            }
            if meta.len() != TILE_BYTES as u64 {
                return Err(format!(
                    "tile ({px}, {pz}) holds {} bytes, {TILE_BYTES} expected",
                    meta.len()
                ));
            }
            let bytes =
                fs::read(&real).map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
            let tile = decode_tile(&bytes).map_err(|e| format!("tile ({px}, {pz}): {e}"))?;
            tiles.insert(tile.place, tile.samples);
        }
    }
    let map = Heightmap::from_tiles(*cols, *rows, tiles)?;
    // What the document names, exactly: the terrain checks.
    let mut copy = terrain.clone();
    copy.set_heightmap(map.clone()).map_err(|e| e.to_string())?;
    Ok(map)
}

/// Writes the tiles of `map` that differ from the folder's and removes the folder's tile
/// files that are not its: (written, removed). The folder is made when it does not exist; a
/// link out of the pack is refused.
pub(crate) fn write(pack: &Path, world: &str, map: &Heightmap) -> Result<(usize, usize), String> {
    let folder = dir(pack, world);
    fs::create_dir_all(&folder)
        .map_err(|e| format!("cannot create the folder `{}`: {e}", folder.display()))?;
    let real = fs::canonicalize(&folder)
        .map_err(|e| format!("cannot use the folder `{}`: {e}", folder.display()))?;
    if !real.starts_with(pack) {
        return Err(format!(
            "`{}` is a link out of the pack; give a plain folder",
            folder.display()
        ));
    }
    let files = encode_tiles(map);
    let mut written = 0;
    for (&(tx, tz), bytes) in &files {
        let path = folder.join(tile_file_name(tx, tz));
        let same = fs::symlink_metadata(&path)
            .is_ok_and(|m| m.is_file() && m.len() as usize == bytes.len())
            && fs::read(&path).is_ok_and(|old| old == *bytes);
        if !same {
            write_whole(&path, bytes)?;
            written += 1;
        }
    }
    let mut removed = 0;
    let entries = fs::read_dir(&folder)
        .map_err(|e| format!("cannot read the folder `{}`: {e}", folder.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot list `{}`: {e}", folder.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name
            .strip_prefix("tile_")
            .and_then(|n| n.strip_suffix(".bin"))
        else {
            continue;
        };
        let Some((tx, tz)) = stem
            .split_once('_')
            .and_then(|(x, z)| Some((x.parse::<i32>().ok()?, z.parse::<i32>().ok()?)))
        else {
            continue;
        };
        if files.contains_key(&(tx, tz)) {
            continue;
        }
        if entry
            .file_type()
            .is_ok_and(|t| t.is_file() || t.is_symlink())
        {
            fs::remove_file(entry.path())
                .map_err(|e| format!("cannot remove `{}`: {e}", entry.path().display()))?;
            removed += 1;
        }
    }
    Ok((written, removed))
}

/// What opening or making a project does with the heightmap its world names. A project made
/// from the session (`prefer_session`) takes the session's own samples when they are the ones
/// the world names, and with `can_write` they are written to the folder; an opened project
/// reads the folder's, whatever the session holds, so that a problem on disk is said. `null` when the terrain's source is no heightmap, else
/// `{tiles, content, from, written, error}` for the `terrain` part of the answer.
pub(crate) fn load(
    session_document: &World,
    pack: &Path,
    world: &str,
    world_mut: &mut World,
    can_write: bool,
    prefer_session: bool,
) -> Value {
    let Some(terrain) = world_mut.terrain.as_mut() else {
        return Value::Null;
    };
    if terrain.heightmap().is_none() {
        return Value::Null;
    }
    let named_content = match &terrain.source {
        TerrainSource::Heightmap { content, .. } => content.clone(),
        _ => String::new(),
    };
    let own = if prefer_session {
        session_document
            .terrain
            .as_ref()
            .and_then(|t| t.heightmap())
            .filter(|m| !m.is_empty())
            .cloned()
    } else {
        None
    };
    let mut error = Value::Null;
    let mut written = 0;
    let mut from = "files";
    match own {
        Some(samples) if terrain.set_heightmap(samples.clone()).is_ok() => {
            from = "session";
            if can_write {
                let Some(samples) = terrain.heightmap() else {
                    return Value::Null;
                };
                match write(pack, world, samples) {
                    Ok((n, _)) => written = n,
                    Err(e) => error = json!(e),
                }
            }
        }
        _ => match read(pack, world, terrain) {
            Ok(samples) => {
                from = "files";
                let _ = terrain.set_heightmap(samples);
            }
            Err(e) => error = json!(e),
        },
    }
    let tiles = terrain.heightmap().map_or(0, |m| m.tiles().len());
    json!({
        "tiles": tiles,
        "content": named_content,
        "from": from,
        "written": written,
        "error": error,
    })
}

/// `terrain.import`'s spec.
pub(crate) fn spec() -> CommandSpec {
    host_spec(
        "terrain.import",
        "Import Terrain",
        "Replaces the terrain with real ground read from an elevation file inside the read \
         root: a heightmap of cols x rows samples at full resolution (1 to 4 m is usual), \
         named in the document by its shape and a hash of its content. The samples are too \
         many for a command: this command reads them, hands them to the engine and, with a \
         project open whose folder lies inside the write root, writes them to its \
         worlds/<world>/terrain/heightmap/ folder, so the project opens with them (without \
         one they live in the session only: save the world in a project to keep them). The \
         terrain's size, cell size, painted layers, paint rules and sculpt edits stay (or \
         `size_m` and `cell_m` set them); entities are not moved (entity.snap_to_ground puts \
         them on the new ground). A file that is not what it claims is refused and nothing \
         changes.",
        vec![
            param(
                "path",
                T::String,
                "The elevation file, inside the read root: a PGM (.pgm, P5: 8 or 16 bits a \
                 sample, the usual export of elevation data), rows of text (.txt: metres, a \
                 row of samples per line), 16-bit integers (.u16) or 32-bit floats (.f32), \
                 both little-endian.",
            )
            .required(),
            param(
                "format",
                T::String,
                "pgm, text, u16 or f32; left out, the file's extension says.",
            )
            .allowed(&["pgm", "text", "u16", "f32"]),
            param("cols", T::Integer, "u16 and f32 (the file does not say): samples along x, 2 to 2049."),
            param("rows", T::Integer, "u16 and f32: samples along z, 2 to 2049."),
            param(
                "scale_m",
                T::Number,
                "pgm and u16, whose numbers are raw: metres per unit.",
            )
            .unit("m")
            .default(json!(1)),
            param(
                "offset_m",
                T::Number,
                "pgm and u16: height_m = number * scale_m + offset_m.",
            )
            .unit("m")
            .default(json!(0)),
            param(
                "size_m",
                T::Number,
                "Side of the terrain, 8 to 16384; the terrain's own when there is one, else 256.",
            )
            .unit("m"),
            param(
                "cell_m",
                T::Number,
                "Distance between the height field's samples, 0.25 to 64; the terrain's own, else 2.",
            )
            .unit("m"),
        ],
        "{path, format, cols, rows, content, min_height_m, max_height_m, tiles, stored: \
         \"session\" | \"project\", tiles_written, tiles_removed}",
        json!({"path": "dem/valley.pgm", "scale_m": 0.1, "offset_m": 20.0}),
    )
}

/// `cols` or `rows` as a parameter gives it: a whole number, or an error that names it.
fn shape_param(p: &Fields, name: &str) -> Result<Option<u32>, String> {
    let Some(value) = p.number(name).map_err(strip_owner)? else {
        return Ok(None);
    };
    if !(value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= 4_294_967_295.0) {
        return Err(format!(
            "`{name}` is {value}; it must be a whole number of samples"
        ));
    }
    Ok(Some(value as u32))
}

impl Host {
    pub(crate) fn terrain_import(&mut self, p: &Fields) -> Result<Value, String> {
        let given = p.required_str("path").map_err(strip_owner)?;
        let format = match p.str("format").map_err(strip_owner)? {
            Some(name) => ElevationFormat::parse(name)
                .ok_or_else(|| format!("`format` is {name:?}; it must be pgm, text, u16 or f32"))?,
            None => ElevationFormat::by_extension(given).ok_or_else(|| {
                format!(
                    "`path` is {given:?}: give a file ending in .pgm, .txt, .u16 or .f32, or \
                     say `format`"
                )
            })?,
        };
        // With `format` the extension may be anything (a .bin of floats): the file's own.
        let extension = given
            .rsplit_once('.')
            .map(|(_, e)| e)
            .filter(|e| {
                !e.is_empty() && e.len() <= 16 && e.chars().all(|c| c.is_ascii_alphanumeric())
            })
            .ok_or_else(|| {
                format!(
                    "`path` is {given:?}: it names no file with an extension; give a file ending \
                     in .pgm, .txt, .u16 or .f32"
                )
            })?;
        let path = resolve(&self.access, given, Direction::Read, extension)?;
        let meta =
            fs::symlink_metadata(&path).map_err(|e| format!("cannot read `{given}`: {e}"))?;
        if !meta.file_type().is_file() {
            return Err(format!("`{given}` is not a plain file"));
        }
        let cap = self.access.max_world_bytes().min(MAX_ELEVATION_BYTES);
        if meta.len() > cap {
            return Err(format!(
                "`{given}` is {} MB, more than the {} MB this host reads in one file; raise the \
                 cap with --max-world-mb",
                mb(meta.len()),
                mb(cap)
            ));
        }
        let (scale_m, offset_m) = (
            p.number("scale_m").map_err(strip_owner)?.unwrap_or(1.0),
            p.number("offset_m").map_err(strip_owner)?.unwrap_or(0.0),
        );
        if !format.takes_scale() && (p.get("scale_m").is_some() || p.get("offset_m").is_some()) {
            return Err(format!(
                "`scale_m` and `offset_m` belong to the formats pgm and u16, whose numbers are \
                 raw; the format {} holds metres",
                format.name()
            ));
        }
        let cols = shape_param(p, "cols")?;
        let rows = shape_param(p, "rows")?;
        let bytes = fs::read(&path).map_err(|e| format!("cannot read `{given}`: {e}"))?;
        let map = read_elevation(format, &bytes, cols, rows, scale_m, offset_m)?;
        let (lo, hi) = map.height_range();
        let content = Heightmap::content_text(map.content_hash());
        // Name it in the document (an undo step, like terrain.set), then hand the samples over.
        let mut params = json!({
            "kind": "heightmap",
            "cols": map.cols(),
            "rows": map.rows(),
            "content": content,
        });
        if let Some(map_params) = params.as_object_mut() {
            for key in ["size_m", "cell_m"] {
                if let Some(value) = p.get(key) {
                    map_params.insert(key.into(), value.clone());
                }
            }
        }
        self.session
            .run("terrain.set", &params)
            .map_err(|e| e.to_string())?;
        self.session
            .set_heightmap(map.clone())
            .map_err(|e| e.to_string())?;
        // Kept in the open project when its folder may be written.
        let mut stored = "session";
        let (mut written, mut removed) = (0, 0);
        if let Some(open) = self.project.as_ref()
            && writable(&self.access, &open.dir)
        {
            (written, removed) = write(&open.dir, open.project.world(), &map)?;
            stored = "project";
        }
        Ok(json!({
            "path": given,
            "format": format.name(),
            "cols": map.cols(),
            "rows": map.rows(),
            "content": content,
            "min_height_m": lo,
            "max_height_m": hi,
            "tiles": map.tiles().len(),
            "stored": stored,
            "tiles_written": written,
            "tiles_removed": removed,
        }))
    }
}
