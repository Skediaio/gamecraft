//! The determinism check across targets (AGENTS.md rule 3, `docs/architecture.md`,
//! *Determinism*).
//!
//! A fixed set of deterministic scenarios, each giving a list of labelled hashes: the example
//! world played for 600 ticks with scripted input, the Lantern and Skirmish prototypes played
//! through to the end, `query.aim` solutions, a 4 km noise terrain's height field and a
//! scatter, a grove whose trunks block a walking character and its shots. `cargo xtask wasm-hash` runs them natively (the binary of this crate) and as
//! WebAssembly in Node (the export [`wasm_hash_report`], built with wasm-bindgen) and compares
//! every hash. The plays are hashed every 60 ticks, so a difference names the first second
//! in which the two targets part.
//!
//! Everything here is pure: the prototype worlds and playthroughs are included at compile
//! time, so the WebAssembly module needs no file system.

mod scenarios;

pub use scenarios::Hashes;

use gamecraft_math::StateHasher;
use serde_json::{Value, json};

/// One scenario: a name and the function that plays it.
pub struct Scenario {
    /// The name the comparison reports.
    pub name: &'static str,
    /// Plays the scenario, pushing its labelled hashes in order; `Err` is the sentence that
    /// says why it stopped (the hashes pushed before stay).
    pub run: fn(&mut Hashes) -> Result<(), String>,
}

/// What one scenario gave on this target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The scenario's name.
    pub name: String,
    /// The labelled checkpoints in order: `("tick 60", "0123456789abcdef")`, ...
    pub hashes: Vec<(String, String)>,
    /// Why the scenario stopped, when it did; the hashes up to that point are kept.
    pub error: Option<String>,
}

/// Every scenario, in the order they run.
pub const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "example-600-ticks",
        run: scenarios::example_play,
    },
    Scenario {
        name: "lantern-playthrough",
        run: scenarios::lantern,
    },
    Scenario {
        name: "skirmish-playthrough",
        run: scenarios::skirmish,
    },
    Scenario {
        name: "query-aim",
        run: scenarios::aim,
    },
    Scenario {
        name: "terrain-4km",
        run: scenarios::terrain,
    },
    Scenario {
        name: "scatter",
        run: scenarios::scatter,
    },
    Scenario {
        name: "grove",
        run: scenarios::grove,
    },
];

/// Plays one scenario.
pub fn run_scenario(scenario: &Scenario) -> Outcome {
    let mut hashes = Vec::new();
    let error = (scenario.run)(&mut hashes).err();
    Outcome {
        name: scenario.name.to_owned(),
        hashes,
        error,
    }
}

/// Plays every scenario.
pub fn report() -> Vec<Outcome> {
    SCENARIOS.iter().map(run_scenario).collect()
}

/// Every scenario's outcome as JSON: `{"scenarios": [{"name", "error", "hashes": [[label,
/// hash], ...]}]}`. Hashes are 16 lowercase hex digits (strings: a JSON number cannot hold a
/// `u64` exactly).
pub fn report_json() -> String {
    let scenarios: Vec<Value> = report()
        .iter()
        .map(|o| {
            let hashes: Vec<Value> = o.hashes.iter().map(|(l, h)| json!([l, h])).collect();
            json!({"name": o.name, "error": o.error, "hashes": hashes})
        })
        .collect();
    json!({ "scenarios": scenarios }).to_string()
}

/// The report of [`report_json`], exported to JavaScript for the WebAssembly run in Node.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn wasm_hash_report() -> String {
    report_json()
}

/// A hash as 16 lowercase hex digits.
fn hex(hash: u64) -> String {
    format!("{hash:016x}")
}

/// The hash of a text (its length and UTF-8 bytes).
fn hash_text(text: &str) -> String {
    let mut h = StateHasher::new();
    h.write_str(text);
    hex(h.finish())
}
