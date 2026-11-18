//! The JSON-lines protocol (`gamecraft-cli serve`): one request per line, one reply per line
//! with the same `id`.
//!
//! ```text
//! {"id": 1, "method": "engine.execute", "params": {"command": "world.set", "params": {"name": "Valley"}}}
//! {"id": 1, "ok": true, "result": null, "ms": 0.1}
//! {"id": 2, "method": "engine.execute", "params": {"command": "world.sett"}}
//! {"id": 2, "ok": false, "error": "unknown command `world.sett`; did you mean `world.set`?", "ms": 0.0}
//! ```
//!
//! Ported from VisionCraft's `crates/visioncraft-automation/src/rpc.rs` (MIT), itself from
//! 3DCraft's `crates/automation/src/rpc.rs`: the same envelope, methods, limits and checks.
//! Every request gets exactly one reply; when its id cannot be read the reply has `"id": null`.
//! Blank lines are skipped. Adapted: no host methods of a window, no `stopOnBreak`.

use crate::commands::{BatchOptions, ListQuery, Stopwatch, command_of, panic_text, parse_steps};
use crate::fields::{Fields, describe, shorten};
use crate::host::Host;
use crate::{MAX_BATCH_STEPS, MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

/// A method, as `methods` lists it.
pub struct Method {
    /// The method's name.
    pub name: &'static str,
    /// The parameters, in the notation of the command list.
    pub params: &'static str,
    /// What it does.
    pub summary: &'static str,
}

/// Every method of the protocol, in a fixed order.
pub const METHODS: &[Method] = &[
    Method {
        name: "engine.execute",
        params: r#"{"command": string, "params": object = {}, "intent": string = not set}"#,
        summary: "Runs a command by id and returns its result; the reply has ms, the wall-clock milliseconds it took. intent (optional): why, in the agent's own words; checked to be a string, not recorded yet.",
    },
    Method {
        name: "engine.commands",
        params: r#"{"filter": string = not set, "id": string = not set, "brief": boolean = false}"#,
        summary: "Lists the commands with their parameters (name, type, unit, required, default, allowed values, doc), result, example, scope and whether each can run now. filter is a case-insensitive part of the id or label; id picks one command; brief gives {id, params, summary, enabled}.",
    },
    Method {
        name: "batch",
        params: r#"{"steps": [{"command": string, "params": object = {}, "intent": string = not set}], "stopOnError": boolean = true, "terse": boolean = false, "atomic": boolean = false, "intent": string = not set}"#,
        summary: "Runs up to 256 commands in order and returns {completed, failed, results: [{ok, result | error, ms}], ms}; with terse, {completed, failed, errors: [{step, error, ms}], last: {step, result, ms}, ms}. stopOnError stops at the first command that returns an error. Not a transaction: earlier steps stay applied, each its own undo step. atomic: true makes it all or nothing: the first failure takes back every step (rolledBack: true), and a batch that succeeds is one undo step; file.new, file.open, edit.undo and edit.redo cannot be part of one.",
    },
    Method {
        name: "methods",
        params: "{}",
        summary: "Lists the methods of this protocol and its limits.",
    },
];

/// Handles one request line. Returns the reply line (without a newline), or `None` for a
/// blank line. Never panics, whatever the line holds.
pub fn handle_line(host: &mut Host, line: &str) -> Option<String> {
    handle_line_capped(host, line, MAX_REPLY_BYTES)
}

/// Serves requests from `input` until it ends, flushing each reply before the next request is
/// read.
pub fn serve(host: &mut Host, input: impl BufRead, output: impl Write) -> io::Result<()> {
    crate::lines::serve(input, output, |line| match line {
        Ok(line) => handle_line(host, line),
        Err(why) => Some(error_reply(&Value::Null, &why)),
    })
}

/// The reply to a request that failed, as one line.
fn error_reply(id: &Value, error: &str) -> String {
    serialize(&json!({ "id": id, "ok": false, "error": error }))
}

fn handle_line_capped(host: &mut Host, line: &str, max_reply: usize) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    if line.len() > MAX_REQUEST_BYTES {
        return Some(error_reply(&Value::Null, &crate::lines::too_long_message()));
    }
    let request: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => {
            return Some(error_reply(
                &Value::Null,
                &format!("the request is not valid JSON: {e}"),
            ));
        }
    };
    let Value::Object(request) = request else {
        return Some(error_reply(
            &Value::Null,
            &format!(
                "a request must be a JSON object {{\"id\", \"method\", \"params\"}}, got {}",
                describe(&request)
            ),
        ));
    };
    let id = request.get("id").unwrap_or(&Value::Null);
    if let Some(unknown) = request
        .keys()
        .find(|k| !matches!(k.as_str(), "id" | "method" | "params"))
    {
        return Some(error_reply(
            id,
            &format!(
                "unknown request field `{}`; a request has id, method and params",
                shorten(unknown)
            ),
        ));
    }
    let method = match request.get("method") {
        Some(Value::String(m)) => m.as_str(),
        Some(other) => {
            return Some(error_reply(
                id,
                &format!("`method` must be a string, got {}", describe(other)),
            ));
        }
        None => {
            return Some(error_reply(
                id,
                "the request has no `method`; call `methods` for the list",
            ));
        }
    };
    let params = request.get("params");
    let clock = Stopwatch::start();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(host, method, params)
    }))
    .unwrap_or_else(|payload| {
        Err(format!(
            "{}: internal error (a bug): {}",
            shorten(method),
            panic_text(payload.as_ref())
        ))
    });
    let timed = method == "engine.execute";
    let reply = match (outcome, timed) {
        (Ok(result), true) => {
            serialize(&json!({ "id": id, "ok": true, "result": result, "ms": clock.ms() }))
        }
        (Ok(result), false) => serialize(&json!({ "id": id, "ok": true, "result": result })),
        (Err(error), true) => {
            return Some(serialize(
                &json!({ "id": id, "ok": false, "error": error, "ms": clock.ms() }),
            ));
        }
        (Err(error), false) => return Some(error_reply(id, &error)),
    };
    if reply.len() > max_reply {
        return Some(error_reply(
            id,
            &format!(
                "{}: the reply is {} bytes, over the limit of {max_reply} bytes; ask for less at \
                 a time (a filter, a smaller limit, fewer batch steps, a smaller picture, a path)",
                shorten(method),
                reply.len()
            ),
        ));
    }
    Some(reply)
}

fn dispatch(host: &mut Host, method: &str, params: Option<&Value>) -> Result<Value, String> {
    match method {
        "engine.execute" => {
            let p = Fields::new(
                method,
                "parameter",
                params,
                &["command", "params", "intent"],
            )?;
            let command = command_of(&p, &["command"])?;
            let command_params = p.object("params")?;
            p.str("intent")?;
            host.execute(command, &command_params)
        }
        "engine.commands" => {
            let p = Fields::new(method, "parameter", params, &["filter", "id", "brief"])?;
            let query = ListQuery {
                filter: p.str("filter")?,
                id: p.str("id")?,
                brief: p.bool("brief")?.unwrap_or(false),
            };
            host.command_list(&query)
                .map_err(|e| format!("{method}: {e}"))
        }
        "batch" => {
            let p = Fields::new(
                method,
                "parameter",
                params,
                &["steps", "stopOnError", "terse", "intent", "atomic"],
            )?;
            let options = BatchOptions {
                stop_on_error: p.bool("stopOnError")?.unwrap_or(true),
                terse: p.bool("terse")?.unwrap_or(false),
                atomic: p.bool("atomic")?.unwrap_or(false),
            };
            p.str("intent")?;
            let steps = parse_steps(method, "parameter", &p, &["command"])?;
            Ok(host.run_batch(&steps, &options).report)
        }
        "methods" => {
            Fields::new(method, "parameter", params, &[])?;
            let methods: Vec<Value> = METHODS
                .iter()
                .map(|m| json!({ "name": m.name, "params": m.params, "summary": m.summary }))
                .collect();
            Ok(json!({
                "methods": methods,
                "limits": {"requestBytes": MAX_REQUEST_BYTES, "replyBytes": MAX_REPLY_BYTES, "batchSteps": MAX_BATCH_STEPS},
            }))
        }
        other => {
            let names: Vec<&str> = METHODS.iter().map(|m| m.name).collect();
            Err(format!(
                "unknown method `{}`; the methods are: {}",
                shorten(other),
                names.join(", ")
            ))
        }
    }
}

/// One line of JSON; the fallback keeps a failure to serialise an error reply, not a panic.
pub(crate) fn serialize(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| {
        r#"{"id":null,"ok":false,"error":"the reply could not be written as JSON (a bug)"}"#.into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Access;
    use gamecraft_world::World;

    #[test]
    fn a_reply_over_the_limit_is_an_error() {
        let mut host = Host::new(World::new("x", 0), Access::default());
        let line = r#"{"id": 3, "method": "engine.commands"}"#;
        let reply: Value =
            serde_json::from_str(&handle_line_capped(&mut host, line, 100).unwrap()).unwrap();
        assert_eq!(reply["id"], 3);
        let error = reply["error"].as_str().unwrap();
        assert!(error.contains("over the limit of 100 bytes"), "{error}");
    }

    #[test]
    fn the_methods_are_documented_in_order() {
        let names: Vec<&str> = METHODS.iter().map(|m| m.name).collect();
        assert_eq!(
            names,
            ["engine.execute", "engine.commands", "batch", "methods"]
        );
    }
}
