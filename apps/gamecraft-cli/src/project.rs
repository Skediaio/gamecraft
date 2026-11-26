//! `project new|open|info|export|query|bake`: projects on disk from the command line.

use crate::args::Args;
use crate::commands::{absolute, exec, folder, host, opened, print, print_json};
use crate::script::Script;
use crate::{CliError, Res, help};
use gamecraft_project::{BakeRecord, DEFAULT_CELL_SIZE_M};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

/// How many steps between two lines of progress on stderr.
const PROGRESS_EVERY: usize = 5000;

/// `project <action> ...`.
pub fn project(raw: &[String]) -> Res {
    let Some((action, rest)) = raw.split_first() else {
        return print(help::PROJECT);
    };
    match action.as_str() {
        "--help" | "-h" => print(help::PROJECT),
        "new" => new(rest),
        "open" => open(rest, false),
        "info" => open(rest, true),
        "export" => export(rest),
        "query" => query(rest),
        "bake" => bake(rest),
        other => Err(CliError::Usage(format!(
            "unknown action `{other}`; the actions are: new, open, info, export, query, bake"
        ))),
    }
}

/// The project folder of the first positional, and the folder that holds it.
fn project_dir(a: &Args, what: &str) -> Result<(String, PathBuf), CliError> {
    let dir = a.required_file(what)?.to_string();
    let parent = folder(&absolute(&dir)?);
    Ok((dir, parent))
}

/// `project new DIR --id ID --name NAME [--world W] [--from FILE]`.
fn new(raw: &[String]) -> Res {
    let a = Args::parse(
        raw,
        &["--id", "--name", "--world", "--from", "--cell-size-m"],
        &[],
    )?;
    if a.help() {
        return print(help::PROJECT);
    }
    let dir = absolute(a.required_file("the project folder DIR to make")?)?;
    let id = a
        .value("--id")?
        .ok_or_else(|| CliError::Usage("give the pack's id with --id ID".into()))?;
    let name = a.value("--name")?.unwrap_or(id);
    let from = a.value("--from")?.map(absolute).transpose()?;
    let mut host = host(from.as_deref().map(folder), Some(folder(&dir)));
    if let Some(file) = &from {
        exec(
            &mut host,
            "file.open",
            &json!({"path": file.display().to_string()}),
        )?;
    }
    let mut params = json!({"path": dir.display().to_string(), "id": id, "name": name});
    if let (Some(world), Some(map)) = (a.value("--world")?, params.as_object_mut()) {
        map.insert("world".into(), json!(world));
    }
    if let Some(size) = a.value("--cell-size-m")? {
        let size: f64 = size.parse().map_err(|_| {
            CliError::Usage(format!(
                "`--cell-size-m {size}`: the side of a cell in metres, 16 to 16384 (512 is usual)"
            ))
        })?;
        if let Some(map) = params.as_object_mut() {
            map.insert("cell_size_m".into(), json!(size));
        }
    }
    print_json(&exec(&mut host, "project.new", &params)?)
}

/// `project open DIR` (what opened) and `project info DIR` (project.info).
fn open(raw: &[String], info: bool) -> Res {
    let a = Args::parse(raw, &["--world"], &[])?;
    if a.help() {
        return print(help::PROJECT);
    }
    let (dir, parent) = project_dir(&a, "the project folder DIR")?;
    // The folder that holds the project is the read root (dependencies beside it are found)
    // and the write root (the index goes into the project's .gamecraft-index).
    let mut host = host(Some(parent.clone()), Some(parent));
    let mut params = json!({"path": absolute(&dir)?.display().to_string()});
    if let (Some(world), Some(map)) = (a.value("--world")?, params.as_object_mut()) {
        map.insert("world".into(), json!(world));
    }
    let opened = exec(&mut host, "project.open", &params)?;
    if info {
        return print_json(&exec(&mut host, "project.info", &json!({}))?);
    }
    print_json(&opened)
}

/// `project export DIR --out FILE`.
fn export(raw: &[String]) -> Res {
    let a = Args::parse(raw, &["--out"], &[])?;
    if a.help() {
        return print(help::PROJECT);
    }
    let dir = a.required_file("the project folder DIR")?;
    let out = absolute(
        a.value("--out")?
            .ok_or_else(|| CliError::Usage("give the file to write with --out FILE".into()))?,
    )?;
    let mut host = opened(dir, Some(folder(&out)), None)?;
    print_json(&exec(
        &mut host,
        "project.export",
        &json!({"path": out.display().to_string(), "overwrite": true}),
    )?)
}

/// `project query DIR SQL [--limit N]`.
fn query(raw: &[String]) -> Res {
    let a = Args::parse(raw, &["--limit"], &[])?;
    if a.help() {
        return print(help::PROJECT);
    }
    let (Some(dir), Some(sql)) = (a.positionals.first(), a.positionals.get(1)) else {
        return Err(CliError::Usage(
            "give the project folder and one SELECT: project query DIR \"SELECT ...\"".into(),
        ));
    };
    if a.positionals.len() > 2 {
        return Err(CliError::Usage(
            "give the SQL as one argument, in quotes".into(),
        ));
    }
    let parent = folder(&absolute(dir)?);
    let mut host = host(Some(parent.clone()), Some(parent));
    exec(
        &mut host,
        "project.open",
        &json!({"path": absolute(dir)?.display().to_string()}),
    )?;
    let mut params = json!({"sql": sql});
    if let Some(limit) = a.value("--limit")? {
        let limit: u64 = limit.parse().map_err(|_| {
            CliError::Usage(format!(
                "`--limit {limit}` must be a whole number from 1 to 1000"
            ))
        })?;
        if let Some(map) = params.as_object_mut() {
            map.insert("limit".into(), json!(limit));
        }
    }
    print_json(&exec(&mut host, "project.query", &params)?)
}

/// `project bake SCRIPT --into DIR [--cell-size-m M] [--id ID --name NAME --world W]`.
///
/// Runs the script (the JSON list of steps `run --script` takes, read one step at a time) and
/// saves the result as a project of cells in one step: a new project DIR (which needs `--id`)
/// made after every step succeeded, or an existing one where the bake replaces only what a
/// script owns (the layers its steps add entities to, and the terrain edits it added: recorded
/// in `worlds/<world>/bake.json`), so hand edits in other layers survive. Nothing is written
/// while a step can still fail.
fn bake(raw: &[String]) -> Res {
    let a = Args::parse(
        raw,
        &["--into", "--cell-size-m", "--id", "--name", "--world"],
        &[],
    )?;
    if a.help() {
        return print(help::PROJECT);
    }
    let script = absolute(a.required_file("the script FILE to bake")?)?;
    let script = script.display().to_string();
    let into = absolute(
        a.value("--into")?
            .ok_or_else(|| CliError::Usage("give the project folder with --into DIR".into()))?,
    )?;
    let cell = match a.value("--cell-size-m")? {
        Some(text) => text.parse::<f64>().map_err(|_| {
            CliError::Usage(format!(
                "`--cell-size-m {text}`: the side of a cell in metres, 16 to 16384 (512 is usual)"
            ))
        })?,
        None => DEFAULT_CELL_SIZE_M,
    };
    let (id, name, world) = (a.value("--id")?, a.value("--name")?, a.value("--world")?);
    if into.exists() && !into.is_dir() {
        return Err(CliError::Failed(format!(
            "`{}` is not a folder",
            into.display()
        )));
    }
    let exists = into.join("pack.json").is_file();
    if !exists && id.is_none() {
        return Err(CliError::Usage(format!(
            "`{}` does not exist yet: give the pack's id with --id ID to make the project there",
            into.display()
        )));
    }
    let parent = folder(&into);
    let mut host = host(Some(parent.clone()), Some(parent));
    let started = Instant::now();

    // What the script owns: every layer a step names (the default layer is added below).
    let mut owned = BTreeSet::new();
    let mut reader = Script::open(&script)?;
    while let Some((_, _, step)) = reader.next_step()? {
        if let Some(layer) = step.params.get("layer").and_then(Value::as_str) {
            owned.insert(layer.to_string());
        }
    }
    drop(reader);

    let mut world_name = String::new();
    let mut before = 0usize;
    let mut record_path = None;
    // Where commands that name no layer put their entities: a new project's default layer is
    // `main`.
    let mut default_layer = "main".to_string();
    if exists {
        let mut params = json!({"path": into.display().to_string()});
        if let (Some(world), Some(map)) = (world, params.as_object_mut()) {
            map.insert("world".into(), json!(world));
        }
        let opened = exec(&mut host, "project.open", &params)?;
        world_name = text_of(&opened, "world", "project.open")?;
        let info = exec(&mut host, "project.info", &json!({}))?;
        default_layer = text_of(&info, "default_layer", "project.info")?;
        // What the last bake left: its layers are owned too, and its terrain edits go first.
        record_path = Some(BakeRecord::path(&into.join("worlds").join(&world_name)));
        let previous = match record_path.as_deref().filter(|p| p.is_file()) {
            Some(path) => {
                let text = std::fs::read(path).map_err(|e| {
                    CliError::Failed(format!("cannot read `{}`: {e}", path.display()))
                })?;
                Some(BakeRecord::parse(&text).map_err(CliError::Failed)?)
            }
            None => None,
        };
        let edits_now = edit_count(&mut host)?;
        match &previous {
            Some(record) => {
                owned.extend(record.layers.iter().cloned());
                let [first, last] = record.terrain_edits;
                if edits_now != last {
                    return Err(CliError::Failed(format!(
                        "the project has {edits_now} terrain edits where the last bake left \
                         {last}: a re-run would stack its own edits on edits made since. \
                         Remove them with terrain.remove_edit, or bake into another folder"
                    )));
                }
                for index in (first..last).rev() {
                    exec(&mut host, "terrain.remove_edit", &json!({"index": index}))?;
                }
                before = first;
            }
            None => before = edits_now,
        }
        remove_owned(&mut host, &owned, &default_layer)?;
    }
    owned.insert(default_layer);

    // The script, one step at a time; a failing step names its line and leaves nothing written.
    let mut reader = Script::open(&script)?;
    let mut steps = 0usize;
    while let Some((index, line, step)) = reader.next_step()? {
        host.execute(&step.id, &step.params).map_err(|e| {
            CliError::Failed(format!("{script}:{line}: step {index} `{}`: {e}", step.id))
        })?;
        steps += 1;
        if steps.is_multiple_of(PROGRESS_EVERY)
            && let Err(e) = writeln!(std::io::stderr(), "gamecraft-cli: bake: {steps} steps")
        {
            return Err(CliError::Failed(format!("cannot write the progress: {e}")));
        }
    }
    drop(reader);
    let after = edit_count(&mut host)?;
    let entities = host.session().document().entity_count();

    let (written, removed, unchanged) = if exists {
        let saved = exec(&mut host, "project.save", &json!({"cell_size_m": cell}))?;
        (
            saved.get("written").cloned().unwrap_or(Value::Null),
            saved.get("removed").cloned().unwrap_or(Value::Null),
            saved.get("unchanged").cloned().unwrap_or(Value::Null),
        )
    } else {
        let world = world.unwrap_or("main");
        world_name = world.to_string();
        let id = id.unwrap_or_default();
        let params = json!({
            "path": into.display().to_string(),
            "id": id,
            "name": name.unwrap_or(id),
            "world": world,
            "cell_size_m": cell,
        });
        let made = exec(&mut host, "project.new", &params)?;
        let path = made.get("files").cloned().unwrap_or(Value::Null);
        (path, Value::Array(Vec::new()), Value::Bool(false))
    };

    // The record of what this bake owns, beside the world's world.json.
    let record = BakeRecord::new(owned.iter().cloned().collect(), before, after);
    let record_at =
        record_path.unwrap_or_else(|| BakeRecord::path(&into.join("worlds").join(&world_name)));
    std::fs::write(&record_at, record.to_text()).map_err(|e| {
        CliError::Failed(format!(
            "cannot write the bake record `{}`: {e}",
            record_at.display()
        ))
    })?;
    print_json(&json!({
        "path": into.display().to_string(),
        "world": world_name,
        "cell_size_m": cell,
        "steps": steps,
        "entities": entities,
        "layers": owned.iter().cloned().collect::<Vec<_>>(),
        "terrain_edits": [before, after],
        "written": written,
        "removed": removed,
        "unchanged": unchanged,
        "record": record_at.display().to_string(),
        "seconds": started.elapsed().as_secs_f64(),
    }))
}

/// A text field of a command's answer, or the error that it is missing.
fn text_of(answer: &Value, field: &str, command: &str) -> Result<String, CliError> {
    answer
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            CliError::Failed(format!(
                "{command} did not say `{field}`; the project cannot be baked into"
            ))
        })
}

/// How many terrain edits the world shown has.
fn edit_count(host: &mut gamecraft_automation::Host) -> Result<usize, CliError> {
    let edits = exec(host, "terrain.edits", &json!({}))?;
    Ok(edits
        .get("edits")
        .and_then(Value::as_array)
        .map_or(0, Vec::len))
}

/// Removes every entity of the owned layers (an entity that names no layer is in the
/// project's `default_layer`) with one command, so that the world is walked once, not once
/// per entity: a generated layer holds a hundred thousand.
fn remove_owned(
    host: &mut gamecraft_automation::Host,
    owned: &BTreeSet<String>,
    default_layer: &str,
) -> Result<(), CliError> {
    let ids: Vec<u64> = host
        .session()
        .document()
        .entities()
        .filter(|e| owned.contains(e.layer.as_deref().unwrap_or(default_layer)))
        .map(|e| e.id.0)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    exec(host, "entity.remove_many", &json!({"ids": ids}))?;
    Ok(())
}
