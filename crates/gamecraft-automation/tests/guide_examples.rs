//! The worked example of `docs/agent.md` runs: every line of its `jsonl gamecraft-example`
//! block goes through the JSON-lines server on a fresh host and must succeed.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use gamecraft_automation::{Access, Host, rpc};
use gamecraft_world::World;
use serde_json::Value;

const GUIDE: &str = include_str!("../../../docs/agent.md");

/// The lines of the fenced blocks tagged `jsonl gamecraft-example`.
fn example_blocks() -> Vec<Vec<&'static str>> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in GUIDE.lines() {
        let trimmed = line.trim();
        match &mut current {
            None if trimmed == "```jsonl gamecraft-example" => current = Some(Vec::new()),
            None => {}
            Some(_) if trimmed == "```" => blocks.extend(current.take()),
            Some(_) if trimmed.is_empty() => {}
            Some(lines) => lines.push(line),
        }
    }
    assert!(current.is_none(), "an example block is not closed");
    blocks
}

#[test]
fn the_worked_example_runs() {
    let blocks = example_blocks();
    assert_eq!(blocks.len(), 1, "docs/agent.md has one worked example");
    let lines = &blocks[0];
    assert!(lines.len() >= 12, "{} lines", lines.len());
    let mut host = Host::new(World::new("Untitled", 0), Access::default());
    let mut replies = Vec::new();
    for line in lines {
        assert!(
            !line.contains("view.render"),
            "no pictures in the example: {line}"
        );
        assert!(
            !line.contains("\"path\""),
            "no file paths in the example: {line}"
        );
        let reply = rpc::handle_line(&mut host, line).expect("a reply");
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["ok"], true, "{line}\n-> {reply}");
        replies.push(reply);
    }
    let commands: Vec<String> = lines
        .iter()
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            v["params"]["command"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    for wanted in [
        "file.new",
        "terrain.set",
        "entity.add",
        "sim.start",
        "sim.input",
        "sim.step",
        "sim.state",
        "sim.stop",
    ] {
        assert!(
            commands.iter().any(|c| c == wanted),
            "the example lacks {wanted}"
        );
    }
    assert!(!host.session().is_playing());

    // The character moved north, and the wall dropped in its way stopped it.
    let steps: Vec<&Value> = replies
        .iter()
        .zip(&commands)
        .filter(|(_, c)| *c == "sim.step")
        .map(|(r, _)| r)
        .collect();
    let scout_z = |r: &Value| {
        r["result"]["characters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "Scout")
            .unwrap()["position"][2]
            .as_f64()
            .unwrap()
    };
    let first = scout_z(steps[0]);
    let last = scout_z(steps.last().unwrap());
    assert!(first < -2.0, "the scout walked north: z {first}");
    assert!(last < first, "the scout kept walking: z {first} -> {last}");
    assert!(last > -7.8, "the wall at z -8 stopped the scout: z {last}");
}
