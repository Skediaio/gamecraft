//! Random and mutated lines into both servers: nothing panics, and every line that must be
//! answered gets exactly one reply that is valid JSON.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use gamecraft_automation::{mcp, rpc};
use gamecraft_math::Rng;
use serde_json::Value;

const LINES: usize = 20_000;

const SEEDS: &[&str] = &[
    r#"{"id": 1, "method": "engine.execute", "params": {"command": "entity.add", "params": {"name": "Rock", "at": [10, 5], "components": [{"type": "shape", "kind": "box"}]}}}"#,
    r#"{"id": 2, "method": "engine.execute", "params": {"command": "sim.start"}}"#,
    r#"{"id": 3, "method": "engine.execute", "params": {"command": "sim.step", "params": {"ticks": 3}}}"#,
    r#"{"id": 4, "method": "engine.execute", "params": {"command": "sim.input", "params": {"entity": "Scout", "forward": 1}}}"#,
    r#"{"id": 5, "method": "batch", "params": {"steps": [{"command": "world.set", "params": {"seed": 4}}, {"command": "edit.undo"}], "atomic": true}}"#,
    r#"{"id": 6, "method": "engine.commands", "params": {"brief": true, "filter": "sim"}}"#,
    r#"{"id": 7, "method": "engine.execute", "params": {"command": "view.render", "params": {"follow": {"entity": 3}}}}"#,
    r#"{"id": 8, "method": "engine.execute", "params": {"command": "file.save", "params": {"path": "x.gamecraft"}}}"#,
    r#"{"id": 9, "method": "engine.execute", "params": {"command": "edit.undo"}}"#,
    r#"{"id": 10, "method": "engine.execute", "params": {"command": "sim.stop", "params": {"keep": true}}}"#,
    r#"{"id": 11, "method": "methods"}"#,
    r#"{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "command_run", "arguments": {"id": "entity.set", "params": {"id": "Crate", "at": [1, 2]}}}}"#,
    r#"{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "doc_inspect", "arguments": {"limit": 2}}}"#,
    r#"{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "doc_render_preview", "arguments": {"width": 64}}}"#,
    r#"{"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "command_batch", "arguments": {"steps": [{"id": "sim.step"}]}}}"#,
    r#"{"jsonrpc": "2.0", "id": 5, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}"#,
    r#"{"jsonrpc": "2.0", "id": 6, "method": "resources/read", "params": {"uri": "gamecraft://world"}}"#,
    r#"{"jsonrpc": "2.0", "id": 7, "method": "prompts/get", "params": {"name": "build_world", "arguments": {"description": "x"}}}"#,
];

const ALPHABET: &[char] = &[
    '{', '}', '[', ']', '"', ':', ',', ' ', '0', '1', '9', '-', '.', 'e', 'a', 'z', '\\', 'n', 't',
    'u', 'l', 'é', '😀', '\u{0}', '\u{202e}', '\t',
];

fn pick<'a, T>(rng: &mut Rng, items: &'a [T]) -> &'a T {
    &items[rng.below(items.len() as u64) as usize]
}

fn random_line(rng: &mut Rng) -> String {
    let len = rng.below(60) as usize;
    (0..len).map(|_| *pick(rng, ALPHABET)).collect()
}

fn mutated_line(rng: &mut Rng) -> String {
    let mut chars: Vec<char> = pick(rng, SEEDS).chars().collect();
    for _ in 0..=rng.below(4) {
        let at = rng.below(chars.len() as u64 + 1) as usize;
        match rng.below(3) {
            0 if at < chars.len() => {
                chars.remove(at);
            }
            1 if at < chars.len() => chars[at] = *pick(rng, ALPHABET),
            _ => chars.insert(at, *pick(rng, ALPHABET)),
        }
    }
    chars.into_iter().collect()
}

/// Whether a line the MCP server may leave unanswered is one: a notification or a response.
fn may_be_silent(line: &str) -> bool {
    let Ok(Value::Object(m)) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    let id_ok = matches!(
        m.get("id"),
        None | Some(Value::String(_) | Value::Number(_))
    );
    let jsonrpc = m.get("jsonrpc").and_then(Value::as_str) == Some("2.0");
    id_ok
        && jsonrpc
        && (m.get("id").is_none() && m.get("method").is_some_and(Value::is_string)
            || m.get("id").is_some()
                && m.get("method").is_none()
                && (m.contains_key("result") || m.contains_key("error")))
}

#[test]
fn twenty_thousand_lines_into_both_servers() {
    let mut rng = Rng::new(0x5eed);
    let (mut rpc_host, _) = host_with_pictures(host());
    let (mut mcp_host, _) = host_with_pictures(host());
    for i in 0..LINES {
        let line = if rng.below(3) == 0 {
            random_line(&mut rng)
        } else {
            mutated_line(&mut rng)
        };
        let blank = line.trim().is_empty();
        let reply = if i % 2 == 0 {
            let reply = rpc::handle_line(&mut rpc_host, &line);
            assert_eq!(reply.is_some(), !blank, "{line:?}");
            reply
        } else {
            let reply = mcp::handle_line(&mut mcp_host, &line);
            if reply.is_none() {
                assert!(blank || may_be_silent(&line), "no reply to {line:?}");
            }
            reply
        };
        if let Some(reply) = reply {
            assert!(!reply.contains('\n'), "{reply}");
            let parsed: Result<Value, _> = serde_json::from_str(&reply);
            assert!(parsed.is_ok(), "{line:?} -> {reply}");
        }
    }
}
