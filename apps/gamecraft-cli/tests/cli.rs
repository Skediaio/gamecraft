//! The command line as an agent meets it: arguments in, JSON lines out, exit codes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
use common::*;
use serde_json::{Value, json};

const SUBCOMMANDS: [&str; 9] = [
    "commands", "run", "info", "render", "play", "new", "serve", "mcp", "help",
];

#[test]
fn version_and_help() {
    let v = run(&["--version"]);
    assert_eq!(v.code, 0);
    assert!(v.out.starts_with("gamecraft-cli "), "{}", v.out);
    for flag in ["help", "--help", "-h"] {
        let h = run(&[flag]);
        assert_eq!(h.code, 0, "{flag}");
        for sub in SUBCOMMANDS {
            assert!(
                h.out.lines().any(|l| l.trim_start().starts_with(sub)),
                "`{sub}` is not in the help:\n{}",
                h.out
            );
        }
    }
    assert_eq!(run(&[]).code, 0);
}

#[test]
fn every_subcommand_explains_itself() {
    for sub in SUBCOMMANDS.iter().filter(|s| **s != "help") {
        let h = run(&[sub, "--help"]);
        assert_eq!(h.code, 0, "{sub}");
        assert!(
            h.out.contains(&format!("gamecraft-cli {sub}")),
            "{sub}: {}",
            h.out
        );
        let via_help = run(&["help", sub]);
        assert_eq!(via_help.out, h.out);
    }
}

#[test]
fn usage_mistakes_exit_2() {
    for args in [
        &["frobnicate"][..],
        &["--frobnicate"],
        &["info"],
        &["render", "x.gamecraft"],
        &["play"],
        &["new"],
        &["run", "--bogus"],
        &["run", "--params", "{}"],
        &["run", "--new", "{}", "x.gamecraft"],
        &["commands", "--filter"],
    ] {
        let r = run(args);
        assert_eq!(r.code, 2, "{args:?}: {}", r.err);
        assert!(r.err.contains("gamecraft-cli"), "{args:?}: {}", r.err);
    }
    let r = run(&["run", "--cmd", "world.inspect", "--params", "{not json"]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("--params"), "{}", r.err);
}

#[test]
fn commands_lists_filters_and_gives_json() {
    let all = run(&["commands"]);
    assert_eq!(all.code, 0);
    assert!(all.out.contains("entity.add"));
    assert!(all.out.contains("view.render"));
    let sim = run(&["commands", "--filter", "sim"]);
    assert!(sim.out.contains("sim.step"));
    assert!(!sim.out.contains("entity.add"));
    let none = run(&["commands", "--filter", "zzzz-nothing"]);
    assert_eq!(none.code, 0);
    assert!(none.out.contains("No command matches"));
    let json = run(&["commands", "--json"]);
    let v: Value = serde_json::from_str(json.out.trim()).unwrap();
    let ids: Vec<&str> = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"entity.add") && ids.contains(&"view.render"));
    let filtered: Value =
        serde_json::from_str(run(&["commands", "--json", "--filter", "sim"]).out.trim()).unwrap();
    assert!(filtered["commands"].as_array().unwrap().len() < ids.len());
}

const TERRAIN: &str =
    r#"{"kind":"noise","size_m":64,"cell_m":2,"relief_m":3,"feature_m":40,"seed":1}"#;

#[test]
fn new_run_and_info_round_trip() {
    let dir = Temp::new("roundtrip");
    let new = run_in(
        &dir.0,
        &["new", "world.gamecraft", "--name", "Test", "--seed", "9"],
    );
    assert_eq!(new.code, 0, "{}", new.err);
    assert!(dir.path("world.gamecraft").exists());
    // Never over an existing file.
    assert_eq!(run_in(&dir.0, &["new", "world.gamecraft"]).code, 1);
    let r = run_in(
        &dir.0,
        &[
            "run",
            "world.gamecraft",
            "--cmd",
            "terrain.set",
            "--params",
            TERRAIN,
            "--out",
            "world.gamecraft",
        ],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let info = run_in(&dir.0, &["info", "world.gamecraft"]);
    assert_eq!(info.code, 0, "{}", info.err);
    let v: Value = serde_json::from_str(info.out.trim()).unwrap();
    assert_eq!(v["name"], "Test");
    assert_eq!(v["seed"], 9);
    assert_eq!(v["terrain"]["size_m"], 64.0);
}

#[test]
fn run_prints_one_line_per_step_and_stops_at_a_failure() {
    let dir = Temp::new("run");
    let ok = run_in(
        &dir.0,
        &[
            "run",
            "--new",
            r#"{"name":"A","seed":3}"#,
            "--cmd",
            "world.inspect",
            "--cmd",
            "terrain.set",
            "--params",
            TERRAIN,
            "--out",
            "a.gamecraft",
        ],
    );
    assert_eq!(ok.code, 0, "{}", ok.err);
    let lines = ok.json_lines();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["name"], "A");
    assert!(dir.path("a.gamecraft").exists());

    let bad = run_in(
        &dir.0,
        &[
            "run",
            "--cmd",
            "terrain.set",
            "--params",
            TERRAIN,
            "--cmd",
            "no.such",
            "--cmd",
            "world.inspect",
            "--out",
            "b.gamecraft",
        ],
    );
    assert_eq!(bad.code, 1);
    assert_eq!(bad.json_lines().len(), 1, "stops at the failing step");
    assert!(bad.err.contains("unknown command `no.such`"), "{}", bad.err);
    assert!(!dir.path("b.gamecraft").exists(), "nothing is saved");

    let keep = run_in(
        &dir.0,
        &[
            "run",
            "--cmd",
            "terrain.set",
            "--params",
            TERRAIN,
            "--cmd",
            "no.such",
            "--cmd",
            "world.inspect",
            "--keep-going",
            "--out",
            "c.gamecraft",
        ],
    );
    assert_eq!(keep.code, 1);
    let lines = keep.json_lines();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1]["failed"], "no.such");
    assert_eq!(lines[1]["step"], 1);
    assert!(lines[1]["error"].as_str().unwrap().contains("no.such"));
    assert!(dir.path("c.gamecraft").exists(), "keep-going saves");
}

#[test]
fn a_missing_or_bad_file_is_an_error_not_a_panic() {
    let dir = Temp::new("badfile");
    dir.write("junk.gamecraft", "not a world");
    for args in [
        &["info", "missing.gamecraft"][..],
        &["info", "junk.gamecraft"],
        &["run", "junk.gamecraft"],
        &["run", "--script", "missing.json"],
        &["play", "missing.gamecraft"],
    ] {
        let r = run_in(&dir.0, args);
        assert_eq!(r.code, 1, "{args:?}: {}", r.err);
        assert!(!r.err.contains("panicked"), "{}", r.err);
        assert!(!r.err.trim().is_empty());
    }
    dir.write("bad.json", "{not json");
    assert_eq!(run_in(&dir.0, &["run", "--script", "bad.json"]).code, 1);
}

#[test]
fn scripts_in_both_formats_and_mixed_with_cmd_keep_their_order() {
    let dir = Temp::new("script");
    dir.write(
        "pairs.json",
        r#"[["world.set", {"name": "One"}], ["world.inspect", {"entities": false}]]"#,
    );
    dir.write(
        "objects.json",
        r#"[{"command": "world.set", "params": {"name": "Three"}}, {"command": "world.inspect", "params": {"entities": false}}]"#,
    );
    let pairs = run_in(&dir.0, &["run", "--script", "pairs.json"]);
    assert_eq!(pairs.code, 0, "{}", pairs.err);
    assert_eq!(pairs.json_lines()[1]["name"], "One");
    let objects = run_in(&dir.0, &["run", "--script", "objects.json"]);
    assert_eq!(objects.json_lines()[1]["name"], "Three");

    let mixed = run_in(
        &dir.0,
        &[
            "run",
            "--cmd",
            "world.set",
            "--params",
            r#"{"name":"Zero"}"#,
            "--script",
            "pairs.json",
            "--cmd",
            "world.set",
            "--params",
            r#"{"name":"Two"}"#,
            "--cmd",
            "world.inspect",
            "--params",
            r#"{"entities":false}"#,
            "--script",
            "objects.json",
            "--cmd",
            "world.inspect",
            "--params",
            r#"{"entities":false}"#,
        ],
    );
    assert_eq!(mixed.code, 0, "{}", mixed.err);
    let names: Vec<String> = mixed
        .json_lines()
        .iter()
        .filter_map(|v| v["name"].as_str().map(String::from))
        .collect();
    // world.set answers null; world.inspect answers the name current at that step.
    assert_eq!(names, ["One", "Two", "Three", "Three"]);

    dir.write("odd.json", r#"[{"command": "world.inspect", "extra": 1}]"#);
    assert_eq!(run_in(&dir.0, &["run", "--script", "odd.json"]).code, 1);
}

#[test]
fn play_is_deterministic_and_leaves_the_file_alone() {
    let dir = Temp::new("play");
    let made = run_in(
        &dir.0,
        &[
            "run",
            "--new",
            r#"{"name":"P","seed":4}"#,
            "--cmd",
            "terrain.set",
            "--params",
            TERRAIN,
            "--cmd",
            "entity.add",
            "--params",
            r#"{"name":"Scout","at":[0,0],"components":[{"type":"character"}]}"#,
            "--out",
            "p.gamecraft",
        ],
    );
    assert_eq!(made.code, 0, "{}", made.err);
    dir.write(
        "inputs.json",
        r#"[["sim.input", {"entity": "Scout", "forward": 1, "yaw_deg": 0}], ["sim.step", {"ticks": 30}]]"#,
    );
    let before = std::fs::read(dir.path("p.gamecraft")).unwrap();
    let args = [
        "play",
        "p.gamecraft",
        "--script",
        "inputs.json",
        "--seconds",
        "1",
        "--out-state",
        "state.json",
    ];
    let a = run_in(&dir.0, &args);
    assert_eq!(a.code, 0, "{}", a.err);
    let b = run_in(&dir.0, &args);
    assert_eq!(a.out, b.out);
    let state = a.json_lines().remove(0);
    assert_eq!(
        state["tick"], 60,
        "the script's 30 ticks count towards the second"
    );
    assert_eq!(state["hash"].as_str().unwrap().len(), 16);
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path("state.json")).unwrap()).unwrap();
    assert_eq!(saved["hash"], state["hash"]);
    assert_eq!(std::fs::read(dir.path("p.gamecraft")).unwrap(), before);

    let ticks = run_in(&dir.0, &["play", "p.gamecraft", "--ticks", "10"]);
    assert_eq!(ticks.json_lines()[0]["tick"], 10);
    let both = run_in(
        &dir.0,
        &["play", "p.gamecraft", "--ticks", "10", "--seconds", "1"],
    );
    assert_eq!(both.code, 2);
}

#[test]
fn serve_answers_every_line_and_guards_files() {
    let dir = Temp::new("serve");
    let mut s = Server::start("serve", &[], &dir.0);
    let bad = s.raw("this is not json");
    assert_eq!(bad["ok"], false);
    assert_eq!(bad["id"], Value::Null);
    let methods = s.raw(r#"{"id": 1, "method": "methods"}"#);
    assert_eq!(methods["ok"], true);
    s.ok("file.new", json!({"name": "Served"}));
    let refused = s.reply("file.save", json!({"path": "x.gamecraft"}));
    assert_eq!(refused["ok"], false);
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("--automation-write-root"),
        "{refused}"
    );
    let refused = s.reply("file.open", json!({"path": "x.gamecraft"}));
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("--automation-read-root")
    );
    assert!(s.finish());

    let root = dir.path("root");
    std::fs::create_dir(&root).unwrap();
    let root_arg = root.to_str().unwrap();
    let mut s = Server::start("serve", &["--automation-write-root", root_arg], &dir.0);
    s.ok("file.new", json!({"name": "Served"}));
    s.ok("file.save", json!({"path": "x.gamecraft"}));
    assert!(root.join("x.gamecraft").exists());
    let outside = s.reply("file.save", json!({"path": "../y.gamecraft"}));
    assert_eq!(outside["ok"], false);
    let outside = s.reply(
        "file.save",
        json!({"path": dir.path("z.gamecraft").to_str().unwrap()}),
    );
    assert_eq!(outside["ok"], false);
    assert!(outside["error"].as_str().unwrap().contains("outside"));
    assert!(!dir.path("z.gamecraft").exists());
    assert!(s.finish());

    let mut s = Server::start(
        "serve",
        &[
            "--automation-read-root",
            root_arg,
            "--file",
            root.join("x.gamecraft").to_str().unwrap(),
        ],
        &dir.0,
    );
    let inspected = s.ok("world.inspect", json!({"entities": false}));
    assert_eq!(inspected["name"], "Served");
    assert!(s.finish());

    let missing = run(&["serve", "--automation-read-root", "/definitely/not/a/dir"]);
    assert_eq!(missing.code, 2);
}

#[test]
fn mcp_initializes_and_lists_tools() {
    let dir = Temp::new("mcp");
    let mut s = Server::start("mcp", &[], &dir.0);
    let init = s.raw(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
    );
    assert!(init["result"]["serverInfo"].is_object(), "{init}");
    let tools = s.raw(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"command_list") && names.contains(&"doc_render_preview"),
        "{names:?}"
    );
    assert!(s.finish());
}

#[test]
fn the_world_file_cap_is_a_safety_net_the_flag_raises() {
    let temp = Temp::new("world-cap");
    // A valid world padded to a little over 1 MiB.
    let text = format!(
        "{{\"gamecraft\": 1, \"name\": \"big\"{}}}",
        " ".repeat(1024 * 1024 + 10)
    );
    let file = temp.write("big.gamecraft", &text);
    let file = file.to_str().unwrap();

    let refused = run(&["info", file, "--max-world-mb", "1"]);
    assert_eq!(refused.code, 1, "{}", refused.err);
    assert!(refused.err.contains("at most 1 MB"), "{}", refused.err);
    assert!(refused.err.contains("--max-world-mb"), "{}", refused.err);

    // The flag may stand anywhere, also before the subcommand.
    let read = run(&["--max-world-mb", "2", "info", file]);
    assert_eq!(read.code, 0, "{}", read.err);
    // The default (512 MiB) reads it as well.
    assert_eq!(run(&["info", file]).code, 0);

    for bad in ["0", "-3", "lots", "2000000"] {
        let r = run(&["info", file, "--max-world-mb", bad]);
        assert_eq!(r.code, 2, "{bad}: {}", r.err);
        assert!(r.err.contains("MiB"), "{}", r.err);
    }
    assert_eq!(run(&["info", file, "--max-world-mb"]).code, 2);
}
