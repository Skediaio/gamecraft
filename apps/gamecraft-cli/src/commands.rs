//! The subcommands: each builds a host, runs commands on it and prints the results.

use crate::args::Args;
use crate::gpu::GpuPictures;
use crate::script::{Step, collect_steps};
use crate::{CliError, Res, help};
use gamecraft_automation::{Access, Host, mcp, rpc};
use gamecraft_world::World;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Most ticks one `sim.step` takes (the command's own limit).
const MAX_STEP_TICKS: u64 = 36_000;
/// The engine's tick rate, per second.
const TICKS_PER_SECOND: f64 = 60.0;
/// Longest total time `play` is asked for, seconds (a day).
const MAX_PLAY_SECONDS: f64 = 86_400.0;
/// Entities `info` lists.
const INFO_ENTITY_LIMIT: u64 = 1000;
/// Picture size of `render` when `--size` is left out.
const DEFAULT_SIZE: (u32, u32) = (960, 540);

// ------------------------------------------------------------------------------- helpers

/// Writes one line to stdout.
pub fn print(text: &str) -> Res {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())
        .and_then(|()| out.write_all(b"\n"))
        .and_then(|()| out.flush())
        .map_err(|e| CliError::Failed(format!("cannot write to standard output: {e}")))
}

fn print_json(value: &Value) -> Res {
    print(&serde_json::to_string(value).unwrap_or_else(|_| "null".into()))
}

/// The absolute path of an argument (the file need not exist).
fn absolute(given: &str) -> Result<PathBuf, CliError> {
    std::path::absolute(given)
        .map_err(|e| CliError::Failed(format!("cannot resolve the path `{given}`: {e}")))
}

/// The folder a file is in.
fn folder(file: &Path) -> PathBuf {
    file.parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// A host with an empty world and the renderer, that may read and write only in the folders
/// given.
fn host(read_root: Option<PathBuf>, write_root: Option<PathBuf>) -> Host {
    Host::new(
        World::new("Untitled", 0),
        Access {
            read_root,
            write_root,
        },
    )
    .with_pictures(Box::new(GpuPictures::default()))
}

fn exec(host: &mut Host, id: &str, params: &Value) -> Result<Value, CliError> {
    host.execute(id, params).map_err(CliError::Failed)
}

/// Opens `file` in a host whose read root is the file's own folder.
fn open_in(host: &mut Host, file: &Path) -> Res {
    exec(
        host,
        "file.open",
        &json!({"path": file.display().to_string()}),
    )
    .map(drop)
}

/// A host that has opened `file`, writing only in `write_folder`.
fn opened(file: &str, write_folder: Option<PathBuf>) -> Result<Host, CliError> {
    let path = absolute(file)?;
    let mut host = host(Some(folder(&path)), write_folder);
    open_in(&mut host, &path)?;
    Ok(host)
}

/// Runs the steps in order. Without `keep_going` the first failure is the error; with it, a
/// failing step prints its `{"failed", "step", "error"}` line and the first failure's text is
/// returned at the end. `quiet` leaves the result lines out.
fn run_steps(
    host: &mut Host,
    steps: &[Step],
    keep_going: bool,
    quiet: bool,
) -> Result<Option<String>, CliError> {
    let mut first_failure = None;
    for (index, step) in steps.iter().enumerate() {
        match host.execute(&step.id, &step.params) {
            Ok(result) if !quiet => print_json(&result)?,
            Ok(_) => {}
            Err(error) if keep_going => {
                print_json(&json!({"failed": step.id, "step": index, "error": error}))?;
                first_failure.get_or_insert(format!("step {index}: {error}"));
            }
            Err(error) => return Err(CliError::Failed(format!("step {index}: {error}"))),
        }
    }
    Ok(first_failure)
}

fn size_of(text: &str) -> Result<(u32, u32), CliError> {
    let bad = || {
        CliError::Usage(format!(
            "`--size {text}` must be WIDTHxHEIGHT in pixels, e.g. 1280x720"
        ))
    };
    let (w, h) = text.split_once(['x', 'X']).ok_or_else(bad)?;
    Ok((
        w.trim().parse().map_err(|_| bad())?,
        h.trim().parse().map_err(|_| bad())?,
    ))
}

fn json_arg(flag: &str, text: &str) -> Result<Value, CliError> {
    serde_json::from_str(text)
        .map_err(|e| CliError::Usage(format!("`{flag}` is not valid JSON: {e}")))
}

// ------------------------------------------------------------------------------ commands

/// `commands`: every command with its parameters.
pub fn list(raw: &[String]) -> Res {
    let a = Args::parse(raw, &["--filter"], &["--json"])?;
    if a.help() {
        return print(help::COMMANDS);
    }
    a.positional(0)?;
    let filter = a.value("--filter")?;
    let mut host = host(None, None);
    if a.has("--json") {
        let mut params = json!({});
        if let (Some(f), Some(map)) = (filter, params.as_object_mut()) {
            map.insert("filter".into(), json!(f));
        }
        let request = json!({"id": 1, "method": "engine.commands", "params": params});
        let reply = rpc::handle_line(&mut host, &request.to_string()).unwrap_or_default();
        let reply: Value = serde_json::from_str(&reply)
            .map_err(|e| CliError::Failed(format!("the command list is not JSON (a bug): {e}")))?;
        return match reply.get("result") {
            Some(result) => print_json(result),
            None => Err(CliError::Failed(
                reply
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("engine.commands failed")
                    .to_string(),
            )),
        };
    }
    let needle = filter.map(str::to_lowercase);
    let blocks: Vec<String> = host
        .command_docs()
        .iter()
        .filter(|d| {
            needle.as_deref().is_none_or(|n| {
                d.id.to_lowercase().contains(n) || d.label.to_lowercase().contains(n)
            })
        })
        .map(|d| {
            format!(
                "{}\n  params:  {}\n  summary: {}",
                d.id, d.notation, d.summary
            )
        })
        .collect();
    if blocks.is_empty() {
        return print(&format!(
            "No command matches `{}`.",
            filter.unwrap_or_default()
        ));
    }
    print(&blocks.join("\n\n"))
}

/// `run`: open or create a world, run steps, save.
pub fn run(raw: &[String]) -> Res {
    let a = Args::parse(
        raw,
        &["--new", "--script", "--cmd", "--params", "--out"],
        &["--keep-going"],
    )?;
    if a.help() {
        return print(help::RUN);
    }
    let file = a.positional(1)?;
    let new = a.value("--new")?;
    if new.is_some() && file.is_some() {
        return Err(CliError::Usage(
            "`--new` starts a new world; it does not go with a FILE".into(),
        ));
    }
    let new = new.map(|text| json_arg("--new", text)).transpose()?;
    let steps = collect_steps(&a)?;
    let out = a.value("--out")?.map(absolute).transpose()?;
    let keep_going = a.has("--keep-going");
    let write_root = out.as_deref().map(folder);
    let mut host = match file {
        Some(file) => opened(file, write_root)?,
        None => {
            let mut host = host(None, write_root);
            if let Some(params) = new {
                exec(&mut host, "file.new", &params)?;
            }
            host
        }
    };
    let failure = run_steps(&mut host, &steps, keep_going, false)?;
    if let Some(out) = out {
        exec(
            &mut host,
            "file.save",
            &json!({"path": out.display().to_string(), "overwrite": true}),
        )?;
    }
    match failure {
        Some(first) => Err(CliError::Failed(format!(
            "a step failed, the first: {first}"
        ))),
        None => Ok(()),
    }
}

/// `info`: `world.inspect` of a file.
pub fn info(raw: &[String]) -> Res {
    let a = Args::parse(raw, &[], &[])?;
    if a.help() {
        return print(help::INFO);
    }
    let file = a.required_file("the world FILE to describe")?;
    let mut host = opened(file, None)?;
    let result = exec(
        &mut host,
        "world.inspect",
        &json!({"limit": INFO_ENTITY_LIMIT}),
    )?;
    print_json(&result)
}

/// `new`: an empty world file.
pub fn new(raw: &[String]) -> Res {
    let a = Args::parse(raw, &["--name", "--seed"], &[])?;
    if a.help() {
        return print(help::NEW);
    }
    let file = absolute(a.required_file("the FILE to write")?)?;
    let mut params = serde_json::Map::new();
    if let Some(name) = a.value("--name")? {
        params.insert("name".into(), json!(name));
    }
    if let Some(seed) = a.value("--seed")? {
        let seed: u64 = seed.parse().map_err(|_| {
            CliError::Usage(format!("`--seed {seed}` must be a whole number from 0"))
        })?;
        params.insert("seed".into(), json!(seed));
    }
    let mut host = host(None, Some(folder(&file)));
    let made = exec(&mut host, "file.new", &Value::Object(params))?;
    let saved = exec(
        &mut host,
        "file.save",
        &json!({"path": file.display().to_string()}),
    )?;
    print_json(&json!({
        "path": saved.get("path"),
        "bytes": saved.get("bytes"),
        "name": made.get("name"),
        "seed": made.get("seed"),
    }))
}

/// `render`: a picture of a world file.
pub fn render(raw: &[String]) -> Res {
    let a = Args::parse(
        raw,
        &[
            "--out",
            "--size",
            "--camera",
            "--follow",
            "--view",
            "--distance",
        ],
        &[],
    )?;
    if a.help() {
        return print(help::RENDER);
    }
    let file = a.required_file("the world FILE to draw")?;
    let out = absolute(
        a.value("--out")?
            .ok_or_else(|| CliError::Usage("give the picture to write with --out PNG".into()))?,
    )?;
    let (width, height) = match a.value("--size")? {
        Some(text) => size_of(text)?,
        None => DEFAULT_SIZE,
    };
    let mut params = serde_json::Map::new();
    params.insert("path".into(), json!(out.display().to_string()));
    params.insert("width".into(), json!(width));
    params.insert("height".into(), json!(height));
    let camera = a.value("--camera")?;
    let follow = a.value("--follow")?;
    match (camera, follow) {
        (Some(_), Some(_)) => {
            return Err(CliError::Usage(
                "give --camera or --follow, not both; give neither to frame the whole world".into(),
            ));
        }
        (Some(text), None) => {
            params.insert("camera".into(), json_arg("--camera", text)?);
        }
        (None, Some(entity)) => {
            let entity = entity
                .parse::<u64>()
                .map_or_else(|_| json!(entity), |id| json!(id));
            let mut follow = serde_json::Map::new();
            follow.insert("entity".into(), entity);
            match a.value("--view")? {
                None | Some("third" | "third_person") => {}
                Some("first" | "first_person") => {
                    follow.insert("view".into(), json!("first_person"));
                }
                Some(other) => {
                    return Err(CliError::Usage(format!(
                        "`--view {other}` must be first or third"
                    )));
                }
            }
            if let Some(distance) = a.value("--distance")? {
                let distance: f64 = distance.parse().map_err(|_| {
                    CliError::Usage(format!(
                        "`--distance {distance}` must be a number of metres"
                    ))
                })?;
                follow.insert("distance_m".into(), json!(distance));
            }
            params.insert("follow".into(), Value::Object(follow));
        }
        (None, None) => {
            if a.has("--view") || a.has("--distance") {
                return Err(CliError::Usage(
                    "`--view` and `--distance` go with `--follow`".into(),
                ));
            }
        }
    }
    let mut host = opened(file, Some(folder(&out)))?;
    let result = exec(&mut host, "view.render", &Value::Object(params))?;
    print_json(&result)
}

/// `play`: the simulation of a world file, for a while.
pub fn play(raw: &[String]) -> Res {
    let a = Args::parse(
        raw,
        &[
            "--script",
            "--seconds",
            "--ticks",
            "--out-state",
            "--picture",
        ],
        &[],
    )?;
    if a.help() {
        return print(help::PLAY);
    }
    let file = a.required_file("the world FILE to play")?;
    let target_ticks = match (a.value("--seconds")?, a.value("--ticks")?) {
        (Some(_), Some(_)) => {
            return Err(CliError::Usage(
                "give --seconds or --ticks, not both".into(),
            ));
        }
        (Some(text), None) => {
            let seconds: f64 = text
                .parse()
                .ok()
                .filter(|s: &f64| s.is_finite() && (0.0..=MAX_PLAY_SECONDS).contains(s))
                .ok_or_else(|| {
                    CliError::Usage(format!(
                        "`--seconds {text}` must be a number from 0 to {MAX_PLAY_SECONDS}"
                    ))
                })?;
            (seconds * TICKS_PER_SECOND).round() as u64
        }
        (None, Some(text)) => text
            .parse::<u64>()
            .ok()
            .filter(|t| *t as f64 <= MAX_PLAY_SECONDS * TICKS_PER_SECOND)
            .ok_or_else(|| {
                CliError::Usage(format!(
                    "`--ticks {text}` must be a whole number of ticks, at most {}",
                    MAX_PLAY_SECONDS * TICKS_PER_SECOND
                ))
            })?,
        (None, None) => 0,
    };
    let steps = match a.value("--script")? {
        Some(path) => crate::script::read_script(path)?,
        None => Vec::new(),
    };
    let picture = a.value("--picture")?.map(absolute).transpose()?;
    let mut host = opened(file, picture.as_deref().map(folder))?;
    exec(&mut host, "sim.start", &json!({}))?;
    run_steps(&mut host, &steps, false, true)?;
    let mut state = exec(&mut host, "sim.state", &json!({}))?;
    loop {
        let tick = state.get("tick").and_then(Value::as_u64).unwrap_or(0);
        if tick >= target_ticks {
            break;
        }
        let ticks = (target_ticks - tick).min(MAX_STEP_TICKS);
        exec(&mut host, "sim.step", &json!({"ticks": ticks}))?;
        state = exec(&mut host, "sim.state", &json!({}))?;
    }
    if let Some(path) = a.value("--out-state")? {
        let text = serde_json::to_string_pretty(&state).unwrap_or_default();
        std::fs::write(path, format!("{text}\n"))
            .map_err(|e| CliError::Failed(format!("cannot write the state to `{path}`: {e}")))?;
    }
    if let (Some(path), Some(map)) = (picture, state.as_object_mut()) {
        let drawn = exec(
            &mut host,
            "view.render",
            &json!({"path": path.display().to_string(), "width": DEFAULT_SIZE.0, "height": DEFAULT_SIZE.1}),
        );
        map.insert("picture".into(), drawn?);
    }
    print_json(&state)
}

/// `serve` and `mcp`: the protocol servers over stdin and stdout.
pub fn serve(raw: &[String], mcp_protocol: bool) -> Res {
    let a = Args::parse(
        raw,
        &[
            "--automation-read-root",
            "--automation-write-root",
            "--file",
        ],
        &[],
    )?;
    if a.help() {
        return print(if mcp_protocol { help::MCP } else { help::SERVE });
    }
    a.positional(0)?;
    let root = |flag: &str| -> Result<Option<PathBuf>, CliError> {
        match a.value(flag)? {
            None => Ok(None),
            Some(dir) if Path::new(dir).is_dir() => Ok(Some(PathBuf::from(dir))),
            Some(dir) => Err(CliError::Usage(format!(
                "`{flag} {dir}`: not an existing directory"
            ))),
        }
    };
    let access = Access {
        read_root: root("--automation-read-root")?,
        write_root: root("--automation-write-root")?,
    };
    let world = match a.value("--file")? {
        Some(file) => {
            // Read through a host that may read the file's folder only; the served host
            // keeps just the roots the flags gave.
            let loader = opened(file, None)?;
            loader.session().document().clone()
        }
        None => World::new("Untitled", 0),
    };
    let mut host = Host::new(world, access).with_pictures(Box::new(GpuPictures::default()));
    let (stdin, stdout) = (std::io::stdin().lock(), std::io::stdout().lock());
    let served = if mcp_protocol {
        mcp::serve(&mut host, stdin, stdout)
    } else {
        rpc::serve(&mut host, stdin, stdout)
    };
    served.map_err(|e| CliError::Failed(format!("standard input or output failed: {e}")))
}
