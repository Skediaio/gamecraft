//! `pack.json`, the manifest of a pack, and the order packs load in.

use crate::ProjectError;
use crate::paths::is_plain_name;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The version of the `pack.json` format this crate reads and writes (`gamecraft_pack`).
pub const PACK_FORMAT: u32 = 1;
/// Most packs one project loads, itself and its dependencies together.
pub const MAX_PACKS: usize = 64;

/// The manifest of a pack (`pack.json`). A game is a pack with `worlds`; a mod is a pack that
/// depends on the game and changes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The format version, [`PACK_FORMAT`].
    pub gamecraft_pack: u32,
    /// The pack's id: 1 to 64 ASCII letters, digits, `-` and `_`. Other packs depend on it by
    /// this id.
    pub id: String,
    /// The pack's name for people. Default empty.
    #[serde(default)]
    pub name: String,
    /// The pack's version, as text (`0.1.0`). Default `0.1.0`.
    #[serde(default = "default_version")]
    pub version: String,
    /// The packs this one is layered over, in the order given. Default none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends: Vec<Dependency>,
    /// The worlds this pack brings (folders under `worlds/`). Default none.
    #[serde(default)]
    pub worlds: Vec<String>,
    /// The licence of the pack's content (an SPDX id). Default none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Who made it. Default none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
}

/// A pack another pack is layered over.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    /// The pack's id.
    pub id: String,
    /// The versions that fit, as text (`>=0.1`); not checked yet. Default none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Where the pack's folder is, relative to the folder of the pack that depends on it.
    /// Default `packs/<id>` inside the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

impl Manifest {
    /// A manifest of a new pack with one world and no dependencies.
    pub fn new(id: &str, name: &str, worlds: Vec<String>) -> Manifest {
        Manifest {
            gamecraft_pack: PACK_FORMAT,
            id: id.to_string(),
            name: name.to_string(),
            version: default_version(),
            depends: Vec::new(),
            worlds,
            license: None,
            authors: Vec::new(),
        }
    }

    /// Reads and checks a `pack.json` (`file` names it in errors): the format version, a
    /// plain id, plain world names, dependencies with plain ids, none twice and not itself.
    pub fn parse(text: &str, file: &str) -> Result<Manifest, ProjectError> {
        let probe: serde_json::Value = serde_json::from_str(text).map_err(|e| {
            ProjectError::in_file(
                file,
                format!("not valid JSON: {e}; a pack.json is an object"),
            )
        })?;
        match probe
            .get("gamecraft_pack")
            .and_then(serde_json::Value::as_u64)
        {
            Some(v) if v == u64::from(PACK_FORMAT) => {}
            _ => {
                return Err(ProjectError::in_file(
                    file,
                    format!(
                        "`gamecraft_pack` must be {PACK_FORMAT}, the version of the pack format \
                         this engine reads"
                    ),
                ));
            }
        }
        let manifest: Manifest = serde_json::from_value(probe)
            .map_err(|e| ProjectError::in_file(file, format!("not a valid pack.json: {e}")))?;
        manifest.validate(file)?;
        Ok(manifest)
    }

    /// Checks the rules of [`Manifest::parse`].
    pub fn validate(&self, file: &str) -> Result<(), ProjectError> {
        let plain = "1 to 64 ASCII letters, digits, `-` and `_`";
        if !is_plain_name(&self.id) {
            return Err(ProjectError::in_file(
                file,
                format!("the pack id {:?} is not a plain name: {plain}", self.id),
            ));
        }
        for (i, world) in self.worlds.iter().enumerate() {
            if !is_plain_name(world) {
                return Err(ProjectError::in_file(
                    file,
                    format!("worlds[{i}] {world:?} is not a plain name: {plain}"),
                ));
            }
            if self.worlds.iter().take(i).any(|w| w == world) {
                return Err(ProjectError::in_file(
                    file,
                    format!("the world {world:?} is listed twice in `worlds`"),
                ));
            }
        }
        for (i, dep) in self.depends.iter().enumerate() {
            if !is_plain_name(&dep.id) {
                return Err(ProjectError::in_file(
                    file,
                    format!("depends[{i}].id {:?} is not a plain name: {plain}", dep.id),
                ));
            }
            if dep.id == self.id {
                return Err(ProjectError::in_file(
                    file,
                    format!("the pack `{}` depends on itself", self.id),
                ));
            }
            if self.depends.iter().take(i).any(|d| d.id == dep.id) {
                return Err(ProjectError::in_file(
                    file,
                    format!("the pack `{}` is listed twice in `depends`", dep.id),
                ));
            }
        }
        Ok(())
    }

    /// The manifest as the text of a `pack.json`: pretty, fields in a fixed order, a newline
    /// at the end.
    pub fn to_text(&self) -> Result<String, ProjectError> {
        crate::text::pretty_typed(self)
    }

    /// Adds a dependency on `id` at `path` (relative to this pack's folder), or changes the
    /// path of an existing one.
    pub fn add_dependency(&mut self, id: &str, path: &str) -> Result<(), ProjectError> {
        if !is_plain_name(id) {
            return Err(ProjectError::new(format!(
                "the pack id {id:?} is not a plain name: 1 to 64 ASCII letters, digits, `-` and `_`"
            )));
        }
        if id == self.id {
            return Err(ProjectError::new(format!(
                "the pack `{id}` cannot depend on itself"
            )));
        }
        match self.depends.iter_mut().find(|d| d.id == id) {
            Some(dep) => dep.path = Some(path.to_string()),
            None => self.depends.push(Dependency {
                id: id.to_string(),
                version: None,
                path: Some(path.to_string()),
            }),
        }
        Ok(())
    }
}

/// The order packs load in: every dependency before the packs that depend on it, `root`
/// last, dependencies in the order each manifest lists them.
///
/// A dependency that is not in `manifests` is an error naming the chain that needs it
/// (`mod -> game -> nature`), and so is a cycle (`a -> b -> a`), or more than [`MAX_PACKS`]
/// packs.
pub fn dependency_order(
    root: &str,
    manifests: &BTreeMap<String, Manifest>,
) -> Result<Vec<String>, ProjectError> {
    let mut order = Vec::new();
    let mut chain = Vec::new();
    visit(root, manifests, &mut chain, &mut order)?;
    Ok(order)
}

fn visit(
    id: &str,
    manifests: &BTreeMap<String, Manifest>,
    chain: &mut Vec<String>,
    order: &mut Vec<String>,
) -> Result<(), ProjectError> {
    if order.iter().any(|o| o == id) {
        return Ok(());
    }
    if let Some(start) = chain.iter().position(|c| c == id) {
        let mut cycle: Vec<&str> = chain.iter().skip(start).map(String::as_str).collect();
        cycle.push(id);
        return Err(ProjectError::new(format!(
            "the packs depend on each other in a cycle: {}; remove one of these dependencies",
            cycle.join(" -> ")
        )));
    }
    let Some(manifest) = manifests.get(id) else {
        let mut needed: Vec<&str> = chain.iter().map(String::as_str).collect();
        needed.push(id);
        return Err(ProjectError::new(format!(
            "the pack `{id}` is missing (needed by {}); give its folder with `path` in the \
             dependency, or put it in the project's packs/{id}",
            needed.join(" -> ")
        )));
    };
    if chain.len() >= MAX_PACKS || order.len() >= MAX_PACKS {
        return Err(ProjectError::new(format!(
            "a project loads at most {MAX_PACKS} packs"
        )));
    }
    chain.push(id.to_string());
    for dep in &manifest.depends {
        visit(&dep.id, manifests, chain, order)?;
    }
    chain.pop();
    order.push(id.to_string());
    Ok(())
}
