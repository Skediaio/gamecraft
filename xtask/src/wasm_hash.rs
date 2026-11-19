//! `cargo xtask wasm-hash`: the scenarios of `gamecraft-wasmcheck` natively and as WebAssembly
//! in Node, every hash compared (AGENTS.md rule 3).
//!
//! [`compare`] is pure, so its unit tests run on synthetic reports; [`run`] builds and runs.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The checking crate.
const CRATE: &str = "gamecraft-wasmcheck";
/// The wasm-bindgen CLI must be the version of the `wasm-bindgen` crate the workspace pins.
pub const WASM_BINDGEN_VERSION: &str = "0.2.129";
/// Overrides the wasm-bindgen CLI found on `PATH`.
const WASM_BINDGEN_ENV: &str = "GAMECRAFT_WASM_BINDGEN";

/// What the two runs agreed on.
#[derive(Debug, PartialEq, Eq)]
pub struct Agreement {
    /// Scenarios compared.
    pub scenarios: usize,
    /// Hashes compared.
    pub hashes: usize,
}

/// One scenario of a report: its name, its error and its labelled hashes.
struct Scenario<'a> {
    name: &'a str,
    error: Option<&'a str>,
    hashes: Vec<(&'a str, &'a str)>,
}

fn scenarios<'a>(report: &'a Value, side: &str) -> Result<Vec<Scenario<'a>>, String> {
    let malformed = |why: String| format!("the {side} report is malformed: {why}");
    let list = report
        .get("scenarios")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("it has no `scenarios` list".into()))?;
    let mut out = Vec::new();
    for (index, scenario) in list.iter().enumerate() {
        let name = scenario
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed(format!("scenario {index} has no string `name`")))?;
        let error = match scenario.get("error") {
            None | Some(Value::Null) => None,
            Some(Value::String(e)) => Some(e.as_str()),
            Some(_) => {
                return Err(malformed(format!(
                    "`{name}` has an `error` that is neither null nor a string"
                )));
            }
        };
        let hashes = scenario
            .get("hashes")
            .and_then(Value::as_array)
            .ok_or_else(|| malformed(format!("`{name}` has no `hashes` list")))?
            .iter()
            .map(|pair| match pair.as_array().map(Vec::as_slice) {
                Some([Value::String(label), Value::String(hash)]) => {
                    Ok((label.as_str(), hash.as_str()))
                }
                _ => Err(malformed(format!(
                    "`{name}` has a hash that is not [label, hash]: {pair}"
                ))),
            })
            .collect::<Result<Vec<_>, String>>()?;
        out.push(Scenario {
            name,
            error,
            hashes,
        });
    }
    Ok(out)
}

/// Compares the native report with the WebAssembly one. `Err` lists every scenario that
/// failed or differs, each with the first label (a tick, a band of rows, a shot) that differs.
pub fn compare(native: &Value, wasm: &Value) -> Result<Agreement, String> {
    let native = scenarios(native, "native")?;
    let wasm = scenarios(wasm, "wasm")?;
    if native.is_empty() {
        return Err("wasm-hash: the native run reported no scenarios; check `gamecraft_wasmcheck::SCENARIOS`".into());
    }
    let mut problems = Vec::new();
    let mut hashes = 0;
    for n in &native {
        let Some(w) = wasm.iter().find(|w| w.name == n.name) else {
            problems.push(format!(
                "`{}`: missing from the wasm run (it ran natively)",
                n.name
            ));
            continue;
        };
        for (side, scenario) in [("native", n), ("wasm", w)] {
            if let Some(error) = scenario.error {
                problems.push(format!("`{}`: failed in the {side} run: {error}", n.name));
            }
        }
        let first = n
            .hashes
            .iter()
            .zip(&w.hashes)
            .find(|((nl, nh), (wl, wh))| nl != wl || nh != wh);
        match first {
            Some(((nl, nh), (wl, wh))) if nl == wl => problems.push(format!(
                "`{}`: differs first at `{nl}`: native {nh}, wasm {wh}",
                n.name
            )),
            Some(((nl, _), (wl, _))) => problems.push(format!(
                "`{}`: the checkpoints part at `{nl}` (native) and `{wl}` (wasm)",
                n.name
            )),
            None if n.hashes.len() != w.hashes.len() => {
                let (longer, side) = if n.hashes.len() > w.hashes.len() {
                    (&n.hashes, "native")
                } else {
                    (&w.hashes, "wasm")
                };
                let shorter = n.hashes.len().min(w.hashes.len());
                let label = longer.get(shorter).map_or("?", |(l, _)| *l);
                problems.push(format!(
                    "`{}`: only the {side} run reached `{label}` ({} native, {} wasm checkpoints)",
                    n.name,
                    n.hashes.len(),
                    w.hashes.len()
                ));
            }
            None => hashes += n.hashes.len(),
        }
    }
    for w in &wasm {
        if !native.iter().any(|n| n.name == w.name) {
            problems.push(format!("`{}`: ran in wasm only", w.name));
        }
    }
    if problems.is_empty() {
        Ok(Agreement {
            scenarios: native.len(),
            hashes,
        })
    } else {
        Err(format!(
            "wasm-hash: {} problem(s); the simulation is not bit-identical natively and in wasm (AGENTS.md rule 3):\n{}",
            problems.len(),
            problems
                .iter()
                .map(|p| format!("- {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

/// The version in the banner of `wasm-bindgen --version` (`wasm-bindgen 0.2.129`).
pub fn version_of(banner: &str) -> Option<&str> {
    let mut words = banner.split_whitespace();
    match (words.next(), words.next()) {
        (Some("wasm-bindgen"), Some(version)) => Some(version),
        _ => None,
    }
}

/// Runs a command and returns its standard output; its standard error passes through.
fn output(mut command: Command, what: &str) -> Result<String, String> {
    let out = command
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("Could not start {what}: {e}."))?;
    if !out.status.success() {
        return Err(format!("{what} failed ({}).", out.status));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("{what} printed text that is not UTF-8: {e}"))
}

fn parse(text: &str, side: &str) -> Result<Value, String> {
    serde_json::from_str(text.trim())
        .map_err(|e| format!("the {side} run did not print a JSON report: {e}"))
}

/// The wasm-bindgen CLI: `GAMECRAFT_WASM_BINDGEN`, else `wasm-bindgen` on `PATH`, checked to be
/// [`WASM_BINDGEN_VERSION`].
fn wasm_bindgen() -> Result<PathBuf, String> {
    let path = std::env::var_os(WASM_BINDGEN_ENV)
        .map_or_else(|| PathBuf::from("wasm-bindgen"), PathBuf::from);
    let install = format!(
        "Install it with `cargo install wasm-bindgen-cli --version {WASM_BINDGEN_VERSION} --root <dir>` and put <dir>/bin on PATH, or set {WASM_BINDGEN_ENV} to the binary."
    );
    let mut command = Command::new(&path);
    command.arg("--version");
    let banner = output(command, &format!("`{} --version`", path.display())).map_err(|e| {
        format!("{e} The wasm-bindgen CLI {WASM_BINDGEN_VERSION} is needed. {install}")
    })?;
    match version_of(&banner) {
        Some(WASM_BINDGEN_VERSION) => Ok(path),
        other => Err(format!(
            "`{}` is version {}, but the workspace pins wasm-bindgen {WASM_BINDGEN_VERSION}, and the CLI must match. {install}",
            path.display(),
            other.unwrap_or("unknown")
        )),
    }
}

/// Cargo's target directory (`CARGO_TARGET_DIR` and the configuration respected).
fn target_dir(cargo: &str) -> Result<PathBuf, String> {
    let mut command = Command::new(cargo);
    command.args(["metadata", "--format-version", "1", "--no-deps"]);
    let metadata = parse(&output(command, "`cargo metadata`")?, "`cargo metadata`")?;
    metadata
        .get("target_directory")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| "`cargo metadata` gave no `target_directory`".to_owned())
}

/// Loads the wasm-bindgen module for Node and prints its report.
const NODE_SCRIPT: &str =
    "process.stdout.write(require(process.argv[1]).wasm_hash_report() + '\\n')";

/// Builds and runs both sides and compares them.
pub fn run(cargo: &str) -> Result<(), String> {
    let bindgen = wasm_bindgen()?;
    println!("== wasm-hash 1/4: the scenarios natively");
    let mut command = Command::new(cargo);
    command.args(["run", "--quiet", "--release", "-p", CRATE]);
    let native = parse(
        &output(command, &format!("`cargo run --release -p {CRATE}`"))?,
        "native",
    )?;

    println!("== wasm-hash 2/4: the WebAssembly module");
    let status = Command::new(cargo)
        .args([
            "build",
            "--release",
            "--lib",
            "-p",
            CRATE,
            "--target",
            "wasm32-unknown-unknown",
        ])
        .status()
        .map_err(|e| format!("Could not start `cargo build`: {e}."))?;
    if !status.success() {
        return Err(format!(
            "`cargo build --target wasm32-unknown-unknown -p {CRATE}` failed ({status}); is the target installed (`rustup target add wasm32-unknown-unknown`)?"
        ));
    }
    let target = target_dir(cargo)?;
    let module = target
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(format!("{}.wasm", CRATE.replace('-', "_")));
    let out_dir = target.join("wasm-hash");
    let mut command = Command::new(&bindgen);
    command
        .args(["--target", "nodejs", "--out-dir"])
        .arg(&out_dir)
        .arg(&module);
    output(command, "wasm-bindgen")?;

    println!("== wasm-hash 3/4: the scenarios in Node");
    let script = out_dir.join(format!("{}.js", CRATE.replace('-', "_")));
    let mut command = Command::new("node");
    command.args(["-e", NODE_SCRIPT]).arg(absolute(&script));
    let wasm = parse(
        &output(command, "`node` (is Node installed and on PATH?)")?,
        "wasm",
    )?;

    println!("== wasm-hash 4/4: comparing");
    let agreement = compare(&native, &wasm)?;
    println!(
        "wasm-hash: {} scenarios, {} hashes, natively and in wasm bit-identical",
        agreement.scenarios, agreement.hashes
    );
    Ok(())
}

/// `path` made absolute against the current directory, so Node's `require` finds it.
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir().map_or_else(|_| path.to_path_buf(), |dir| dir.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(scenarios: Value) -> Value {
        json!({ "scenarios": scenarios })
    }

    fn play(hashes: &[(&str, &str)]) -> Value {
        let hashes: Vec<Value> = hashes.iter().map(|(l, h)| json!([l, h])).collect();
        json!({"name": "play", "error": null, "hashes": hashes})
    }

    #[test]
    fn equal_reports_agree() {
        let a = report(json!([play(&[("tick 0", "aa"), ("tick 60", "bb")])]));
        assert_eq!(
            compare(&a, &a),
            Ok(Agreement {
                scenarios: 1,
                hashes: 2
            })
        );
    }

    #[test]
    fn a_difference_names_the_scenario_and_the_first_tick() {
        let native = report(json!([play(&[
            ("tick 0", "aa"),
            ("tick 60", "bb"),
            ("tick 120", "cc")
        ])]));
        let wasm = report(json!([play(&[
            ("tick 0", "aa"),
            ("tick 60", "b0"),
            ("tick 120", "c0")
        ])]));
        let e = compare(&native, &wasm).unwrap_err();
        assert!(e.contains("`play`"), "{e}");
        assert!(e.contains("first at `tick 60`"), "{e}");
        assert!(e.contains("native bb") && e.contains("wasm b0"), "{e}");
        assert!(!e.contains("tick 120"), "{e}");
    }

    #[test]
    fn an_error_on_either_side_fails() {
        let ok = report(json!([play(&[("tick 0", "aa")])]));
        let failed = report(
            json!([{"name": "play", "error": "`sim.step` failed", "hashes": [["tick 0", "aa"]]}]),
        );
        let e = compare(&ok, &failed).unwrap_err();
        assert!(
            e.contains("`play`") && e.contains("wasm") && e.contains("sim.step"),
            "{e}"
        );
        let e = compare(&failed, &ok).unwrap_err();
        assert!(e.contains("native"), "{e}");
    }

    #[test]
    fn a_missing_scenario_or_checkpoint_fails() {
        let native = report(json!([play(&[("tick 0", "aa"), ("tick 60", "bb")])]));
        let shorter = report(json!([play(&[("tick 0", "aa")])]));
        let e = compare(&native, &shorter).unwrap_err();
        assert!(e.contains("tick 60"), "{e}");
        let e = compare(&native, &report(json!([]))).unwrap_err();
        assert!(e.contains("`play`") && e.contains("missing"), "{e}");
        let e = compare(&report(json!([])), &report(json!([]))).unwrap_err();
        assert!(e.contains("no scenarios"), "{e}");
    }

    #[test]
    fn malformed_reports_are_errors_not_panics() {
        let good = report(json!([play(&[("tick 0", "aa")])]));
        for bad in [
            json!(null),
            json!({}),
            json!({"scenarios": 3}),
            json!({"scenarios": [{}]}),
            json!({"scenarios": [{"name": "x", "error": null, "hashes": [["a"]]}]}),
            json!({"scenarios": [{"name": "x", "error": 1, "hashes": []}]}),
        ] {
            assert!(compare(&good, &bad).is_err(), "{bad}");
            assert!(compare(&bad, &good).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_wasm_bindgen_version_is_read_from_its_banner() {
        assert_eq!(version_of("wasm-bindgen 0.2.129\n"), Some("0.2.129"));
        assert_eq!(version_of("nonsense"), None);
    }
}
