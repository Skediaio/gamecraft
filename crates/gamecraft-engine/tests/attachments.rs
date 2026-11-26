//! Attachments through the commands: a mount tree is added, listed with where each attachment
//! lands (a two-deep chain to a millimetre and a hundredth of a degree, through a socket rolled
//! 90 degrees), swapped at depth 2 while depth 1 stays, removed leaf first or with `cascade`;
//! an unknown socket is an error listing the sockets, an occupied socket is refused; the file
//! says the tree and nothing for a world without one; while playing the game master mounts and
//! the tree is placed on the running world; hostile parameters change nothing.

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
use gamecraft_world::{Component, Model, Socket, World};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const ROLL_Z: [f64; 4] = [
    0.0,
    0.0,
    std::f64::consts::FRAC_1_SQRT_2,
    std::f64::consts::FRAC_1_SQRT_2,
];
const IDENTITY: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

fn socket(position: [f64; 3], rotation: [f64; 4]) -> Socket {
    Socket { position, rotation }
}

/// The sockets of the three test models, as a host measures them.
fn sockets() -> BTreeMap<String, BTreeMap<String, Socket>> {
    [
        (
            "models/base.glb".to_string(),
            [
                ("rail_top".to_string(), socket([0.0, 1.0, 0.0], ROLL_Z)),
                ("rail_left".to_string(), socket([-0.3, 0.6, 0.1], IDENTITY)),
            ]
            .into_iter()
            .collect(),
        ),
        (
            "models/adapter.glb".to_string(),
            [("top".to_string(), socket([0.0, 0.05, 0.2], IDENTITY))]
                .into_iter()
                .collect(),
        ),
        ("models/optic.glb".to_string(), BTreeMap::new()),
    ]
    .into_iter()
    .collect()
}

/// A session with a flat terrain, a base entity turned 30 degrees with its model twice as
/// large, and the sockets handed over.
fn with_base() -> Session {
    let mut w = World::new("Flat", 1);
    w.terrain = Some(gamecraft_world::Terrain {
        size_m: 64.0,
        cell_m: 2.0,
        source: gamecraft_world::TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    });
    let mut s = Session::new(w);
    s.run(
        "entity.add",
        &json!({"name": "Base", "position": [10, 0, 20], "rotation_y_deg": 30,
                 "components": [{"type": "model", "asset": "models/base.glb", "scale": 2}]}),
    )
    .unwrap();
    s.set_model_sockets(sockets());
    s
}

/// Mounts the adapter on `rail_top` and the optic on the adapter's `top`.
fn with_chain() -> Session {
    let mut s = with_base();
    s.run(
        "attachment.add",
        &json!({"entity": "Base", "socket": "rail_top", "asset": "models/adapter.glb", "scale": 2}),
    )
    .unwrap();
    s.run(
        "attachment.add",
        &json!({"entity": "Base", "path": ["rail_top"], "socket": "top", "asset": "models/optic.glb", "scale": 0.5}),
    )
    .unwrap();
    s
}

fn tree_of(answer: &Value, entity: &str) -> Value {
    answer["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == entity)
        .unwrap_or_else(|| panic!("no tree for {entity}: {answer}"))
        .clone()
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn a_chain_is_mounted_listed_swapped_and_removed() {
    let mut s = with_chain();
    // The list: the tree with the path and the place of every attachment.
    let list = s.run("attachment.list", &json!({})).unwrap();
    let tree = tree_of(&list, "Base");
    assert_eq!(tree["asset"], "models/base.glb");
    assert_eq!(tree["sockets"], json!(["rail_left", "rail_top"]));
    let adapter = &tree["attachments"][0];
    assert_eq!(adapter["path"], json!(["rail_top"]));
    assert_eq!(adapter["asset"], "models/adapter.glb");
    assert_eq!(adapter["sockets"], json!(["top"]));
    assert_eq!(
        adapter["attachments"][0]["path"],
        json!(["rail_top", "top"])
    );
    assert_eq!(adapter["attachments"][0]["sockets"], json!([]));
    // Depth 1: on top of the base at its scale, rolled 90 degrees, turned with it.
    let p = adapter["position"].as_array().unwrap();
    assert!(
        close(p[0].as_f64().unwrap(), 10.0, 1e-9)
            && close(p[1].as_f64().unwrap(), 2.0, 1e-9)
            && close(p[2].as_f64().unwrap(), 20.0, 1e-9),
        "{adapter}"
    );
    for ws in adapter["world_scale"].as_array().unwrap() {
        assert!(close(ws.as_f64().unwrap(), 4.0, 1e-9), "{adapter}");
    }
    let r = adapter["rotation"].as_array().unwrap();
    // ry(30) * rz(90): its parts from the half angles.
    let (s30, c30) = gamecraft_math::Portable::psin_cos(gamecraft_math::deg_to_rad(15.0));
    let want = [
        s30 * std::f64::consts::FRAC_1_SQRT_2,
        s30 * std::f64::consts::FRAC_1_SQRT_2,
        c30 * std::f64::consts::FRAC_1_SQRT_2,
        c30 * std::f64::consts::FRAC_1_SQRT_2,
    ];
    for k in 0..4 {
        // A hundredth of a degree is about 1e-4 in a quaternion's part.
        assert!(close(r[k].as_f64().unwrap(), want[k], 1e-4), "{adapter}");
    }
    // Depth 2: the optic on the adapter's top: to a millimetre.
    let optic = &adapter["attachments"][0];
    let p = optic["position"].as_array().unwrap();
    assert!(close(p[0].as_f64().unwrap(), 10.2268, 1e-3), "{optic}");
    assert!(close(p[1].as_f64().unwrap(), 2.0, 1e-3), "{optic}");
    assert!(close(p[2].as_f64().unwrap(), 20.7928, 1e-3), "{optic}");
    for ws in optic["world_scale"].as_array().unwrap() {
        assert!(close(ws.as_f64().unwrap(), 2.0, 1e-9), "{optic}");
    }
    // One entity's tree only.
    let one = s
        .run("attachment.list", &json!({"entity": "Base"}))
        .unwrap();
    assert_eq!(one["entities"].as_array().unwrap().len(), 1);
    // The file says the tree, readably.
    let e = s.run("entity.get", &json!({"id": "Base"})).unwrap();
    assert_eq!(
        e["components"][0]["attachments"][0]["attachments"][0]["asset"],
        "models/optic.glb"
    );
    let text = s.document().to_json().unwrap();
    assert!(text.contains("\"attachments\""), "{text}");

    // Swapping at depth 2 keeps depth 1.
    let swapped = s
        .run(
            "attachment.set",
            &json!({"entity": "Base", "path": ["rail_top", "top"], "asset": "models/red_dot.glb"}),
        )
        .unwrap();
    assert_eq!(swapped["asset"], "models/red_dot.glb");
    assert_eq!(swapped["path"], json!(["rail_top", "top"]));
    let tree = tree_of(&s.run("attachment.list", &json!({})).unwrap(), "Base");
    assert_eq!(
        tree["attachments"][0]["asset"], "models/adapter.glb",
        "depth 1 stays"
    );
    assert_eq!(
        tree["attachments"][0]["attachments"][0]["asset"],
        "models/red_dot.glb"
    );
    assert_eq!(
        tree["attachments"][0]["attachments"][0]["scale"], 0.5,
        "the scale stays"
    );

    // Removing one that holds attachments is refused; the leaf goes; then the adapter.
    let before = snapshot(&s);
    let e = s
        .run(
            "attachment.remove",
            &json!({"entity": "Base", "path": ["rail_top"]}),
        )
        .unwrap_err()
        .to_string();
    assert!(
        e.starts_with("attachment.remove: ")
            && e.contains("holds 1 attachments")
            && e.contains("cascade"),
        "{e}"
    );
    assert_eq!(snapshot(&s), before);
    let removed = s
        .run(
            "attachment.remove",
            &json!({"entity": "Base", "path": ["rail_top", "top"]}),
        )
        .unwrap();
    assert_eq!(removed["removed"], 1);
    let tree = tree_of(&s.run("attachment.list", &json!({})).unwrap(), "Base");
    assert_eq!(tree["attachments"][0]["attachments"], json!([]));
    // Mount again and take the whole branch along.
    s.run(
        "attachment.add",
        &json!({"entity": "Base", "path": ["rail_top"], "socket": "top", "asset": "models/optic.glb"}),
    )
    .unwrap();
    let removed = s
        .run(
            "attachment.remove",
            &json!({"entity": "Base", "path": ["rail_top"], "cascade": true}),
        )
        .unwrap();
    assert_eq!(removed["removed"], 2);
    let list = s.run("attachment.list", &json!({})).unwrap();
    assert_eq!(list["entities"], json!([]), "nothing mounted: not listed");
    // Each command is one undo step, back to a world without attachments (and without the text).
    s.undo().unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    let text = s.document().to_json().unwrap();
    assert!(!text.contains("attachments"), "{text}");
}

#[test]
fn the_sockets_are_checked_and_listed() {
    let mut s = with_base();
    // An unknown socket: the sockets the model has.
    let e = s
        .run(
            "attachment.add",
            &json!({"entity": "Base", "socket": "rail_bottom", "asset": "models/adapter.glb"}),
        )
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("`rail_bottom`") && e.contains("rail_left, rail_top"),
        "{e}"
    );
    // A model without sockets says none; an asset whose sockets are unknown says so.
    s.run(
        "attachment.add",
        &json!({"entity": "Base", "socket": "rail_top", "asset": "models/adapter.glb"}),
    )
    .unwrap();
    // A model whose sockets were not measured can be mounted (its socket is the adapter's);
    // nothing can be mounted on it.
    let e = s
        .run(
            "attachment.add",
            &json!({"entity": "Base", "path": ["rail_top"], "socket": "top", "asset": "models/unmeasured.glb"}),
        )
        .unwrap();
    assert!(
        !e["position"].is_null(),
        "placed on the adapter's socket: {e}"
    );
    let e = s
        .run(
            "attachment.add",
            &json!({"entity": "Base", "path": ["rail_top", "top"], "socket": "anything", "asset": "models/x.glb"}),
        )
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("`models/unmeasured.glb`") && e.contains("not known"),
        "{e}"
    );
    s.run(
        "attachment.remove",
        &json!({"entity": "Base", "path": ["rail_top", "top"]}),
    )
    .unwrap();
    let e = s
        .run(
            "attachment.add",
            &json!({"entity": "Base", "path": ["rail_top"], "socket": "top", "asset": "models/unmeasured.glb"}),
        )
        .unwrap();
    assert!(!e["position"].is_null(), "{e}");
    // An occupied socket is refused, and so is the same name twice in a file.
    let before = snapshot(&s);
    let e = s
        .run(
            "attachment.add",
            &json!({"entity": "Base", "socket": "rail_top", "asset": "models/other.glb"}),
        )
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("`rail_top`") && e.contains("already holds") && e.contains("attachment.set"),
        "{e}"
    );
    assert_eq!(snapshot(&s), before);
    // Bad paths and parameters name them.
    for (command, params, needles) in [
        (
            "attachment.add",
            json!({"entity": "Base", "socket": "rail_left", "asset": "models/a.obj"}),
            &["`asset`"][..],
        ),
        (
            "attachment.add",
            json!({"entity": "Base", "socket": "with space", "asset": "models/a.glb"}),
            &["`socket` is not a socket name"][..],
        ),
        (
            "attachment.add",
            json!({"entity": "Base", "socket": "rail_left", "asset": "models/a.glb", "path": ["nope"]}),
            &["`nope`"][..],
        ),
        (
            "attachment.add",
            json!({"entity": "Nobody", "socket": "rail_left", "asset": "models/a.glb"}),
            &["Nobody"][..],
        ),
        (
            "attachment.add",
            json!({"socket": "rail_left", "asset": "models/a.glb"}),
            &["missing `entity`"][..],
        ),
        (
            "attachment.add",
            json!({"entity": "Base", "socket": "rail_left", "asset": "models/a.glb", "scale": 0}),
            &["scale"][..],
        ),
        (
            "attachment.set",
            json!({"entity": "Base", "path": [], "asset": "models/a.glb"}),
            &["`path` names the attachment"][..],
        ),
        (
            "attachment.set",
            json!({"entity": "Base", "path": ["rail_top", "nope"], "asset": "models/a.glb"}),
            &["`nope`"][..],
        ),
        (
            "attachment.remove",
            json!({"entity": "Base", "path": ["rail_left"]}),
            &["no attachment is mounted"][..],
        ),
        (
            "attachment.list",
            json!({"entity": "Nobody"}),
            &["Nobody"][..],
        ),
    ] {
        refuse(&mut s, command, params, needles);
    }
    // An entity without a model carries nothing.
    s.run(
        "entity.add",
        &json!({"name": "Rock", "position": [0, 0, 0]}),
    )
    .unwrap();
    let e = s
        .run("attachment.list", &json!({"entity": "Rock"}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("has no `model` component"), "{e}");
    let e = s
        .run(
            "attachment.add",
            &json!({"entity": "Rock", "socket": "rail_left", "asset": "models/a.glb"}),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("has no `model` component"), "{e}");
    // Sockets are derived data: handing them over changes no revision and is not undone.
    let rev = s.revision();
    let text = s.document().to_json().unwrap();
    s.set_model_sockets(sockets());
    assert_eq!((s.revision(), s.document().to_json().unwrap()), (rev, text));
    assert_eq!(
        s.document()
            .model_sockets("models/base.glb")
            .map(BTreeMap::len),
        Some(2)
    );
}

#[test]
fn while_playing_the_game_master_mounts_on_the_running_world() {
    let mut s = with_chain();
    s.run("sim.start", &json!({})).unwrap();
    let before = s.document().to_json().unwrap();
    let added = s
        .run(
            "attachment.add",
            &json!({"entity": "Base", "socket": "rail_left", "asset": "models/torch.glb"}),
        )
        .unwrap();
    assert_eq!(added["path"], json!(["rail_left"]));
    // rail_left is at [-0.3, 0.6, 0.1] in the base's frame, twice as large, turned 30 degrees.
    let (s30, c30) = gamecraft_math::Portable::psin_cos(gamecraft_math::deg_to_rad(30.0));
    let (x, z) = (-0.6, 0.2);
    let want = [10.0 + x * c30 + z * s30, 1.2, 20.0 - x * s30 + z * c30];
    let p = added["position"].as_array().unwrap();
    for k in 0..3 {
        assert!(close(p[k].as_f64().unwrap(), want[k], 1e-9), "{added}");
    }
    assert_eq!(
        s.document().to_json().unwrap(),
        before,
        "the document is untouched"
    );
    let tree = tree_of(&s.run("attachment.list", &json!({})).unwrap(), "Base");
    assert_eq!(tree["attachments"].as_array().unwrap().len(), 2);
    // Kept when the simulation stops with `keep`.
    s.run("sim.stop", &json!({"keep": true})).unwrap();
    assert!(s.document().to_json().unwrap().contains("torch.glb"));
}

#[test]
fn a_world_without_attachments_keeps_its_text_and_prefab_children_carry_them() {
    let s = with_base();
    let text = s.document().to_json().unwrap();
    assert!(!text.contains("attachments"), "{text}");
    // A prefab with a model that carries an attachment: every instance shows the tree.
    let mut world = s.document().clone();
    world
        .put_prefab(gamecraft_world::Prefab {
            id: "mounted".into(),
            name: "Mounted".into(),
            components: vec![Component::Model(Model {
                attachments: vec![gamecraft_world::attachments::Attachment::new(
                    "rail_top",
                    "models/adapter.glb",
                )],
                ..Model::of("models/base.glb")
            })],
            children: Vec::new(),
        })
        .unwrap();
    let mut s = Session::new(world);
    s.set_model_sockets(sockets());
    s.run(
        "prefab.spawn",
        &json!({"prefab": "mounted", "position": [0, 0, 0]}),
    )
    .unwrap();
    let list = s.run("attachment.list", &json!({})).unwrap();
    let trees = list["entities"].as_array().unwrap();
    assert_eq!(trees.len(), 1, "only the instance mounts something: {list}");
    let instance = trees.iter().find(|t| t["name"] == "Mounted").unwrap();
    assert_eq!(instance["attachments"][0]["asset"], "models/adapter.glb");
    assert_eq!(
        instance["attachments"][0]["position"],
        json!([0.0, 1.0, 0.0])
    );
}
