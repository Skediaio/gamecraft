//! The derived, throw-away SQL index of a project (layer L4, `docs/design/projects.md`, *The
//! index*): one row per entity and prefab of the open world with its id, string id, name,
//! tags, components, layer, pack and position, so an agent finds "every enemy within 100 m
//! of the village" in one query.
//!
//! The store is [Turso](https://github.com/tursodatabase/turso), the pure-Rust,
//! SQLite-compatible database, kept in `.gamecraft-index/index.db` beside the project (or in
//! memory). It is rebuilt from the world and thrown away at will: deleting it loses nothing
//! (rule 2). [`Index::query`] runs one read-only `SELECT` ([`check_select`]) on a connection
//! set to `query_only`.
//!
//! Native only: in the browser [`Index::open`] says the index is not available there.
//!
//! Tables:
//!
//! - `entities(id, key, name, layer, pack, prefab, parent, x, y, z, rotation_y_deg, scale,
//!   tags, components)`: `id` the runtime id, `key` the string id in its pack (null until
//!   saved), `tags` and `components` JSON arrays of the tags and the component types, and
//!   `scale` the largest of an entity's scales along its axes (one number when they are equal).
//! - `entity_tags(entity_id, tag)`: one row per tag of an entity.
//! - `entity_components(entity_id, type, data)`: one row per component, `data` its JSON.
//! - `prefabs(id, name, pack, components, instances)`.

mod sql;

pub use sql::check_select;

use gamecraft_project::Project;
use gamecraft_world::World;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The folder of the index beside a project.
pub const INDEX_DIR: &str = ".gamecraft-index";
/// The database file inside [`INDEX_DIR`].
pub const INDEX_FILE: &str = "index.db";
/// Most rows one query returns.
pub const MAX_ROWS: usize = 1000;

/// What a rebuild did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rebuilt {
    /// Entity rows written.
    pub entities: usize,
    /// Prefab rows written.
    pub prefabs: usize,
}

/// The answer of a query.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// The column names, in order.
    pub columns: Vec<String>,
    /// The rows, each a list of values in column order (numbers, strings, null; a blob as
    /// a hex string).
    pub rows: Vec<Vec<Value>>,
    /// Whether more rows matched than were returned.
    pub truncated: bool,
}

/// The index of one project.
pub struct Index {
    #[cfg(not(target_arch = "wasm32"))]
    store: native::Store,
    path: Option<PathBuf>,
}

impl Index {
    /// Opens (creating it if needed) the index beside the project folder `project_dir`, in
    /// `.gamecraft-index/index.db`, or in memory when `project_dir` is `None`. Whatever an
    /// older index held is dropped: the tables are made anew and filled by
    /// [`Index::rebuild`].
    pub fn open(project_dir: Option<&Path>) -> Result<Index, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = match project_dir {
                Some(dir) => {
                    let folder = dir.join(INDEX_DIR);
                    std::fs::create_dir_all(&folder).map_err(|e| {
                        format!("cannot create the index folder `{}`: {e}", folder.display())
                    })?;
                    // The index is derived: git never needs it.
                    let ignore = folder.join(".gitignore");
                    if !ignore.exists() {
                        std::fs::write(&ignore, "*\n")
                            .map_err(|e| format!("cannot write `{}`: {e}", ignore.display()))?;
                    }
                    Some(folder.join(INDEX_FILE))
                }
                None => None,
            };
            let store = native::Store::open(path.as_deref())?;
            Ok(Index { store, path })
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = project_dir;
            Err(UNAVAILABLE.to_string())
        }
    }

    /// Where the database file is; `None` in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Replaces every row with the entities and prefabs of `world`, with their string ids,
    /// layers and packs from `project` (null without one), in one transaction.
    pub fn rebuild(&mut self, world: &World, project: Option<&Project>) -> Result<Rebuilt, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.store.rebuild(world, project)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (world, project);
            Err(UNAVAILABLE.to_string())
        }
    }

    /// Runs one `SELECT` ([`check_select`]) and returns at most `max_rows` rows (at most
    /// [`MAX_ROWS`]).
    pub fn query(&self, sql: &str, max_rows: usize) -> Result<Answer, String> {
        let sql = check_select(sql)?;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.store.query(sql, max_rows.min(MAX_ROWS))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (sql, max_rows);
            Err(UNAVAILABLE.to_string())
        }
    }
}

/// The error in the browser.
#[cfg(target_arch = "wasm32")]
const UNAVAILABLE: &str = "the project index is native only: the browser has no files and no \
                           database; run gamecraft-cli for project.query";

#[cfg(not(target_arch = "wasm32"))]
mod native;
