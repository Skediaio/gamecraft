//! The Model Context Protocol over stdio (`gamecraft-cli mcp`): JSON-RPC 2.0, one message per
//! line.
//!
//! Ported from VisionCraft's `crates/visioncraft-automation/src/mcp.rs` (MIT), itself from
//! 3DCraft's `crates/automation/src/mcp.rs`: the same revisions, the same tool names and
//! argument conventions, the same two kinds of error. A request the server cannot act on
//! (unknown method or tool, an unknown or ill-typed argument) is a JSON-RPC error, raised before
//! anything runs; a command that ran and failed is a result with `isError: true`, so the model
//! reads the message and corrects itself.
//!
//! | Tool | Does |
//! |---|---|
//! | `command_list` | the commands, with their parameter docs and whether each can run now |
//! | `command_run` | runs one command |
//! | `command_batch` | runs several, optionally all or nothing |
//! | `doc_inspect` | `world.inspect` |
//! | `doc_render_preview` | a PNG of the current world as an image block |
//!
//! Adapted: GameCraft's tools, resources and prompt; no window tool (`control_call`) and no
//! agent log; `command_run` and a batch step take the command as `id` or as `command`.

use crate::commands::{BatchOptions, ListQuery, Stopwatch, command_of, panic_text, parse_steps};
use crate::fields::{Fields, describe, shorten};
use crate::host::Host;
use crate::rpc::serialize;
use crate::{MAX_BATCH_STEPS, MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
use serde_json::{Map, Value, json};
use std::io::{self, BufRead, Write};

/// The MCP revisions this server speaks, latest first (as 3DCraft and VisionCraft).
/// `initialize` echoes the client's revision when it is one of these, and offers the first
/// otherwise.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The server's name in `initialize`.
pub const SERVER_NAME: &str = "gamecraft";

/// Default size of a preview, pixels.
const PREVIEW_DEFAULT_WIDTH: u64 = 640;
const PREVIEW_DEFAULT_HEIGHT: u64 = 360;
/// Largest PNG a preview returns, bytes.
const PREVIEW_MAX_PNG_BYTES: usize = 5 << 20;

const INSTRUCTIONS: &str = "GameCraft is a game engine driven by commands: a world of terrain, environment (time, sun, weather) and entities with components (shape, plant, character), \
edited as a document with undo, and played by a deterministic simulation that the same commands steer while it runs. Every action is a command with a stable id and JSON params. \
Start with command_list {\"brief\": true} (then {\"id\": ...} for one command's parameter docs) and doc_inspect. \
Build with command_batch (atomic: true for all or nothing), check with doc_inspect and look with doc_render_preview. \
Units are metres, seconds and degrees; Y is up, the ground is x and z, north is -Z and east is +X; an entity's position is its base, the point it stands on; entities are named by id or by name. \
Play with sim.start, sim.input (what a character is asked to do), sim.step (whole ticks of 1/60 s; the answer has the state hash) and sim.stop; world commands sent while playing change the running world, not the document. \
Open and save with command_run file.open / file.save; undo with edit.undo. The resource gamecraft://guide is the agent guide.";

const GUIDE_URI: &str = "gamecraft://guide";
const WORLD_URI: &str = "gamecraft://world";
const GUIDE: &str = include_str!("../../../docs/agent.md");
const PROMPT: &str = "build_world";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

struct RpcError {
    code: i64,
    message: String,
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError {
        code: INVALID_PARAMS,
        message: message.into(),
    }
}

/// Handles one message. Returns the reply line, or `None` for a notification, a response or a
/// blank line. Never panics, whatever the line holds.
pub fn handle_line(host: &mut Host, line: &str) -> Option<String> {
    handle_line_capped(host, line, MAX_REPLY_BYTES)
}

/// Serves messages from `input` until it ends, flushing each reply.
pub fn serve(host: &mut Host, input: impl BufRead, output: impl Write) -> io::Result<()> {
    crate::lines::serve(input, output, |line| match line {
        Ok(line) => handle_line(host, line),
        Err(why) => Some(error_reply(&Value::Null, PARSE_ERROR, &why)),
    })
}

fn handle_line_capped(host: &mut Host, line: &str, max_reply: usize) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    if line.len() > MAX_REQUEST_BYTES {
        return Some(error_reply(
            &Value::Null,
            INVALID_REQUEST,
            &crate::lines::too_long_message(),
        ));
    }
    let message: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return Some(error_reply(
                &Value::Null,
                PARSE_ERROR,
                &format!("the message is not valid JSON: {e}"),
            ));
        }
    };
    let Value::Object(message) = message else {
        let why = if message.is_array() {
            "JSON-RPC batches are not supported; send one message per line".to_string()
        } else {
            format!(
                "a message must be a JSON-RPC 2.0 object, got {}",
                describe(&message)
            )
        };
        return Some(error_reply(&Value::Null, INVALID_REQUEST, &why));
    };
    let id = match message.get("id") {
        None => None,
        Some(id @ (Value::String(_) | Value::Number(_))) => Some(id),
        Some(other) => {
            return Some(error_reply(
                &Value::Null,
                INVALID_REQUEST,
                &format!("`id` must be a string or a number, got {}", describe(other)),
            ));
        }
    };
    let reply_id = id.unwrap_or(&Value::Null);
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error_reply(
            reply_id,
            INVALID_REQUEST,
            "`jsonrpc` must be \"2.0\"",
        ));
    }
    let method = match message.get("method") {
        Some(Value::String(m)) => m.as_str(),
        // A response to a request of ours: this server sends none.
        None if id.is_some()
            && (message.contains_key("result") || message.contains_key("error")) =>
        {
            return None;
        }
        Some(other) => {
            return Some(error_reply(
                reply_id,
                INVALID_REQUEST,
                &format!("`method` must be a string, got {}", describe(other)),
            ));
        }
        None => {
            return Some(error_reply(
                reply_id,
                INVALID_REQUEST,
                "the message has no `method`",
            ));
        }
    };
    // Notifications are never answered, nor acted on (JSON-RPC 2.0, section 4.1).
    let id = id?;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(host, method, message.get("params"))
    }))
    .unwrap_or_else(|payload| {
        Err(RpcError {
            code: INTERNAL_ERROR,
            message: format!(
                "{}: internal error (a bug): {}",
                shorten(method),
                panic_text(payload.as_ref())
            ),
        })
    });
    let reply = match outcome {
        Ok(result) => serialize(&json!({ "jsonrpc": "2.0", "id": id, "result": result })),
        Err(error) => return Some(error_reply(id, error.code, &error.message)),
    };
    if reply.len() > max_reply {
        return Some(error_reply(
            id,
            INTERNAL_ERROR,
            &format!(
                "{}: the reply is {} bytes, over the limit of {max_reply} bytes; ask for less at \
                 a time (a filter, a smaller limit, fewer batch steps, a smaller picture)",
                shorten(method),
                reply.len()
            ),
        ));
    }
    Some(reply)
}

fn dispatch(host: &mut Host, method: &str, params: Option<&Value>) -> Result<Value, RpcError> {
    let params = match params {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => Some(map),
        Some(other) => {
            return Err(invalid(format!(
                "{}: `params` must be an object, got {}",
                shorten(method),
                describe(other)
            )));
        }
    };
    match method {
        "initialize" => initialize(params),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call_tool(host, params),
        "resources/list" => Ok(json!({ "resources": [
            {"uri": WORLD_URI, "name": "world", "title": "The current world", "description": "world.inspect of the current world (the running world while playing), as the tool doc_inspect gives it.", "mimeType": "application/json"},
            {"uri": GUIDE_URI, "name": "guide", "title": "Driving GameCraft", "description": "The agent guide: conventions, the working loop, editing and playing, and a worked example.", "mimeType": "text/markdown"},
        ]})),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "resources/read" => {
            let uri = params
                .and_then(|p| p.get("uri"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    invalid("resources/read: `uri` must be a string; resources/list lists them")
                })?;
            let (mime, text) = match uri {
                GUIDE_URI => ("text/markdown", GUIDE.to_string()),
                WORLD_URI => {
                    let v = host
                        .execute("world.inspect", &Value::Null)
                        .map_err(|m| RpcError {
                            code: INTERNAL_ERROR,
                            message: m,
                        })?;
                    ("application/json", serialize(&v))
                }
                other => {
                    return Err(invalid(format!(
                        "resources/read: unknown resource `{}`; the resources are: {WORLD_URI}, \
                         {GUIDE_URI}",
                        shorten(other)
                    )));
                }
            };
            Ok(json!({ "contents": [{ "uri": uri, "mimeType": mime, "text": text }] }))
        }
        "prompts/list" => Ok(json!({ "prompts": [{
            "name": PROMPT,
            "title": "Build a world from a description",
            "description": "Turns a description of a place into a GameCraft world, step by step, checks it with doc_inspect and looks at it with doc_render_preview.",
            "arguments": [{"name": "description", "description": "The world in words: its size and relief, the weather and time of day, what stands where, the characters.", "required": true}],
        }]})),
        "prompts/get" => {
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if name != PROMPT {
                return Err(invalid(format!(
                    "prompts/get: unknown prompt `{}`; the prompts are: {PROMPT}",
                    shorten(name)
                )));
            }
            let description = params
                .and_then(|p| p.get("arguments"))
                .and_then(|a| a.get("description"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    invalid("prompts/get: build_world needs the argument `description`")
                })?;
            Ok(json!({
                "description": "Build a world from a description",
                "messages": [{"role": "user", "content": {"type": "text", "text": build_world_prompt(description)}}],
            }))
        }
        other => Err(RpcError {
            code: METHOD_NOT_FOUND,
            message: format!(
                "unknown method `{}`; this server implements initialize, ping, tools/list, \
                 tools/call, resources/list, resources/templates/list, resources/read, \
                 prompts/list and prompts/get",
                shorten(other)
            ),
        }),
    }
}

fn build_world_prompt(description: &str) -> String {
    format!(
        "Build this world in GameCraft:\n\n{description}\n\n\
Work in this order, and check after each stage:\n\
1. Read the resource gamecraft://guide once, then command_list {{\"brief\": true}} and doc_inspect {{\"entities\": false}} to see what is there.\n\
2. Plan in numbers first: metres, Y up, the ground is x and z, north is -Z and east is +X. Write down the terrain (size, cell, relief), the time of day and weather, and every entity's name, position on the ground ([x, z]) and components.\n\
3. Build with command_batch {{\"atomic\": true}}: terrain.set, environment.set, then entity.add for each thing (`at` puts its base on the ground; shapes block out buildings and rocks, plants come from species.list when it exists, a character is what can be played).\n\
4. After each batch: doc_inspect and compare names, positions and counts with your plan; then doc_render_preview (a camera, or follow a character) and look at the picture.\n\
5. To try it: sim.start, sim.input for a character, sim.step, and read the positions and the state hash; sim.stop when done (keep: false leaves the document as built).\n\
6. Save with command_run file.save {{\"path\": \"...\"}} when the server may write, and say what you built, with the numbers."
    )
}

fn initialize(params: Option<&Map<String, Value>>) -> Result<Value, RpcError> {
    let latest = PROTOCOL_VERSIONS.first().copied().unwrap_or("2025-11-25");
    let version = match params.and_then(|p| p.get("protocolVersion")) {
        None | Some(Value::Null) => latest,
        Some(Value::String(requested)) => PROTOCOL_VERSIONS
            .iter()
            .copied()
            .find(|v| v == requested)
            .unwrap_or(latest),
        Some(other) => {
            return Err(invalid(format!(
                "initialize: `protocolVersion` must be a string, got {}",
                describe(other)
            )));
        }
    };
    Ok(json!({
        "protocolVersion": version,
        "capabilities": { "tools": {}, "resources": {}, "prompts": {} },
        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    }))
}

/// The names of the tools, in `tools/list` order.
pub fn tool_names() -> Vec<String> {
    tools()
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn tools() -> Vec<Value> {
    let camera = json!({"type": "object", "description": "{position: [x, y, z], target: [x, y, z], fov_y_deg?: 60}: a camera at position looking at target (metres, degrees)."});
    let follow = json!({"type": "object", "description": "{entity: id or name, view?: \"first_person\" | \"third_person\" (default), distance_m?: 6}: through an entity's eyes, or behind it."});
    vec![
        json!({
            "name": "command_list",
            "description": "Lists GameCraft's commands: id, label, params notation, summary, parameters (name, type, unit, required, default, allowed, doc), result, example, scope (world, query, play or host) and whether each can run now. Call it with brief: true first (id, params and the first sentence of each summary), then with id for the full entry of a command you use.",
            "inputSchema": {"type": "object", "properties": {
                "brief": {"type": "boolean", "description": "Only id, params, the first sentence of the summary and enabled. Default false. Start with this."},
                "id": {"type": "string", "description": "One command by its exact id, e.g. \"entity.add\": its full entry."},
                "filter": {"type": "string", "description": "Case-insensitive part of a command id or label, e.g. \"sim\"."}
            }, "additionalProperties": false}
        }),
        json!({
            "name": "command_run",
            "description": "Runs one command by id and returns its JSON result (its wall-clock milliseconds in the tool result's _meta.ms). A failed command returns isError with a message that starts with the command's id and names the parameter. World: world.set, terrain.set, environment.set, entity.*; play: sim.start, sim.input, sim.step, sim.state, sim.stop; look: world.inspect, view.render (an image block); files: file.new, file.open, file.save; history: edit.undo, edit.redo.",
            "inputSchema": {"type": "object", "properties": {
                "id": {"type": "string", "description": "A command id from command_list, e.g. \"entity.add\"."},
                "command": {"type": "string", "description": "The same as id (the name the JSON-lines protocol uses); give one of the two."},
                "params": {"type": "object", "description": "The command's parameters, as command_list documents them. Omit for none."},
                "intent": {"type": "string", "maxLength": 300, "description": "Why you run it, in a short sentence of your own. Optional; checked, not recorded yet."}
            }, "additionalProperties": false}
        }),
        json!({
            "name": "command_batch",
            "description": "Runs up to 256 commands in order and returns {completed, failed, results: [{ok, result | error, ms}], ms}. stop_on_error (default true) stops at the first command that returns an error, and the batch is then an error (isError) with the report as its content. Not atomic: steps that ran stay applied, each its own undo step. terse reports {completed, failed, errors: [{step, error, ms}], last: {step, result, ms}, ms}. atomic: true makes it all or nothing: the first failure takes back every step (rolledBack: true), and a batch that succeeds is one undo step; file.new, file.open, edit.undo and edit.redo cannot be part of one.",
            "inputSchema": {"type": "object", "properties": {
                "steps": {"type": "array", "maxItems": MAX_BATCH_STEPS, "items": {"type": "object", "properties": {
                    "id": {"type": "string", "description": "A command id."},
                    "command": {"type": "string", "description": "The same as id; give one of the two."},
                    "params": {"type": "object", "description": "The command's parameters."},
                    "intent": {"type": "string", "maxLength": 300, "description": "Why this step."}
                }, "additionalProperties": false}},
                "stop_on_error": {"type": "boolean", "description": "Stop at the first command that returns an error. Default true."},
                "terse": {"type": "boolean", "description": "Report only the failures and the last result. Default false."},
                "atomic": {"type": "boolean", "description": "All or nothing: the first failure takes back every step; success is one undo step. Default false."},
                "intent": {"type": "string", "maxLength": 300, "description": "Why you run these steps, in a short sentence of your own. Optional."}
            }, "required": ["steps"], "additionalProperties": false}
        }),
        json!({
            "name": "doc_inspect",
            "description": "Returns the current world as JSON (world.inspect; the running world while playing): name, seed, revision, whether it plays and its tick, the terrain with its height range, the environment and the sun, the entities in id order with their components (a page at a time), and the history.",
            "inputSchema": {"type": "object", "properties": {
                "entities": {"type": "boolean", "description": "List the entities. Default true."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "description": "Most entities listed. Default 200."},
                "offset": {"type": "integer", "minimum": 0, "description": "Entities skipped first, in id order. Default 0."}
            }, "additionalProperties": false}
        }),
        json!({
            "name": "doc_render_preview",
            "description": "Draws the current world (the running world while playing) to a PNG with GameCraft's renderer and returns it as an image block, with what the renderer says about it as text. The whole world framed, from a camera, or following an entity; at most one of camera and follow. The world does not change.",
            "inputSchema": {"type": "object", "properties": {
                "width": {"type": "integer", "minimum": 16, "maximum": 4096, "description": "Width in pixels. Default 640."},
                "height": {"type": "integer", "minimum": 16, "maximum": 4096, "description": "Height in pixels. Default 360."},
                "camera": camera,
                "follow": follow
            }, "additionalProperties": false}
        }),
    ]
}

/// Why a tool did not answer: a request it cannot act on (a JSON-RPC error), or a command that
/// ran and failed (`isError`).
enum Failure {
    Invalid(String),
    Failed(String),
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure::Invalid(message)
    }
}

fn text_block(text: String) -> Value {
    json!({ "type": "text", "text": text })
}

fn call_tool(host: &mut Host, params: Option<&Map<String, Value>>) -> Result<Value, RpcError> {
    let name = match params.and_then(|p| p.get("name")) {
        Some(Value::String(n)) => n.as_str(),
        Some(other) => {
            return Err(invalid(format!(
                "tools/call: `name` must be a string, got {}",
                describe(other)
            )));
        }
        None => return Err(invalid("tools/call: missing `name`")),
    };
    let args = params.and_then(|p| p.get("arguments"));
    let clock = Stopwatch::start();
    let outcome: Result<Vec<Value>, Failure> = match name {
        "command_list" => (|| {
            let a = Fields::new(name, "argument", args, &["filter", "id", "brief"])?;
            let q = ListQuery {
                filter: a.str("filter")?,
                id: a.str("id")?,
                brief: a.bool("brief")?.unwrap_or(false),
            };
            let list = host
                .command_list(&q)
                .map_err(|e| Failure::Failed(format!("{name}: {e}")))?;
            Ok(vec![text_block(serialize(&list))])
        })(),
        "command_run" => (|| {
            let a = Fields::new(
                name,
                "argument",
                args,
                &["id", "command", "params", "intent"],
            )?;
            let id = command_of(&a, &["id", "command"])?;
            let p = a.object("params")?;
            a.str("intent")?;
            // A picture (`view.render` without a path) comes back as an image block, with the
            // rest of the answer as text.
            host.execute(id, &p)
                .map(|mut r| picture_blocks(&mut r))
                .map_err(Failure::Failed)
        })(),
        "command_batch" => (|| {
            let a = Fields::new(
                name,
                "argument",
                args,
                &["steps", "stop_on_error", "terse", "intent", "atomic"],
            )?;
            let options = BatchOptions {
                stop_on_error: a.bool("stop_on_error")?.unwrap_or(true),
                terse: a.bool("terse")?.unwrap_or(false),
                atomic: a.bool("atomic")?.unwrap_or(false),
            };
            a.str("intent")?;
            let steps = parse_steps(name, "argument", &a, &["id", "command"])?;
            let outcome = host.run_batch(&steps, &options);
            let text = serialize(&outcome.report);
            if outcome.failed {
                Err(Failure::Failed(text))
            } else {
                Ok(vec![text_block(text)])
            }
        })(),
        "doc_inspect" => (|| {
            Fields::new(name, "argument", args, &["entities", "limit", "offset"])?;
            let p = args.cloned().unwrap_or(Value::Null);
            host.execute("world.inspect", &p)
                .map(|r| vec![text_block(serialize(&r))])
                .map_err(Failure::Failed)
        })(),
        "doc_render_preview" => preview(host, args),
        other => Err(Failure::Invalid(format!(
            "unknown tool `{}`; the tools are: {}",
            shorten(other),
            tool_names().join(", ")
        ))),
    };
    let (content, is_error) = match outcome {
        Ok(content) => (content, false),
        Err(Failure::Invalid(message)) => return Err(invalid(message)),
        Err(Failure::Failed(message)) => (vec![text_block(message)], true),
    };
    let mut reply = json!({ "content": content, "isError": is_error });
    if name == "command_run"
        && let Some(map) = reply.as_object_mut()
    {
        map.insert("_meta".into(), json!({ "ms": clock.ms() }));
    }
    Ok(reply)
}

/// A result with `{mime: "image/png", base64}` as an image block and the rest as text.
fn picture_blocks(result: &mut Value) -> Vec<Value> {
    let mut content = Vec::new();
    if let Some(map) = result.as_object_mut()
        && map.get("mime").and_then(Value::as_str) == Some("image/png")
        && let Some(Value::String(data)) = map.remove("base64")
    {
        map.remove("mime");
        content.push(json!({ "type": "image", "data": data, "mimeType": "image/png" }));
    }
    content.push(text_block(serialize(result)));
    content
}

fn preview(host: &mut Host, args: Option<&Value>) -> Result<Vec<Value>, Failure> {
    let name = "doc_render_preview";
    let a = Fields::new(
        name,
        "argument",
        args,
        &["width", "height", "camera", "follow"],
    )?;
    let mut params = Map::new();
    params.insert("width".into(), PREVIEW_DEFAULT_WIDTH.into());
    params.insert("height".into(), PREVIEW_DEFAULT_HEIGHT.into());
    for key in ["width", "height", "camera", "follow"] {
        if let Some(v) = a.get(key) {
            params.insert(key.into(), v.clone());
        }
    }
    let mut result = host
        .execute("view.render", &Value::Object(params))
        .map_err(Failure::Failed)?;
    let bytes = result
        .get("base64")
        .and_then(Value::as_str)
        .map_or(0, |b| b.len() / 4 * 3);
    if bytes > PREVIEW_MAX_PNG_BYTES {
        return Err(Failure::Failed(format!(
            "{name}: the PNG is about {bytes} bytes, over the limit of {PREVIEW_MAX_PNG_BYTES}; \
             ask for a smaller picture"
        )));
    }
    Ok(picture_blocks(&mut result))
}

fn error_reply(id: &Value, code: i64, message: &str) -> String {
    serialize(&json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_latest_protocol_version_is_listed_first() {
        let mut sorted = super::PROTOCOL_VERSIONS.to_vec();
        sorted.sort_unstable();
        sorted.reverse();
        assert_eq!(sorted, super::PROTOCOL_VERSIONS);
    }

    #[test]
    fn the_tools_keep_their_names() {
        assert_eq!(
            super::tool_names(),
            [
                "command_list",
                "command_run",
                "command_batch",
                "doc_inspect",
                "doc_render_preview"
            ]
        );
    }
}
