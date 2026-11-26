//! The assets of an open project (not in the browser): `asset.import`, `asset.list` and
//! `asset.credits`, the bounding boxes the simulation needs, and the bytes a renderer needs.
//! An import takes a model's images into the pack's texture store
//! (`gamecraft_project::textures`): each is stored once, under its content hash, with a licence
//! record of its own.
//!
//! When a project opens, or a world file finds its pack (`file.open`: the nearest `pack.json`
//! above the file, the pack the command names, or the host's own), the host reads every model
//! asset of its packs (each has passed the licence gate of `gamecraft-project`), measures it
//! ([`gamecraft_project::inspect`]: triangles and bounding box) and hands the bounding boxes to
//! the session as derived data ([`gamecraft_engine::Session::set_model_bounds`]): the simulation
//! never reads a file. Before a picture, the host hands the renderer the files of the models
//! the world shows ([`crate::Pictures::provide_asset`]) and says, in one sentence, which models
//! no pack provides.

use crate::commands::{host_spec, strip_owner};
use crate::fields::Fields;
use crate::files::{Direction, MAX_ASSET_BYTES, resolve, write_whole};
use crate::host::Host;
use gamecraft_engine::{CommandSpec, ParamType as T, param};
use gamecraft_project::lod::LodDistances;
use gamecraft_project::model::{folder_of, references, resolve_uri, split_glb};
use gamecraft_project::textures::{self, Imported, is_texture};
use gamecraft_project::{
    ASSETS_DIR, AssetRecord, License, ModelSummary, PackAsset, Project, credits, inspect,
    record_path,
};
use gamecraft_world::{EntityId, ModelBounds, World, is_model_path};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Most files one imported model may bring along.
const MAX_IMPORT_FILES: usize = 256;
/// The fields of `asset.import`'s `textures` object.
const TEXTURE_FIELDS: [&str; 5] = ["license", "author", "source_url", "title", "changes"];

/// An asset of the open project as the host knows it.
pub(crate) struct HostAsset {
    /// The id of the pack it comes from.
    pub pack: String,
    /// That pack's folder.
    pub dir: PathBuf,
    /// The asset and its record.
    pub asset: PackAsset,
    /// For a model, what reading it found, or why it could not be read.
    pub summary: Option<Result<ModelSummary, String>>,
}

/// Every asset of the open project, by path inside `assets/`.
pub(crate) type Catalog = BTreeMap<String, HostAsset>;

/// The bytes of the pack-relative file `relative` of the pack in `dir`; a link out of the pack
/// is refused.
fn read_pack_file(dir: &Path, relative: &str) -> Result<Vec<u8>, String> {
    let path = dir.join(relative);
    let shown = path.display().to_string();
    let real = fs::canonicalize(&path).map_err(|e| format!("cannot read `{shown}`: {e}"))?;
    if !real.starts_with(dir) {
        return Err(format!(
            "`{shown}` is a link that leads out of its pack; a pack's files lie inside its folder"
        ));
    }
    let size = fs::metadata(&real)
        .map_err(|e| format!("cannot read `{shown}`: {e}"))?
        .len();
    if size > MAX_ASSET_BYTES {
        return Err(format!(
            "`{shown}` is too large: an asset file may be at most {} MB",
            MAX_ASSET_BYTES / (1024 * 1024)
        ));
    }
    fs::read(&real).map_err(|e| format!("cannot read `{shown}`: {e}"))
}

/// Reads and measures the model `path` (inside `assets/`) of the pack in `dir`.
fn measure(dir: &Path, path: &str) -> Result<ModelSummary, String> {
    let bytes = read_pack_file(dir, &format!("{ASSETS_DIR}/{path}"))?;
    let folder = folder_of(path);
    let resource = |r: &str| {
        resolve_uri(folder, r)
            .ok()
            .and_then(|inside| read_pack_file(dir, &format!("{ASSETS_DIR}/{inside}")).ok())
    };
    inspect(&bytes, &resource)
}

/// The catalog of `project`: every asset of its packs, the models read and measured.
pub(crate) fn catalog(project: &Project) -> Catalog {
    catalog_of(project.assets())
}

/// The catalog of the assets of a chain of packs ([`gamecraft_project::pack_assets`], or a
/// project's): every model read and measured.
pub(crate) fn catalog_of(assets: &BTreeMap<String, gamecraft_project::ProjectAsset>) -> Catalog {
    assets
        .iter()
        .map(|(path, a)| {
            let dir = PathBuf::from(&a.location);
            let summary = a.asset.is_model().then(|| measure(&dir, path));
            (
                path.clone(),
                HostAsset {
                    pack: a.pack.clone(),
                    dir,
                    asset: a.asset.clone(),
                    summary,
                },
            )
        })
        .collect()
}

/// The bounding boxes of the catalog's models that could be read.
pub(crate) fn bounds(catalog: &Catalog) -> BTreeMap<String, ModelBounds> {
    catalog
        .iter()
        .filter_map(|(path, a)| match &a.summary {
            Some(Ok(s)) => Some((path.clone(), s.bounds)),
            _ => None,
        })
        .collect()
}

/// The model assets `world` refers to, each with the entities that use it, in id order.
fn used(world: &World) -> BTreeMap<String, Vec<EntityId>> {
    let mut out: BTreeMap<String, Vec<EntityId>> = BTreeMap::new();
    for e in world.entities() {
        if let Some(m) = e.model() {
            out.entry(m.asset.clone()).or_default().push(e.id);
        }
    }
    out
}

/// The specs of this module.
pub(crate) fn asset_specs() -> Vec<CommandSpec> {
    vec![
        host_spec(
            "asset.import",
            "Import Asset",
            "Imports a glTF model (`file`: a .glb, or a .gltf with the buffers and images it \
             refers to beside it) from the read root into the open project's root pack as \
             assets/<to> and writes its licence record assets/<to>.license.json. Only CC0-1.0 \
             and CC-BY-4.0 are allowed, with the author and where it came from. The model is \
             read first: a broken file is refused and nothing is written; so is a `to` that \
             exists. Its images go into the pack's texture store, \
             assets/textures/<content-hash>.<png | jpg>: an image the pack has already is \
             reused (nothing is stored again, its record stays), a new one is written with \
             `textures` as its record (else the model's), and the stored model refers to them, \
             so a model draws exactly as before. Solid models that use it collide from then \
             on. Needs the project folder inside the write root.",
            vec![
                param(
                    "file",
                    T::String,
                    "The .glb or .gltf file, inside the read root.",
                )
                .required(),
                param(
                    "to",
                    T::String,
                    "Its path inside the pack's assets/ folder, with the same extension \
                     (`models/crate.glb`); not under textures/, the store of shared images.",
                )
                .required(),
                param("license", T::String, "Its licence.")
                    .required()
                    .allowed(&["CC0-1.0", "CC-BY-4.0"]),
                param(
                    "author",
                    T::String,
                    "Who made it, as they want to be credited.",
                )
                .required(),
                param(
                    "source_url",
                    T::String,
                    "Where it was found (`https://...`).",
                )
                .required(),
                param("title", T::String, "Its name for people."),
                param(
                    "changes",
                    T::String,
                    "What was changed from the source (CC BY asks for it).",
                ),
                param(
                    "textures",
                    T::Object,
                    "The licence record of the images it extracts that are new to the pack: \
                     {license, author, source_url, title?, changes?}; its licence, author and \
                     source left out are the model's. Only the fields named here are allowed.",
                ),
            ],
            "{asset, pack, files: [paths written], record, license, bytes, triangles, bounds: \
             {min, max}, textures: [{path, bytes, stored, record}]}",
            json!({"file": "downloads/crate.glb", "to": "models/crate.glb", "license": "CC0-1.0", "author": "Jane Doe", "source_url": "https://example.org/crate"}),
        ),
        host_spec(
            "asset.list",
            "List Assets",
            "Lists the assets of the open project's packs, or of the pack of the open world \
             file (a later pack's asset replaces an earlier one's of the same path): licence, \
             author, source, size, and for a model its triangle count, bounding box (metres, \
             the asset's own axes) and levels of detail (triangles per level, where each ends \
             and how far it is drawn as its file says, else from its size), or the problem \
             that kept it from being read; how many entities of the world shown use it. The \
             textures of the packs' store (`textures`) are listed apart, each once, with the \
             models that refer to it. And the model assets the world refers to that no pack \
             has.",
            vec![],
            "{assets: [{path, pack, kind, license, author, source_url, title, changes, bytes, \
             files, triangles, bounds: {min, max} | null, lods: [{level, triangles, until_m}], \
             draw_distance_m, lod_source, lod_warnings, problem, used_by}], textures: [{path, \
             pack, bytes, license, author, source_url, title, changes, used_by: [models]}], \
             missing: [{asset, entities}]}",
            json!({}),
        ),
        host_spec(
            "asset.credits",
            "Asset Credits",
            "The attribution text a game must show: one line for every CC-BY asset of the \
             open project, or of the open world file's pack, with its title, author, licence \
             and source (CC0 assets need none; an empty text when there is no CC-BY asset). A \
             texture of the store is one asset, credited once however many models use it.",
            vec![],
            "{text, assets: [{path, kind, title, author, license, source_url, changes}]}",
            json!({}),
        ),
    ]
}

fn bounds_json(b: &ModelBounds) -> Value {
    json!({"min": b.min, "max": b.max})
}

/// The levels of detail of a model as `asset.list` shows them: the levels that draw, each
/// with its triangles and where it ends (its file's distances, else the rule from its size; a
/// component's distances are its entities' own), how far it is drawn, how the levels were
/// found and what is broken in them.
fn lods_json(s: &ModelSummary) -> (Value, Value, Value, Value) {
    let (distances, notes) =
        LodDistances::resolve(s.lod.levels, s.size_m(), s.lod.file.as_ref(), None);
    let ends = distances.ends_m();
    let lods: Vec<Value> = s
        .lod
        .drawn
        .iter()
        .zip(&s.lod.triangles)
        .map(|(&level, &triangles)| {
            json!({"level": level, "triangles": triangles, "until_m": ends.get(level)})
        })
        .collect();
    let mut warnings: Vec<String> = s.lod.warnings.clone();
    warnings.extend(notes);
    let draw_m = s
        .lod
        .drawn_ends(&ends)
        .last()
        .copied()
        .unwrap_or(f64::INFINITY);
    (
        Value::from(lods),
        json!(draw_m),
        s.lod_source
            .map_or(Value::Null, |source| json!(source.name())),
        Value::from(warnings),
    )
}

/// The files inside a pack's folder that the models of `world` need: each model's own files
/// and the files they refer to, as the catalog lists them (`assets/models/crate.glb`, the
/// textures of the store), with the licence record of each, sorted, each once; and the models
/// no pack has.
pub(crate) fn needed_files(world: &World, catalog: &Catalog) -> (Vec<String>, Vec<Value>) {
    let mut files = BTreeSet::new();
    let mut missing = Vec::new();
    for (path, ids) in used(world) {
        match catalog.get(&path) {
            Some(asset) => {
                files.extend(asset.asset.files.iter().cloned());
                // Each file's licence record travels with it: the model's own and those of
                // the textures of the store it refers to.
                files.insert(record_path(&asset.asset.path));
                for file in &asset.asset.files {
                    if let Some(inside) = file.strip_prefix(&format!("{ASSETS_DIR}/"))
                        && is_texture(inside)
                    {
                        files.insert(record_path(inside));
                    }
                }
            }
            None => missing.push(
                json!({"asset": path, "entities": ids.iter().map(|id| id.0).collect::<Vec<_>>()}),
            ),
        }
    }
    (files.into_iter().collect(), missing)
}

impl Host {
    /// The catalog of the open project, or of the pack the open world file found (empty for a
    /// world file without one, a project that has none, or a new world).
    fn catalog(&self) -> Result<&Catalog, String> {
        if self.project.is_none() && self.path.is_none() {
            return Err(
                "no project is open; open one with project.open, or a world file (the ones \
                 inside a pack, or opened with `pack`, list that pack's): assets live in packs"
                    .to_string(),
            );
        }
        Ok(&self.assets)
    }

    /// The models of the world shown that no open pack provides, each with how many
    /// entities show it, in path order.
    fn missing_models(&self) -> Vec<(String, usize)> {
        used(self.session.current_world())
            .into_iter()
            .filter(|(path, _)| !self.assets.contains_key(path))
            .map(|(path, ids)| (path, ids.len()))
            .collect()
    }

    /// One sentence for `view.render` when models are drawn as magenta boxes because no pack
    /// provides their files: which models, why, and what to do. `None` when every model of the
    /// world shown has its files.
    pub(crate) fn missing_pack_line(&self) -> Option<String> {
        let missing = self.missing_models();
        if missing.is_empty() {
            return None;
        }
        let one = missing.len() == 1;
        let names: Vec<String> = missing
            .iter()
            .map(|(path, n)| format!("`{path}` ({n} entit{})", if *n == 1 { "y" } else { "ies" }))
            .collect();
        let list = match names.split_last() {
            Some((last, [])) => last.clone(),
            Some((last, head)) => format!("{} and {}", head.join(", "), last),
            None => String::new(),
        };
        let why = match (&self.pack, &self.path) {
            (Some(dir), _) => format!("the pack `{}` has no such file", dir.display()),
            (None, Some(path)) => format!(
                "no pack.json lies above `{}` and none was given",
                path.display()
            ),
            (None, None) => "the world has no file, and no pack was given".to_string(),
        };
        let it = if one { "it" } else { "they" };
        let is = if one { "is" } else { "are" };
        Some(format!(
            "{} {list}: no pack provides {it} ({why}), so {it} {is} drawn as small magenta \
             boxes; open the world with --pack DIR (file.open `pack`), a folder with a \
             pack.json whose assets/ holds {it}, or import {it} with asset.import.",
            if one { "Model" } else { "Models" }
        ))
    }

    /// Hands `give` the files of every model of the world shown that it was not handed yet
    /// (by this host, since the assets were set): paths inside `assets/`, a `.gltf`'s
    /// buffers and images under their own paths. A window that draws for itself calls it
    /// before every frame; `view.render` does it for the host's [`crate::Pictures`]. A file
    /// that cannot be read is left out: the renderer then draws its stand-in and says why.
    /// Nothing without an open project or a world file's pack.
    pub fn hand_over_assets(&mut self, give: &mut dyn FnMut(&str, Vec<u8>)) {
        let wanted: Vec<String> = used(self.session.current_world()).into_keys().collect();
        for path in wanted {
            if self.provided.contains(&path) {
                continue;
            }
            let Some(asset) = self.assets.get(&path) else {
                continue;
            };
            self.provided.insert(path.clone());
            for file in &asset.asset.files {
                let Some(inside) = file.strip_prefix(&format!("{ASSETS_DIR}/")) else {
                    continue;
                };
                if let Ok(bytes) = read_pack_file(&asset.dir, file) {
                    give(inside, bytes);
                }
            }
        }
    }

    /// [`Host::hand_over_assets`] to the host's pictures, if it has any.
    pub(crate) fn provide_assets(&mut self) {
        if let Some(mut pictures) = self.pictures.take() {
            self.hand_over_assets(&mut |path, bytes| pictures.provide_asset(path, bytes));
            self.pictures = Some(pictures);
        }
    }

    pub(crate) fn asset_list(&mut self) -> Result<Value, String> {
        let catalog = self.catalog()?;
        let uses = used(self.session.current_world());
        // Which models refer to which texture of the store (their files, as the gate lists them).
        let mut users: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let prefix = format!("{ASSETS_DIR}/");
        for (path, a) in catalog.iter().filter(|(_, a)| a.asset.is_model()) {
            for file in &a.asset.files {
                if let Some(inside) = file.strip_prefix(prefix.as_str())
                    && is_texture(inside)
                {
                    users.entry(inside).or_default().push(path.as_str());
                }
            }
        }
        let mut assets: Vec<Value> = Vec::new();
        let mut textures: Vec<Value> = Vec::new();
        for (path, a) in catalog {
            let r = &a.asset.record;
            if is_texture(path) {
                textures.push(json!({
                    "path": path,
                    "pack": a.pack,
                    "bytes": a.asset.bytes,
                    "license": r.license.id(),
                    "author": r.author,
                    "source_url": r.source_url,
                    "title": r.title,
                    "changes": r.changes,
                    "used_by": users.get(path.as_str()).cloned().unwrap_or_default(),
                }));
                continue;
            }
            let (triangles, bounds, problem) = match &a.summary {
                Some(Ok(s)) => (json!(s.triangles), bounds_json(&s.bounds), Value::Null),
                Some(Err(e)) => (Value::Null, Value::Null, json!(e)),
                None => (Value::Null, Value::Null, Value::Null),
            };
            let (lods, draw_distance_m, lod_source, lod_warnings) = match &a.summary {
                Some(Ok(s)) => lods_json(s),
                _ => (Value::Null, Value::Null, Value::Null, Value::Null),
            };
            assets.push(json!({
                "path": path,
                "pack": a.pack,
                "kind": a.asset.kind(),
                "license": r.license.id(),
                "author": r.author,
                "source_url": r.source_url,
                "title": r.title,
                "changes": r.changes,
                "bytes": a.asset.bytes,
                "files": a.asset.files,
                "triangles": triangles,
                "bounds": bounds,
                "lods": lods,
                "draw_distance_m": draw_distance_m,
                "lod_source": lod_source,
                "lod_warnings": lod_warnings,
                "problem": problem,
                "used_by": uses.get(path).map_or(0, Vec::len),
            }));
        }
        let missing: Vec<Value> = uses
            .iter()
            .filter(|(path, _)| !catalog.contains_key(*path))
            .map(|(path, ids)| {
                json!({"asset": path, "entities": ids.iter().map(|id| id.0).collect::<Vec<_>>()})
            })
            .collect();
        Ok(json!({"assets": assets, "textures": textures, "missing": missing}))
    }

    pub(crate) fn asset_credits(&mut self) -> Result<Value, String> {
        let catalog = self.catalog()?;
        let text = credits(catalog.values().map(|a| &a.asset));
        let assets: Vec<Value> = catalog
            .values()
            .filter(|a| a.asset.record.license.needs_attribution())
            .map(|a| {
                let r = &a.asset.record;
                json!({
                    "path": a.asset.path,
                    "kind": a.asset.kind(),
                    "title": r.title,
                    "author": r.author,
                    "license": r.license.id(),
                    "source_url": r.source_url,
                    "changes": r.changes,
                })
            })
            .collect();
        Ok(json!({"text": text, "assets": assets}))
    }

    pub(crate) fn asset_import(&mut self, p: &Fields) -> Result<Value, String> {
        let file = p.required_str("file").map_err(strip_owner)?;
        let to = p.required_str("to").map_err(strip_owner)?;
        let license = License::parse(p.required_str("license").map_err(strip_owner)?)?;
        let record = AssetRecord {
            license,
            author: p.required_str("author").map_err(strip_owner)?.to_string(),
            source_url: p
                .required_str("source_url")
                .map_err(strip_owner)?
                .to_string(),
            title: p.str("title").map_err(strip_owner)?.map(str::to_string),
            changes: p.str("changes").map_err(strip_owner)?.map(str::to_string),
        };
        record.validate()?;
        // The record of the textures the import stores that are new to the pack: the model's,
        // with what `textures` gives (its licence, author and source) in place of theirs.
        let given = p.object("textures")?;
        let t = Fields::new(
            "asset.import: `textures`",
            "field",
            Some(&given),
            &TEXTURE_FIELDS,
        )
        .map_err(strip_owner)?;
        let texture_record = AssetRecord {
            license: match t.str("license").map_err(strip_owner)? {
                Some(id) => License::parse(id)?,
                None => license,
            },
            author: t
                .str("author")
                .map_err(strip_owner)?
                .unwrap_or(record.author.as_str())
                .to_string(),
            source_url: t
                .str("source_url")
                .map_err(strip_owner)?
                .unwrap_or(record.source_url.as_str())
                .to_string(),
            title: t.str("title").map_err(strip_owner)?.map(str::to_string),
            changes: t.str("changes").map_err(strip_owner)?.map(str::to_string),
        };
        texture_record.validate()?;
        if !is_model_path(to) {
            return Err(format!(
                "`to` {to:?} is not a model path inside assets/: ASCII letters, digits, `-`, \
                 `_`, `.` and `/`, without `..`, ending in .glb or .gltf, such as \
                 \"models/crate.glb\""
            ));
        }
        if is_texture(to) {
            return Err(format!(
                "`to` {to:?} lies in the pack's texture store (textures/), which holds the \
                 images models share; import the model under a path of its own, such as \
                 \"models/crate.glb\""
            ));
        }
        let extension = if file.ends_with(".gltf") {
            "gltf"
        } else {
            "glb"
        };
        if !to.ends_with(&format!(".{extension}")) {
            return Err(format!(
                "`to` {to:?} must end in .{extension} like the file it copies"
            ));
        }
        let open = self.project.as_ref().ok_or_else(|| {
            "no project is open; open one with project.open, then import into its pack".to_string()
        })?;
        let dir = open.dir.clone();
        if !crate::projects::writable(&self.access, &dir) {
            return Err(format!(
                "the project folder `{}` is not inside the write root, so nothing can be \
                 imported into it",
                dir.display()
            ));
        }
        let source = resolve(&self.access, file, Direction::Read, extension)?;
        let read = |path: &Path| -> Result<Vec<u8>, String> {
            let meta =
                fs::metadata(path).map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
            if !meta.is_file() || meta.len() > MAX_ASSET_BYTES {
                return Err(format!(
                    "`{}` is not a file of at most {} MB",
                    path.display(),
                    MAX_ASSET_BYTES / (1024 * 1024)
                ));
            }
            fs::read(path).map_err(|e| format!("cannot read `{}`: {e}", path.display()))
        };
        let bytes = read(&source)?;
        // The files the model refers to (a .gltf's buffers and images, a .glb's images beside
        // it) are read from beside it, inside the read root.
        let mut parts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        {
            let json = split_glb(&bytes)
                .map_err(|e| format!("`{file}` is not a model that can be used: {e}"))?
                .0;
            let refs = references(json).map_err(|e| format!("`{file}`: {e}"))?;
            if refs.len() > MAX_IMPORT_FILES {
                return Err(format!(
                    "`{file}` refers to {} files; at most {MAX_IMPORT_FILES}",
                    refs.len()
                ));
            }
            let base = source.parent().unwrap_or(Path::new(""));
            let root = self
                .access
                .read_root
                .as_ref()
                .and_then(|r| fs::canonicalize(r).ok())
                .ok_or("no read root is set")?;
            for r in refs {
                let real = fs::canonicalize(base.join(&r)).map_err(|e| {
                    format!("`{file}` refers to `{r}`, which cannot be read beside it: {e}")
                })?;
                if !real.starts_with(&root) {
                    return Err(format!(
                        "`{file}` refers to `{r}`, which lies outside the read root"
                    ));
                }
                parts.insert(r, read(&real)?);
            }
        }
        let resource = |r: &str| parts.get(r).cloned();
        let Imported {
            model,
            summary,
            textures,
            files: buffers,
        } = textures::import(&bytes, &resource, to)
            .map_err(|e| format!("`{file}` is not a model that can be used: {e}"))?;
        let record_file = record_path(to);
        // The texture store: an image the pack has is reused once its bytes match its name and
        // it has its record; a new one is written with the texture's record.
        let mut store: Vec<Value> = Vec::new();
        let mut new_textures: Vec<(String, u64)> = Vec::new();
        let mut targets: Vec<(String, Vec<u8>)> = Vec::new();
        for texture in &textures {
            let inside = texture.path.as_str();
            let relative = format!("{ASSETS_DIR}/{inside}");
            let texture_record_file = record_path(inside);
            let stored = if dir.join(&relative).exists() {
                // The file must be this very image: another image with a colliding hash is
                // an error, not a reuse.
                let existing = read_pack_file(&dir, &relative)?;
                if existing != texture.bytes {
                    return Err(format!(
                        "`{relative}` exists already in the pack's texture store but holds other \
                         bytes than the image it is named after (it was replaced or renamed by \
                         hand): remove it and its record, then import again"
                    ));
                }
                if !dir.join(&texture_record_file).exists() {
                    return Err(format!(
                        "`{relative}` exists already in the pack's texture store without its \
                         licence record `{texture_record_file}`; add it, then import again"
                    ));
                }
                false
            } else {
                if dir.join(&texture_record_file).exists() {
                    return Err(format!(
                        "`{texture_record_file}` is the record of `{relative}`, which does not \
                         exist; remove the record, then import again"
                    ));
                }
                targets.push((relative, texture.bytes.clone()));
                targets.push((
                    texture_record_file.clone(),
                    texture_record.to_text().into_bytes(),
                ));
                new_textures.push((inside.to_string(), texture.bytes.len() as u64));
                true
            };
            store.push(json!({
                "path": inside,
                "bytes": texture.bytes.len(),
                "stored": stored,
                "record": texture_record_file,
            }));
        }
        // The model's other files (a .gltf's buffers: rewritten when they held images, else
        // copied as they came), the model and its record. A model's bytes are its own files;
        // the textures it refers to are assets of their own, counted once.
        let mut files = vec![format!("{ASSETS_DIR}/{to}")];
        let mut total = model.len() as u64;
        for kept in &buffers {
            let data = match kept.bytes.clone().or_else(|| parts.get(&kept.uri).cloned()) {
                Some(data) => data,
                None => {
                    return Err(format!(
                        "`{file}` refers to `{}`, which was not read",
                        kept.uri
                    ));
                }
            };
            files.push(format!("{ASSETS_DIR}/{}", kept.path));
            total = total.saturating_add(data.len() as u64);
            targets.push((format!("{ASSETS_DIR}/{}", kept.path), data));
        }
        for texture in &textures {
            files.push(format!("{ASSETS_DIR}/{}", texture.path));
        }
        targets.push((format!("{ASSETS_DIR}/{to}"), model));
        for (relative, _) in &targets {
            if dir.join(relative).exists() {
                return Err(format!(
                    "`{relative}` exists already in the pack; choose another `to`"
                ));
            }
        }
        if dir.join(&record_file).exists() {
            return Err(format!(
                "`{record_file}` exists already in the pack; choose another `to`"
            ));
        }
        let mut written = Vec::new();
        for (relative, data) in targets
            .iter()
            .chain([(record_file.clone(), record.to_text().into_bytes())].iter())
        {
            let target = dir.join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create the folder `{}`: {e}", parent.display()))?;
                let real = fs::canonicalize(parent)
                    .map_err(|e| format!("cannot use the folder `{}`: {e}", parent.display()))?;
                if !real.starts_with(&dir) {
                    return Err(format!(
                        "`{}` is a link that leads out of the project; nothing more is written",
                        parent.display()
                    ));
                }
            }
            write_whole(&target, data)?;
            written.push(relative.clone());
        }
        let pack = self
            .project
            .as_ref()
            .map(|o| o.project.root().to_string())
            .unwrap_or_default();
        let answer = json!({
            "asset": to,
            "pack": pack,
            "files": written,
            "record": record_file,
            "license": license.id(),
            "bytes": total,
            "triangles": summary.triangles,
            "bounds": bounds_json(&summary.bounds),
            "textures": store,
        });
        for (path, bytes) in new_textures {
            let files = vec![format!("{ASSETS_DIR}/{path}")];
            self.assets.insert(
                path.clone(),
                HostAsset {
                    pack: pack.clone(),
                    dir: dir.clone(),
                    asset: PackAsset {
                        path,
                        record: texture_record.clone(),
                        files,
                        bytes,
                    },
                    summary: None,
                },
            );
        }
        self.assets.insert(
            to.to_string(),
            HostAsset {
                pack,
                dir,
                asset: PackAsset {
                    path: to.to_string(),
                    record,
                    files,
                    bytes: total,
                },
                summary: Some(Ok(summary)),
            },
        );
        self.provided.remove(to);
        let all = bounds(&self.assets);
        self.session.set_model_bounds(all);
        Ok(answer)
    }
}
