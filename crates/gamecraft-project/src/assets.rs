//! The assets of a pack and their licence records (AGENTS.md rule 15: CC0 or CC-BY only, with
//! licence, author and source recorded).
//!
//! An asset is a file under the pack's `assets/` folder. Each one has a record beside it,
//! `assets/<path>.license.json`: `{"license": "CC0-1.0" | "CC-BY-4.0", "author", "source_url",
//! "title"?, "changes"?}`. The files a `.gltf` model refers to (its buffers and images) belong to
//! the model and are covered by its record, except the textures of the pack's texture store
//! (`assets/textures/`, [`crate::textures`]): those are assets of their own with a record each.
//! Loading a pack refuses an asset without a record, a record with another licence, a record
//! whose file does not exist, an asset path that is not a plain path and a model that refers to
//! a file that is missing, each with a sentence that names the file.

use crate::ProjectError;
use crate::model::{folder_of, references, resolve_uri};
use crate::pack::PackSource;
use crate::textures::is_texture;
use gamecraft_world::{is_asset_path, is_model_path};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The folder of a pack that holds its assets.
pub const ASSETS_DIR: &str = "assets";
/// What a licence record's file name adds to its asset's.
pub const RECORD_SUFFIX: &str = ".license.json";
/// Longest author, title or `changes` text of a record, in characters.
pub const MAX_RECORD_TEXT_CHARS: usize = 2000;

/// The licences an asset may have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum License {
    /// Creative Commons Zero 1.0: no rights reserved, no attribution needed.
    #[serde(rename = "CC0-1.0")]
    Cc0,
    /// Creative Commons Attribution 4.0: a game must credit the author.
    #[serde(rename = "CC-BY-4.0")]
    CcBy,
}

impl License {
    /// Every licence an asset may have.
    pub const ALL: [License; 2] = [License::Cc0, License::CcBy];

    /// Its SPDX id, as a record writes it: `CC0-1.0` or `CC-BY-4.0`.
    pub fn id(self) -> &'static str {
        match self {
            License::Cc0 => "CC0-1.0",
            License::CcBy => "CC-BY-4.0",
        }
    }

    /// Its name for people: `CC0 1.0` or `CC BY 4.0`.
    pub fn name(self) -> &'static str {
        match self {
            License::Cc0 => "CC0 1.0",
            License::CcBy => "CC BY 4.0",
        }
    }

    /// The licence's deed.
    pub fn url(self) -> &'static str {
        match self {
            License::Cc0 => "https://creativecommons.org/publicdomain/zero/1.0/",
            License::CcBy => "https://creativecommons.org/licenses/by/4.0/",
        }
    }

    /// The licence of an SPDX id; the error names the value and the licences allowed.
    pub fn parse(id: &str) -> Result<License, String> {
        License::ALL
            .into_iter()
            .find(|l| l.id() == id)
            .ok_or_else(|| {
                format!(
                    "the licence {id:?} is not allowed; assets are CC0-1.0 or CC-BY-4.0 only \
                     (AGENTS.md rule 15)"
                )
            })
    }

    /// Whether a game must credit the author.
    pub fn needs_attribution(self) -> bool {
        self == License::CcBy
    }
}

/// The licence record of an asset (`assets/<path>.license.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRecord {
    /// `CC0-1.0` or `CC-BY-4.0`.
    pub license: License,
    /// Who made it, as they want to be credited.
    pub author: String,
    /// Where it was found (`https://...`), or where it was made.
    pub source_url: String,
    /// Its name for people.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// What was changed from the source (CC BY asks for it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<String>,
}

impl AssetRecord {
    /// Reads a record; `file` names it in errors.
    pub fn parse(text: &str, file: &str) -> Result<AssetRecord, ProjectError> {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| ProjectError::in_file(file, format!("not valid JSON: {e}")))?;
        if let Some(license) = value.get("license").and_then(|l| l.as_str()) {
            License::parse(license).map_err(|e| ProjectError::in_file(file, e))?;
        }
        let record: AssetRecord = serde_json::from_value(value).map_err(|e| {
            ProjectError::in_file(
                file,
                format!(
                    "not a licence record ({e}); a record is {{\"license\": \"CC0-1.0\" | \
                     \"CC-BY-4.0\", \"author\": ..., \"source_url\": ..., \"title\"?, \
                     \"changes\"?}}"
                ),
            )
        })?;
        record
            .validate()
            .map_err(|e| ProjectError::in_file(file, e))?;
        Ok(record)
    }

    /// Checks the texts: an author and a source, none too long.
    pub fn validate(&self) -> Result<(), String> {
        for (field, text) in [
            ("author", Some(&self.author)),
            ("source_url", Some(&self.source_url)),
            ("title", self.title.as_ref()),
            ("changes", self.changes.as_ref()),
        ] {
            let Some(text) = text else { continue };
            if (field == "author" || field == "source_url") && text.trim().is_empty() {
                return Err(format!(
                    "`{field}` is empty; a record names who made the asset and where it came from"
                ));
            }
            let chars = text.chars().count();
            if chars > MAX_RECORD_TEXT_CHARS {
                return Err(format!(
                    "`{field}` is {chars} characters long; at most {MAX_RECORD_TEXT_CHARS}"
                ));
            }
        }
        Ok(())
    }

    /// The record as a file: pretty JSON with a final newline.
    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }

    /// The credit line of an asset at `path`: `"Title" by Author, CC BY 4.0, source.`
    pub fn credit(&self, path: &str) -> String {
        let title = self.title.as_deref().unwrap_or(path);
        let changes = self
            .changes
            .as_deref()
            .map(|c| format!(" Changes: {c}."))
            .unwrap_or_default();
        format!(
            "\"{title}\" by {}, {} ({}), from {}.{changes}",
            self.author,
            self.license.name(),
            self.license.url(),
            self.source_url
        )
    }
}

/// The pack-relative path of an asset's licence record: `assets/<path>.license.json`.
pub fn record_path(asset: &str) -> String {
    format!("{ASSETS_DIR}/{asset}{RECORD_SUFFIX}")
}

/// One asset of a pack, with its record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackAsset {
    /// Its path inside `assets/` (`models/crate.glb`).
    pub path: String,
    /// Its licence record.
    pub record: AssetRecord,
    /// The pack-relative files it reads: itself, then the files it refers to, in order (a
    /// `.gltf`'s buffers and images; the textures of the store included).
    pub files: Vec<String>,
    /// The bytes of its own files: itself and the files beside it it refers to. A texture of
    /// the store it refers to is not counted here: it is an asset of its own, counted once.
    pub bytes: u64,
}

impl PackAsset {
    /// Whether it is a model (`.glb` or `.gltf`).
    pub fn is_model(&self) -> bool {
        is_model_path(&self.path)
    }

    /// `model`, `texture` (a file of the pack's texture store) or `file`, as listings say it.
    pub fn kind(&self) -> &'static str {
        if self.is_model() {
            "model"
        } else if is_texture(&self.path) {
            "texture"
        } else {
            "file"
        }
    }
}

/// The assets of a pack, checked: every file under `assets/` is an asset with a record or a
/// file a model refers to, every record has its file, every record is valid, every file a model
/// refers to exists. Sorted by path. `source.assets` lists the files with their sizes; the
/// records' and the model documents' texts (a `.gltf` as it is, a `.glb`'s JSON chunk) are in
/// `source.files`.
pub fn check_assets(source: &PackSource) -> Result<Vec<PackAsset>, ProjectError> {
    let file = |path: &str| format!("{}/{path}", source.location);
    let prefix = format!("{ASSETS_DIR}/");
    let mut records = BTreeMap::new();
    let mut plain = BTreeMap::new();
    for (path, size) in &source.assets {
        let Some(inside) = path.strip_prefix(&prefix) else {
            return Err(ProjectError::in_file(
                &file(path),
                "is listed as an asset but is not inside the pack's assets/ folder",
            ));
        };
        if let Some(asset) = inside.strip_suffix(RECORD_SUFFIX) {
            records.insert(asset.to_string(), path.clone());
        } else {
            if !is_asset_path(inside) {
                return Err(ProjectError::in_file(
                    &file(path),
                    "is not a plain asset path: ASCII letters, digits, `-`, `_` and `.`, \
                     folders separated by `/`; rename it",
                ));
            }
            plain.insert(inside.to_string(), *size);
        }
    }
    // The files the model documents refer to belong to the models (their paths inside assets/,
    // `..` resolved) when they lie in the model's own folder; the textures of the store and
    // files anywhere else have records of their own.
    let mut parts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut referred = BTreeSet::new();
    for asset in plain.keys().filter(|p| is_model_path(p)) {
        let path = format!("{prefix}{asset}");
        let text = source.files.get(&path).ok_or_else(|| {
            ProjectError::in_file(
                &file(&path),
                "was not read; the host reads every model document (a .gltf's text, a .glb's \
                 JSON chunk)",
            )
        })?;
        let refs =
            references(text.as_bytes()).map_err(|e| ProjectError::in_file(&file(&path), e))?;
        let folder = folder_of(asset);
        let mut list = Vec::new();
        for r in refs {
            let inside =
                resolve_uri(folder, &r).map_err(|e| ProjectError::in_file(&file(&path), e))?;
            if !plain.contains_key(&inside) {
                return Err(ProjectError::in_file(
                    &file(&path),
                    format!("refers to `{prefix}{inside}`, which does not exist in the pack"),
                ));
            }
            if !is_texture(&inside) && inside.starts_with(folder) {
                referred.insert(inside.clone());
            }
            list.push(inside);
        }
        parts.insert(asset.clone(), list);
    }
    for (asset, record_file) in &records {
        if !plain.contains_key(asset) {
            return Err(ProjectError::in_file(
                &file(record_file),
                format!(
                    "is the licence record of `{prefix}{asset}`, which does not exist; a record \
                     sits beside its asset with the asset's file name plus {RECORD_SUFFIX}"
                ),
            ));
        }
    }
    let mut out = Vec::new();
    for (asset, size) in &plain {
        let record_file = record_path(asset);
        let Some(text) = source.files.get(&record_file) else {
            if referred.contains(asset) && !is_model_path(asset) {
                continue;
            }
            let what = if is_texture(asset) {
                "a texture of the pack's texture store (textures/) has a record of its own"
            } else {
                "every asset needs"
            };
            return Err(ProjectError::in_file(
                &file(&format!("{prefix}{asset}")),
                format!(
                    "has no licence record; {what} `{record_file}` with \
                     {{\"license\": \"CC0-1.0\" | \"CC-BY-4.0\", \"author\": ..., \
                     \"source_url\": ...}} (asset.import writes it)"
                ),
            ));
        };
        let record = AssetRecord::parse(text, &file(&record_file))?;
        let mut files = vec![format!("{prefix}{asset}")];
        let mut bytes = *size;
        for part in parts.get(asset).into_iter().flatten() {
            files.push(format!("{prefix}{part}"));
            if !is_texture(part) {
                bytes = bytes.saturating_add(plain.get(part).copied().unwrap_or(0));
            }
        }
        out.push(PackAsset {
            path: asset.clone(),
            record,
            files,
            bytes,
        });
    }
    Ok(out)
}

/// The attribution text a game must show: every CC-BY asset with its title, author, licence
/// and source, one line each, in path order. Assets under CC0 need none.
pub fn credits<'a>(assets: impl IntoIterator<Item = &'a PackAsset>) -> String {
    let lines: Vec<String> = assets
        .into_iter()
        .filter(|a| a.record.license.needs_attribution())
        .map(|a| a.record.credit(&a.path))
        .collect();
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}
