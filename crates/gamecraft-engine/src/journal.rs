//! The journal: a directed play session recorded with its ticks, so it replays to the same
//! state hash and can be bisected (`docs/design/journal.md`).
//!
//! While a simulation runs, the session records every command sent to it from outside that
//! changes the running world or the simulation: world commands, `sim.input`, `sim.fire`,
//! `sim.release`, `message.show` and `sim.step` ([`records`]), each with the tick at which it
//! took effect and the state hash after it; a `sim.step` also keeps the tick it ended on.
//! Queries are not recorded, and neither are the commands of triggers: they follow from the
//! rest. The journal starts with `sim.start`, embedding the document the simulation started
//! from, and stays readable after `sim.stop` until the next `sim.start`.
//!
//! [`Journal::replay`] runs a journal again from its embedded world and compares the hash
//! after every entry: the first entry whose hash differs names the tick where the replay
//! diverges. [`Journal::to_json`] and [`Journal::from_json`] read and write the journal file
//! format, version [`JOURNAL_FORMAT`]: one JSON document, with the world and each entry on a
//! line of its own so a person can read and edit it.

use crate::error::CommandError;
use crate::session::Session;
use gamecraft_world::World;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The version of the journal file format.
pub const JOURNAL_FORMAT: u32 = 1;

/// Most entries one journal keeps; later commands are not recorded and the journal is marked
/// truncated (it can no longer replay to the end).
pub const MAX_JOURNAL_ENTRIES: usize = 1_000_000;

/// The play commands a journal records, besides every world command.
pub const RECORDED_PLAY_COMMANDS: [&str; 5] = [
    "sim.input",
    "sim.fire",
    "sim.release",
    "message.show",
    "sim.step",
];

/// Whether a successful run of `command` (of scope `scope`) while playing is recorded.
pub fn records(command: &str, scope: crate::Scope) -> bool {
    scope == crate::Scope::World || RECORDED_PLAY_COMMANDS.contains(&command)
}

/// One recorded command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalEntry {
    /// The tick at which it took effect (for `sim.step`, the tick it started from).
    pub tick: u64,
    /// The command id.
    pub command: String,
    /// Its parameters, as sent.
    #[serde(default)]
    pub params: Value,
    /// For `sim.step`: the tick it ended on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_tick: Option<u64>,
    /// The state hash after it, as 16 hex digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// Where a journal ends: the tick and state hash of the simulation after its last entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalEnd {
    /// The tick.
    pub tick: u64,
    /// The state hash, as 16 hex digits.
    pub hash: String,
}

/// A directed play session: the world it started from and the commands sent to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Journal {
    /// The document the simulation started from.
    pub world: World,
    /// The recorded commands, in order.
    pub entries: Vec<JournalEntry>,
    /// The tick and hash after the last entry (after `sim.start` when there is none).
    pub end: Option<JournalEnd>,
    /// Commands were left out because the journal was full ([`MAX_JOURNAL_ENTRIES`]).
    pub truncated: bool,
}

/// The file layout.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalFile {
    gamecraft_journal: Value,
    world: Value,
    #[serde(default)]
    entries: Vec<JournalEntry>,
    #[serde(default, rename = "final")]
    end: Option<JournalEnd>,
    #[serde(default)]
    truncated: bool,
}

/// The first place where a replay differs from its journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// The index of the entry, from 0; `None` for the journal's final state.
    pub entry: Option<usize>,
    /// The tick where the hash first differs (or where the replay went another way).
    pub tick: u64,
    /// A sentence that says what differs.
    pub message: String,
}

/// What [`Journal::replay`] did.
pub struct Replay {
    /// The session after the replay, still playing: ask it `sim.state`.
    pub session: Session,
    /// The tick the replay stopped at.
    pub tick: u64,
    /// The state hash there, 16 hex digits.
    pub hash: String,
    /// How many entries ran.
    pub entries_run: usize,
    /// How many recorded hashes were compared (and matched up to the divergence).
    pub hashes_checked: usize,
    /// The first difference, `None` when the replay matches its journal (up to `to_tick`).
    pub divergence: Option<Divergence>,
}

/// A state hash as written in journals and answers.
pub(crate) fn hash_text(hash: u64) -> String {
    format!("{hash:016x}")
}

impl Journal {
    /// An empty journal of a simulation started from `world`.
    pub fn new(world: World) -> Journal {
        Journal {
            world,
            entries: Vec::new(),
            end: None,
            truncated: false,
        }
    }

    /// The journal as JSON: `{gamecraft_journal: 1, world: {...}, entries: [...], final:
    /// {tick, hash}, truncated?}`, with the world and every entry on a line of their own.
    pub fn to_json(&self) -> Result<String, String> {
        let world = self
            .world
            .to_json()
            .map_err(|e| format!("the journal's world cannot be written: {e}"))?;
        let world: Value = serde_json::from_str(&world)
            .map_err(|e| format!("the journal's world cannot be written: {e}"))?;
        let line = |v: &Value| serde_json::to_string(v).map_err(|e| e.to_string());
        let mut out = String::new();
        out.push_str(&format!("{{\"gamecraft_journal\": {JOURNAL_FORMAT},\n"));
        out.push_str(&format!("\"world\": {},\n", line(&world)?));
        out.push_str("\"entries\": [");
        for (i, entry) in self.entries.iter().enumerate() {
            let v = serde_json::to_value(entry).map_err(|e| e.to_string())?;
            out.push_str(if i == 0 { "\n" } else { ",\n" });
            out.push_str(&line(&v)?);
        }
        out.push_str("\n],\n");
        let end = serde_json::to_value(&self.end).map_err(|e| e.to_string())?;
        out.push_str(&format!("\"final\": {}", line(&end)?));
        if self.truncated {
            out.push_str(",\n\"truncated\": true");
        }
        out.push_str("\n}\n");
        Ok(out)
    }

    /// Reads a journal written by [`Journal::to_json`] (or by hand): checks the format
    /// version, rejects unknown fields and reads the embedded world with all its checks.
    pub fn from_json(text: &str) -> Result<Journal, String> {
        let file: JournalFile =
            serde_json::from_str(text).map_err(|e| format!("the journal is not valid: {e}"))?;
        if file.gamecraft_journal.as_u64() != Some(u64::from(JOURNAL_FORMAT)) {
            return Err(format!(
                "gamecraft_journal (the format version) is {}; this engine reads version \
                 {JOURNAL_FORMAT} only",
                crate::spec::excerpt(&file.gamecraft_journal.to_string())
            ));
        }
        if file.entries.len() > MAX_JOURNAL_ENTRIES {
            return Err(format!(
                "the journal holds {} entries; at most {MAX_JOURNAL_ENTRIES} are allowed",
                file.entries.len()
            ));
        }
        let world_text = serde_json::to_string(&file.world)
            .map_err(|e| format!("the journal's world is not valid: {e}"))?;
        let world = World::from_json(&world_text)
            .map_err(|e| format!("the journal's world is not valid: {e}"))?;
        Ok(Journal {
            world,
            entries: file.entries,
            end: file.end,
            truncated: file.truncated,
        })
    }

    /// The entries whose tick is within `from_tick..=to_tick`, with their index, at most
    /// `limit`; and how many matched in all.
    pub fn select(
        &self,
        from_tick: Option<u64>,
        to_tick: Option<u64>,
        limit: usize,
    ) -> (Vec<Value>, usize) {
        let matching = self.entries.iter().enumerate().filter(|(_, e)| {
            from_tick.is_none_or(|t| e.tick >= t) && to_tick.is_none_or(|t| e.tick <= t)
        });
        let mut total = 0usize;
        let mut out = Vec::new();
        for (index, entry) in matching {
            total += 1;
            if out.len() < limit
                && let Ok(Value::Object(mut map)) = serde_json::to_value(entry)
            {
                map.insert("index".into(), json!(index));
                out.push(Value::Object(map));
            }
        }
        (out, total)
    }

    /// Runs the journal again: a new session on the embedded world, `sim.start`, then every
    /// entry in order, comparing the tick it was recorded at and the state hash after it.
    ///
    /// With `to_tick` the replay stops when the simulation reaches that tick, before the
    /// commands recorded at it (a `sim.step` that crosses it is cut short), and the final
    /// state is not compared. The first difference is kept in [`Replay::divergence`]; the
    /// replay goes on after it, so the session ends where the journal ends. An error only
    /// when the simulation cannot start at all.
    pub fn replay(&self, to_tick: Option<u64>) -> Result<Replay, CommandError> {
        let mut session = Session::new(self.world.clone());
        session.run("sim.start", &json!({}))?;
        let mut divergence: Option<Divergence> = None;
        let (mut entries_run, mut hashes_checked) = (0usize, 0usize);
        for (index, entry) in self.entries.iter().enumerate() {
            let tick = current_tick(&session);
            if let Some(limit) = to_tick
                && (tick >= limit || entry.tick >= limit)
            {
                break;
            }
            if entry.tick != tick {
                note(
                    &mut divergence,
                    Divergence {
                        entry: Some(index),
                        tick,
                        message: format!(
                            "entry {index} (`{}`) was recorded at tick {}, but the replay reached it \
                         at tick {tick}",
                            crate::spec::excerpt(&entry.command),
                            entry.tick
                        ),
                    },
                );
            }
            let mut params = entry.params.clone();
            let mut cut = false;
            if let (Some(limit), Some(end)) = (to_tick, entry.end_tick)
                && entry.command == "sim.step"
                && end > limit
            {
                params = json!({"ticks": limit.saturating_sub(tick)});
                cut = true;
            }
            entries_run += 1;
            if let Err(e) = session.run(&entry.command, &params) {
                note(
                    &mut divergence,
                    Divergence {
                        entry: Some(index),
                        tick,
                        message: format!(
                            "entry {index} (`{}` at tick {}) failed in the replay: {e}",
                            crate::spec::excerpt(&entry.command),
                            entry.tick
                        ),
                    },
                );
                continue;
            }
            if cut {
                break;
            }
            let after = current_tick(&session);
            if let Some(end) = entry.end_tick
                && end != after
            {
                note(
                    &mut divergence,
                    Divergence {
                        entry: Some(index),
                        tick: after,
                        message: format!(
                            "entry {index} (`{}` from tick {}) ended at tick {after}; it was \
                         recorded ending at tick {end}",
                            crate::spec::excerpt(&entry.command),
                            entry.tick
                        ),
                    },
                );
            }
            if let Some(recorded) = &entry.hash {
                let replayed = current_hash(&session);
                if *recorded == replayed {
                    if divergence.is_none() {
                        hashes_checked += 1;
                    }
                } else {
                    note(
                        &mut divergence,
                        Divergence {
                            entry: Some(index),
                            tick: after,
                            message: format!(
                                "the state hash diverges at tick {after}: after entry {index} (`{}` \
                             at tick {}{}) the journal has {recorded}, the replay {replayed}",
                                crate::spec::excerpt(&entry.command),
                                entry.tick,
                                entry
                                    .end_tick
                                    .map(|e| format!(" to tick {e}"))
                                    .unwrap_or_default()
                            ),
                        },
                    );
                }
            }
        }
        let tick = current_tick(&session);
        let hash = current_hash(&session);
        if to_tick.is_none()
            && let Some(end) = &self.end
        {
            if end.tick != tick || end.hash != hash {
                note(
                    &mut divergence,
                    Divergence {
                        entry: None,
                        tick,
                        message: format!(
                            "the replay ends at tick {tick} with hash {hash}; the journal ends at \
                         tick {} with hash {}",
                            end.tick, end.hash
                        ),
                    },
                );
            } else if divergence.is_none() {
                hashes_checked += 1;
            }
        }
        Ok(Replay {
            session,
            tick,
            hash,
            entries_run,
            hashes_checked,
            divergence,
        })
    }

    /// The journal as the answer of `journal.get`: a page of entries and the summary.
    pub(crate) fn summary(&self, recording: bool, entries: Vec<Value>, total: usize) -> Value {
        let mut out = Map::new();
        out.insert("recording".into(), json!(recording));
        out.insert("world".into(), json!(self.world.name));
        out.insert("entry_count".into(), json!(self.entries.len()));
        out.insert("matched".into(), json!(total));
        out.insert("entries".into(), Value::Array(entries));
        out.insert(
            "final".into(),
            serde_json::to_value(&self.end).unwrap_or(Value::Null),
        );
        out.insert("truncated".into(), json!(self.truncated));
        Value::Object(out)
    }
}

/// Keeps `d` when no difference was found before.
fn note(slot: &mut Option<Divergence>, d: Divergence) {
    if slot.is_none() {
        *slot = Some(d);
    }
}

fn current_tick(session: &Session) -> u64 {
    session.live().map_or(0, gamecraft_sim::Sim::tick)
}

fn current_hash(session: &Session) -> String {
    session
        .live()
        .map_or_else(String::new, |sim| hash_text(sim.state_hash()))
}
