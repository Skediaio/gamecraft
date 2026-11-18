//! Helpers shared by the integration tests.
#![allow(dead_code)]

use gamecraft_engine::{CommandError, Session, example_world};
use gamecraft_world::World;
use serde_json::Value;

/// A fresh session of the example world.
pub fn example() -> Session {
    Session::new(example_world())
}

/// A fresh session of the example world, playing.
pub fn playing() -> Session {
    let mut s = example();
    s.run("sim.start", &Value::Null).unwrap();
    s
}

/// Everything a failed command must leave unchanged.
#[derive(Debug, PartialEq)]
pub struct Snapshot {
    pub document: World,
    pub current: World,
    pub revision: u64,
    pub view_revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_depth: usize,
    pub playing: bool,
    pub tick: Option<u64>,
}

/// How many steps `undo` can take back, counted on a clone.
pub fn undo_depth(s: &Session) -> usize {
    let mut c = s.clone();
    let mut n = 0;
    while c.undo().is_ok() {
        n += 1;
    }
    n
}

pub fn snapshot(s: &Session) -> Snapshot {
    Snapshot {
        document: s.document().clone(),
        current: s.current_world().clone(),
        revision: s.revision(),
        view_revision: s.view_revision(),
        can_undo: s.can_undo(),
        can_redo: s.can_redo(),
        undo_depth: undo_depth(s),
        playing: s.is_playing(),
        tick: s.live().map(|l| l.tick()),
    }
}

/// Runs a command that must fail; checks the message starts with the command id and contains
/// every one of `needles`, and that the session is exactly as before.
pub fn refuse(s: &mut Session, command: &str, params: Value, needles: &[&str]) -> CommandError {
    let before = snapshot(s);
    let e = s
        .run(command, &params)
        .expect_err(&format!("{command} {params} should fail"));
    let msg = e.to_string();
    assert!(
        msg.starts_with(&format!("{command}: ")),
        "`{msg}` does not start with `{command}: `"
    );
    for n in needles {
        assert!(msg.contains(n), "`{msg}` does not contain `{n}`");
    }
    assert_eq!(
        snapshot(s),
        before,
        "{command} {params} changed the session"
    );
    e
}
