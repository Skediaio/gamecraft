//! `journal.get`: the recorded commands of the running (or last) simulation
//! (`docs/design/journal.md`). Saving a journal to a file is a host command (`journal.save`),
//! because the engine touches no files.

use crate::error::{CommandError, bad};
use crate::params::Params;
use crate::session::{Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use serde_json::{Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[Registration {
    spec: get_spec,
    handler: Handler::Query(get),
}];

/// Most entries one `journal.get` lists.
pub const MAX_JOURNAL_PAGE: u64 = 10_000;
/// Entries `journal.get` lists when `limit` is left out.
const DEFAULT_JOURNAL_PAGE: u64 = 1000;

fn get_spec() -> CommandSpec {
    spec(
        "journal.get",
        "Journal",
        "The journal of the running simulation, or of the last one until the next sim.start: \
         every command that changed the running world or the simulation (world commands, \
         sim.input, sim.fire, sim.release, message.show, sim.step), in order, with the tick at \
         which it took effect and the state hash after it. Queries and the commands of \
         triggers are not recorded. Filter by tick; journal.save writes the whole journal, \
         with the world it started from, to a file that `gamecraft-cli replay` runs again.",
        vec![
            param(
                "from_tick",
                T::Integer,
                "Only entries at this tick or later.",
            ),
            param(
                "to_tick",
                T::Integer,
                "Only entries at this tick or earlier.",
            ),
            param("limit", T::Integer, "Most entries listed, 1 to 10000.").default(json!(1000)),
        ],
        "{recording: bool, world: name, entry_count, matched, entries: [{index, tick, command, \
         params, end_tick? (sim.step), hash}], final: {tick, hash} | null, truncated}; \
         recording is false and entries empty before the first sim.start",
        json!({"from_tick": 0, "limit": 100}),
    )
    .query()
}

fn get(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let limit = p.u64("limit")?.unwrap_or(DEFAULT_JOURNAL_PAGE);
    if !(1..=MAX_JOURNAL_PAGE).contains(&limit) {
        return Err(bad(format!(
            "`limit` is {limit}; it must be from 1 to {MAX_JOURNAL_PAGE}; page with `from_tick`"
        )));
    }
    let (from, to) = (p.u64("from_tick")?, p.u64("to_tick")?);
    if let (Some(f), Some(t)) = (from, to)
        && f > t
    {
        return Err(bad(format!(
            "`from_tick` {f} is after `to_tick` {t}; give from_tick <= to_tick"
        )));
    }
    let Some(journal) = session.journal() else {
        return Ok(json!({
            "recording": false,
            "world": Value::Null,
            "entry_count": 0,
            "matched": 0,
            "entries": [],
            "final": Value::Null,
            "truncated": false,
        }));
    };
    let (entries, total) = journal.select(from, to, usize::try_from(limit).unwrap_or(0));
    Ok(journal.summary(session.is_playing(), entries, total))
}
