//! A directed Skirmish, through `gamecraft-cli serve`: a scripted game master, plain Rust that
//! decides only from `gm.overview` answers, directs the game while the scripted player plays
//! it; and every move of `docs/guide/game-master.md` runs on the running Skirmish.
//!
//! The directed game's journal is `examples/skirmish/directed.journal.json`. When the engine
//! changes the game on purpose, write it again with `GAMECRAFT_BLESS=1 cargo test -p
//! gamecraft-cli --test game_master`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
use common::*;
use serde_json::{Value, json};
use std::path::PathBuf;

const GUIDE: &str = include_str!("../../../docs/guide/game-master.md");

/// Most game time after the player's script ends before the game must be over, seconds.
const MAX_EXTRA_S: u64 = 300;
/// How often the game master looks, ticks (one second).
const LOOK_EVERY_TICKS: u64 = 60;
/// The houses, from the Skirmish's build script: within this distance the player is there.
const HOUSES: [[f64; 2]; 2] = [[-8.0, -50.0], [16.0, -58.0]];
const AT_THE_HOUSES_M: f64 = 20.0;

fn skirmish() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .join("examples")
        .join("skirmish")
}

/// The moves of the guide: the lines of each `jsonl gamecraft-gm` block, parsed.
fn moves() -> Vec<Vec<Value>> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<Value>> = None;
    for line in GUIDE.lines() {
        let trimmed = line.trim();
        match &mut current {
            None if trimmed == "```jsonl gamecraft-gm" => current = Some(Vec::new()),
            None => {}
            Some(_) if trimmed == "```" => blocks.extend(current.take()),
            Some(_) if trimmed.is_empty() => {}
            Some(lines) => lines.push(
                serde_json::from_str(trimmed)
                    .unwrap_or_else(|e| panic!("not JSON in the guide: {e}: {trimmed}")),
            ),
        }
    }
    assert!(current.is_none(), "a move block is not closed");
    blocks
}

/// Sends one move's commands, each with its intent.
fn act(server: &mut Server, commands: &[Value]) {
    for c in commands {
        let line = json!({"id": 0, "method": "engine.execute", "params": c});
        let reply = server.raw(&line.to_string());
        assert_eq!(reply["ok"], true, "{c}\n-> {reply}");
    }
}

fn start(dir: &Temp) -> Server {
    let world = skirmish().join("world.gamecraft");
    let root = dir.0.display().to_string();
    let mut server = Server::start(
        "serve",
        &[
            "--file",
            &world.display().to_string(),
            "--automation-write-root",
            &root,
        ],
        &dir.0,
    );
    server.ok("sim.start", json!({}));
    server
}

fn player(overview: &Value) -> Value {
    overview["characters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Player")
        .cloned()
        .unwrap_or_else(|| panic!("no Player in {overview}"))
}

fn red_down(overview: &Value) -> u64 {
    overview["teams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["team"] == "red")
        .map_or(0, |t| t["down"].as_u64().unwrap())
}

fn over(overview: &Value) -> bool {
    overview["objectives"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["id"] == "clear" || o["id"] == "alive")
        .all(|o| o["status"] != "open")
}

/// The scripted game master: what it did, as (tick, move).
#[derive(Default)]
struct GameMaster {
    reinforced: bool,
    fogged: bool,
    wind: bool,
    log: Vec<(u64, &'static str)>,
}

impl GameMaster {
    /// Looks at the overview and acts on it.
    fn direct(&mut self, server: &mut Server, moves: &[Vec<Value>]) {
        let o = server.ok("gm.overview", json!({}));
        let tick = o["tick"].as_u64().unwrap();
        let me = player(&o);
        if !self.reinforced && red_down(&o) >= 2 {
            act(server, &moves[1]);
            self.reinforced = true;
            self.log.push((tick, "reinforcements"));
        }
        let health = me["health"].as_f64().unwrap();
        if !self.fogged && !me["down"].as_bool().unwrap() && health < 50.0 {
            act(server, &moves[0]);
            self.fogged = true;
            self.log.push((tick, "fog"));
        }
        let p = me["position"].as_array().unwrap();
        let (x, z) = (p[0].as_f64().unwrap(), p[2].as_f64().unwrap());
        let at_houses = HOUSES.iter().any(|[hx, hz]| {
            let (dx, dz) = (x - hx, z - hz);
            (dx * dx + dz * dz).sqrt() <= AT_THE_HOUSES_M
        });
        if !self.wind && at_houses {
            act(server, &moves[3]);
            self.wind = true;
            self.log.push((tick, "wind"));
        }
    }
}

/// Steps `ticks`, one second at a time, with the game master looking after each second.
fn step(server: &mut Server, ticks: u64, gm: &mut Option<GameMaster>, moves: &[Vec<Value>]) {
    let mut left = ticks;
    while left > 0 {
        let n = left.min(LOOK_EVERY_TICKS);
        server.ok("sim.step", json!({"ticks": n}));
        left -= n;
        if let Some(gm) = gm {
            gm.direct(server, moves);
        }
    }
}

/// Plays the Skirmish with the scripted player, directed or not, until the game is over: the
/// final overview, the game master (when directing) and the server.
fn play(dir: &Temp, directed: bool) -> (Value, Option<GameMaster>, Server) {
    let moves = moves();
    let mut server = start(dir);
    let mut gm = directed.then(GameMaster::default);
    let script: Value = serde_json::from_str(
        &std::fs::read_to_string(skirmish().join("playthrough.json")).unwrap(),
    )
    .unwrap();
    for item in script.as_array().unwrap() {
        let (command, params) = (item[0].as_str().unwrap(), &item[1]);
        match command {
            "sim.step" => step(
                &mut server,
                params["ticks"].as_u64().unwrap(),
                &mut gm,
                &moves,
            ),
            _ => {
                // The player's own commands; a player that is down takes no input.
                let reply = server.reply(command, params.clone());
                if reply["ok"] != true {
                    let o = server.ok("gm.overview", json!({}));
                    assert!(player(&o)["down"] == true, "{command} {params}: {reply}");
                }
            }
        }
    }
    // The player's script is over: it stands; the game runs on until it ends.
    for _ in 0..MAX_EXTRA_S {
        let o = server.ok("gm.overview", json!({}));
        if over(&o) {
            break;
        }
        step(&mut server, LOOK_EVERY_TICKS, &mut gm, &moves);
    }
    let last = server.ok("gm.overview", json!({}));
    (last, gm, server)
}

#[test]
fn a_scripted_game_master_directs_the_skirmish() {
    let plain_dir = Temp::new("gm-plain");
    let (plain, _, plain_server) = play(&plain_dir, false);
    assert!(plain_server.finish());
    assert!(over(&plain), "the undirected game ends: {plain}");

    let dir = Temp::new("gm-directed");
    let (directed, gm, mut server) = play(&dir, true);
    let gm = gm.unwrap();
    eprintln!("game master moves (tick, move): {:?}", gm.log);
    eprintln!(
        "objectives: {}, player: {}",
        directed["objectives"],
        player(&directed)
    );
    assert!(
        gm.reinforced,
        "two enemies were downed: reinforcements came"
    );
    assert!(
        gm.fogged,
        "the player fell below half health: fog rolled in"
    );
    assert!(gm.wind, "the player reached the houses: the wind turned");
    assert!(over(&directed), "the directed game ends: {directed}");
    assert_ne!(
        directed["hash"], plain["hash"],
        "directing changed the game"
    );
    let names: Vec<&str> = directed["characters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Reinforcement 1") && names.contains(&"Reinforcement 2"));
    assert_eq!(directed["weather"]["mist"], 1.0);
    assert_eq!(directed["weather"]["wind_direction_deg"], 90.0);

    // The journal replays to the same hash.
    let saved = server.ok("journal.save", json!({"path": "directed.journal.json"}));
    let path = PathBuf::from(saved["path"].as_str().unwrap());
    assert!(server.finish());
    let ran = run(&["replay", &path.display().to_string(), "--check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
    let replay: Value = serde_json::from_str(ran.out.trim()).unwrap();
    assert_eq!(replay["hash"], directed["hash"], "{replay}");
    assert_eq!(replay["matches"], true);

    // The committed journal is this one.
    let text = std::fs::read_to_string(&path).unwrap();
    let committed = skirmish().join("directed.journal.json");
    if std::env::var_os("GAMECRAFT_BLESS").is_some() {
        std::fs::write(&committed, &text).unwrap();
    }
    let old = std::fs::read_to_string(&committed).unwrap_or_default();
    assert!(
        old == text,
        "examples/skirmish/directed.journal.json is not the journal of this directed game; \
         write it again with GAMECRAFT_BLESS=1 if the change is intended"
    );
}

#[test]
fn the_committed_directed_journal_replays() {
    let committed = skirmish().join("directed.journal.json");
    let ran = run(&["replay", &committed.display().to_string(), "--check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
}

#[test]
fn every_move_of_the_guide_runs_on_the_skirmish() {
    let moves = moves();
    assert_eq!(moves.len(), 10, "the guide has ten moves");
    for (i, commands) in moves.iter().enumerate() {
        let dir = Temp::new(&format!("gm-move-{i}"));
        let mut server = start(&dir);
        server.ok("sim.step", json!({"seconds": 2}));
        act(&mut server, commands);
        let after = server.ok("sim.step", json!({"seconds": 3}));
        assert!(after["hash"].is_string(), "move {}: {after}", i + 1);
        assert!(server.finish());
    }
}
