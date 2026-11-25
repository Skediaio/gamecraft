//! The Turso store behind the index (not in the browser).
//!
//! Turso's API is async, but a local database needs no runtime: a step that waits for its
//! file returns `Pending` after doing the I/O itself, and the next poll goes on; nothing in the
//! API promises a wake-up. [`block_on`] is the smallest executor that fits: it polls the future
//! again until it is ready, without relying on one.

use crate::{Answer, Rebuilt};
use gamecraft_project::Project;
use gamecraft_world::{Component, World};
use serde_json::{Value, json};
use std::future::Future;
use std::path::Path;
use std::task::{Context, Poll, Waker};
use turso::{Builder, Connection, Database};

/// Most polls of one database call before it is given up (a local call needs a handful).
const MAX_POLLS: u64 = 100_000_000;

/// Runs `future` to completion on this thread.
fn block_on<F: Future>(future: F) -> Result<F::Output, String> {
    let mut future = std::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..MAX_POLLS {
        if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return Ok(out);
        }
        std::thread::yield_now();
    }
    Err(
        "the index database did not answer; delete the .gamecraft-index folder and open the \
         project again"
            .into(),
    )
}

/// A database call as a sentence error.
fn run<T>(what: &str, future: impl Future<Output = turso::Result<T>>) -> Result<T, String> {
    block_on(future)?.map_err(|e| format!("the index cannot {what}: {e}"))
}

const SCHEMA: &[&str] = &[
    "DROP TABLE IF EXISTS entities",
    "DROP TABLE IF EXISTS entity_tags",
    "DROP TABLE IF EXISTS entity_components",
    "DROP TABLE IF EXISTS prefabs",
    "CREATE TABLE entities (id INTEGER PRIMARY KEY, key TEXT, name TEXT NOT NULL, layer TEXT, \
     pack TEXT, prefab TEXT, parent INTEGER, x REAL NOT NULL, y REAL NOT NULL, z REAL NOT NULL, \
     rotation_y_deg REAL NOT NULL, scale REAL NOT NULL, tags TEXT NOT NULL, components TEXT NOT NULL)",
    "CREATE TABLE entity_tags (entity_id INTEGER NOT NULL, tag TEXT NOT NULL)",
    "CREATE TABLE entity_components (entity_id INTEGER NOT NULL, type TEXT NOT NULL, data TEXT NOT NULL)",
    "CREATE TABLE prefabs (id TEXT PRIMARY KEY, name TEXT NOT NULL, pack TEXT, components TEXT NOT NULL, \
     instances INTEGER NOT NULL)",
    "CREATE INDEX entity_tags_tag ON entity_tags (tag)",
    "CREATE INDEX entity_components_type ON entity_components (type)",
];

/// The database, a connection that writes and one that may only read.
pub(crate) struct Store {
    _db: Database,
    writer: Connection,
    reader: Connection,
}

impl Store {
    /// Opens the database at `path` (in memory when `None`) and makes the tables anew.
    pub(crate) fn open(path: Option<&Path>) -> Result<Store, String> {
        let location = match path {
            Some(p) => p
                .to_str()
                .ok_or_else(|| format!("the index path `{}` is not UTF-8", p.display()))?
                .to_string(),
            None => ":memory:".to_string(),
        };
        let db = run("open", Builder::new_local(&location).build())?;
        let writer = db
            .connect()
            .map_err(|e| format!("the index cannot connect: {e}"))?;
        for statement in SCHEMA {
            run("make its tables", writer.execute(*statement, ()))?;
        }
        let reader = db
            .connect()
            .map_err(|e| format!("the index cannot connect: {e}"))?;
        run(
            "make a read-only connection",
            reader.execute("PRAGMA query_only = 1", ()),
        )?;
        Ok(Store {
            _db: db,
            writer,
            reader,
        })
    }

    pub(crate) fn rebuild(
        &mut self,
        world: &World,
        project: Option<&Project>,
    ) -> Result<Rebuilt, String> {
        run("begin a rebuild", self.writer.execute("BEGIN", ()))?;
        match self.fill(world, project) {
            Ok(done) => {
                run("commit a rebuild", self.writer.execute("COMMIT", ()))?;
                Ok(done)
            }
            Err(e) => {
                let _ = run("roll back", self.writer.execute("ROLLBACK", ()));
                Err(e)
            }
        }
    }

    fn fill(&mut self, world: &World, project: Option<&Project>) -> Result<Rebuilt, String> {
        let w = &self.writer;
        for table in ["entities", "entity_tags", "entity_components", "prefabs"] {
            run(
                "empty a table",
                w.execute(format!("DELETE FROM {table}"), ()),
            )?;
        }
        let mut entity = run(
            "prepare an insert",
            w.prepare(
                "INSERT INTO entities (id, key, name, layer, pack, prefab, parent, x, y, z, \
                 rotation_y_deg, scale, tags, components) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, \
                 ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            ),
        )?;
        let mut tag = run(
            "prepare an insert",
            w.prepare("INSERT INTO entity_tags (entity_id, tag) VALUES (?1, ?2)"),
        )?;
        let mut component = run(
            "prepare an insert",
            w.prepare("INSERT INTO entity_components (entity_id, type, data) VALUES (?1, ?2, ?3)"),
        )?;
        let text = |s: &str| turso::Value::Text(s.to_string());
        let opt = |s: Option<&str>| s.map_or(turso::Value::Null, text);
        let int = |n: u64| turso::Value::Integer(i64::try_from(n).unwrap_or(i64::MAX));
        let mut entities = 0;
        for e in world.entities() {
            let source = project.and_then(|p| p.provenance().entities.get(&e.id));
            let layer = match project {
                Some(p) => Some(p.layer_of(e)),
                None => e.layer.as_deref(),
            };
            let kinds: Vec<&str> = e.components.iter().map(Component::kind).collect();
            let [x, y, z] = e.position;
            run(
                "write an entity",
                entity.execute(vec![
                    int(e.id.0),
                    opt(source.map(|s| s.key.as_str())),
                    text(&e.name),
                    opt(layer),
                    opt(source.map(|s| s.pack.as_str())),
                    opt(e.prefab.as_deref()),
                    e.parent.map_or(turso::Value::Null, |p| int(p.0)),
                    turso::Value::Real(x),
                    turso::Value::Real(y),
                    turso::Value::Real(z),
                    turso::Value::Real(e.rotation_y_deg),
                    turso::Value::Real(e.scale.largest()),
                    text(&json!(e.tags()).to_string()),
                    text(&json!(kinds).to_string()),
                ]),
            )?;
            for t in e.tags() {
                run("write a tag", tag.execute(vec![int(e.id.0), text(t)]))?;
            }
            for c in &e.components {
                let data = serde_json::to_string(c).map_err(|e| e.to_string())?;
                run(
                    "write a component",
                    component.execute(vec![int(e.id.0), text(c.kind()), text(&data)]),
                )?;
            }
            entities += 1;
        }
        let mut prefab = run(
            "prepare an insert",
            w.prepare(
                "INSERT INTO prefabs (id, name, pack, components, instances) VALUES (?1, ?2, \
                 ?3, ?4, ?5)",
            ),
        )?;
        for p in &world.prefabs {
            let kinds: Vec<&str> = p.components.iter().map(Component::kind).collect();
            let pack = project.and_then(|pr| pr.provenance().prefabs.get(&p.id));
            run(
                "write a prefab",
                prefab.execute(vec![
                    text(&p.id),
                    text(&p.name),
                    opt(pack.map(String::as_str)),
                    text(&json!(kinds).to_string()),
                    int(world.prefab_instances(&p.id).len() as u64),
                ]),
            )?;
        }
        Ok(Rebuilt {
            entities,
            prefabs: world.prefabs.len(),
        })
    }

    pub(crate) fn query(&self, sql: &str, max_rows: usize) -> Result<Answer, String> {
        let mut statement = block_on(self.reader.prepare(sql))?
            .map_err(|e| format!("the query is not valid SQL for the index: {e}"))?;
        let columns = statement.column_names();
        let mut rows_out = Vec::new();
        let mut rows =
            block_on(statement.query(()))?.map_err(|e| format!("the query failed: {e}"))?;
        let mut truncated = false;
        loop {
            let Some(row) = block_on(rows.next())?.map_err(|e| format!("the query failed: {e}"))?
            else {
                break;
            };
            if rows_out.len() >= max_rows {
                truncated = true;
                break;
            }
            let mut values = Vec::with_capacity(row.column_count());
            for i in 0..row.column_count() {
                let v = row
                    .get_value(i)
                    .map_err(|e| format!("the query failed: {e}"))?;
                values.push(to_json(v));
            }
            rows_out.push(values);
        }
        Ok(Answer {
            columns,
            rows: rows_out,
            truncated,
        })
    }
}

/// A database value as JSON.
fn to_json(v: turso::Value) -> Value {
    match v {
        turso::Value::Null => Value::Null,
        turso::Value::Integer(n) => json!(n),
        turso::Value::Real(f) => serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number),
        turso::Value::Text(s) => Value::String(s),
        turso::Value::Blob(b) => {
            Value::String(b.iter().map(|byte| format!("{byte:02x}")).collect())
        }
    }
}
