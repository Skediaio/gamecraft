//! What both protocols run: every command of the engine's [`Session`] plus the host's own
//! (`file.*`, `edit.*`, `view.render`), one namespace with one listing, and batches.
//!
//! Ported from the host-command and batch parts of VisionCraft's
//! `crates/visioncraft-automation/src/commands.rs` (MIT). Adapted: the host's commands are
//! [`CommandSpec`]s like the engine's, so their parameters are checked the same way and listed
//! in the same shape (scope `host`); there is no agent log, catalogue or model geometry.

use crate::docs::{CommandDoc, from_spec};
use crate::fields::{Fields, describe, shorten};
use crate::host::{CameraRequest, Host, RenderRequest};
use crate::{MAX_BATCH_STEPS, base64};
use gamecraft_engine::spec::nearest;
use gamecraft_engine::{CommandError, CommandSpec, ParamType as T, Session, param, spec};
use gamecraft_world::{EntityId, World};
use serde_json::{Map, Value, json};

/// Smallest and largest side of a picture, pixels.
const PICTURE_MIN_SIDE: u64 = 16;
const PICTURE_MAX_SIDE: u64 = 4096;
/// Default picture size of `view.render`, pixels.
const RENDER_DEFAULT_WIDTH: u64 = 960;
const RENDER_DEFAULT_HEIGHT: u64 = 540;
/// Vertical field of view of a `camera` when it gives none, and its limits, degrees.
const DEFAULT_FOV_Y_DEG: f64 = 60.0;
const MIN_FOV_Y_DEG: f64 = 1.0;
const MAX_FOV_Y_DEG: f64 = 179.0;
/// Distance of a third-person camera when `follow` gives none, and its limits, metres.
const DEFAULT_FOLLOW_DISTANCE_M: f64 = 6.0;
const MIN_FOLLOW_DISTANCE_M: f64 = 0.5;
const MAX_FOLLOW_DISTANCE_M: f64 = 1000.0;

/// Commands that replace the session or step through its history: they cannot be part of an
/// atomic batch.
const NOT_ATOMIC: [&str; 7] = [
    "file.new",
    "file.open",
    "edit.undo",
    "edit.redo",
    "project.new",
    "project.open",
    "pack.add_dependency",
];

/// Wall-clock milliseconds to a tenth; `null` where there is no clock (the browser).
pub(crate) struct Stopwatch(#[cfg(not(target_arch = "wasm32"))] std::time::Instant);

impl Stopwatch {
    /// Starts measuring.
    pub fn start() -> Self {
        Self(
            #[cfg(not(target_arch = "wasm32"))]
            std::time::Instant::now(),
        )
    }

    /// Milliseconds since [`Stopwatch::start`], as JSON.
    pub fn ms(&self) -> Value {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ms = self.0.elapsed().as_secs_f64() * 1000.0;
            json!((ms * 10.0).round() / 10.0)
        }
        #[cfg(target_arch = "wasm32")]
        Value::Null
    }
}

// ----------------------------------------------------------------------------- the specs

/// A host command: not an undo step of its own, scope `host` in the listing.
pub(crate) fn host_spec(
    name: &str,
    label: &str,
    summary: &str,
    params: Vec<gamecraft_engine::ParamSpec>,
    result: &str,
    example: Value,
) -> CommandSpec {
    let mut s = spec(name, label, summary, params, result, example);
    s.undoable = false;
    s
}

/// The host's own commands, in the shape of the engine's specs.
pub(crate) fn host_specs() -> Vec<CommandSpec> {
    let specs = vec![
        host_spec(
            "edit.undo",
            "Undo",
            "Takes back the last change of the document (one command, or one whole atomic \
             batch). Refused while a simulation runs: a running world has no history.",
            vec![],
            "{can_undo, can_redo}",
            json!({}),
        ),
        host_spec(
            "edit.redo",
            "Redo",
            "Does the change last undone again. Refused while a simulation runs.",
            vec![],
            "{can_undo, can_redo}",
            json!({}),
        ),
        host_spec(
            "view.render",
            "Render",
            "Draws the current world (the running world while playing) to a PNG: the whole \
             world framed, from a `camera`, or following an entity (`follow`); at most one of \
             the two. With `path` the picture is written to a .png file inside the write root, \
             otherwise it comes back as base64 (over MCP as an image block). Needs a renderer; \
             the document does not change.",
            vec![
                param(
                    "path",
                    T::String,
                    "A .png file inside the write root; left out, the picture is in the answer.",
                ),
                param("width", T::Integer, "Picture width, 16 to 4096.")
                    .unit("px")
                    .default(json!(RENDER_DEFAULT_WIDTH)),
                param("height", T::Integer, "Picture height, 16 to 4096.")
                    .unit("px")
                    .default(json!(RENDER_DEFAULT_HEIGHT)),
                param(
                    "camera",
                    T::Object,
                    "{\"position\": [x, y, z], \"target\": [x, y, z], \"fov_y_deg\": 1 to 179 = \
                     60}: a camera at position looking at target, metres and degrees.",
                ),
                param(
                    "follow",
                    T::Object,
                    "{\"entity\": id or name, \"view\": \"first_person\"|\"third_person\" = \
                     \"third_person\", \"distance_m\": 0.5 to 1000 = 6}: through the entity's \
                     eyes, or behind it at distance_m (third_person only).",
                ),
                param(
                    "time_s",
                    T::Number,
                    "The animation clock (wind, water), at least 0; the simulation's time while \
                     playing, else 0.",
                )
                .unit("s"),
            ],
            "{mime: \"image/png\", base64, width, height, ...the renderer's info}, or with path \
             {path, bytes, width, height, ...the renderer's info}",
            json!({"width": 320, "height": 180, "follow": {"entity": "Scout", "view": "third_person", "distance_m": 8}}),
        ),
        host_spec(
            "project.query",
            "Query Project",
            "Runs one read-only SQL SELECT on the index of the open project (rebuilt first when \
             the world changed): tables entities(id, key, name, layer, pack, prefab, parent, x, \
             y, z, rotation_y_deg, scale, tags, components), entity_tags(entity_id, tag), \
             entity_components(entity_id, type, data) and prefabs(id, name, pack, components, \
             instances). Anything but one SELECT is refused. Native only: the browser has no \
             index.",
            vec![
                param("sql", T::String, "One SELECT statement.").required(),
                param("limit", T::Integer, "Most rows returned, 1 to 1000.").default(json!(1000)),
            ],
            "{columns: [names], rows: [[values]], truncated, index}",
            json!({"sql": "SELECT e.name, e.x, e.z FROM entities e JOIN entity_tags t ON t.entity_id = e.id WHERE t.tag = 'enemy'"}),
        ),
    ];
    // The browser has no files.
    #[cfg(not(target_arch = "wasm32"))]
    let specs = {
        let mut specs = specs;
        specs.extend(file_specs());
        specs.extend(crate::projects::project_specs());
        specs.extend(crate::assets::asset_specs());
        specs
    };
    specs
}

#[cfg(not(target_arch = "wasm32"))]
fn file_specs() -> Vec<CommandSpec> {
    vec![
        host_spec(
            "file.new",
            "New World",
            "Starts an empty world (no terrain, the default environment, no entities) with a new \
             history; a running simulation stops and the open file is forgotten.",
            vec![
                param("name", T::String, "The world's name, at most 200 characters.")
                    .default(json!("Untitled")),
                param("seed", T::Integer, "The world seed, 0 to 2^64 - 1.").default(json!(0)),
            ],
            "{name, seed}",
            json!({"name": "Green valley", "seed": 42}),
        ),
        host_spec(
            "file.open",
            "Open",
            "Opens a .gamecraft file inside the read root (at most 64 MB): it becomes the \
             document, with a new history; a running simulation stops. A file that cannot be \
             read or is not a valid world is an error, and the session stays as it was.",
            vec![
                param(
                    "path",
                    T::String,
                    "A .gamecraft file inside the read root; relative to it, or absolute inside it.",
                )
                .required(),
            ],
            "{path, name, entities, terrain}: terrain is whether the world has one",
            json!({"path": "valley.gamecraft"}),
        ),
        host_spec(
            "file.save",
            "Save",
            "Writes the document (never the running world) as a .gamecraft file inside the \
             write root, through a temporary file and a rename, so a failed save leaves no half \
             file. Without `path`, the open file. Replacing another existing file needs \
             `overwrite`.",
            vec![
                param(
                    "path",
                    T::String,
                    "A .gamecraft file inside the write root; the open file when left out.",
                ),
                param(
                    "overwrite",
                    T::Boolean,
                    "Replace an existing file that is not the open file.",
                )
                .default(json!(false)),
            ],
            "{path, bytes}",
            json!({"path": "valley.gamecraft"}),
        ),
        host_spec(
            "journal.save",
            "Save Journal",
            "Writes the journal of the running simulation, or of the last one, as a .json file \
             inside the write root: {gamecraft_journal: 1, world: <the document the simulation \
             started from>, entries: [{tick, command, params, end_tick?, hash}], final: {tick, \
             hash}}, one JSON document with the world and each entry on a line of its own. \
             `gamecraft-cli replay FILE --check` runs it again and compares every hash. Written \
             through a temporary file and a rename; replacing an existing file needs \
             `overwrite`.",
            vec![
                param("path", T::String, "A .json file inside the write root.").required(),
                param("overwrite", T::Boolean, "Replace an existing file.")
                    .default(json!(false)),
            ],
            "{path, bytes, entries, final: {tick, hash}}",
            json!({"path": "session.journal.json"}),
        ),
    ]
}

fn find_host_spec(id: &str) -> Option<CommandSpec> {
    host_specs().into_iter().find(|s| s.name == id)
}

// ---------------------------------------------------------------------------- the listing

/// What a listing lists.
#[derive(Default)]
pub(crate) struct ListQuery<'a> {
    /// Case-insensitive part of an id or label.
    pub filter: Option<&'a str>,
    /// One command by its exact id.
    pub id: Option<&'a str>,
    /// `{id, params, summary, enabled}` only.
    pub brief: bool,
}

impl Host {
    /// Whether a project is open (never in the browser).
    fn has_project(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.project.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        false
    }

    /// The documentation of every command, the engine's and the host's, sorted by id.
    pub fn command_docs(&self) -> Vec<CommandDoc> {
        let mut docs: Vec<CommandDoc> = self.session.specs().into_iter().map(from_spec).collect();
        docs.extend(host_specs().iter().map(|s| {
            let mut d = from_spec(s);
            d.scope = "host".into();
            d
        }));
        docs.sort_by(|a, b| a.id.cmp(&b.id));
        docs
    }

    /// Whether a command can run now, and why not.
    fn enabled(&self, id: &str) -> Result<(), String> {
        let playing = self.session.is_playing();
        match id {
            "edit.undo" | "edit.redo" if playing => {
                Err("a running world has no history; stop it with sim.stop first".into())
            }
            "edit.undo" if !self.session.can_undo() => Err("nothing to undo".into()),
            "edit.redo" if !self.session.can_redo() => Err("nothing to redo".into()),
            "view.render" if self.pictures.is_none() => {
                Err("this host has no renderer, and a picture needs one".into())
            }
            "file.open" if self.access.read_root.is_none() => {
                Err("no read root is set (--automation-read-root DIR)".into())
            }
            "journal.save" if self.access.write_root.is_none() => {
                Err("no write root is set (--automation-write-root DIR)".into())
            }
            "journal.save" if self.session.journal().is_none() => {
                Err("there is no journal yet; it starts with sim.start".into())
            }
            "file.save"
            | "project.new"
            | "project.save"
            | "pack.add_dependency"
            | "project.export"
                if self.access.write_root.is_none() =>
            {
                Err("no write root is set (--automation-write-root DIR)".into())
            }
            "project.open" if self.access.read_root.is_none() => {
                Err("no read root is set (--automation-read-root DIR)".into())
            }
            "project.save"
            | "project.info"
            | "project.export"
            | "project.query"
            | "pack.add_dependency"
                if !self.has_project() =>
            {
                Err("no project is open; open one with project.open".into())
            }
            "sim.start" if playing => {
                Err("a simulation is already running; stop it with sim.stop".into())
            }
            "sim.stop" | "sim.step" | "sim.input" | "sim.fire" | "sim.release" | "message.show"
                if !playing =>
            {
                Err("no simulation is running; start one with sim.start".into())
            }
            _ => Ok(()),
        }
    }

    /// `{"commands": [...]}`, as `engine.commands` and `command_list` give it. An `id` that
    /// names no command is an error with the nearest id.
    pub(crate) fn command_list(&self, query: &ListQuery) -> Result<Value, String> {
        let docs = self.command_docs();
        if let Some(id) = query.id
            && !docs.iter().any(|d| d.id == id)
        {
            return Err(format!(
                "{}; engine.commands without `id` lists them all",
                unknown_command(id, &docs)
            ));
        }
        let needle = query.filter.map(str::to_lowercase);
        let commands: Vec<Value> = docs
            .iter()
            .filter(|d| query.id.is_none_or(|id| d.id == id))
            .filter(|d| {
                needle.as_deref().is_none_or(|n| {
                    d.id.to_lowercase().contains(n) || d.label.to_lowercase().contains(n)
                })
            })
            .map(|d| d.to_json(query.brief, self.enabled(&d.id)))
            .collect();
        Ok(json!({ "commands": commands }))
    }
}

/// `unknown command `x`; did you mean `y`?`, over the engine's and the host's ids.
fn unknown_command(wrong: &str, docs: &[CommandDoc]) -> String {
    match nearest(wrong, docs.iter().map(|d| d.id.as_str())) {
        Some(id) => format!("unknown command `{}`; did you mean `{id}`?", shorten(wrong)),
        None => format!(
            "unknown command `{}`; engine.commands (or the MCP tool command_list) lists them",
            shorten(wrong)
        ),
    }
}

/// The text of a panic's payload.
pub(crate) fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic".into())
}

// ---------------------------------------------------------------------------- running

impl Host {
    /// Runs one command by id, an engine command or a host command, with JSON parameters (an
    /// object, or null for none). Errors are sentences that start with the command id. A panic
    /// in a command (a bug) is caught and reported as an error.
    pub fn execute(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.dispatch(command, params)
        }))
        .unwrap_or_else(|payload| {
            Err(format!(
                "{}: internal error (a bug): {}",
                shorten(command),
                panic_text(payload.as_ref())
            ))
        })
    }

    fn dispatch(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        if let Some(spec) = find_host_spec(command) {
            spec.check(params)
                .map_err(|why| format!("{command}: bad parameters: {why}"))?;
            let fields = Fields::new(command, "parameter", Some(params), &spec_keys(&spec))?;
            return self
                .run_host(command, &fields)
                .map_err(|why| format!("{command}: {why}"));
        }
        if self.session.spec(command).is_some() {
            return self.session.run(command, params).map_err(|e| e.to_string());
        }
        Err(unknown_command(command, &self.command_docs()))
    }

    fn run_host(&mut self, command: &str, p: &Fields) -> Result<Value, String> {
        match command {
            "edit.undo" | "edit.redo" => self.history(command),
            "view.render" => self.view_render(p),
            #[cfg(not(target_arch = "wasm32"))]
            "file.new" => self.file_new(p),
            #[cfg(not(target_arch = "wasm32"))]
            "file.open" => self.file_open(p),
            #[cfg(not(target_arch = "wasm32"))]
            "journal.save" => self.journal_save(p),
            #[cfg(not(target_arch = "wasm32"))]
            "file.save" => self.file_save(p),
            #[cfg(not(target_arch = "wasm32"))]
            "project.new" => self.project_new(p),
            #[cfg(not(target_arch = "wasm32"))]
            "project.open" => self.project_open(p),
            #[cfg(not(target_arch = "wasm32"))]
            "project.save" => self.project_save(),
            #[cfg(not(target_arch = "wasm32"))]
            "project.export" => self.project_export(p),
            #[cfg(not(target_arch = "wasm32"))]
            "project.info" => self.project_info(),
            #[cfg(not(target_arch = "wasm32"))]
            "pack.add_dependency" => self.add_dependency(p),
            #[cfg(not(target_arch = "wasm32"))]
            "project.query" => self.project_query(p),
            #[cfg(not(target_arch = "wasm32"))]
            "asset.import" => self.asset_import(p),
            #[cfg(not(target_arch = "wasm32"))]
            "asset.list" => self.asset_list(),
            #[cfg(not(target_arch = "wasm32"))]
            "asset.credits" => self.asset_credits(),
            #[cfg(target_arch = "wasm32")]
            "project.query" => Err(
                "the project index is native only: the browser has no files and no database; \
                 run gamecraft-cli to query a project"
                    .into(),
            ),
            other => Err(format!("`{other}` is not a host command (a bug)")),
        }
    }

    fn history(&mut self, id: &str) -> Result<Value, String> {
        let undo = id == "edit.undo";
        if self.session.is_playing() {
            return Err(
                "a running world has no history; stop the simulation with sim.stop first".into(),
            );
        }
        let outcome = if undo {
            if !self.session.can_undo() {
                return Err("there is nothing to undo".into());
            }
            self.session.undo()
        } else {
            if !self.session.can_redo() {
                return Err("there is nothing to redo".into());
            }
            self.session.redo()
        };
        outcome.map_err(|e| e.to_string())?;
        Ok(json!({"can_undo": self.session.can_undo(), "can_redo": self.session.can_redo()}))
    }

    fn view_render(&mut self, p: &Fields) -> Result<Value, String> {
        let width = picture_side(p, "width", RENDER_DEFAULT_WIDTH)?;
        let height = picture_side(p, "height", RENDER_DEFAULT_HEIGHT)?;
        let camera = match (p.get("camera"), p.get("follow")) {
            (Some(_), Some(_)) => {
                return Err(
                    "give `camera` or `follow`, not both; give neither to frame the whole world"
                        .into(),
                );
            }
            (Some(camera), None) => look_at(camera)?,
            (None, Some(follow)) => follow_camera(follow, self.session.current_world())?,
            (None, None) => CameraRequest::Framing,
        };
        let time_s = match p.number("time_s").map_err(strip_owner)? {
            Some(t) if t < 0.0 => {
                return Err(format!("`time_s` is {t}; it must be at least 0"));
            }
            Some(t) => t,
            None => self.session.live().map_or(0.0, |sim| sim.time_s()),
        };
        let target = match p.str("path").map_err(strip_owner)? {
            Some(path) => Some(self.picture_path(path)?),
            None => None,
        };
        let request = RenderRequest {
            width,
            height,
            camera,
            time_s,
        };
        #[cfg(not(target_arch = "wasm32"))]
        self.provide_assets();
        let Some(pictures) = self.pictures.as_mut() else {
            return Err(
                "this host has no renderer, and a renderer is needed to draw a picture; run \
                 gamecraft-cli on a machine with a GPU adapter, which sets one up"
                    .into(),
            );
        };
        let rendered = pictures.render(
            self.session.current_world(),
            self.session.current_heights(),
            self.session.view_revision(),
            &request,
        )?;
        let mut out = match rendered.info {
            Value::Object(map) => map,
            Value::Null => Map::new(),
            other => {
                let mut map = Map::new();
                map.insert("info".into(), other);
                map
            }
        };
        match target {
            Some(path) => {
                write_picture(&path, &rendered.png)?;
                out.insert("path".into(), json!(path.display().to_string()));
                out.insert("bytes".into(), json!(rendered.png.len()));
            }
            None => {
                out.insert("mime".into(), json!("image/png"));
                out.insert("base64".into(), json!(base64(&rendered.png)));
            }
        }
        out.insert("width".into(), json!(width));
        out.insert("height".into(), json!(height));
        Ok(Value::Object(out))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn picture_path(&self, path: &str) -> Result<std::path::PathBuf, String> {
        crate::files::resolve(&self.access, path, crate::files::Direction::Write, "png")
    }

    #[cfg(target_arch = "wasm32")]
    fn picture_path(&self, path: &str) -> Result<std::path::PathBuf, String> {
        Err(format!(
            "`{}`: pictures cannot be written to files in the browser; leave `path` out",
            shorten(path)
        ))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_picture(path: &std::path::Path, png: &[u8]) -> Result<(), String> {
    crate::files::write_whole(path, png)
}

#[cfg(target_arch = "wasm32")]
fn write_picture(_path: &std::path::Path, _png: &[u8]) -> Result<(), String> {
    Err("pictures cannot be written to files in the browser".into())
}

/// The keys of a spec, for [`Fields`] (the spec has already refused any other key).
fn spec_keys(spec: &CommandSpec) -> Vec<&str> {
    spec.params.iter().map(|p| p.name.as_str()).collect()
}

/// A [`Fields`] error without the `owner: ` in front (the caller puts the command id there).
pub(crate) fn strip_owner(message: String) -> String {
    match message.split_once(": ") {
        Some((_, rest)) => rest.to_string(),
        None => message,
    }
}

/// A picture side from `key`, 16 to 4096 pixels.
fn picture_side(p: &Fields, key: &str, default: u64) -> Result<u32, String> {
    let Some(value) = p.get(key) else {
        return u32::try_from(default).map_err(|e| e.to_string());
    };
    value
        .as_u64()
        .filter(|n| (PICTURE_MIN_SIDE..=PICTURE_MAX_SIDE).contains(n))
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| {
            format!(
                "`{key}` must be a whole number of pixels from {PICTURE_MIN_SIDE} to \
                 {PICTURE_MAX_SIDE}, got {}",
                shorten(&value.to_string())
            )
        })
}

/// The `camera` object of `view.render`.
fn look_at(value: &Value) -> Result<CameraRequest, String> {
    let c = Fields::new(
        "`camera`",
        "key",
        Some(value),
        &["position", "target", "fov_y_deg"],
    )?;
    let position = c.point3("position")?.ok_or_else(|| c.missing("position"))?;
    let target = c.point3("target")?.ok_or_else(|| c.missing("target"))?;
    if position == target {
        return Err(
            "`camera`: `target` is where the camera stands; give a point it looks at".into(),
        );
    }
    let fov_y_deg = c.number("fov_y_deg")?.unwrap_or(DEFAULT_FOV_Y_DEG);
    if !(MIN_FOV_Y_DEG..=MAX_FOV_Y_DEG).contains(&fov_y_deg) {
        return Err(format!(
            "`camera`: `fov_y_deg` is {fov_y_deg}; it must be from {MIN_FOV_Y_DEG} to \
             {MAX_FOV_Y_DEG} degrees"
        ));
    }
    Ok(CameraRequest::LookAt {
        position,
        target,
        fov_y_deg,
    })
}

/// The `follow` object of `view.render`, its entity looked up in `world`.
fn follow_camera(value: &Value, world: &World) -> Result<CameraRequest, String> {
    let f = Fields::new(
        "`follow`",
        "key",
        Some(value),
        &["entity", "view", "distance_m"],
    )?;
    let entity = match f.get("entity") {
        None => return Err(f.missing("entity")),
        Some(v) => entity_of(v, world)?,
    };
    let view = f.str("view")?.unwrap_or("third_person");
    let distance = f.number("distance_m")?;
    match view {
        "first_person" => {
            if distance.is_some() {
                return Err(
                    "`follow`: `distance_m` belongs to view \"third_person\"; leave it out for \
                     \"first_person\""
                        .into(),
                );
            }
            Ok(CameraRequest::FirstPerson { entity })
        }
        "third_person" => {
            let distance_m = distance.unwrap_or(DEFAULT_FOLLOW_DISTANCE_M);
            if !(MIN_FOLLOW_DISTANCE_M..=MAX_FOLLOW_DISTANCE_M).contains(&distance_m) {
                return Err(format!(
                    "`follow`: `distance_m` is {distance_m}; it must be from \
                     {MIN_FOLLOW_DISTANCE_M} to {MAX_FOLLOW_DISTANCE_M} metres"
                ));
            }
            Ok(CameraRequest::ThirdPerson { entity, distance_m })
        }
        other => Err(format!(
            "`follow`: `view` must be \"first_person\" or \"third_person\", got \"{}\"",
            shorten(other)
        )),
    }
}

/// An entity of `world` by id (a number) or name (a string), as the engine's commands take it.
fn entity_of(value: &Value, world: &World) -> Result<EntityId, String> {
    match value {
        Value::Number(n) => {
            let id = n.as_u64().map(EntityId).ok_or_else(|| {
                format!(
                    "`follow`: `entity` must be an entity id (a whole number from 1) or a name, \
                     got {n}"
                )
            })?;
            match world.entity(id) {
                Some(_) => Ok(id),
                None => Err(format!(
                    "`follow`: no entity has the id {id}; world.inspect lists them"
                )),
            }
        }
        Value::String(name) => world.find(name).map_err(|e| format!("`follow`: {e}")),
        other => Err(format!(
            "`follow`: `entity` must be an entity id or name, got {}",
            describe(other)
        )),
    }
}

// ------------------------------------------------------------------------------ files

#[cfg(not(target_arch = "wasm32"))]
impl Host {
    fn file_new(&mut self, p: &Fields) -> Result<Value, String> {
        let name = p.str("name").map_err(strip_owner)?.unwrap_or("Untitled");
        let seed = match p.get("seed") {
            None => 0,
            Some(v) => v
                .as_u64()
                .ok_or_else(|| "`seed` must be a whole number from 0 to 2^64 - 1".to_string())?,
        };
        let world = World::new(name, seed);
        world.validate().map_err(|e| e.to_string())?;
        self.session.replace_document(world);
        self.path = None;
        self.project = None;
        self.saved_revision = self.session.revision();
        Ok(json!({"name": name, "seed": seed}))
    }

    fn file_open(&mut self, p: &Fields) -> Result<Value, String> {
        use crate::files::{Direction, read_world, resolve};
        let given = p.required_str("path").map_err(strip_owner)?;
        let path = resolve(&self.access, given, Direction::Read, "gamecraft")?;
        let world = read_world(&path, given)?;
        let result = json!({
            "path": path.display().to_string(),
            "name": world.name,
            "entities": world.entity_count(),
            "terrain": world.terrain.is_some(),
        });
        self.session.replace_document(world);
        self.path = Some(path);
        self.project = None;
        self.saved_revision = self.session.revision();
        Ok(result)
    }

    fn journal_save(&self, p: &Fields) -> Result<Value, String> {
        use crate::files::{Direction, resolve, write_whole};
        let overwrite = p.bool("overwrite").map_err(strip_owner)?.unwrap_or(false);
        let given = p.required_str("path").map_err(strip_owner)?;
        let journal = self
            .session
            .journal()
            .ok_or("there is no journal yet; it starts with sim.start")?;
        let target = resolve(&self.access, given, Direction::Write, "json")?;
        if target.exists() && !overwrite {
            return Err(format!(
                "`{}` exists; give overwrite: true to replace it",
                target.display()
            ));
        }
        let text = journal.to_json()?;
        write_whole(&target, text.as_bytes())?;
        Ok(json!({
            "path": target.display().to_string(),
            "bytes": text.len(),
            "entries": journal.entries.len(),
            "final": journal.end.as_ref().map(|e| json!({"tick": e.tick, "hash": e.hash})),
        }))
    }

    fn file_save(&mut self, p: &Fields) -> Result<Value, String> {
        use crate::files::{Direction, resolve, write_whole};
        let overwrite = p.bool("overwrite").map_err(strip_owner)?.unwrap_or(false);
        let given = match p.str("path").map_err(strip_owner)? {
            Some(path) => path.to_string(),
            None => self
                .path
                .as_ref()
                .map(|f| f.to_string_lossy().into_owned())
                .ok_or("the world has no file yet; give `path`")?,
        };
        let target = resolve(&self.access, &given, Direction::Write, "gamecraft")?;
        if target.exists() && self.path.as_ref() != Some(&target) && !overwrite {
            return Err(format!(
                "`{}` exists and is not the open file; give overwrite: true to replace it",
                target.display()
            ));
        }
        let text = self
            .session
            .document()
            .to_json()
            .map_err(|e| format!("the document cannot be written: {e}"))?;
        write_whole(&target, text.as_bytes())?;
        let result = json!({"path": target.display().to_string(), "bytes": text.len()});
        self.path = Some(target);
        self.saved_revision = self.session.revision();
        Ok(result)
    }
}

// ---------------------------------------------------------------------------- batches

/// The `failed` count of a batch report.
fn failed_steps(report: &Value) -> u64 {
    report.get("failed").and_then(Value::as_u64).unwrap_or(0)
}

/// Sets `key` of a batch report (an object).
fn set(report: &mut Value, key: &str, value: impl Into<Value>) {
    if let Some(map) = report.as_object_mut() {
        map.insert(key.into(), value.into());
    }
}

/// One step of a batch.
pub(crate) struct Step {
    /// The command id.
    pub command: String,
    /// Its parameters (an object).
    pub params: Value,
}

/// Reads and checks every step before any runs. `id_keys` is what the protocol calls the
/// command in a step (`command` in JSON lines; `id` or `command` in MCP); a step names it once.
/// `intent` is checked (a string) and not kept.
pub(crate) fn parse_steps(
    owner: &str,
    noun: &str,
    fields: &Fields,
    id_keys: &[&str],
) -> Result<Vec<Step>, String> {
    let items = fields
        .array("steps")?
        .ok_or_else(|| fields.missing("steps"))?;
    if items.len() > MAX_BATCH_STEPS {
        return Err(format!(
            "{owner}: `steps` has {} steps, and a batch takes at most {MAX_BATCH_STEPS}; split it \
             into several batches",
            items.len()
        ));
    }
    let mut allowed = id_keys.to_vec();
    allowed.extend(["params", "intent"]);
    let mut steps = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let step_owner = format!("{owner}: steps[{index}]");
        let id_key = id_keys.first().copied().unwrap_or("command");
        if !item.is_object() {
            return Err(format!(
                "{step_owner} must be an object {{\"{id_key}\": string, \"params\": object}}, got {}",
                describe(item)
            ));
        }
        let step = Fields::new(&step_owner, noun, Some(item), &allowed)?;
        let command = command_of(&step, id_keys)?;
        step.str("intent")?;
        steps.push(Step {
            command: command.to_string(),
            params: step.object("params")?,
        });
    }
    Ok(steps)
}

/// The command a request names under one of `keys` (exactly one of them).
pub(crate) fn command_of<'a>(fields: &Fields<'a>, keys: &[&str]) -> Result<&'a str, String> {
    let mut found = None;
    for key in keys {
        if let Some(command) = fields.str(key)? {
            if found.is_some() {
                return Err(format!(
                    "{}: give `{}`, not more than one of them",
                    fields.owner(),
                    keys.join("` or `")
                ));
            }
            found = Some(command);
        }
    }
    found.ok_or_else(|| fields.missing(&keys.join("` or `")))
}

/// How a batch runs.
#[derive(Clone, Copy)]
pub(crate) struct BatchOptions {
    /// Stop at the first command that returns an error (default true).
    pub stop_on_error: bool,
    /// Report only the failures and the last result.
    pub terse: bool,
    /// All or nothing: see [`Host::run_batch`].
    pub atomic: bool,
}

/// The outcome of a batch: the report both protocols return, and whether a step failed.
pub(crate) struct BatchOutcome {
    pub report: Value,
    pub failed: bool,
}

impl Host {
    /// Runs the steps in order, as 3DCraft and VisionCraft do: not a transaction, each step its
    /// own undo step; `failed` counts the commands that returned an error, and `stop_on_error`
    /// stops at the first. Report: `{completed, failed, results: [{ok, result | error, ms}],
    /// ms}`, or with `terse` `{completed, failed, errors: [{step, error, ms}], last: {step,
    /// result, ms}, ms}`.
    ///
    /// With `atomic` the steps run inside one [`Session::transaction`]: the first failure stops
    /// the batch and takes back everything it did (the document, the history, the play state
    /// and the open file are as before, `rolledBack: true`); a batch that succeeds is one undo
    /// step. Files a step wrote stay written either way.
    pub(crate) fn run_batch(&mut self, steps: &[Step], options: &BatchOptions) -> BatchOutcome {
        let whole = Stopwatch::start();
        let mut report = if options.atomic {
            self.atomic(steps, options)
        } else {
            self.run_steps(steps, options)
        };
        let failed = failed_steps(&report) > 0;
        if let Some(map) = report.as_object_mut() {
            map.insert("ms".into(), whole.ms());
        }
        BatchOutcome { report, failed }
    }

    fn atomic(&mut self, steps: &[Step], options: &BatchOptions) -> Value {
        if let Some((index, step)) = steps
            .iter()
            .enumerate()
            .find(|(_, s)| NOT_ATOMIC.contains(&s.command.as_str()))
        {
            let error = format!(
                "{}: cannot be part of an atomic batch; run it on its own",
                step.command
            );
            let mut report = json!({"completed": 0, "failed": 1, "rolledBack": true});
            if options.terse {
                set(
                    &mut report,
                    "errors",
                    json!([{"step": index, "error": error, "ms": Value::Null}]),
                );
                set(&mut report, "last", Value::Null);
            } else {
                set(
                    &mut report,
                    "results",
                    json!([{"ok": false, "error": error, "ms": Value::Null}]),
                );
            }
            return report;
        }
        let (path, saved_revision) = (self.path.clone(), self.saved_revision);
        // The session leaves the host for the transaction and comes back inside it, so the
        // steps run on the host as always (host commands included).
        let mut session = std::mem::replace(&mut self.session, Session::new(World::new("", 0)));
        let mut report = Value::Null;
        let inner = BatchOptions {
            stop_on_error: true,
            ..*options
        };
        let outcome = session.transaction(|inside| {
            std::mem::swap(&mut self.session, inside);
            report = self.run_steps(steps, &inner);
            std::mem::swap(&mut self.session, inside);
            if failed_steps(&report) > 0 {
                Err(CommandError::Failed("a step failed; rolled back".into()))
            } else {
                Ok(())
            }
        });
        self.session = session;
        let rolled_back = outcome.is_err();
        if rolled_back {
            self.path = path;
            self.saved_revision = saved_revision;
        }
        set(&mut report, "rolledBack", rolled_back);
        report
    }

    fn run_steps(&mut self, steps: &[Step], options: &BatchOptions) -> Value {
        let (mut results, mut errors, mut last) = (Vec::new(), Vec::new(), Value::Null);
        let (mut completed, mut failed) = (0usize, 0usize);
        for (index, step) in steps.iter().enumerate() {
            let clock = Stopwatch::start();
            let outcome = self.execute(&step.command, &step.params);
            let ms = clock.ms();
            match outcome {
                Ok(result) => {
                    completed += 1;
                    if options.terse {
                        last = json!({"step": index, "result": result, "ms": ms});
                    } else {
                        results.push(json!({"ok": true, "result": result, "ms": ms}));
                    }
                }
                Err(error) => {
                    failed += 1;
                    if options.terse {
                        errors.push(json!({"step": index, "error": error, "ms": ms}));
                    } else {
                        results.push(json!({"ok": false, "error": error, "ms": ms}));
                    }
                    if options.stop_on_error {
                        break;
                    }
                }
            }
        }
        let mut report = json!({"completed": completed, "failed": failed});
        if options.terse {
            set(&mut report, "errors", errors);
            set(&mut report, "last", last);
        } else {
            set(&mut report, "results", results);
        }
        report
    }
}
