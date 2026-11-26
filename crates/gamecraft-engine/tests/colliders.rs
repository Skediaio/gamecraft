//! Collision from a model's own geometry through the commands (`collider: "mesh"`: the parts
//! come from the host) and shapes that collide but are not drawn (`visible: false`): the file
//! writes nothing for a visible shape, the simulation and the rays keep a hidden one, and the
//! command line of a model says which collider it has.

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{refuse, snapshot};
use gamecraft_engine::Session;
use gamecraft_world::{ModelBounds, ModelBox, Terrain, TerrainSource, World};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const HOUSE: &str = "models/house.glb";

/// A session of a flat terrain at height 0, 256 m square, 2 m cells, without entities.
fn flat() -> Session {
    let mut w = World::new("Flat", 1);
    w.terrain = Some(Terrain {
        size_m: 256.0,
        cell_m: 2.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    Session::new(w)
}

fn facade_parts() -> Vec<ModelBox> {
    vec![
        ModelBox {
            center: [-2.5, 2.0, 0.0],
            half: [1.5, 2.0, 0.15],
            yaw_deg: 0.0,
        },
        ModelBox {
            center: [2.5, 2.0, 0.0],
            half: [1.5, 2.0, 0.15],
            yaw_deg: 0.0,
        },
        ModelBox {
            center: [0.0, 3.25, 0.0],
            half: [1.0, 0.75, 0.15],
            yaw_deg: 0.0,
        },
    ]
}

fn bounds() -> ModelBounds {
    ModelBounds {
        min: [-4.0, 0.0, -0.15],
        max: [4.0, 4.0, 0.15],
    }
}

/// The session with a solid house model of the `mesh` collider and its bounds; the parts are
/// handed over later, as a host does when it has measured the asset.
fn house() -> Session {
    let mut s = flat();
    s.set_model_bounds(BTreeMap::from([(HOUSE.to_string(), bounds())]));
    s.run(
        "entity.add",
        &json!({"name": "House", "position": [0, 0, 0],
                 "components": [{"type": "model", "asset": "models/house.glb", "solid": true,
                                 "collider": "mesh"}]}),
    )
    .unwrap();
    s
}

fn z_of(s: &mut Session, name: &str) -> f64 {
    let e = s.run("entity.get", &json!({"id": name})).unwrap();
    e["position"][2].as_f64().unwrap()
}

fn walker(s: &mut Session, name: &str, at: [f64; 3]) -> Value {
    s.run(
        "entity.add",
        &json!({"name": name, "position": at, "components": [{"type": "character"}]}),
    )
    .unwrap()
}

fn ray(s: &mut Session, origin: [f64; 3], direction: [f64; 3]) -> Value {
    s.run(
        "query.raycast",
        &json!({"origin": origin, "direction": direction, "max_m": 12}),
    )
    .unwrap()
}

#[test]
fn a_mesh_collider_walks_through_the_doorway_and_a_ray_passes_the_opening() {
    let mut s = house();
    // Before the host measured the asset there is nothing to collide with, as for bounds.
    walker(&mut s, "Through", [0.0, 0.0, 6.0]);
    walker(&mut s, "Wall", [2.5, 0.0, 6.0]);
    s.run("sim.start", &json!({})).unwrap();
    for name in ["Through", "Wall"] {
        s.run(
            "sim.input",
            &json!({"entity": name, "forward": 1, "yaw_deg": 0}),
        )
        .unwrap();
    }
    s.run("sim.step", &json!({"seconds": 1})).unwrap();
    assert!(
        (z_of(&mut s, "Wall") - (6.0 - 1.6)).abs() < 1e-9,
        "through the wall: no parts yet"
    );
    // The host hands the parts over: from the next tick the walls stop the walker.
    s.set_model_parts(BTreeMap::from([(HOUSE.to_string(), facade_parts())]));
    s.run("sim.input", &json!({"entity": "Wall", "yaw_deg": 180}))
        .unwrap();
    s.run("sim.step", &json!({"ticks": 5})).unwrap();
    s.run("sim.input", &json!({"entity": "Wall", "yaw_deg": 0}))
        .unwrap();
    s.run("sim.step", &json!({"seconds": 4})).unwrap();
    let z = z_of(&mut s, "Wall");
    assert!(
        (z - 0.45).abs() < 1e-9 || (z > 0.45 && z < 0.7),
        "stopped at the wall: z {z}"
    );
    // The other walker walks through the doorway.
    let z = z_of(&mut s, "Through");
    assert!(z < -1.0, "through the doorway: z {z}");
    // Rays: through the opening, and against the wall.
    let hit = ray(&mut s, [0.0, 1.0, 6.0], [0.0, 0.0, -1.0]);
    assert_eq!(hit["hit"], false, "{hit}");
    let hit = ray(&mut s, [2.5, 1.0, 6.0], [0.0, 0.0, -1.0]);
    assert_eq!(hit["hit"], true, "{hit}");
    assert!(
        (hit["distance_m"].as_f64().unwrap() - 5.85).abs() < 1e-9,
        "{hit}"
    );
    assert_eq!(hit["name"], "House");
    let hit = ray(&mut s, [0.0, 3.0, 6.0], [0.0, 0.0, -1.0]);
    assert_eq!(hit["hit"], true, "the lintel: {hit}");
    // While editing too (the document's index).
    let mut edit = house();
    edit.set_model_bounds(BTreeMap::from([(HOUSE.to_string(), bounds())]));
    edit.set_model_parts(BTreeMap::from([(HOUSE.to_string(), facade_parts())]));
    assert_eq!(
        ray(&mut edit, [0.0, 1.0, 6.0], [0.0, 0.0, -1.0])["hit"],
        false
    );
    assert_eq!(
        ray(&mut edit, [3.0, 1.0, 6.0], [0.0, 0.0, -1.0])["hit"],
        true
    );
}

#[test]
fn the_collider_is_the_component_s_and_the_file_and_the_list_say_it() {
    let mut s = flat();
    s.run(
        "entity.add",
        &json!({"name": "House", "position": [0, 0, 0],
                 "components": [{"type": "model", "asset": "models/house.glb", "solid": true,
                                 "collider": "mesh"}]}),
    )
    .unwrap();
    let e = s.run("entity.get", &json!({"id": "House"})).unwrap();
    assert_eq!(e["components"][0]["collider"], "mesh");
    let text = s.document().to_json().unwrap();
    assert!(text.contains(r#""collider": "mesh""#), "{text}");
    // The default stays `box`, and it is not written.
    s.run(
        "entity.add",
        &json!({"name": "Crate", "position": [10, 0, 0],
                 "components": [{"type": "model", "asset": "models/crate.glb"}]}),
    )
    .unwrap();
    let e = s.run("entity.get", &json!({"id": "Crate"})).unwrap();
    assert_eq!(e["components"][0]["collider"], "box");
    let text = s.document().to_json().unwrap();
    assert_eq!(text.matches("\"collider\": \"mesh\"").count(), 1, "{text}");
    assert_eq!(text.matches("\"collider\": \"box\"").count(), 1, "{text}");
    // Only the three colliders are accepted, and the nearest is named.
    refuse(
        &mut s,
        "entity.add",
        json!({"name": "X", "position": [0, 0, 0],
               "components": [{"type": "model", "asset": "models/x.glb", "collider": "meshh"}]}),
        &["collider", "mesh"],
    );
    refuse(
        &mut s,
        "entity.add",
        json!({"name": "X", "position": [0, 0, 0],
               "components": [{"type": "model", "asset": "models/x.glb", "collider": 3}]}),
        &["collider"],
    );
    let list = s.run("component.list", &json!({})).unwrap();
    let model = list["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "model")
        .unwrap();
    let collider = model["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "collider")
        .unwrap();
    assert!(
        collider["doc"].as_str().unwrap().contains("mesh"),
        "{collider}"
    );
    // Parts are derived data: setting them changes no revision and is not undone.
    let rev = s.revision();
    s.set_model_parts(BTreeMap::from([(HOUSE.to_string(), facade_parts())]));
    assert_eq!(s.revision(), rev);
    assert_eq!(s.document().to_json().unwrap(), text);
    assert_eq!(
        s.document().model_parts(HOUSE).map(<[ModelBox]>::len),
        Some(3)
    );
    // A part that is not finite is not taken.
    s.set_model_parts(BTreeMap::from([(
        HOUSE.to_string(),
        vec![ModelBox {
            center: [f64::NAN, 0.0, 0.0],
            half: [1.0; 3],
            yaw_deg: 0.0,
        }],
    )]));
    assert_eq!(s.document().model_parts(HOUSE), None);
}

#[test]
fn a_hidden_shape_collides_rays_and_is_written_only_when_hidden() {
    let mut s = flat();
    s.run(
        "entity.add",
        &json!({"name": "Curb", "position": [0, 0, 0],
                 "components": [{"type": "shape", "kind": "box", "size": [6, 1, 0.5], "visible": false}]}),
    )
    .unwrap();
    s.run(
        "entity.add",
        &json!({"name": "Wall", "position": [0, 0, -4],
                 "components": [{"type": "shape", "kind": "box", "size": [6, 1, 0.5]}]}),
    )
    .unwrap();
    let text = s.document().to_json().unwrap();
    assert!(text.contains(r#""visible": false"#), "{text}");
    assert_eq!(
        text.matches("\"visible\"").count(),
        1,
        "a visible shape writes nothing"
    );
    let e = s.run("entity.get", &json!({"id": "Curb"})).unwrap();
    assert_eq!(e["components"][0]["visible"], false);
    // A shown shape has no `visible` in the file form: the default is left out.
    let e = s.run("entity.get", &json!({"id": "Wall"})).unwrap();
    assert!(e["components"][0].get("visible").is_none(), "{e}");
    // Shown and hidden again: the text follows.
    s.run("entity.set", &json!({"id": "Curb", "visible": true}))
        .unwrap();
    assert!(!s.document().to_json().unwrap().contains("\"visible\""));
    s.run("entity.set", &json!({"id": "Curb", "visible": false}))
        .unwrap();
    assert!(
        s.document()
            .to_json()
            .unwrap()
            .contains(r#""visible": false"#)
    );
    // The simulation and the rays keep it.
    let hit = ray(&mut s, [0.0, 0.5, 6.0], [0.0, 0.0, -1.0]);
    assert_eq!(hit["hit"], true, "{hit}");
    assert_eq!(hit["name"], "Curb");
    assert!(
        (hit["distance_m"].as_f64().unwrap() - 5.75).abs() < 1e-9,
        "{hit}"
    );
    walker(&mut s, "Walker", [0.0, 0.0, 6.0]);
    s.run("sim.start", &json!({})).unwrap();
    s.run(
        "sim.input",
        &json!({"entity": "Walker", "forward": 1, "yaw_deg": 0}),
    )
    .unwrap();
    s.run("sim.step", &json!({"seconds": 4})).unwrap();
    assert!(
        (z_of(&mut s, "Walker") - 0.55).abs() < 1e-9,
        "stopped by the hidden curb"
    );
    // Prefab children take it too, and a spawn keeps it.
    let mut p = flat();
    p.run("prefab.add", &json!({
        "id": "gate", "children": [
            {"name": "Post", "offset": [0, 0, 0], "components": [{"type": "shape", "kind": "box", "size": [0.2, 1.5, 0.2]}]},
            {"name": "Bar", "offset": [0, 1.2, 0], "components": [{"type": "shape", "kind": "box", "size": [2, 0.1, 0.1], "visible": false}]}
        ]}))
        .unwrap();
    p.run(
        "prefab.spawn",
        &json!({"prefab": "gate", "position": [20, 0, 20]}),
    )
    .unwrap();
    let e = p.run("entity.find", &json!({"name": "Bar"})).unwrap();
    let id = e["entities"][0]["id"].clone();
    let e = p.run("entity.get", &json!({"id": id})).unwrap();
    assert_eq!(e["components"][0]["visible"], false, "{e}");
    // Bad values name the parameter.
    refuse(
        &mut s,
        "entity.add",
        json!({"name": "X", "position": [0, 0, 0],
               "components": [{"type": "shape", "kind": "box", "visible": "no"}]}),
        &["components[0]", "shape", "visible"],
    );
    refuse(
        &mut s,
        "entity.set",
        json!({"id": "Walker", "visible": "no"}),
        &["`visible` must be boolean"],
    );
    refuse(
        &mut s,
        "entity.set",
        json!({"id": "Walker", "visible": true}),
        &["no shape"],
    );
    let _ = snapshot(&s);
}
