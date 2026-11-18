//! `cargo xtask layers | wasm | ci`: the workspace's own checks.

mod layers;

use std::collections::BTreeMap;
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: cargo xtask <layers|wasm|ci>
  layers  check the layer table, forbidden dependencies and workspace lints
  wasm    cargo check --target wasm32-unknown-unknown for layers L0 to L4
  ci      fmt, clippy, tests, layers and wasm, stopping at the first failure";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if args.next().is_some() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let result = match command.as_deref() {
        Some("layers") => run_layers(),
        Some("wasm") => run_wasm(),
        Some("ci") => run_ci(),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

/// Runs cargo with `args`, inheriting the output; `Err` when it fails or cannot start.
fn run_cargo(args: &[&str]) -> Result<(), String> {
    let status = Command::new(cargo()).args(args).status().map_err(|e| {
        format!(
            "Could not start `{} {}`: {e}. Check that cargo is installed and on PATH.",
            cargo(),
            args.join(" ")
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed ({status}).", args.join(" ")))
    }
}

fn run_layers() -> Result<(), String> {
    let output = Command::new(cargo())
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| {
            format!(
                "Could not start `cargo metadata`: {e}. Check that cargo is installed and on PATH."
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "`cargo metadata` failed ({}): {}\nFix the workspace manifests so that cargo can read them.",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("`cargo metadata` did not print valid JSON: {e}."))?;
    let mut manifests = BTreeMap::new();
    for path in layers::manifest_paths(&metadata)? {
        if let Ok(text) = std::fs::read_to_string(&path) {
            manifests.insert(path, text);
        }
    }
    let violations = layers::check(&metadata, &manifests)?;
    if violations.is_empty() {
        let count = metadata
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        println!("layers: {count} crates, ok");
        Ok(())
    } else {
        let list = violations
            .iter()
            .map(|v| format!("- {v}"))
            .collect::<Vec<_>>()
            .join("\n");
        Err(format!("layers: {} violation(s)\n{list}", violations.len()))
    }
}

fn run_wasm() -> Result<(), String> {
    let mut args = vec!["check", "--target", "wasm32-unknown-unknown"];
    for name in layers::crates_up_to(layers::WASM_MAX_LAYER) {
        args.push("-p");
        args.push(name);
    }
    run_cargo(&args).map_err(|e| {
        format!("{e} Layers L0 to L4 must build for wasm32-unknown-unknown (AGENTS.md, rule 10); is the target installed (`rustup target add wasm32-unknown-unknown`)?")
    })
}

fn run_ci() -> Result<(), String> {
    type Step = (&'static str, fn() -> Result<(), String>);
    let steps: [Step; 5] = [
        ("cargo fmt --all --check", || {
            run_cargo(&["fmt", "--all", "--check"])
        }),
        (
            "cargo clippy --workspace --all-targets -- -D warnings",
            || {
                run_cargo(&[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ])
            },
        ),
        ("cargo test --workspace", || {
            run_cargo(&["test", "--workspace"])
        }),
        ("cargo xtask layers", run_layers),
        ("cargo xtask wasm", run_wasm),
    ];
    for (index, (name, step)) in steps.iter().enumerate() {
        println!("== ci step {}/{}: {name}", index + 1, steps.len());
        step().map_err(|e| {
            format!(
                "{e}\nci: step {} ({name}) failed; fix it and run `cargo xtask ci` again.",
                index + 1
            )
        })?;
    }
    println!("ci: all steps passed");
    Ok(())
}
