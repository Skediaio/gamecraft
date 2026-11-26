//! The subcommands: each builds a host, runs commands on it and prints the results.

use crate::args::Args;
use crate::gpu::GpuPictures;
use crate::script::{Step, collect_steps};
use crate::{CliError, Res, help};
use gamecraft_automation::{Access, Host, mcp, rpc};
use gamecraft_engine::journal::Journal;
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
/// Largest journal `replay` reads, bytes.
const MAX_JOURNAL_BYTES: u64 = 512 * 1024 * 1024;
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

pub fn print_json(value: &Value) -> Res {
    print(&serde_json::to_string(value).unwrap_or_else(|_| "null".into()))
}

/// The absolute path of an argument (the file need not exist).
pub fn absolute(given: &str) -> Result<PathBuf, CliError> {
    std::path::absolute(given)
        .map_err(|e| CliError::Failed(format!("cannot resolve the path `{given}`: {e}")))
}

/// The folder a file is in.
pub fn folder(file: &Path) -> PathBuf {
    file.parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// The world-file cap of this run, bytes: `--max-world-mb` (taken out of the arguments by
/// `main`), else `GAMECRAFT_MAX_WORLD_MB`, else the host's default (512 MiB).
pub static MAX_WORLD_BYTES: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

/// The environment variable that sets the world-file cap, in MiB.
pub const MAX_WORLD_ENV: &str = "GAMECRAFT_MAX_WORLD_MB";

/// A cap in MiB as given on the command line or in the environment (`what` names it), in
/// bytes: a whole number from 1 to 1 048 576 (1 TiB).
pub fn parse_world_mb(given: &str, what: &str) -> Result<u64, CliError> {
    match given.trim().parse::<u64>() {
        Ok(mb @ 1..=1_048_576) => Ok(mb * 1024 * 1024),
        _ => Err(CliError::Usage(format!(
            "`{what} {given}`: the largest world file to read, in MiB, a whole number from 1 \
             to 1048576 (the default is 512)"
        ))),
    }
}

/// The world-file cap for the hosts of this run; `None` is the host's default.
fn max_world_bytes() -> Result<Option<u64>, CliError> {
    if let Some(cap) = MAX_WORLD_BYTES.get() {
        return Ok(Some(*cap));
    }
    match std::env::var(MAX_WORLD_ENV) {
        Ok(v) => parse_world_mb(&v, MAX_WORLD_ENV).map(Some),
        Err(_) => Ok(None),
    }
}

/// A host with an empty world, the renderer and the project index, that may read and write
/// only in the folders given.
pub fn host(read_root: Option<PathBuf>, write_root: Option<PathBuf>) -> Host {
    Host::new(
        World::new("Untitled", 0),
        Access {
            read_root,
            write_root,
            max_world_bytes: max_world_bytes().ok().flatten(),
            cache_beside_files: std::env::var_os("GAMECRAFT_NO_CACHE").is_none(),
        },
    )
    .with_pictures(Box::new(GpuPictures::default()))
    .with_indexer(Box::new(crate::index::TursoIndexer))
}

pub fn exec(host: &mut Host, id: &str, params: &Value) -> Result<Value, CliError> {
    host.execute(id, params).map_err(CliError::Failed)
}

/// The `--pack DIR` flag of a subcommand that opens a world file: the folder, checked
/// against `file` (a project folder brings its own assets and takes none).
fn pack_flag<'a>(a: &'a Args, file: Option<&str>) -> Result<Option<&'a str>, CliError> {
    let pack = a.value("--pack")?;
    if let (Some(pack), Some(file)) = (pack, file)
        && absolute(file)?.is_dir()
    {
        return Err(CliError::Usage(format!(
            "`{file}` is a project folder, which brings its own assets; --pack {pack} goes \
             with a world file"
        )));
    }
    Ok(pack)
}

/// A host that has opened `file`, writing only in `write_folder`. A folder is a project:
/// it is opened with project.open, reading in the folder that holds it (so a dependency
/// beside it, `../game`, is found). A world file finds the pack above it, or the pack
/// `pack` names (`--pack DIR`, a folder with a pack.json whose assets/ holds the world's
/// models); that folder is read too, from the folder that holds it.
pub fn opened(
    file: &str,
    write_folder: Option<PathBuf>,
    pack: Option<&str>,
) -> Result<Host, CliError> {
    let path = absolute(file)?;
    if path.is_dir() {
        let mut host = host(Some(folder(&path)), write_folder);
        exec(
            &mut host,
            "project.open",
            &json!({"path": path.display().to_string()}),
        )?;
        return Ok(host);
    }
    let path = std::fs::canonicalize(&path)
        .map_err(|e| CliError::Failed(format!("cannot open `{file}`: {e}")))?;
    // Where the pack is: the one given, else the nearest pack.json above the file. Only the
    // one given is the host's own pack (it stands for every world file opened later).
    let given = pack.is_some();
    let pack_dir = match pack {
        Some(dir) => {
            let dir = absolute(dir)?;
            if !dir.is_dir() {
                return Err(CliError::Failed(format!(
                    "`--pack {}`: the folder does not exist; give the folder of a pack (it has \
                     a pack.json whose assets/ holds the world's models)",
                    dir.display()
                )));
            }
            Some(std::fs::canonicalize(&dir).map_err(|e| {
                CliError::Failed(format!("cannot use the pack `{}`: {e}", dir.display()))
            })?)
        }
        None => gamecraft_automation::find_pack_above(&folder(&path), None),
    };
    // The file's own folder, widened to the folder that holds the pack (where its
    // dependencies lie, `../game`).
    let read_root = match &pack_dir {
        Some(pack) => gamecraft_automation::common_ancestor(&folder(&path), &folder(pack)),
        None => folder(&path),
    };
    let mut host = host(Some(read_root), write_folder);
    if given && let Some(pack) = &pack_dir {
        host = host.with_pack(pack.clone());
    }
    exec(
        &mut host,
        "file.open",
        &json!({"path": path.display().to_string()}),
    )?;
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
        &["--new", "--script", "--cmd", "--params", "--out", "--pack"],
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
    let pack = pack_flag(&a, file)?;
    let new = new.map(|text| json_arg("--new", text)).transpose()?;
    let steps = collect_steps(&a)?;
    let out = a.value("--out")?.map(absolute).transpose()?;
    let keep_going = a.has("--keep-going");
    let write_root = out.as_deref().map(folder);
    let mut host = match file {
        Some(file) => opened(file, write_root, pack)?,
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
    let a = Args::parse(raw, &["--pack"], &[])?;
    if a.help() {
        return print(help::INFO);
    }
    let file = a.required_file("the world FILE to describe")?;
    let mut host = opened(file, None, pack_flag(&a, Some(file))?)?;
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
            "--pack",
        ],
        &[],
    )?;
    if a.help() {
        return print(help::RENDER);
    }
    let file = a.required_file("the world FILE to draw")?;
    let pack = pack_flag(&a, Some(file))?;
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
    let mut host = opened(file, Some(folder(&out)), pack)?;
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
            "--journal",
            "--pack",
        ],
        &[],
    )?;
    if a.help() {
        return print(help::PLAY);
    }
    let journal_out = a.value("--journal")?.map(absolute).transpose()?;
    let file = a.required_file("the world FILE to play")?;
    let pack = pack_flag(&a, Some(file))?;
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
    let mut host = opened(file, picture.as_deref().map(folder), pack)?;
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
    if let Some(path) = &journal_out {
        let text = host
            .session()
            .journal()
            .ok_or_else(|| CliError::Failed("the run kept no journal (a bug)".into()))?
            .to_json()
            .map_err(CliError::Failed)?;
        std::fs::write(path, text).map_err(|e| {
            CliError::Failed(format!(
                "cannot write the journal to `{}`: {e}",
                path.display()
            ))
        })?;
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

/// Reads and checks a journal file.
fn read_journal(given: &str) -> Result<Journal, CliError> {
    let failed = |why: String| CliError::Failed(format!("`{given}`: {why}"));
    let meta = std::fs::metadata(given).map_err(|e| failed(format!("cannot read it: {e}")))?;
    if meta.len() > MAX_JOURNAL_BYTES {
        return Err(failed(format!(
            "it is {} MB; a journal may be at most {} MB",
            meta.len().div_ceil(1024 * 1024),
            MAX_JOURNAL_BYTES / (1024 * 1024)
        )));
    }
    let text =
        std::fs::read_to_string(given).map_err(|e| failed(format!("cannot read it: {e}")))?;
    Journal::from_json(&text).map_err(failed)
}

/// `replay`: runs a journal again and compares its hashes.
pub fn replay(raw: &[String]) -> Res {
    let a = Args::parse(raw, &["--to-tick"], &["--check"])?;
    if a.help() {
        return print(help::REPLAY);
    }
    let file = a.required_file("the JOURNAL to replay")?;
    let to_tick = match a.value("--to-tick")? {
        None => None,
        Some(text) => Some(text.parse::<u64>().map_err(|_| {
            CliError::Usage(format!(
                "`--to-tick {text}` must be a whole number of ticks (60 per second)"
            ))
        })?),
    };
    let journal = read_journal(file)?;
    let replay = journal
        .replay(to_tick)
        .map_err(|e| CliError::Failed(format!("the journal cannot start: {e}")))?;
    let divergence = replay
        .divergence
        .as_ref()
        .map(|d| json!({"entry": d.entry, "tick": d.tick, "message": d.message}));
    let summary = json!({
        "tick": replay.tick,
        "hash": replay.hash,
        "entries": journal.entries.len(),
        "entries_run": replay.entries_run,
        "hashes_checked": replay.hashes_checked,
        "final": journal.end.as_ref().map(|e| json!({"tick": e.tick, "hash": e.hash})),
        "matches": replay.divergence.is_none(),
        "divergence": divergence.clone(),
    });
    let out = match to_tick {
        None => summary,
        Some(_) => {
            let mut session = replay.session;
            let mut state = session
                .run("sim.state", &json!({}))
                .map_err(|e| CliError::Failed(e.to_string()))?;
            if let Some(map) = state.as_object_mut() {
                map.insert(
                    "replay".into(),
                    json!({
                        "tick": replay.tick,
                        "entries_run": replay.entries_run,
                        "hashes_checked": replay.hashes_checked,
                        "divergence": divergence,
                    }),
                );
            }
            state
        }
    };
    print_json(&out)?;
    if a.has("--check")
        && let Some(d) = &replay.divergence
    {
        return Err(CliError::Failed(format!(
            "the replay of `{file}` diverges: {}",
            d.message
        )));
    }
    if a.has("--check") && journal.truncated && to_tick.is_none() {
        return Err(CliError::Failed(format!(
            "`{file}` is truncated (it was full), so it cannot be checked to its end; check \
             up to a tick with --to-tick"
        )));
    }
    Ok(())
}

/// `serve` and `mcp`: the protocol servers over stdin and stdout.
pub fn serve(raw: &[String], mcp_protocol: bool) -> Res {
    let a = Args::parse(
        raw,
        &[
            "--automation-read-root",
            "--automation-write-root",
            "--file",
            "--pack",
        ],
        &[],
    )?;
    if a.help() {
        return print(if mcp_protocol { help::MCP } else { help::SERVE });
    }
    a.positional(0)?;
    let pack = a.value("--pack")?;
    if pack.is_some() && a.value("--file")?.is_none() {
        return Err(CliError::Usage(
            "`--pack` says where the world file of --file finds its models; give --file FILE"
                .into(),
        ));
    }
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
        max_world_bytes: max_world_bytes()?,
        cache_beside_files: std::env::var_os("GAMECRAFT_NO_CACHE").is_none(),
    };
    let mut host = match a.value("--file")? {
        // Read through a host that may read the file's folder (and its pack's); the served
        // host keeps what was read and just the roots the flags gave.
        Some(file) => {
            let mut loaded = opened(file, None, pack)?;
            loaded.set_access(access);
            loaded
        }
        None => Host::new(World::new("Untitled", 0), access)
            .with_pictures(Box::new(GpuPictures::default())),
    };
    let (stdin, stdout) = (std::io::stdin().lock(), std::io::stdout().lock());
    let served = if mcp_protocol {
        mcp::serve(&mut host, stdin, stdout)
    } else {
        rpc::serve(&mut host, stdin, stdout)
    };
    served.map_err(|e| CliError::Failed(format!("standard input or output failed: {e}")))
}
