//! An MCP conversation over an in-memory stdio pair: initialize, tools/list, every tool,
//! resources, the prompt and the errors.
//!
//! Adapted from VisionCraft's `crates/visioncraft-automation/tests/mcp.rs` (MIT).

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
use gamecraft_automation::{base64, mcp};
use serde_json::{Value, json};

fn call(id: u64, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
}

fn text(reply: &Value) -> Value {
    let content = reply["result"]["content"].as_array().unwrap();
    let block = content.iter().find(|b| b["type"] == "text").unwrap();
    serde_json::from_str(block["text"].as_str().unwrap()).unwrap()
}

fn one(h: &mut gamecraft_automation::Host, message: Value) -> Value {
    serde_json::from_str(&mcp::handle_line(h, &message.to_string()).expect("a reply")).unwrap()
}

#[test]
fn a_whole_conversation() {
    let mut h = empty_host();
    let replies = converse(
        &mut h,
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            call(3, "command_list", json!({"brief": true, "filter": "sim."})),
            call(
                4,
                "command_batch",
                json!({"atomic": true, "intent": "a valley", "steps": [
                    {"id": "terrain.set", "params": {"kind": "flat", "size_m": 64}},
                    {"id": "entity.add", "params": {"name": "Scout", "at": [0, 0], "components": [{"type": "character"}]}}
                ]}),
            ),
            call(5, "doc_inspect", json!({"limit": 1})),
            call(
                6,
                "command_run",
                json!({"id": "entity.remove", "params": {"id": 99}}),
            ),
            call(7, "command_run", json!({"id": "edit.undo"})),
            call(8, "command_bach", json!({})),
            call(9, "command_run", json!({"id": "world.set", "parms": {}})),
            call(10, "doc_render_preview", json!({})),
            json!({"jsonrpc": "2.0", "id": 11, "method": "resources/read", "params": {"uri": "gamecraft://guide"}}),
            json!({"jsonrpc": "2.0", "id": 12, "method": "prompts/get", "params": {"name": "build_world", "arguments": {"description": "a hill with a pine"}}}),
            json!({"jsonrpc": "2.0", "id": 13, "method": "nope/nothing"}),
            json!({"jsonrpc": "2.0", "id": 14, "method": "ping"}),
        ],
    );
    assert_eq!(replies.len(), 14, "{replies:#?}");
    let by_id = |id: u64| replies.iter().find(|r| r["id"] == id).unwrap().clone();

    let init = by_id(1);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "gamecraft");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("command_list")
    );

    let tools: Vec<String> = by_id(2)["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        tools,
        [
            "command_list",
            "command_run",
            "command_batch",
            "doc_inspect",
            "doc_render_preview"
        ]
    );
    for t in by_id(2)["result"]["tools"].as_array().unwrap() {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
    }

    let list = text(&by_id(3));
    let ids: Vec<&str> = list["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&"sim.step") && !ids.contains(&"entity.add"),
        "{ids:?}"
    );

    let batch = by_id(4);
    assert_eq!(batch["result"]["isError"], false, "{batch}");
    assert_eq!(text(&batch)["completed"], 2);

    let inspect = text(&by_id(5));
    assert_eq!(inspect["entity_count"], 1);
    assert_eq!(inspect["entities"][0]["name"], "Scout");

    let failed = by_id(6);
    assert_eq!(failed["result"]["isError"], true);
    let message = failed["result"]["content"][0]["text"].as_str().unwrap();
    assert!(message.starts_with("entity.remove"), "{message}");

    let undo = by_id(7);
    assert_eq!(undo["result"]["isError"], false);
    assert_eq!(text(&undo)["can_redo"], true);
    assert!(undo["result"]["_meta"]["ms"].is_number());
    // The atomic batch was one undo step.
    assert_eq!(h.session().document().entity_count(), 0);
    assert!(h.session().document().terrain.is_none());

    assert_eq!(by_id(8)["error"]["code"], -32602);
    assert!(
        by_id(8)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown tool")
    );
    let wrong = by_id(9);
    assert_eq!(wrong["error"]["code"], -32602);
    assert!(
        wrong["error"]["message"]
            .as_str()
            .unwrap()
            .contains("parms")
    );

    let preview = by_id(10);
    assert_eq!(preview["result"]["isError"], true);
    assert!(
        preview["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("renderer is needed")
    );

    let guide = by_id(11);
    assert_eq!(guide["result"]["contents"][0]["mimeType"], "text/markdown");
    assert!(
        guide["result"]["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("gamecraft-example")
    );

    let prompt = by_id(12)["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(prompt.contains("a hill with a pine"), "{prompt}");
    assert!(prompt.contains("doc_inspect") && prompt.contains("doc_render_preview"));

    assert_eq!(by_id(13)["error"]["code"], -32601);
    assert_eq!(by_id(14)["result"], json!({}));
}

#[test]
fn command_run_takes_command_as_well_as_id() {
    let mut h = empty_host();
    let r = one(
        &mut h,
        call(
            1,
            "command_run",
            json!({"command": "entity.add", "params": {"position": [0, 0, 0]}, "intent": "a marker"}),
        ),
    );
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(text(&r)["id"], 1);
    let r = one(
        &mut h,
        call(2, "command_run", json!({"id": "a", "command": "b"})),
    );
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = one(&mut h, call(3, "command_run", json!({})));
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = one(
        &mut h,
        call(4, "command_run", json!({"id": "world.set", "intent": 4})),
    );
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = one(
        &mut h,
        call(
            5,
            "command_batch",
            json!({"steps": [{"command": "world.set", "params": {"seed": 2}}]}),
        ),
    );
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(h.session().document().seed, 2);
}

#[test]
fn a_failing_batch_is_an_error_with_its_report() {
    let mut h = empty_host();
    let r = one(
        &mut h,
        call(
            1,
            "command_batch",
            json!({"steps": [
            {"id": "entity.add", "params": {"position": [0, 0, 0]}},
            {"id": "entity.remove", "params": {"id": 42}}
        ], "terse": true}),
        ),
    );
    assert_eq!(r["result"]["isError"], true);
    let report = text(&r);
    assert_eq!(
        (report["completed"].clone(), report["failed"].clone()),
        (json!(1), json!(1))
    );
}

#[test]
fn the_preview_is_an_image_block_with_the_info() {
    let (mut h, seen) = host_with_pictures(host());
    let r = one(
        &mut h,
        call(
            1,
            "doc_render_preview",
            json!({"follow": {"entity": "Scout"}}),
        ),
    );
    assert_eq!(r["result"]["isError"], false, "{r}");
    let content = r["result"]["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["mimeType"], "image/png");
    assert_eq!(content[0]["data"], base64(PNG_1X1));
    let info = text(&r);
    assert_eq!(
        (info["width"].clone(), info["height"].clone()),
        (json!(640), json!(360))
    );
    assert_eq!(info["renderer"], "fake");
    assert!(info.get("base64").is_none());
    assert_eq!(seen.borrow().len(), 1);
    let r = one(&mut h, call(2, "doc_render_preview", json!({"width": 10})));
    assert_eq!(r["result"]["isError"], true, "{r}");
    let r = one(
        &mut h,
        call(3, "doc_render_preview", json!({"path": "x.png"})),
    );
    assert_eq!(r["error"]["code"], -32602, "{r}");
    // command_run of view.render gives an image block too.
    let r = one(
        &mut h,
        call(
            4,
            "command_run",
            json!({"id": "view.render", "params": {"width": 20, "height": 20}}),
        ),
    );
    assert_eq!(r["result"]["content"][0]["type"], "image", "{r}");
}

#[test]
fn resources_and_prompts_are_listed_and_read() {
    let mut h = host();
    let list = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"}),
    );
    let uris: Vec<&str> = list["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert_eq!(
        uris,
        [
            "gamecraft://world",
            "gamecraft://guide",
            "gamecraft://guide/game-master"
        ]
    );
    let world = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 2, "method": "resources/read", "params": {"uri": "gamecraft://world"}}),
    );
    let content = &world["result"]["contents"][0];
    assert_eq!(content["mimeType"], "application/json");
    let inspect: Value = serde_json::from_str(content["text"].as_str().unwrap()).unwrap();
    assert_eq!(inspect["name"], "Example valley");
    let bad = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": "gamecraft://nothing"}}),
    );
    assert_eq!(bad["error"]["code"], -32602);
    let prompts = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 4, "method": "prompts/list"}),
    );
    assert_eq!(prompts["result"]["prompts"][0]["name"], "build_world");
    let missing = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 5, "method": "prompts/get", "params": {"name": "build_world"}}),
    );
    assert_eq!(missing["error"]["code"], -32602);
    let unknown = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 6, "method": "prompts/get", "params": {"name": "build_garden", "arguments": {"description": "x"}}}),
    );
    assert_eq!(unknown["error"]["code"], -32602);
    let templates = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 7, "method": "resources/templates/list"}),
    );
    assert_eq!(templates["result"]["resourceTemplates"], json!([]));
}

#[test]
fn malformed_messages_get_errors_and_the_server_keeps_serving() {
    let mut h = host();
    let cases = [
        ("not json", -32700),
        ("[1, 2]", -32600),
        ("5", -32600),
        (r#"{"jsonrpc": "1.0", "id": 1, "method": "ping"}"#, -32600),
        (r#"{"jsonrpc": "2.0", "id": [1], "method": "ping"}"#, -32600),
        (r#"{"jsonrpc": "2.0", "id": 1}"#, -32600),
        (r#"{"jsonrpc": "2.0", "id": 1, "method": 3}"#, -32600),
        (
            r#"{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": 3}"#,
            -32602,
        ),
        (
            r#"{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {}}"#,
            -32602,
        ),
        (
            r#"{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 3}}"#,
            -32602,
        ),
    ];
    for (line, code) in cases {
        let reply: Value = serde_json::from_str(&mcp::handle_line(&mut h, line).unwrap()).unwrap();
        assert_eq!(reply["error"]["code"], code, "{line}: {reply}");
        assert_eq!(reply["jsonrpc"], "2.0");
    }
    // Notifications and responses get no reply; neither does a blank line.
    for line in [
        r#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#,
        r#"{"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "command_run", "arguments": {"id": "world.set", "params": {"name": "Silent"}}}}"#,
        r#"{"jsonrpc": "2.0", "id": 9, "result": {}}"#,
        "  ",
    ] {
        assert_eq!(mcp::handle_line(&mut h, line), None, "{line}");
    }
    // An unknown revision is answered with the latest.
    let init = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}),
    );
    assert_eq!(init["result"]["protocolVersion"], mcp::PROTOCOL_VERSIONS[0]);
    let replies = converse(
        &mut h,
        &[
            json!("text"),
            json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}),
        ],
    );
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[1]["result"], json!({}));
}

#[test]
fn the_game_master_has_a_prompt_and_a_guide() {
    let mut h = host();
    let prompts = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 1, "method": "prompts/list"}),
    );
    let names: Vec<&str> = prompts["result"]["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["build_world", "direct_scenario"]);
    let got = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 2, "method": "prompts/get", "params": {"name": "direct_scenario", "arguments": {"goal": "a hard but fair fight"}}}),
    );
    let text = got["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("a hard but fair fight"), "{text}");
    for word in [
        "gm.overview",
        "gm.order",
        "gm.sight",
        "prefab.spawn",
        "environment.set",
        "message.show",
        "sim.step",
        "intent",
        "fair",
        "gamecraft://guide/game-master",
    ] {
        assert!(text.contains(word), "the prompt lacks {word}: {text}");
    }
    let missing = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 3, "method": "prompts/get", "params": {"name": "direct_scenario"}}),
    );
    assert_eq!(missing["error"]["code"], -32602);
    assert!(
        missing["error"]["message"]
            .as_str()
            .unwrap()
            .contains("goal"),
        "{missing}"
    );
    let guide = one(
        &mut h,
        json!({"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": "gamecraft://guide/game-master"}}),
    );
    let content = &guide["result"]["contents"][0];
    assert_eq!(content["mimeType"], "text/markdown");
    let text = content["text"].as_str().unwrap();
    assert!(text.starts_with("# Directing a game"), "{text}");
    assert!(text.contains("gm.overview"));
}
