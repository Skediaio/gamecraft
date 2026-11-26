//! Projects on disk (not in the browser): `project.new`, `project.open`, `project.save`,
//! `project.export`, `project.info`, `pack.add_dependency` and `project.query`
//! (`docs/design/projects.md`).
//!
//! The host reads a pack's files into a map of path to text and hands them to
//! `gamecraft-project`, which layers them into one world; saving goes the other way. Every
//! file read lies inside its pack's folder (a link out is refused) and every pack inside the
//! read root; every file written lies inside the project's folder, inside the write root.
//!
//! A **world file** finds its pack too (`file.open`): the nearest `pack.json` above the file,
//! the pack the command names, or the host's own; only that chain's assets are read then, not
//! its worlds ([`open_pack_assets`], [`find_pack_above`]).

use crate::commands::{host_spec, strip_owner};
use crate::fields::Fields;
use crate::files::{Direction, RAISE_CAP, mb, resolve, write_whole};
use crate::host::{Access, Host, ProjectIndex};
use gamecraft_engine::{CommandSpec, ParamType as T, param};
use gamecraft_project::{Cell, MAX_PACKS, Manifest, PackSource, Project, ProjectAsset, Saving};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Most bytes all the files of one project may hold together, unless the host's world-file
/// cap ([`Access::max_world_bytes`]) is higher.
const MAX_PROJECT_BYTES: u64 = 512 * 1024 * 1024;
/// Most files read from one pack.
const MAX_PACK_FILES: usize = 100_000;
/// Rows `project.query` returns when `limit` is left out, and at most.
pub(crate) const MAX_QUERY_ROWS: u64 = 1000;

/// A project the host has open.
pub(crate) struct OpenProject {
    pub project: Project,
    /// The root pack's folder (resolved).
    pub dir: PathBuf,
    pub index: Option<Box<dyn ProjectIndex>>,
    /// The view revision the index was built at.
    pub indexed: Option<u64>,
    /// The terrain tiles the cache folder holds, by key, as last written or checked.
    pub terrain_keys: crate::terrain_cache::Keys,
}

/// The text of a `cell_size_m` parameter.
const CELL_SIZE_DOC: &str = "Save the world's layers in cells of this side (16 to 16384 m; 512 \
    is usual for a map of several km), one file per cell (layers/<layer>/<cx>_<cz>.json), or 0 \
    for one file per layer. Left out: as the project's world.json says.";

/// `cell_size_m` of `p`: `None` left out, `Some(None)` for 0 (one file per layer).
fn cell_size_param(p: &Fields) -> Result<Option<Option<f64>>, String> {
    match p.get("cell_size_m") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let n = v.as_f64().ok_or(
                "`cell_size_m` must be a number: the side of a cell in metres (512 is usual), \
                 or 0 for one file per layer",
            )?;
            if n == 0.0 {
                return Ok(Some(None));
            }
            gamecraft_project::cells::check_cell_size(n, "`cell_size_m`")
                .map_err(|e| e.to_string())?;
            Ok(Some(Some(n)))
        }
    }
}

/// `cells` of `p`: a list of `[cx, cz]` whole numbers.
fn cells_param(p: &Fields) -> Result<Option<BTreeSet<Cell>>, String> {
    let Some(value) = p.get("cells").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let list = value
        .as_array()
        .ok_or("`cells` must be a list of cells, [[cx, cz], ...]")?;
    if list.len() > gamecraft_project::cells::MAX_AREA_CELLS {
        return Err(format!(
            "`cells` lists {} cells; give at most {}",
            list.len(),
            gamecraft_project::cells::MAX_AREA_CELLS
        ));
    }
    let mut out = BTreeSet::new();
    for (i, item) in list.iter().enumerate() {
        let pair = item.as_array().filter(|a| a.len() == 2);
        let index = |k: usize| {
            pair.and_then(|a| a.get(k))
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
        };
        let (Some(x), Some(z)) = (index(0), index(1)) else {
            return Err(format!(
                "`cells[{i}]` must be [cx, cz], two whole numbers (project.cells lists them)"
            ));
        };
        out.insert(Cell { x, z });
    }
    Ok(Some(out))
}

/// A point `[x, z]` of `p`, `None` when left out.
fn point2(p: &Fields, name: &str) -> Result<Option<[f64; 2]>, String> {
    let Some(value) = p.get(name).filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    match value.as_array().map(Vec::as_slice) {
        Some([x, z]) => match (x.as_f64(), z.as_f64()) {
            (Some(x), Some(z)) if x.is_finite() && z.is_finite() => Ok(Some([x, z])),
            _ => Err(format!("`{name}` must be [x, z], two numbers in metres")),
        },
        _ => Err(format!("`{name}` must be [x, z], two numbers in metres")),
    }
}

/// The specs of this module.
pub(crate) fn project_specs() -> Vec<CommandSpec> {
    vec![
        host_spec(
            "project.new",
            "New Project",
            "Makes a new project folder inside the write root from the current document: a pack \
             `id` with pack.json, the world as `worlds/<world>/` (world.json and one file per \
             layer, entities without a layer in `main`) and one file per prefab. The folder must \
             not exist yet or be empty. The project is then open, with nothing to save.",
            vec![
                param(
                    "path",
                    T::String,
                    "The new project folder, inside the write root.",
                )
                .required(),
                param(
                    "id",
                    T::String,
                    "The pack's id: 1 to 64 ASCII letters, digits, `-` and `_`.",
                )
                .required(),
                param("name", T::String, "The pack's name for people.").required(),
                param(
                    "world",
                    T::String,
                    "The world's folder name (a plain name).",
                )
                .default(json!("main")),
                param("cell_size_m", T::Number, CELL_SIZE_DOC).unit("m"),
            ],
            "{path, id, world, files: [paths written], index, terrain}",
            json!({"path": "valley", "id": "valley", "name": "Green valley"}),
        ),
        host_spec(
            "project.open",
            "Open Project",
            "Opens the project folder `path` (with a pack.json) inside the read root and its \
             dependencies (a dependency's `path` relative to the pack that names it, else the \
             project's packs/<id>), layers the packs into one world and makes it the document, \
             with a new history; a running simulation stops. Remembers which file every \
             entity, prefab and world field came from, and builds the index. The terrain is \
             built from the root pack's tile cache (worlds/<world>/terrain/cache, ignored by \
             git): current tiles are read, missing or stale ones built (and written when the \
             folder is inside the write root). A pack that is missing, a cycle, a file that is \
             not valid or a link out of a pack is an error, and the session stays as it was.",
            vec![
                param(
                    "path",
                    T::String,
                    "The project folder, inside the read root.",
                )
                .required(),
                param(
                    "world",
                    T::String,
                    "The world to open; the root pack's first world when left out.",
                ),
                param(
                    "cells",
                    T::Array,
                    "Load only these cells of layers split into cells, [[cx, cz], ...] \
                     (project.cells finds the cells of an area); layers that are one file load \
                     whole. Such a project cannot be saved. Left out: everything.",
                ),
            ],
            "{path, root, world, worlds, packs: [ids in load order], entities, cell_size_m, \
             cells: null | [[cx, cz]], warnings, index, terrain: null | {tiles, from_cache, \
             built, written, cache, problem, ms, heightmap: null | {tiles, content, from, \
             written, error}}}",
            json!({"path": "skirmish-night-mod"}),
        ),
        host_spec(
            "project.save",
            "Save Project",
            "Writes what changed in the document (never the running world) since the project \
             was opened or saved, each change to the file it belongs to in the root pack: one \
             line of a layer file for an edited entity, a prefab file, world.json as a patch \
             over the dependencies, terrain/edits.json for the terrain's edits. A change to \
             something a dependency brought is written to the root pack as an override or a \
             removal, never to the dependency. Nothing changed: nothing is written. The terrain \
             tiles whose recipe changed are written to the tile cache. Needs the project folder \
             inside the write root. With `cell_size_m` the layers are from now on saved in \
             cells of that size (layers/<layer>/<cx>_<cz>.json, each entity in the cell of the \
             entity at the top of its parents), or with 0 in one file per layer; the old files \
             are removed. Refused for a project opened with only some `cells`.",
            vec![param("cell_size_m", T::Number, CELL_SIZE_DOC).unit("m")],
            "{path, written: [paths], removed: [paths], unchanged, terrain_tiles_written}",
            json!({}),
        ),
        host_spec(
            "project.cells",
            "Cells of an Area",
            "The cells that intersect an area, for loading a large world near a place \
             (project.open with `cells`): the area from `min` to `max`, or the square around \
             `center` reaching `radius_m`. Cell (cx, cz) covers cx * size <= x < (cx + 1) * size \
             and the same for z. The cell size is the open project's, or `cell_size_m`.",
            vec![
                param("min", T::Point2, "One corner of the area, [x, z] metres.").unit("m"),
                param("max", T::Point2, "The other corner, [x, z] metres.").unit("m"),
                param(
                    "center",
                    T::Point2,
                    "The middle of the area, [x, z] metres.",
                )
                .unit("m"),
                param(
                    "radius_m",
                    T::Number,
                    "How far the area reaches from `center`.",
                )
                .unit("m"),
                param(
                    "cell_size_m",
                    T::Number,
                    "The cell size, 16 to 16384; the open project's when left out.",
                )
                .unit("m"),
            ],
            "{cell_size_m, count, cells: [[cx, cz]]}",
            json!({"center": [0, 0], "radius_m": 600, "cell_size_m": 512}),
        ),
        host_spec(
            "project.export",
            "Export Project",
            "Writes the open project's world (the document, never the running world) as one \
             .gamecraft file inside the write root, entities with its layers. The file refers to \
             its models by their paths inside a pack's assets/ folder: `pack` is the pack that \
             has them (open the file with --pack DIR, or keep it inside that pack, to draw \
             them), `needs` the files it needs of a pack and `missing` the models no pack has. \
             Replacing an existing file needs `overwrite`.",
            vec![
                param(
                    "path",
                    T::String,
                    "A .gamecraft file inside the write root.",
                )
                .required(),
                param("overwrite", T::Boolean, "Replace an existing file.").default(json!(false)),
            ],
            "{path, bytes, entities, pack: {path, id, packs}, needs: [files inside the pack's \
             folder], missing: [assets no pack has]}",
            json!({"path": "skirmish.gamecraft"}),
        ),
        host_spec(
            "project.info",
            "Project Info",
            "Describes the open project: its packs in load order, its worlds, the layers of the \
             current world with their entity counts, the prefabs with the pack each came from, \
             and what changed since it was opened or saved (the files project.save would \
             write).",
            vec![],
            "{path, root, world, worlds, default_layer, packs: [{id, name, version, location, \
             depends, worlds, root}], layers: [{name, entities, default}], prefabs: [{id, pack, \
             instances}], entities, changes: {entities_added, entities_changed, \
             entities_removed, prefabs_changed, world_fields_changed, files, removes}, \
             warnings, index}",
            json!({}),
        ),
        host_spec(
            "pack.add_dependency",
            "Add Dependency",
            "Makes the open project's root pack depend on the pack `id` in the folder `path` \
             (relative to the root pack's folder), writes its pack.json and opens the project \
             again with it layered below. Refused while the document has unsaved changes \
             (project.save first) and when the pack is missing, has another id or makes a \
             cycle; then pack.json stays as it was.",
            vec![
                param("id", T::String, "The pack's id, as its pack.json gives it.").required(),
                param(
                    "path",
                    T::String,
                    "The pack's folder, relative to the root pack's folder (`../skirmish`).",
                )
                .required(),
            ],
            "{id, path, packs: [ids in load order], entities}",
            json!({"id": "skirmish", "path": "../skirmish/project"}),
        ),
    ]
}

/// The folder `given` inside the root of `direction`: `..` is refused, a relative path is
/// inside the root, and the existing folder (or, when `must_exist` is false and it does not
/// exist yet, its parent) is resolved with its links and must lie inside the root.
pub(crate) fn resolve_dir(
    access: &Access,
    given: &str,
    direction: Direction,
    must_exist: bool,
) -> Result<PathBuf, String> {
    let (root, verb, flag) = match direction {
        Direction::Read => (&access.read_root, "read", "--automation-read-root"),
        Direction::Write => (&access.write_root, "write", "--automation-write-root"),
    };
    let Some(root) = root else {
        return Err(format!(
            "this server may not {verb} files: no {verb} root is set; start it with {flag} DIR"
        ));
    };
    if given.trim().is_empty() {
        return Err(format!(
            "`path` is empty; give a folder inside the {verb} root"
        ));
    }
    let path = Path::new(given);
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!(
            "`{given}`: `..` is not allowed in a path; give a folder inside the {verb} root"
        ));
    }
    let real_root = fs::canonicalize(root)
        .map_err(|e| format!("the {verb} root `{}` cannot be used: {e}", root.display()))?;
    let joined = if path.is_relative() {
        real_root.join(path)
    } else {
        path.to_path_buf()
    };
    let outside = || {
        format!(
            "`{given}` is outside the {verb} root `{}`; give a folder inside it",
            root.display()
        )
    };
    match fs::canonicalize(&joined) {
        Ok(real) => {
            if !real.starts_with(&real_root) {
                return Err(outside());
            }
            if !real.is_dir() {
                return Err(format!("`{given}` is a file, not a project folder"));
            }
            Ok(real)
        }
        Err(_) if must_exist => Err(format!(
            "`{given}`: the folder does not exist inside the {verb} root"
        )),
        Err(_) => {
            let (Some(name), Some(parent)) = (joined.file_name(), joined.parent()) else {
                return Err(format!("`{given}` names no folder"));
            };
            let real_parent = fs::canonicalize(parent).map_err(|_| {
                format!(
                    "`{given}`: the folder `{}` does not exist; create it first",
                    parent.display()
                )
            })?;
            if !real_parent.starts_with(&real_root) {
                return Err(outside());
            }
            Ok(real_parent.join(name))
        }
    }
}

/// Whether `dir` lies inside the write root.
pub(crate) fn writable(access: &Access, dir: &Path) -> bool {
    access
        .write_root
        .as_ref()
        .and_then(|r| fs::canonicalize(r).ok())
        .is_some_and(|root| dir.starts_with(root))
}

/// Reads the files of the pack in `dir` (resolved, inside `real_root`). Only `pack.json`,
/// `prefabs/*.json`, `worlds/*/world.json`, `worlds/*/layers/*.json`,
/// `worlds/*/terrain/edits.json` and (for the licence gate) the asset records, the `.gltf`
/// documents and the JSON chunks of the `.glb` models are read (the terrain's tile cache is
/// read when the world is built); a file or folder that is a link leading out of the pack is
/// refused. Without `worlds` the prefabs and worlds are left out: a world file inside the
/// pack needs its assets only.
fn read_pack(dir: &Path, budget: &mut u64, cap: u64, worlds: bool) -> Result<PackSource, String> {
    let mut source = PackSource {
        location: dir.display().to_string(),
        files: BTreeMap::new(),
        assets: BTreeMap::new(),
    };
    let read = |relative: &str, source: &mut PackSource, budget: &mut u64| -> Result<(), String> {
        let path = dir.join(relative);
        let shown = path.display().to_string();
        let real = fs::canonicalize(&path).map_err(|e| format!("cannot read `{shown}`: {e}"))?;
        if !real.starts_with(dir) {
            return Err(format!(
                "`{shown}` is a link that leads out of its pack `{}`; a pack's files lie inside \
                 its folder",
                dir.display()
            ));
        }
        let meta = fs::metadata(&real).map_err(|e| format!("cannot read `{shown}`: {e}"))?;
        if !meta.is_file() {
            return Err(format!("`{shown}` is not a file"));
        }
        if meta.len() > cap || meta.len() > *budget {
            return Err(format!(
                "`{shown}` is too large: this host reads pack files of at most {} MB, and a \
                 project of at most {} MB together ({RAISE_CAP})",
                mb(cap),
                mb(cap.max(MAX_PROJECT_BYTES))
            ));
        }
        *budget -= meta.len();
        if source.files.len() >= MAX_PACK_FILES {
            return Err(format!(
                "the pack `{}` has more than {MAX_PACK_FILES} files",
                dir.display()
            ));
        }
        let bytes = fs::read(&real).map_err(|e| format!("cannot read `{shown}`: {e}"))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("`{shown}` is not UTF-8 text, so it is no pack file"))?;
        source.files.insert(relative.to_string(), text);
        Ok(())
    };
    if !dir.join("pack.json").exists() {
        return Err(format!(
            "`{}` is not a pack: it has no pack.json ({{\"gamecraft_pack\": 1, \"id\": ..., \
             \"name\": ...}})",
            dir.display()
        ));
    }
    read("pack.json", &mut source, budget)?;
    if worlds {
        read_worlds(dir, &mut source, budget, &read)?;
    }
    let mut listed = Vec::new();
    list_assets(dir, "assets", 0, &mut listed)?;
    for (relative, size) in listed {
        if source.assets.len() + source.files.len() >= MAX_PACK_FILES {
            return Err(format!(
                "the pack `{}` has more than {MAX_PACK_FILES} files",
                dir.display()
            ));
        }
        // The licence records and the .gltf documents are read for the licence gate; of a
        // .glb only its JSON chunk (the whole file when a renderer or the measure needs it).
        if relative.ends_with(gamecraft_project::RECORD_SUFFIX) || relative.ends_with(".gltf") {
            read(&relative, &mut source, budget)?;
        } else if relative.ends_with(".glb") {
            let json = glb_json(&dir.join(&relative), cap.min(*budget))?;
            *budget -= json.len() as u64;
            source.files.insert(relative.clone(), json);
        }
        source.assets.insert(relative, size);
    }
    Ok(source)
}

/// The prefabs and worlds of the pack in `dir`: `prefabs/*.json`, `worlds/*/world.json`, each
/// layer file (a layer split in cells too) and `worlds/*/terrain/edits.json`.
fn read_worlds(
    dir: &Path,
    source: &mut PackSource,
    budget: &mut u64,
    read: &dyn Fn(&str, &mut PackSource, &mut u64) -> Result<(), String>,
) -> Result<(), String> {
    for name in json_files(&dir.join("prefabs"))? {
        read(&format!("prefabs/{name}"), source, budget)?;
    }
    for world in sub_folders(&dir.join("worlds"))? {
        let base = format!("worlds/{world}");
        if dir.join(&base).join("world.json").exists() {
            read(&format!("{base}/world.json"), source, budget)?;
        }
        for name in json_files(&dir.join(&base).join("layers"))? {
            read(&format!("{base}/layers/{name}"), source, budget)?;
        }
        // A layer split into cells: layers/<layer>/<cx>_<cz>.json.
        for layer in sub_folders(&dir.join(&base).join("layers"))? {
            for name in json_files(&dir.join(&base).join("layers").join(&layer))? {
                read(&format!("{base}/layers/{layer}/{name}"), source, budget)?;
            }
        }
        if dir.join(&base).join("terrain").join("edits.json").exists() {
            read(&format!("{base}/terrain/edits.json"), source, budget)?;
        }
    }
    Ok(())
}

/// The JSON chunk of the GLB file `path`, at most `cap` bytes, for the licence gate: only the
/// chunk is read, not the whole model.
fn glb_json(path: &Path, cap: u64) -> Result<String, String> {
    use std::io::Read;
    let shown = path.display().to_string();
    let mut file = fs::File::open(path).map_err(|e| format!("cannot read `{shown}`: {e}"))?;
    let len = fs::metadata(path)
        .map_err(|e| format!("cannot read `{shown}`: {e}"))?
        .len();
    let mut head = [0u8; 20];
    file.read_exact(&mut head)
        .map_err(|e| format!("`{shown}` is cut short: {e}"))?;
    let json_len = gamecraft_project::model::glb_json_len(&head, len)
        .map_err(|e| format!("`{shown}` is not a GLB model: {e}"))?;
    if json_len as u64 > cap {
        return Err(format!(
            "`{shown}` has a JSON chunk of {} MB; this host reads pack files of at most {} MB",
            mb(json_len as u64),
            mb(cap)
        ));
    }
    let mut json = vec![0u8; json_len];
    file.read_exact(&mut json)
        .map_err(|e| format!("`{shown}` is cut short: {e}"))?;
    String::from_utf8(json)
        .map_err(|_| format!("`{shown}` has a JSON chunk that is not UTF-8 text"))
}

/// Deepest folder below `assets/` that is listed.
const MAX_ASSET_DEPTH: usize = 16;

/// Every file under the pack-relative folder `relative` of the pack in `dir` with its size,
/// sorted; a file or folder that is a link out of the pack is refused.
fn list_assets(
    dir: &Path,
    relative: &str,
    depth: usize,
    out: &mut Vec<(String, u64)>,
) -> Result<(), String> {
    let folder = dir.join(relative);
    if depth > MAX_ASSET_DEPTH {
        return Err(format!(
            "`{}` lies more than {MAX_ASSET_DEPTH} folders deep in assets/",
            folder.display()
        ));
    }
    for (name, _) in entries(&folder)? {
        if name.starts_with('.') {
            continue;
        }
        let path = folder.join(&name);
        let shown = path.display().to_string();
        let real = fs::canonicalize(&path).map_err(|e| format!("cannot read `{shown}`: {e}"))?;
        if !real.starts_with(dir) {
            return Err(format!(
                "`{shown}` is a link that leads out of its pack `{}`; a pack's files lie inside \
                 its folder",
                dir.display()
            ));
        }
        let child = format!("{relative}/{name}");
        let meta = fs::metadata(&real).map_err(|e| format!("cannot read `{shown}`: {e}"))?;
        if meta.is_dir() {
            list_assets(dir, &child, depth + 1, out)?;
        } else if meta.is_file() {
            out.push((child, meta.len()));
            if out.len() > MAX_PACK_FILES {
                return Err(format!(
                    "the pack `{}` has more than {MAX_PACK_FILES} files",
                    dir.display()
                ));
            }
        }
    }
    Ok(())
}

/// The names of the entries of `dir` (none when it does not exist), sorted.
fn entries(dir: &Path) -> Result<Vec<(String, bool)>, String> {
    let Ok(list) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in list {
        let entry = entry.map_err(|e| format!("cannot list `{}`: {e}", dir.display()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(format!(
                "`{}` holds a file name that is not UTF-8; pack files have plain names",
                dir.display()
            ));
        };
        let is_dir = entry.path().is_dir();
        out.push((name.to_string(), is_dir));
    }
    out.sort();
    Ok(out)
}

fn json_files(dir: &Path) -> Result<Vec<String>, String> {
    Ok(entries(dir)?
        .into_iter()
        .filter(|(n, d)| !d && n.ends_with(".json") && !n.starts_with('.'))
        .map(|(n, _)| n)
        .collect())
}

fn sub_folders(dir: &Path) -> Result<Vec<String>, String> {
    Ok(entries(dir)?
        .into_iter()
        .filter(|(n, d)| *d && !n.starts_with('.'))
        .map(|(n, _)| n)
        .collect())
}

/// Most folders [`find_pack_above`] climbs: deeper than any path it is given.
const MAX_PACK_CLIMB: usize = 255;

/// The nearest pack above the folder `from`: the first ancestor (itself included) that holds
/// a `pack.json`, resolved with its links; the climb stops at `up_to` (inclusive) and at the
/// root of the file system. As most engines find the project a scene belongs to.
pub fn find_pack_above(from: &Path, up_to: Option<&Path>) -> Option<PathBuf> {
    let mut at = Some(from);
    let mut climbs = 0usize;
    while let Some(dir) = at {
        if dir.join("pack.json").is_file()
            && let Ok(real) = fs::canonicalize(dir)
            && real.is_dir()
        {
            return Some(real);
        }
        if up_to.is_some_and(|top| dir == top) || climbs >= MAX_PACK_CLIMB {
            return None;
        }
        climbs += 1;
        at = dir.parent();
    }
    None
}

/// The deepest folder that holds both `a` and `b`: their common ancestor (either when one
/// holds the other, `/` when they share nothing).
pub fn common_ancestor(a: &Path, b: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for (x, y) in a.components().zip(b.components()) {
        if x != y {
            break;
        }
        out.push(x.as_os_str());
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}

/// The assets of the pack in `dir` (resolved, inside the read root) and of every pack it
/// depends on: the host's catalog, the models measured, as for an open project. Only the
/// packs' manifests and assets are read. Returns the catalog, the root pack's id and the ids
/// of the packs in load order.
pub(crate) fn open_pack_assets(
    access: &Access,
    dir: &Path,
) -> Result<(crate::assets::Catalog, String, Vec<String>), String> {
    let (root, packs) = read_packs(access, dir, false)?;
    let assets: BTreeMap<String, ProjectAsset> =
        gamecraft_project::pack_assets(&root, &packs).map_err(|e| e.to_string())?;
    let ids = packs.keys().cloned().collect();
    Ok((crate::assets::catalog_of(&assets), root, ids))
}

/// Reads the root pack in `dir` and every pack it depends on, directly or not; without
/// `worlds`, the manifests and assets only.
fn read_packs(
    access: &Access,
    dir: &Path,
    worlds: bool,
) -> Result<(String, BTreeMap<String, PackSource>), String> {
    let real_root = access
        .read_root
        .as_ref()
        .and_then(|r| fs::canonicalize(r).ok())
        .ok_or("this server may not read files: no read root is set")?;
    let cap = access.max_world_bytes();
    let mut budget = cap.max(MAX_PROJECT_BYTES);
    let root_source = read_pack(dir, &mut budget, cap, worlds)?;
    let root_text = root_source
        .files
        .get("pack.json")
        .cloned()
        .unwrap_or_default();
    let manifest = Manifest::parse(&root_text, &format!("{}/pack.json", dir.display()))
        .map_err(|e| e.to_string())?
        .id;
    let mut packs = BTreeMap::from([(manifest.clone(), root_source)]);
    let mut queue: VecDeque<String> = VecDeque::from([manifest.clone()]);
    while let Some(id) = queue.pop_front() {
        let Some(source) = packs.get(&id) else {
            continue;
        };
        let text = source.files.get("pack.json").cloned().unwrap_or_default();
        let pack_dir = PathBuf::from(&source.location);
        let Ok(m) = Manifest::parse(&text, &format!("{}/pack.json", source.location)) else {
            continue;
        };
        for dep in m.depends {
            if packs.contains_key(&dep.id) {
                continue;
            }
            if packs.len() >= MAX_PACKS {
                return Err(format!("a project loads at most {MAX_PACKS} packs"));
            }
            let wanted = match &dep.path {
                Some(p) => pack_dir.join(p),
                None => dir.join("packs").join(&dep.id),
            };
            // A pack that is not there is reported by the layering, with the chain that needs it.
            let Ok(real) = fs::canonicalize(&wanted) else {
                continue;
            };
            if !real.starts_with(&real_root) {
                return Err(format!(
                    "the pack `{}` that `{}` depends on is at `{}`, outside the read root `{}`",
                    dep.id,
                    id,
                    real.display(),
                    real_root.display()
                ));
            }
            if !real.join("pack.json").is_file() {
                continue;
            }
            packs.insert(dep.id.clone(), read_pack(&real, &mut budget, cap, worlds)?);
            queue.push_back(dep.id);
        }
    }
    Ok((manifest, packs))
}

/// Writes a save plan into `dir`, creating folders as needed. Paths come from the project
/// crate; each is checked to stay inside `dir` all the same.
fn write_plan(dir: &Path, saving: &Saving) -> Result<(Vec<String>, Vec<String>), String> {
    let inside = |relative: &str| -> Result<PathBuf, String> {
        let p = Path::new(relative);
        if relative.is_empty()
            || p.is_absolute()
            || p.components().any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(format!(
                "`{relative}` is not a path inside the project (a bug)"
            ));
        }
        Ok(dir.join(p))
    };
    let mut written = Vec::new();
    for (relative, text) in &saving.plan.writes {
        let target = inside(relative)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create the folder `{}`: {e}", parent.display()))?;
            let real = fs::canonicalize(parent)
                .map_err(|e| format!("cannot use the folder `{}`: {e}", parent.display()))?;
            if !real.starts_with(dir) {
                return Err(format!(
                    "`{}` is a link that leads out of the project; it is not written",
                    parent.display()
                ));
            }
        }
        if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!(
                "`{}` is a link; a project's files are written in place, not through links",
                target.display()
            ));
        }
        write_whole(&target, text.as_bytes())?;
        written.push(relative.clone());
    }
    let mut removed = Vec::new();
    for relative in &saving.plan.removes {
        let target = inside(relative)?;
        if target.exists() {
            fs::remove_file(&target)
                .map_err(|e| format!("cannot remove `{}`: {e}", target.display()))?;
        }
        removed.push(relative.clone());
    }
    Ok((written, removed))
}

impl Host {
    /// Opens an index for the project in `dir` (on disk when the folder is writable) and
    /// builds it; `null` without an indexer.
    fn index_project(&mut self, dir: &Path) -> (Option<Box<dyn ProjectIndex>>, Value) {
        let on_disk = writable(&self.access, dir).then_some(dir);
        let Some(indexer) = self.indexer.as_mut() else {
            return (None, Value::Null);
        };
        match indexer.open(on_disk) {
            Ok(index) => (Some(index), Value::Null),
            Err(e) => (None, json!({"error": e})),
        }
    }

    /// Rebuilds the index of the open project when the world changed since it was built.
    fn refresh_index(&mut self) -> Result<Value, String> {
        let revision = self.session.view_revision();
        let world = self.session.current_world();
        let Some(open) = self.project.as_mut() else {
            return Ok(Value::Null);
        };
        let Some(index) = open.index.as_mut() else {
            return Ok(Value::Null);
        };
        if open.indexed == Some(revision) {
            return Ok(json!({"location": index.location(), "rebuilt": false}));
        }
        let clock = crate::commands::Stopwatch::start();
        let mut done = index.rebuild(world, &open.project)?;
        open.indexed = Some(revision);
        if let Some(map) = done.as_object_mut() {
            map.insert("location".into(), json!(index.location()));
            map.insert("rebuilt".into(), json!(true));
            map.insert("ms".into(), clock.ms());
        }
        Ok(done)
    }

    /// Makes `project` (from `dir`) and `world` the open project and the document; the
    /// terrain is built with the tile cache. Returns the index's report and the terrain's.
    fn take_project(
        &mut self,
        project: Project,
        dir: PathBuf,
        world: gamecraft_world::World,
        from_session: bool,
    ) -> (Value, Value) {
        let assets = crate::assets::catalog(&project);
        let mut world = world;
        world.set_model_bounds(crate::assets::bounds(&assets));
        world.set_model_parts(crate::assets::parts(&assets));
        world.set_model_sockets(crate::assets::sockets(&assets));
        // The heightmap's samples: the session's when they are the ones this world names (a
        // project made from the session), else the folder's.
        let heightmap = crate::heightmap::load(
            self.session.document(),
            &dir,
            project.world(),
            &mut world,
            writable(&self.access, &dir),
            from_session,
        );
        let mut terrain_keys = crate::terrain_cache::Keys::new();
        // Without the heightmap's samples the derived cache is not used either: the next
        // change would rebuild from nothing.
        let samples_missing = heightmap.get("error").is_some_and(|e| !e.is_null());
        let cached = if samples_missing {
            None
        } else {
            world.terrain.as_ref().map(|terrain| {
                crate::terrain_cache::load(
                    &dir,
                    project.world(),
                    terrain,
                    writable(&self.access, &dir),
                )
            })
        };
        let mut terrain = if samples_missing {
            self.session.replace_document(world);
            json!({ "error": heightmap.get("error").cloned().unwrap_or(Value::Null) })
        } else {
            match cached {
                Some(Ok(loaded)) => {
                    terrain_keys = loaded.on_disk;
                    self.session
                        .replace_document_with_heights(world, loaded.field);
                    loaded.report
                }
                Some(Err(e)) => {
                    self.session.replace_document(world);
                    json!({"error": e})
                }
                None => {
                    self.session.replace_document(world);
                    Value::Null
                }
            }
        };
        let terrain = if heightmap == Value::Null {
            terrain
        } else if let Some(map) = terrain.as_object_mut() {
            map.insert("heightmap".into(), heightmap);
            terrain
        } else {
            json!({"heightmap": heightmap})
        };
        self.path = None;
        self.saved_revision = self.session.revision();
        self.assets = assets;
        self.provided.clear();
        self.pack = Some(dir.clone());
        let (index, problem) = self.index_project(&dir);
        self.project = Some(OpenProject {
            project,
            dir,
            index,
            indexed: None,
            terrain_keys,
        });
        let index = if problem.is_null() {
            self.refresh_index().unwrap_or_else(|e| json!({"error": e}))
        } else {
            problem
        };
        (index, terrain)
    }

    /// Writes the terrain tiles of the document whose key changed since they were written.
    fn save_terrain_cache(&mut self) -> Value {
        let Some(open) = self.project.as_mut() else {
            return Value::Null;
        };
        let (Some(terrain), Some(field)) = (
            self.session.document().terrain.as_ref(),
            self.session.document_heights(),
        ) else {
            return Value::Null;
        };
        let cache = crate::terrain_cache::cache_dir(&open.dir, open.project.world());
        let written = terrain
            .tile_keys()
            .map_err(|e| e.to_string())
            .and_then(|keys| {
                crate::terrain_cache::write_tiles(
                    &open.dir,
                    &cache,
                    field,
                    &keys,
                    &mut open.terrain_keys,
                )
            });
        match written {
            Ok(n) => json!(n),
            Err(e) => json!({"error": e}),
        }
    }

    pub(crate) fn project_new(&mut self, p: &Fields) -> Result<Value, String> {
        let given = p.required_str("path").map_err(strip_owner)?;
        let id = p.required_str("id").map_err(strip_owner)?;
        let name = p.required_str("name").map_err(strip_owner)?;
        let world_name = p.str("world").map_err(strip_owner)?.unwrap_or("main");
        let dir = resolve_dir(&self.access, given, Direction::Write, false)?;
        if fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("`{given}` is a link; give a plain folder"));
        }
        if dir.exists()
            && fs::read_dir(&dir)
                .map(|mut d| d.next().is_some())
                .unwrap_or(true)
        {
            return Err(format!(
                "`{given}` exists and is not empty; a new project needs a new or empty folder"
            ));
        }
        let cell_size = cell_size_param(p)?.flatten();
        let (mut project, saving) =
            Project::create_cells(id, name, world_name, self.session.document(), cell_size)
                .map_err(|e| e.to_string())?;
        fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create the folder `{}`: {e}", dir.display()))?;
        let dir = fs::canonicalize(&dir)
            .map_err(|e| format!("cannot use the folder `{}`: {e}", dir.display()))?;
        let (written, _) = write_plan(&dir, &saving)?;
        project.finish_save(saving);
        // Reload from the files: the document gets the ids and layers a later open gives.
        let (_, packs) = read_packs(
            &Access {
                read_root: Some(dir.clone()),
                write_root: None,
                max_world_bytes: self.access.max_world_bytes,
                cache_beside_files: self.access.cache_beside_files,
            },
            &dir,
            true,
        )?;
        let (project, world) =
            Project::load(id, &packs, Some(world_name)).map_err(|e| e.to_string())?;
        let (index, terrain) = self.take_project(project, dir.clone(), world, true);
        Ok(json!({
            "path": dir.display().to_string(),
            "id": id,
            "world": world_name,
            "files": written,
            "index": index,
            "terrain": terrain,
        }))
    }

    pub(crate) fn project_open(&mut self, p: &Fields) -> Result<Value, String> {
        let given = p.required_str("path").map_err(strip_owner)?;
        let world = p.str("world").map_err(strip_owner)?;
        let dir = resolve_dir(&self.access, given, Direction::Read, true)?;
        let cells = cells_param(p)?;
        let (root, packs) = read_packs(&self.access, &dir, true)?;
        let (project, loaded) =
            Project::load_cells(&root, &packs, world, cells.as_ref()).map_err(|e| e.to_string())?;
        let result = json!({
            "cell_size_m": project.cell_size(),
            "cells": cells.map(|c| c.iter().map(|c| [c.x, c.z]).collect::<Vec<_>>()),
            "path": dir.display().to_string(),
            "root": root,
            "world": project.world(),
            "worlds": project.worlds(),
            "packs": project.packs().iter().map(|p| p.manifest.id.clone()).collect::<Vec<_>>(),
            "entities": loaded.entity_count(),
            "warnings": project.warnings(),
        });
        let (index, terrain) = self.take_project(project, dir, loaded, false);
        let mut result = result;
        if let Some(map) = result.as_object_mut() {
            map.insert("index".into(), index);
            map.insert("terrain".into(), terrain);
        }
        Ok(result)
    }

    fn open_project(&self) -> Result<&OpenProject, String> {
        self.project.as_ref().ok_or_else(|| {
            "no project is open; open one with project.open or make one with project.new"
                .to_string()
        })
    }

    pub(crate) fn project_save(&mut self, p: &Fields) -> Result<Value, String> {
        let cell_size = cell_size_param(p)?;
        let open = self.open_project()?;
        if !writable(&self.access, &open.dir) {
            return Err(format!(
                "the project folder `{}` is not inside the write root; start the server with \
                 --automation-write-root set to a folder that holds it",
                open.dir.display()
            ));
        }
        // A new cell size is tried on a copy, so a save that fails changes nothing.
        let resized = match cell_size {
            Some(size) => {
                let mut copy = open.project.clone();
                copy.set_cell_size(size).map_err(|e| e.to_string())?;
                Some(copy)
            }
            None => None,
        };
        let saving = resized
            .as_ref()
            .unwrap_or(&open.project)
            .plan_save(self.session.document())
            .map_err(|e| format!("the document cannot be saved: {e}"))?;
        let dir = open.dir.clone();
        let (written, removed) = write_plan(&dir, &saving)?;
        // The heightmap's samples the document holds go with it (an import into a project
        // that could not be written until now).
        let heightmap_tiles = match self
            .session
            .document()
            .terrain
            .as_ref()
            .and_then(|t| t.heightmap())
            .filter(|m| !m.is_empty())
        {
            Some(map) => crate::heightmap::write(&dir, open.project.world(), map).map(|w| w.0)?,
            None => 0,
        };
        if let Some(open) = self.project.as_mut() {
            if let Some(copy) = resized {
                open.project = copy;
            }
            open.project.finish_save(saving);
            open.indexed = None;
        }
        self.saved_revision = self.session.revision();
        let unchanged = written.is_empty() && removed.is_empty();
        let tiles = self.save_terrain_cache();
        Ok(json!({
            "path": dir.display().to_string(),
            "written": written,
            "removed": removed,
            "unchanged": unchanged,
            "terrain_tiles_written": tiles,
            "heightmap_tiles_written": heightmap_tiles,
        }))
    }

    pub(crate) fn project_export(&mut self, p: &Fields) -> Result<Value, String> {
        let open = self.open_project()?;
        let overwrite = p.bool("overwrite").map_err(strip_owner)?.unwrap_or(false);
        let given = p.required_str("path").map_err(strip_owner)?;
        let target = resolve(&self.access, given, Direction::Write, "gamecraft")?;
        if target.exists() && !overwrite {
            return Err(format!(
                "`{}` exists; give overwrite: true to replace it",
                target.display()
            ));
        }
        let text = self
            .session
            .document()
            .to_json()
            .map_err(|e| format!("the document cannot be written: {e}"))?;
        write_whole(&target, text.as_bytes())?;
        // What the exported world needs to draw its models: the files, inside a pack's
        // folder, and the pack that has them now.
        let (needs, missing) = crate::assets::needed_files(self.session.document(), &self.assets);
        let packs: Vec<String> = open
            .project
            .packs()
            .iter()
            .map(|p| p.manifest.id.clone())
            .collect();
        let root = open.project.root().to_string();
        let dir = open.dir.display().to_string();
        Ok(json!({
            "path": target.display().to_string(),
            "bytes": text.len(),
            "entities": self.session.document().entity_count(),
            "pack": {"path": dir, "id": root, "packs": packs},
            "needs": needs,
            "missing": missing,
        }))
    }

    pub(crate) fn project_info(&mut self) -> Result<Value, String> {
        let open = self.open_project()?;
        let mut info = open
            .project
            .info(self.session.document())
            .map_err(|e| e.to_string())?;
        let dir = open.dir.display().to_string();
        let location = open.index.as_ref().map(|i| i.location());
        if let Some(map) = info.as_object_mut() {
            map.insert("path".into(), json!(dir));
            map.insert(
                "index".into(),
                match location {
                    None => Value::Null,
                    Some(l) => json!({"location": l}),
                },
            );
        }
        Ok(info)
    }

    pub(crate) fn add_dependency(&mut self, p: &Fields) -> Result<Value, String> {
        let id = p.required_str("id").map_err(strip_owner)?;
        let path = p.required_str("path").map_err(strip_owner)?;
        let open = self.open_project()?;
        if self.is_dirty() {
            return Err(
                "the document has unsaved changes; save them with project.save first, since \
                 the project is opened again with the new dependency"
                    .into(),
            );
        }
        if !writable(&self.access, &open.dir) {
            return Err(format!(
                "the project folder `{}` is not inside the write root, so its pack.json cannot \
                 be written",
                open.dir.display()
            ));
        }
        let dir = open.dir.clone();
        let world = open.project.world().to_string();
        let manifest_path = dir.join("pack.json");
        let old = fs::read_to_string(&manifest_path)
            .map_err(|e| format!("cannot read `{}`: {e}", manifest_path.display()))?;
        let mut manifest = Manifest::parse(&old, &manifest_path.display().to_string())
            .map_err(|e| e.to_string())?;
        manifest
            .add_dependency(id, path)
            .map_err(|e| e.to_string())?;
        let text = manifest.to_text().map_err(|e| e.to_string())?;
        write_whole(&manifest_path, text.as_bytes())?;
        let reopened = read_packs(&self.access, &dir, true).and_then(|(root, packs)| {
            Project::load(&root, &packs, Some(&world)).map_err(|e| e.to_string())
        });
        let (project, loaded) = match reopened {
            Ok(done) => done,
            Err(e) => {
                write_whole(&manifest_path, old.as_bytes())?;
                return Err(format!("{e}; pack.json is left as it was"));
            }
        };
        let packs: Vec<String> = project
            .packs()
            .iter()
            .map(|p| p.manifest.id.clone())
            .collect();
        let entities = loaded.entity_count();
        let _ = self.take_project(project, dir, loaded, false);
        Ok(json!({"id": id, "path": path, "packs": packs, "entities": entities}))
    }

    pub(crate) fn project_cells(&mut self, p: &Fields) -> Result<Value, String> {
        let size = match p.get("cell_size_m").filter(|v| !v.is_null()) {
            Some(v) => v.as_f64().ok_or(
                "`cell_size_m` must be a number: the side of a cell in metres, 16 to 16384",
            )?,
            None => self
                .project
                .as_ref()
                .and_then(|o| o.project.cell_size())
                .ok_or(
                    "no cell size: the open project's world is not saved in cells (or no \
                     project is open); give `cell_size_m`",
                )?,
        };
        let (min, max) = match (point2(p, "min")?, point2(p, "max")?, point2(p, "center")?) {
            (Some(a), Some(b), None) => (a, b),
            (None, None, Some(c)) => {
                let r = p
                    .get("radius_m")
                    .and_then(Value::as_f64)
                    .filter(|r| r.is_finite() && *r >= 0.0)
                    .ok_or("`radius_m` must be a number of metres, at least 0, with `center`")?;
                ([c[0] - r, c[1] - r], [c[0] + r, c[1] + r])
            }
            _ => {
                return Err(
                    "give the area as `min` and `max` ([x, z] corners), or as `center` and \
                     `radius_m`"
                        .into(),
                );
            }
        };
        let cells = gamecraft_project::cells_in_area(min, max, size).map_err(|e| e.to_string())?;
        Ok(json!({
            "cell_size_m": size,
            "count": cells.len(),
            "cells": cells.iter().map(|c| [c.x, c.z]).collect::<Vec<_>>(),
        }))
    }

    pub(crate) fn project_query(&mut self, p: &Fields) -> Result<Value, String> {
        let sql = p.required_str("sql").map_err(strip_owner)?;
        let limit = match p.get("limit") {
            None => MAX_QUERY_ROWS,
            Some(v) => v
                .as_u64()
                .filter(|n| (1..=MAX_QUERY_ROWS).contains(n))
                .ok_or_else(|| {
                    format!("`limit` must be a whole number from 1 to {MAX_QUERY_ROWS}")
                })?,
        };
        let open = self.open_project()?;
        if open.index.is_none() {
            return Err(
                "this host has no project index; gamecraft-cli builds one (with Turso) when a \
                 project opens"
                    .into(),
            );
        }
        let refreshed = self.refresh_index()?;
        let Some(index) = self.project.as_mut().and_then(|o| o.index.as_mut()) else {
            return Err("the project has no index".into());
        };
        let mut answer = index.query(sql, usize::try_from(limit).unwrap_or(1000))?;
        if let Some(map) = answer.as_object_mut() {
            map.insert("index".into(), refreshed);
        }
        Ok(answer)
    }
}
