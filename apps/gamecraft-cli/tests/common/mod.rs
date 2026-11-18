//! Helpers shared by the integration tests: the binary, a temporary directory, a `serve`
//! session and the M0 scenario's requests.
// Test helpers: clippy.toml allows these inside #[test] functions only.
#![allow(
    dead_code,
    clippy::disallowed_methods,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// The built binary.
pub const BIN: &str = env!("CARGO_BIN_EXE_gamecraft-cli");

/// A temporary directory that is removed when it goes out of scope.
pub struct Temp(pub PathBuf);

impl Temp {
    pub fn new(tag: &str) -> Temp {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "gamecraft-cli-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    pub fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path(name);
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a run of the binary left.
pub struct Ran {
    pub code: i32,
    pub out: String,
    pub err: String,
}

impl Ran {
    /// The stdout lines parsed as JSON.
    pub fn json_lines(&self) -> Vec<Value> {
        self.out
            .lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("`{l}` is not JSON: {e}")))
            .collect()
    }
}

pub fn run_in(dir: &Path, args: &[&str]) -> Ran {
    let output: Output = Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    Ran {
        code: output.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&output.stdout).into_owned(),
        err: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub fn run(args: &[&str]) -> Ran {
    run_in(&std::env::temp_dir(), args)
}

/// A `serve` or `mcp` process fed line by line.
pub struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Server {
    pub fn start(subcommand: &str, args: &[&str], dir: &Path) -> Server {
        let mut child = Command::new(BIN)
            .arg(subcommand)
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Server {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    /// Sends one raw line and reads one reply line.
    pub fn raw(&mut self, line: &str) -> Value {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
        let mut reply = String::new();
        self.stdout.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("`{reply}` is not JSON: {e}"))
    }

    /// `engine.execute`: the whole reply.
    pub fn reply(&mut self, command: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"id": id, "method": "engine.execute", "params": {"command": command, "params": params}});
        let reply = self.raw(&line.to_string());
        assert_eq!(reply["id"], id, "{reply}");
        reply
    }

    /// `engine.execute` that must succeed: the result.
    pub fn ok(&mut self, command: &str, params: Value) -> Value {
        let reply = self.reply(command, params);
        assert_eq!(reply["ok"], true, "{command}: {reply}");
        reply["result"].clone()
    }

    /// Closes stdin and waits for the process; true when it exited with code 0.
    pub fn finish(mut self) -> bool {
        drop(self.stdin.take());
        self.child.wait().is_ok_and(|s| s.success())
    }
}

/// Where Scout stands and what the scenario measured.
pub struct Scenario {
    pub hash: String,
    pub walked_m: f64,
    pub final_position: [f64; 3],
    pub wall_front_z: f64,
    pub start: [f64; 3],
}

fn position(state: &Value, name: &str) -> [f64; 3] {
    let c = state["characters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no character {name} in {state}"));
    let p = c["position"].as_array().unwrap();
    [
        p[0].as_f64().unwrap(),
        p[1].as_f64().unwrap(),
        p[2].as_f64().unwrap(),
    ]
}

/// The M0 scenario up to the state hash, with the simulation still running. The wall is
/// dropped at z = -8, 0.5 m deep: its south face is at z = -7.75.
pub fn m0_scenario(server: &mut Server) -> Scenario {
    server.ok("file.new", json!({"name": "M0 valley", "seed": 42}));
    server.ok(
        "terrain.set",
        json!({"kind": "noise", "size_m": 256, "cell_m": 2, "relief_m": 4, "feature_m": 80, "seed": 42}),
    );
    let scattered = server.ok(
        "entity.scatter",
        json!({
            "count": 40, "name": "Oak", "seed": 5, "center": [0, 0], "radius_m": 90,
            "min_distance_m": 5, "scale_range": [0.7, 1.2],
            "components": [{"type": "plant", "species": "quercus-robur", "growth": 1}]
        }),
    );
    assert!(scattered["placed"].as_u64().unwrap() > 10, "{scattered}");
    server.ok(
        "entity.add",
        json!({"name": "Scout", "at": [0, 0], "components": [{"type": "character"}]}),
    );
    let before = server.ok("world.inspect", json!({"limit": 1000}));
    let scout = before["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "Scout")
        .unwrap();
    let start = [
        scout["position"][0].as_f64().unwrap(),
        scout["position"][1].as_f64().unwrap(),
        scout["position"][2].as_f64().unwrap(),
    ];
    server.ok("sim.start", json!({}));
    server.ok(
        "sim.input",
        json!({"entity": "Scout", "forward": 1, "yaw_deg": 0}),
    );
    let after = server.ok("sim.step", json!({"ticks": 120}));
    let p = position(&after, "Scout");
    let walked_m = ((p[0] - start[0]).powi(2) + (p[2] - start[2]).powi(2)).sqrt();
    // The simulation is live: the wall goes into the running world.
    server.ok(
        "entity.add",
        json!({"name": "Wall", "at": [0, -8], "components": [{"type": "shape", "kind": "box", "size": [6, 4, 0.5], "color": [0.6, 0.6, 0.6], "solid": true}]}),
    );
    for _ in 0..4 {
        let state = server.ok("sim.step", json!({"seconds": 1}));
        let z = position(&state, "Scout")[2];
        assert!(z > -7.75, "Scout passed the wall: z = {z}");
    }
    let state = server.ok("sim.state", json!({}));
    Scenario {
        hash: state["hash"].as_str().unwrap().to_string(),
        walked_m,
        final_position: position(&state, "Scout"),
        wall_front_z: -7.75,
        start,
    }
}
