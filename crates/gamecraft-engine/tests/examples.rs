//! Every spec's `example` runs on a fresh session of the example world, and every World
//! command's example is undone and redone exactly.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, playing};
use gamecraft_engine::{Scope, Session};

/// Play commands whose example runs without a simulation.
const RUN_WITHOUT_SIMULATION: [&str; 2] = ["sim.start", "sim.state"];

fn session_for(name: &str, scope: Scope) -> Session {
    if scope == Scope::Play && !RUN_WITHOUT_SIMULATION.contains(&name) {
        playing()
    } else {
        example()
    }
}

#[test]
fn every_example_runs() {
    let base = example();
    let specs: Vec<_> = base.specs().into_iter().cloned().collect();
    assert!(specs.len() >= 16);
    for spec in specs {
        let mut s = session_for(&spec.name, spec.scope);
        let r = s.run(&spec.name, &spec.example);
        assert!(r.is_ok(), "{} {}: {:?}", spec.name, spec.example, r);
        // The example is valid against its own spec, and the notation names every parameter.
        assert!(spec.check(&spec.example).is_ok(), "{}", spec.name);
        for p in &spec.params {
            assert!(
                spec.notation().contains(&format!("\"{}\"", p.name)),
                "{}",
                spec.name
            );
            assert!(!p.doc.is_empty(), "{}.{} has no doc", spec.name, p.name);
        }
    }
}

#[test]
fn every_world_example_is_one_exact_undo_step() {
    let base = example();
    let specs: Vec<_> = base
        .specs()
        .into_iter()
        .filter(|s| s.scope == Scope::World)
        .cloned()
        .collect();
    for spec in specs {
        let mut s = example();
        let before = s.document().clone();
        let rev = s.revision();
        s.run(&spec.name, &spec.example).unwrap();
        let after = s.document().clone();
        assert_ne!(
            after, before,
            "{}: the example changes the document",
            spec.name
        );
        assert!(s.revision() > rev, "{}", spec.name);
        s.undo().unwrap();
        assert_eq!(*s.document(), before, "{}: undo", spec.name);
        assert!(!s.can_undo(), "{}: one step", spec.name);
        s.redo().unwrap();
        assert_eq!(*s.document(), after, "{}: redo", spec.name);
    }
}

#[test]
fn queries_change_nothing() {
    let base = example();
    for spec in base.specs().into_iter().filter(|s| s.scope == Scope::Query) {
        for mut s in [example(), playing()] {
            let (doc, cur, rev, view) = (
                s.document().clone(),
                s.current_world().clone(),
                s.revision(),
                s.view_revision(),
            );
            s.run(&spec.name, &spec.example).unwrap();
            assert_eq!(
                (
                    s.document(),
                    s.current_world(),
                    s.revision(),
                    s.view_revision()
                ),
                (&doc, &cur, rev, view),
                "{}",
                spec.name
            );
        }
    }
}
