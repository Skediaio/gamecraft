//! The worked example of `docs/guide/command-line.md` runs: every fenced `sh gamecraft-example`
//! block is executed in a temporary directory with the binary in place of `gamecraft-cli`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
use common::*;
use std::process::Command;

const GUIDE: &str = include_str!("../../../docs/guide/command-line.md");

/// The fenced blocks tagged `sh gamecraft-example`, each as its text.
fn blocks() -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in GUIDE.lines() {
        match &mut current {
            None if line.trim() == "```sh gamecraft-example" => current = Some(Vec::new()),
            None => {}
            Some(_) if line.trim() == "```" => {
                blocks.extend(current.take().map(|l| l.join("\n")));
            }
            Some(lines) => lines.push(line),
        }
    }
    assert!(current.is_none(), "an example block is not closed");
    blocks
}

fn gpu_available(dir: &Temp) -> bool {
    let made = run_in(&dir.0, &["new", "probe-world.gamecraft"]);
    assert_eq!(made.code, 0, "{}", made.err);
    let r = run_in(
        &dir.0,
        &[
            "render",
            "probe-world.gamecraft",
            "--out",
            "probe.png",
            "--size",
            "32x32",
        ],
    );
    r.code == 0
}

#[test]
fn the_valley_example_runs() {
    let blocks = blocks();
    assert!(
        blocks.len() >= 3,
        "the guide has the valley example in blocks"
    );
    let dir = Temp::new("guide");
    let bin_dir = Temp::new("guide-bin");
    std::os::unix::fs::symlink(BIN, bin_dir.path("gamecraft-cli")).unwrap();
    let gpu = gpu_available(&dir);
    if !gpu {
        assert!(
            std::env::var("GAMECRAFT_REQUIRE_GPU").as_deref() != Ok("1"),
            "GAMECRAFT_REQUIRE_GPU=1 but there is no GPU"
        );
    }
    let path = format!(
        "{}:{}",
        bin_dir.0.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    for block in blocks {
        for command in block.split("\n\n") {
            if (command.contains("gamecraft-cli render") || command.contains("--picture")) && !gpu {
                println!("skipped (no GPU): {command}");
                continue;
            }
            let output = Command::new("sh")
                .arg("-c")
                .arg(command)
                .env("PATH", &path)
                .current_dir(&dir.0)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "`{command}` failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    assert!(dir.path("valley.gamecraft").exists());
    if gpu {
        assert!(dir.path("valley.png").exists());
    }
}
