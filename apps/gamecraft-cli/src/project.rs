//! `project new|open|info|export|query`: projects on disk from the command line.

use crate::args::Args;
use crate::commands::{absolute, exec, folder, host, opened, print, print_json};
use crate::{CliError, Res, help};
use serde_json::json;
use std::path::PathBuf;

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
        other => Err(CliError::Usage(format!(
            "unknown action `{other}`; the actions are: new, open, info, export, query"
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
    let mut host = opened(dir, Some(folder(&out)))?;
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
