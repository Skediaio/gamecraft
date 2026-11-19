//! The scenarios themselves. Each pushes its labelled hashes and stops at the first error.

use crate::{hash_text, hex};
use gamecraft_engine::{Session, example_world};
use gamecraft_math::StateHasher;
use gamecraft_sim::TICK_HZ;
use gamecraft_world::{Terrain, World};
use serde_json::{Value, json};

/// A play is hashed every this many ticks (one second).
const CHECKPOINT_TICKS: u64 = TICK_HZ as u64;

/// The Lantern prototype, as committed.
const LANTERN_WORLD: &str = include_str!("../../../examples/lantern/world.gamecraft");
/// The playthrough that wins the Lantern prototype.
const LANTERN_PLAYTHROUGH: &str = include_str!("../../../examples/lantern/playthrough.json");
/// The Lantern prototype is played to 125 s, as its CLI test does (the mist rises at 120 s).
const LANTERN_END_S: u64 = 125;

/// The Skirmish prototype, as committed.
const SKIRMISH_WORLD: &str = include_str!("../../../examples/skirmish/world.gamecraft");
/// The playthrough that wins the Skirmish prototype.
const SKIRMISH_PLAYTHROUGH: &str = include_str!("../../../examples/skirmish/playthrough.json");
/// The Skirmish prototype is played to 60 s, as its CLI test does (the last enemy falls at
/// about 53 s).
const SKIRMISH_END_S: u64 = 60;

/// Rows of the 4 km height field hashed together, so a difference names a band of rows.
const TERRAIN_BAND_ROWS: usize = 128;

/// Labelled hashes, in order.
pub type Hashes = Vec<(String, String)>;

/// Runs one command, its error as a sentence that names the scenario's step.
fn run(session: &mut Session, command: &str, params: &Value) -> Result<Value, String> {
    session
        .run(command, params)
        .map_err(|e| format!("`{command}` with {params} failed: {e}"))
}

/// A playing session: the hash recorder of a play.
struct Play<'a> {
    session: Session,
    hashes: &'a mut Hashes,
}

impl<'a> Play<'a> {
    /// Starts the simulation of `world` and records the hash of tick 0 in `hashes`.
    fn start(world: World, hashes: &'a mut Hashes) -> Result<Play<'a>, String> {
        let mut play = Play {
            session: Session::new(world),
            hashes,
        };
        run(&mut play.session, "sim.start", &json!({}))?;
        play.checkpoint()?;
        Ok(play)
    }

    /// The tick and state hash of the running simulation.
    fn state(&self) -> Result<(u64, u64), String> {
        self.session
            .live()
            .map(|sim| (sim.tick(), sim.state_hash()))
            .ok_or_else(|| "the simulation stopped during the play".to_owned())
    }

    fn checkpoint(&mut self) -> Result<(), String> {
        let (tick, hash) = self.state()?;
        self.hashes.push((format!("tick {tick}"), hex(hash)));
        Ok(())
    }

    /// Steps `ticks`, stopping at every multiple of [`CHECKPOINT_TICKS`] to record the hash.
    fn step(&mut self, ticks: u64) -> Result<(), String> {
        let (mut tick, _) = self.state()?;
        let end = tick.saturating_add(ticks);
        while tick < end {
            let next = (tick / CHECKPOINT_TICKS + 1)
                .saturating_mul(CHECKPOINT_TICKS)
                .min(end);
            run(
                &mut self.session,
                "sim.step",
                &json!({"ticks": next - tick}),
            )?;
            tick = self.state()?.0;
            if tick != next {
                return Err(format!(
                    "sim.step ended on tick {tick} instead of tick {next}"
                ));
            }
            if tick % CHECKPOINT_TICKS == 0 {
                self.checkpoint()?;
            }
        }
        Ok(())
    }

    /// Runs a script of `[id, params]` steps; `sim.step` is split at the checkpoints.
    fn script(&mut self, steps: &[(String, Value)]) -> Result<(), String> {
        for (index, (command, params)) in steps.iter().enumerate() {
            if command == "sim.step" {
                let ticks = params.get("ticks").and_then(Value::as_u64).ok_or_else(|| {
                    format!("script step {index}: sim.step needs a whole number of `ticks`")
                })?;
                self.step(ticks)?;
            } else {
                run(&mut self.session, command, params)
                    .map_err(|e| format!("script step {index}: {e}"))?;
            }
        }
        Ok(())
    }

    /// Steps until `end` (a tick), recording the checkpoints.
    fn play_to(&mut self, end: u64) -> Result<(), String> {
        let (tick, _) = self.state()?;
        self.step(end.saturating_sub(tick))
    }

    /// Records the status of an objective as the last entry (`done`, `open` or `failed`).
    fn objective(&mut self, id: &str) -> Result<(), String> {
        let sim = self
            .session
            .live()
            .ok_or_else(|| "the simulation stopped during the play".to_owned())?;
        let objective = sim
            .world()
            .objectives
            .iter()
            .find(|o| o.id == id)
            .ok_or_else(|| format!("the world has no objective `{id}`"))?;
        let status = serde_json::to_value(objective.status).map_err(|e| e.to_string())?;
        let status = status.as_str().unwrap_or("unknown").to_owned();
        self.hashes.push((format!("objective {id}"), status));
        Ok(())
    }
}

/// Parses a playthrough: a JSON list of `["id", {params}]`.
fn playthrough(text: &str) -> Result<Vec<(String, Value)>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("the playthrough is not JSON: {e}"))?;
    let list = value
        .as_array()
        .ok_or("the playthrough must be a list of [\"id\", {params}]")?;
    list.iter()
        .enumerate()
        .map(|(index, step)| match step.as_array().map(Vec::as_slice) {
            Some([Value::String(id), params]) => Ok((id.clone(), params.clone())),
            _ => Err(format!(
                "playthrough step {index} must be [\"id\", {{params}}], not {step}"
            )),
        })
        .collect()
}

/// A prototype world played through and on to `end_s`; the last entry is whether `objective`
/// is won.
fn prototype(
    hashes: &mut Hashes,
    world: &str,
    script: &str,
    end_s: u64,
    objective: &str,
) -> Result<(), String> {
    let world = World::from_json(world).map_err(|e| format!("the world does not load: {e}"))?;
    let steps = playthrough(script)?;
    let mut play = Play::start(world, hashes)?;
    play.script(&steps)?;
    play.play_to(end_s.saturating_mul(CHECKPOINT_TICKS))?;
    play.objective(objective)
}

/// The example world played for 600 ticks: the Scout runs, strafes and jumps, turns, fires
/// uphill, walks back and stands. Every input field the simulation has is used once.
pub fn example_play(hashes: &mut Hashes) -> Result<(), String> {
    let mut play = Play::start(example_world(), hashes)?;
    let inputs = [
        json!({"entity": "Scout", "forward": 1, "face": [0, -60], "run": true}),
        json!({"entity": "Scout", "forward": 0.5, "right": 1, "yaw_deg": 75, "jump": true}),
        json!({"entity": "Scout", "forward": 0, "right": 0, "jump": false, "face_entity": "Oak", "fire": true}),
        json!({"entity": "Scout", "face": null, "pitch_deg": 20, "forward": -1, "run": false}),
        json!({"entity": "Scout", "fire": false, "forward": 0.7, "right": -0.4, "yaw_deg": 210, "interact": true}),
        json!({"entity": "Scout", "forward": 0, "right": 0, "interact": false}),
    ];
    for input in &inputs {
        run(&mut play.session, "sim.input", input)?;
        play.step(100)?;
    }
    Ok(())
}

/// The Lantern prototype won by its playthrough.
pub fn lantern(hashes: &mut Hashes) -> Result<(), String> {
    prototype(
        hashes,
        LANTERN_WORLD,
        LANTERN_PLAYTHROUGH,
        LANTERN_END_S,
        "light-all",
    )
}

/// The Skirmish prototype won by its playthrough.
pub fn skirmish(hashes: &mut Hashes) -> Result<(), String> {
    prototype(
        hashes,
        SKIRMISH_WORLD,
        SKIRMISH_PLAYTHROUGH,
        SKIRMISH_END_S,
        "clear",
    )
}

/// Firing solutions of the Scout of the example world: points near and far, up and down hill,
/// at entities, one out of reach, and the same again in a strong cross wind. Each answer is
/// hashed as its JSON text (the numbers' shortest round-trip form, so every bit counts).
pub fn aim(hashes: &mut Hashes) -> Result<(), String> {
    let mut session = Session::new(example_world());
    let targets = [
        json!([0, 1, -60]),
        json!([30, 4, -100]),
        json!([-50, -2, 20]),
        json!([5, 0.5, 8]),
        json!("Crate"),
        json!("Oak"),
        json!([0, 0, -5000]),
    ];
    for (air, environment) in [
        ("still", json!({"wind_speed_mps": 0})),
        ("wind", json!({"wind_speed_mps": 14})),
    ] {
        run(&mut session, "environment.set", &environment)?;
        for to in &targets {
            let answer = run(
                &mut session,
                "query.aim",
                &json!({"from": "Scout", "to": to}),
            )?;
            hashes.push((format!("{air} to {to}"), hash_text(&answer.to_string())));
        }
    }
    Ok(())
}

/// A 4 km noise terrain at 2 m cells (2049 x 2049 samples) with layers, paint rules and
/// sculpting: the heights hashed in bands of 128 rows, then the layer weights, then
/// interpolated heights and normals between the samples.
pub fn terrain(hashes: &mut Hashes) -> Result<(), String> {
    let terrain: Terrain = serde_json::from_value(json!({
        "size_m": 4096,
        "cell_m": 2,
        "source": {"kind": "noise", "seed": 4242, "relief_m": 600, "feature_m": 1500,
                   "octaves": 8, "roughness": 0.55},
        "layers": [
            {"name": "grass", "color": [0.075, 0.13, 0.035], "roughness": 0.9},
            {"name": "rock", "color": [0.28, 0.27, 0.25], "roughness": 0.8},
            {"name": "snow", "color": [0.9, 0.9, 0.95], "roughness": 0.4}
        ],
        "paint_rules": [
            {"layer": "rock", "slope_min_deg": 30},
            {"layer": "snow", "height_min_m": 180}
        ],
        "edits": [
            {"op": "raise", "center": [-900, 400], "radius_m": 300, "strength_m": 80},
            {"op": "lower", "center": [700, -1200], "radius_m": 250, "strength_m": 60},
            {"op": "flatten", "center": [0, 0], "radius_m": 120, "height_m": 10},
            {"op": "smooth", "center": [0, 0], "radius_m": 200, "iterations": 3},
            {"op": "paint", "layer": "grass", "center": [1500, 1500], "radius_m": 90}
        ]
    }))
    .map_err(|e| format!("the 4 km terrain does not parse: {e}"))?;
    let field = terrain
        .build()
        .map_err(|e| format!("the 4 km terrain does not build: {e}"))?;
    let cols = field.cols().max(1);
    let rows: Vec<&[f32]> = field.heights().chunks(cols).collect();
    for (band, chunk) in rows.chunks(TERRAIN_BAND_ROWS).enumerate() {
        let mut h = StateHasher::new();
        for v in chunk.iter().flat_map(|row| row.iter()) {
            h.write_f32(*v);
        }
        let first = band.saturating_mul(TERRAIN_BAND_ROWS);
        let last = first.saturating_add(chunk.len()).saturating_sub(1);
        hashes.push((format!("rows {first}-{last}"), hex(h.finish())));
    }
    let mut h = StateHasher::new();
    for w in field.weights() {
        h.write_bytes(w);
    }
    hashes.push(("weights".to_owned(), hex(h.finish())));
    let mut h = StateHasher::new();
    for i in 0..400_u32 {
        // Points spread over the terrain, between the samples (off the grid).
        let x = f64::from(i * 373 % 40_000) / 10.0 - 2000.0 + 0.37;
        let z = f64::from(i * 917 % 40_000) / 10.0 - 2000.0 + 0.61;
        h.write_f64(field.height_at(x, z).unwrap_or(f64::NAN));
        for n in field.normal_at(x, z).unwrap_or([f64::NAN; 3]) {
            h.write_f64(n);
        }
    }
    hashes.push(("height_at and normal_at".to_owned(), hex(h.finish())));
    Ok(())
}

/// Two scatters on the example world, one over the whole terrain with a minimum distance and
/// a slope limit, one in a circle with a clearing: each answer, then the document's text.
pub fn scatter(hashes: &mut Hashes) -> Result<(), String> {
    let mut session = Session::new(example_world());
    let calls = [
        json!({"count": 600, "seed": 99, "name": "Stone", "min_distance_m": 1.5,
               "max_slope_deg": 30,
               "components": [{"type": "shape", "kind": "box", "size": [0.5, 0.4, 0.6]}]}),
        json!({"count": 200, "name": "Pine", "center": [10, -20], "radius_m": 40,
               "exclude": [{"center": [0, 0], "radius_m": 8}],
               "components": [{"type": "plant", "species": "quercus-robur"}]}),
    ];
    for (index, call) in calls.iter().enumerate() {
        let answer = run(&mut session, "entity.scatter", call)?;
        hashes.push((
            format!("scatter {}", index + 1),
            hash_text(&answer.to_string()),
        ));
    }
    let text = session
        .document()
        .to_json()
        .map_err(|e| format!("the document does not serialise: {e}"))?;
    hashes.push(("document".to_owned(), hash_text(&text)));
    Ok(())
}
