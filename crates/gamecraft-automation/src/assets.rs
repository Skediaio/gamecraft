//! The assets of an open project (not in the browser): `asset.import`, `asset.list` and
//! `asset.credits`, the bounding boxes the simulation needs, and the bytes a renderer needs.
//!
//! When a project opens, the host reads every model asset of its packs (each has passed the
//! licence gate of `gamecraft-project`), measures it ([`gamecraft_project::inspect`]: triangles
//! and bounding box) and hands the bounding boxes to the session as derived data
//! ([`gamecraft_engine::Session::set_model_bounds`]): the simulation never reads a file. Before
//! a picture, the host hands the renderer the files of the models the world shows
//! ([`crate::Pictures::provide_asset`]).

use crate::commands::{host_spec, strip_owner};
use crate::fields::Fields;
use crate::files::{Direction, MAX_OPEN_BYTES, resolve, write_whole};
use crate::host::Host;
use gamecraft_engine::{CommandSpec, ParamType as T, param};
use gamecraft_project::model::{folder_of, references};
use gamecraft_project::{
    ASSETS_DIR, AssetRecord, License, ModelSummary, PackAsset, Project, credits, inspect,
    record_path,
};
use gamecraft_world::{EntityId, ModelBounds, World, is_model_path};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Most files one imported `.gltf` may bring along.
const MAX_IMPORT_FILES: usize = 256;

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
    if size > MAX_OPEN_BYTES {
        return Err(format!(
            "`{shown}` is too large: an asset file may be at most {} MB",
            MAX_OPEN_BYTES / (1024 * 1024)
        ));
    }
    fs::read(&real).map_err(|e| format!("cannot read `{shown}`: {e}"))
}

/// Reads and measures the model `path` (inside `assets/`) of the pack in `dir`.
fn measure(dir: &Path, path: &str) -> Result<ModelSummary, String> {
    let bytes = read_pack_file(dir, &format!("{ASSETS_DIR}/{path}"))?;
    let folder = folder_of(path);
    let resource = |r: &str| read_pack_file(dir, &format!("{ASSETS_DIR}/{folder}{r}")).ok();
    inspect(&bytes, &resource)
}

/// The catalog of `project`: every asset of its packs, the models read and measured.
pub(crate) fn catalog(project: &Project) -> Catalog {
    project
        .assets()
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

/// The model assets `world` refers to (`model` and `held_model` components, a held model's
/// attachments included), each with the
/// entities that use it, in id order.
fn used(world: &World) -> BTreeMap<String, Vec<EntityId>> {
    let mut out: BTreeMap<String, Vec<EntityId>> = BTreeMap::new();
    for e in world.entities() {
        let assets = e
            .model()
            .map(|m| &m.asset)
            .into_iter()
            .chain(e.held_model().into_iter().flat_map(|h| h.assets()));
        for asset in assets {
            let ids = out.entry(asset.clone()).or_default();
            if ids.last() != Some(&e.id) {
                ids.push(e.id);
            }
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
            "Copies a glTF model (`file`: a .glb, or a .gltf with the buffers and images it \
             refers to beside it) from the read root into the open project's root pack as \
             assets/<to> and writes its licence record assets/<to>.license.json. Only CC0-1.0 \
             and CC-BY-4.0 are allowed, with the author and where it came from. The model is \
             read first: a broken file is refused and nothing is written; so is a `to` that \
             exists. Solid models that use it collide from then on. Needs the project folder \
             inside the write root.",
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
                     (`models/crate.glb`).",
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
            ],
            "{asset, pack, files: [paths written], record, license, bytes, triangles, bounds: \
             {min, max}}",
            json!({"file": "downloads/crate.glb", "to": "models/crate.glb", "license": "CC0-1.0", "author": "Jane Doe", "source_url": "https://example.org/crate"}),
        ),
        host_spec(
            "asset.list",
            "List Assets",
            "Lists the assets of the open project's packs (a later pack's asset replaces an \
             earlier one's of the same path): licence, author, source, size, and for a model \
             its triangle count and bounding box (metres, the asset's own axes), or the \
             problem that kept it from being read; how many entities of the world shown use \
             it; and the model assets the world refers to that no pack has.",
            vec![],
            "{assets: [{path, pack, license, author, source_url, title, changes, bytes, files, \
             triangles, bounds: {min, max} | null, problem, used_by}], missing: [{asset, \
             entities}]}",
            json!({}),
        ),
        host_spec(
            "asset.credits",
            "Asset Credits",
            "The attribution text a game must show: one line for every CC-BY asset of the \
             open project with its title, author, licence and source (CC0 assets need none; \
             an empty text when there is no CC-BY asset).",
            vec![],
            "{text, assets: [{path, title, author, license, source_url, changes}]}",
            json!({}),
        ),
    ]
}

fn bounds_json(b: &ModelBounds) -> Value {
    json!({"min": b.min, "max": b.max})
}

impl Host {
    /// The catalog of the open project, or the error that no project is open.
    fn catalog(&self) -> Result<&Catalog, String> {
        self.project.as_ref().map(|p| &p.assets).ok_or_else(|| {
            "no project is open; open one with project.open (assets live in packs)".to_string()
        })
    }

    /// Hands `give` the files of every model of the world shown that it was not handed yet
    /// (by this host, since the project was opened): paths inside `assets/`, a `.gltf`'s
    /// buffers and images under their own paths. A window that draws for itself calls it
    /// before every frame; `view.render` does it for the host's [`crate::Pictures`]. A file
    /// that cannot be read is left out: the renderer then draws its stand-in and says why.
    /// Nothing without an open project.
    pub fn hand_over_assets(&mut self, give: &mut dyn FnMut(&str, Vec<u8>)) {
        let wanted: Vec<String> = used(self.session.current_world()).into_keys().collect();
        let Some(open) = self.project.as_mut() else {
            return;
        };
        for path in wanted {
            if open.provided.contains(&path) {
                continue;
            }
            let Some(asset) = open.assets.get(&path) else {
                continue;
            };
            open.provided.insert(path.clone());
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
        let assets: Vec<Value> = catalog
            .iter()
            .map(|(path, a)| {
                let r = &a.asset.record;
                let (triangles, bounds, problem) = match &a.summary {
                    Some(Ok(s)) => (json!(s.triangles), bounds_json(&s.bounds), Value::Null),
                    Some(Err(e)) => (Value::Null, Value::Null, json!(e)),
                    None => (Value::Null, Value::Null, Value::Null),
                };
                json!({
                    "path": path,
                    "pack": a.pack,
                    "license": r.license.id(),
                    "author": r.author,
                    "source_url": r.source_url,
                    "title": r.title,
                    "changes": r.changes,
                    "bytes": a.asset.bytes,
                    "files": a.asset.files,
                    "triangles": triangles,
                    "bounds": bounds,
                    "problem": problem,
                    "used_by": uses.get(path).map_or(0, Vec::len),
                })
            })
            .collect();
        let missing: Vec<Value> = uses
            .iter()
            .filter(|(path, _)| !catalog.contains_key(*path))
            .map(|(path, ids)| {
                json!({"asset": path, "entities": ids.iter().map(|id| id.0).collect::<Vec<_>>()})
            })
            .collect();
        Ok(json!({"assets": assets, "missing": missing}))
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
        if !is_model_path(to) {
            return Err(format!(
                "`to` {to:?} is not a model path inside assets/: ASCII letters, digits, `-`, \
                 `_`, `.` and `/`, without `..`, ending in .glb or .gltf, such as \
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
            if !meta.is_file() || meta.len() > MAX_OPEN_BYTES {
                return Err(format!(
                    "`{}` is not a file of at most {} MB",
                    path.display(),
                    MAX_OPEN_BYTES / (1024 * 1024)
                ));
            }
            fs::read(path).map_err(|e| format!("cannot read `{}`: {e}", path.display()))
        };
        let bytes = read(&source)?;
        // A .gltf brings the files it refers to, from beside it, inside the read root.
        let mut parts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        if extension == "gltf" {
            let refs = references(&bytes).map_err(|e| format!("`{file}`: {e}"))?;
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
        let summary = inspect(&bytes, &|r: &str| parts.get(r).cloned())
            .map_err(|e| format!("`{file}` is not a model that can be used: {e}"))?;
        let folder = folder_of(to);
        let mut targets: Vec<(String, &[u8])> = vec![(format!("{ASSETS_DIR}/{to}"), &bytes)];
        for (r, data) in &parts {
            targets.push((format!("{ASSETS_DIR}/{folder}{r}"), data.as_slice()));
        }
        let record_file = record_path(to);
        for (relative, _) in targets
            .iter()
            .chain([(record_file.clone(), &[][..])].iter())
        {
            if dir.join(relative).exists() {
                return Err(format!(
                    "`{}` exists already in the pack; choose another `to`",
                    relative
                ));
            }
        }
        let text = record.to_text();
        let mut written = Vec::new();
        for (relative, data) in targets
            .iter()
            .map(|(r, d)| (r.clone(), *d))
            .chain([(record_file.clone(), text.as_bytes())])
        {
            let target = dir.join(&relative);
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
            written.push(relative);
        }
        let files: Vec<String> = written
            .iter()
            .filter(|f| **f != record_file)
            .cloned()
            .collect();
        let total: u64 = targets.iter().map(|(_, d)| d.len() as u64).sum();
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
        });
        let Some(open) = self.project.as_mut() else {
            return Ok(answer);
        };
        open.assets.insert(
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
        open.provided.remove(to);
        let all = bounds(&open.assets);
        self.session.set_model_bounds(all);
        Ok(answer)
    }
}
