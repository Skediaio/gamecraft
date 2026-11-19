//! The project index of the CLI's host: `gamecraft-index` (Turso) behind the automation's
//! [`Indexer`] trait.

use gamecraft_automation::{Indexer, ProjectIndex};
use gamecraft_index::Index;
use gamecraft_project::Project;
use gamecraft_world::World;
use serde_json::{Value, json};
use std::path::Path;

/// Opens a Turso index for each project the host opens.
pub struct TursoIndexer;

impl Indexer for TursoIndexer {
    fn open(&mut self, project_dir: Option<&Path>) -> Result<Box<dyn ProjectIndex>, String> {
        Ok(Box::new(TursoIndex(Index::open(project_dir)?)))
    }
}

/// One project's index.
struct TursoIndex(Index);

impl ProjectIndex for TursoIndex {
    fn rebuild(&mut self, world: &World, project: &Project) -> Result<Value, String> {
        let done = self.0.rebuild(world, Some(project))?;
        Ok(json!({"entities": done.entities, "prefabs": done.prefabs}))
    }

    fn query(&mut self, sql: &str, max_rows: usize) -> Result<Value, String> {
        let answer = self.0.query(sql, max_rows)?;
        Ok(json!({
            "columns": answer.columns,
            "rows": answer.rows,
            "truncated": answer.truncated,
        }))
    }

    fn location(&self) -> Option<String> {
        self.0.path().map(|p| p.display().to_string())
    }
}
