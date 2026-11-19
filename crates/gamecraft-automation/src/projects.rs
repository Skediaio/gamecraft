//! Projects on disk (not in the browser): `project.new`, `project.open`, `project.save`,
//! `project.export`, `project.info`, `pack.add_dependency` and `project.query`
//! (`docs/design/projects.md`).
//!
//! The host reads a pack's files into a map of path to text and hands them to
//! `gamecraft-project`, which layers them into one world; saving goes the other way. Every
//! file read lies inside its pack's folder (a link out is refused) and every pack inside the
//! read root; every file written lies inside the project's folder, inside the write root.

use crate::commands::{host_spec, strip_owner};
use crate::fields::Fields;
use crate::files::{Direction, MAX_OPEN_BYTES, resolve, write_whole};
use crate::host::{Access, Host, ProjectIndex};
use gamecraft_engine::{CommandSpec, ParamType as T, param};
use gamecraft_project::{MAX_PACKS, Manifest, PackSource, Project, Saving};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Most bytes all the files of one project may hold together.
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
    /// The assets of the project's packs, the models measured.
    pub assets: crate::assets::Catalog,
    /// The model assets whose files the renderer was given.
    pub provided: std::collections::BTreeSet<String>,
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
            ],
            "{path, root, world, worlds, packs: [ids in load order], entities, warnings, index, \
             terrain: null | {tiles, from_cache, built, written, cache, problem, ms}}",
            json!({"path": "skirmish-night-mod"}),
        ),
        host_spec(
            "project.save",
            "Save Project",
            "Writes what changed in the document (never the running world) since the project \
             was opened or saved, each change to the file it belongs to in the root pack: one \
             line of a layer file for an edited entity, a prefab file, world.json as a patch \
             over the dependencies, terrain/edits.json for the terrain's edits, generators.json \
             for the generators (as overrides by id over the dependencies). A change to \
             something a dependency brought is written to the root pack as an override or a \
             removal, never to the dependency. Nothing changed: nothing is written. The terrain \
             tiles whose recipe changed are written to the tile cache. Needs the project folder \
             inside the write root.",
            vec![],
            "{path, written: [paths], removed: [paths], unchanged, terrain_tiles_written}",
            json!({}),
        ),
        host_spec(
            "project.export",
            "Export Project",
            "Writes the open project's world (the document, never the running world) as one \
             .gamecraft file inside the write root, entities with their layers. Replacing an \
             existing file needs `overwrite`.",
            vec![
                param(
                    "path",
                    T::String,
                    "A .gamecraft file inside the write root.",
                )
                .required(),
                param("overwrite", T::Boolean, "Replace an existing file.").default(json!(false)),
            ],
            "{path, bytes, entities}",
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
fn resolve_dir(
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
/// `worlds/*/terrain/edits.json` and `worlds/*/generators.json` are read (the terrain's tile cache is read when the world is
/// built); a file or folder that is a link leading out of the pack is refused.
fn read_pack(dir: &Path, budget: &mut u64) -> Result<PackSource, String> {
    let mut source = PackSource {
        location: dir.display().to_string(),
        files: BTreeMap::new(),
        assets: BTreeMap::new(),
    };
    let mut read = |relative: &str, source: &mut PackSource| -> Result<(), String> {
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
        if meta.len() > MAX_OPEN_BYTES || meta.len() > *budget {
            return Err(format!(
                "`{shown}` is too large: a file of a pack may be at most {} MB, and a project \
                 at most {} MB together",
                MAX_OPEN_BYTES / (1024 * 1024),
                MAX_PROJECT_BYTES / (1024 * 1024)
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
    read("pack.json", &mut source)?;
    for name in json_files(&dir.join("prefabs"))? {
        read(&format!("prefabs/{name}"), &mut source)?;
    }
    for world in sub_folders(&dir.join("worlds"))? {
        let base = format!("worlds/{world}");
        if dir.join(&base).join("world.json").exists() {
            read(&format!("{base}/world.json"), &mut source)?;
        }
        for name in json_files(&dir.join(&base).join("layers"))? {
            read(&format!("{base}/layers/{name}"), &mut source)?;
        }
        if dir.join(&base).join("terrain").join("edits.json").exists() {
            read(&format!("{base}/terrain/edits.json"), &mut source)?;
        }
        if dir.join(&base).join("generators.json").exists() {
            read(&format!("{base}/generators.json"), &mut source)?;
        }
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
        // The licence records and the .gltf documents are read for the licence gate.
        if relative.ends_with(gamecraft_project::RECORD_SUFFIX) || relative.ends_with(".gltf") {
            read(&relative, &mut source)?;
        }
        source.assets.insert(relative, size);
    }
    Ok(source)
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

/// Reads the root pack in `dir` and every pack it depends on, directly or not.
fn read_packs(
    access: &Access,
    dir: &Path,
) -> Result<(String, BTreeMap<String, PackSource>), String> {
    let real_root = access
        .read_root
        .as_ref()
        .and_then(|r| fs::canonicalize(r).ok())
        .ok_or("this server may not read files: no read root is set")?;
    let mut budget = MAX_PROJECT_BYTES;
    let root_source = read_pack(dir, &mut budget)?;
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
            packs.insert(dep.id.clone(), read_pack(&real, &mut budget)?);
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
    ) -> (Value, Value) {
        let assets = crate::assets::catalog(&project);
        let mut world = world;
        world.set_model_bounds(crate::assets::bounds(&assets));
        let mut terrain_keys = crate::terrain_cache::Keys::new();
        let cached = world.terrain.as_ref().map(|terrain| {
            crate::terrain_cache::load(&dir, project.world(), terrain, writable(&self.access, &dir))
        });
        let terrain = match cached {
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
        };
        self.path = None;
        self.saved_revision = self.session.revision();
        let (index, problem) = self.index_project(&dir);
        self.project = Some(OpenProject {
            project,
            dir,
            index,
            indexed: None,
            terrain_keys,
            assets,
            provided: std::collections::BTreeSet::new(),
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
        let (mut project, saving) = Project::create(id, name, world_name, self.session.document())
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
            },
            &dir,
        )?;
        let (project, world) =
            Project::load(id, &packs, Some(world_name)).map_err(|e| e.to_string())?;
        let (index, terrain) = self.take_project(project, dir.clone(), world);
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
        let (root, packs) = read_packs(&self.access, &dir)?;
        let (project, loaded) = Project::load(&root, &packs, world).map_err(|e| e.to_string())?;
        let result = json!({
            "path": dir.display().to_string(),
            "root": root,
            "world": project.world(),
            "worlds": project.worlds(),
            "packs": project.packs().iter().map(|p| p.manifest.id.clone()).collect::<Vec<_>>(),
            "entities": loaded.entity_count(),
            "warnings": project.warnings(),
        });
        let (index, terrain) = self.take_project(project, dir, loaded);
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

    pub(crate) fn project_save(&mut self) -> Result<Value, String> {
        let open = self.open_project()?;
        if !writable(&self.access, &open.dir) {
            return Err(format!(
                "the project folder `{}` is not inside the write root; start the server with \
                 --automation-write-root set to a folder that holds it",
                open.dir.display()
            ));
        }
        let saving = open
            .project
            .plan_save(self.session.document())
            .map_err(|e| format!("the document cannot be saved: {e}"))?;
        let dir = open.dir.clone();
        let (written, removed) = write_plan(&dir, &saving)?;
        if let Some(open) = self.project.as_mut() {
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
        }))
    }

    pub(crate) fn project_export(&mut self, p: &Fields) -> Result<Value, String> {
        self.open_project()?;
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
        Ok(json!({
            "path": target.display().to_string(),
            "bytes": text.len(),
            "entities": self.session.document().entity_count(),
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
        let reopened = read_packs(&self.access, &dir).and_then(|(root, packs)| {
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
        let _ = self.take_project(project, dir, loaded);
        Ok(json!({"id": id, "path": path, "packs": packs, "entities": entities}))
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
