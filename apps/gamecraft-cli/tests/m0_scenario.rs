//! The M0 scenario through `gamecraft-cli serve`: a world is built, played, steered while it
//! runs and compared across two processes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
use common::*;
use serde_json::json;

#[test]
fn the_scenario_is_deterministic_across_processes() {
    let dir = Temp::new("m0");
    let mut first = Server::start("serve", &[], &dir.0);
    let a = m0_scenario(&mut first);
    // 120 ticks at the walk speed of 1.6 m/s.
    assert!(
        (a.walked_m - 3.2).abs() < 0.05,
        "walked {} m, expected 3.2",
        a.walked_m
    );
    assert!(a.final_position[2] > a.wall_front_z);
    assert!(
        (a.final_position[2] - (a.wall_front_z + 0.3)).abs() < 0.1,
        "Scout ended at {:?}",
        a.final_position
    );

    // Stopping throws the live wall away; the document never had it.
    first.ok("sim.stop", json!({"keep": false}));
    let inspected = first.ok("world.inspect", json!({"limit": 1000}));
    let entities = inspected["entities"].as_array().unwrap();
    assert!(entities.iter().all(|e| e["name"] != "Wall"), "{inspected}");
    let scout = entities.iter().find(|e| e["name"] == "Scout").unwrap();
    for axis in 0..3 {
        assert_eq!(
            scout["position"][axis].as_f64().unwrap(),
            a.start[axis],
            "Scout is back at its start"
        );
    }
    assert!(first.finish());

    let mut second = Server::start("serve", &[], &dir.0);
    let b = m0_scenario(&mut second);
    assert_eq!(a.hash, b.hash);
    assert_eq!(a.final_position, b.final_position);
    assert_eq!(a.hash.len(), 16);
    assert!(second.finish());
}
