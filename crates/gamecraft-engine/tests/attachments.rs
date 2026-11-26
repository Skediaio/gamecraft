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
use gamecraft_world::attachments::{Attachment, MAX_ATTACHMENT_DEPTH};
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
        // A link with one socket, for a deep chain; a relay with the adapter's socket.
        (
            "models/link.glb".to_string(),
            [("next".to_string(), socket([0.0, 0.5, 0.0], IDENTITY))]
                .into_iter()
                .collect(),
        ),
        (
            "models/relay.glb".to_string(),
            [("top".to_string(), socket([0.0, 0.1, 0.3], IDENTITY))]
                .into_iter()
                .collect(),
        ),
        // A socket turned about x, by a right angle and by half a right angle.
        (
            "models/tilted.glb".to_string(),
            [("side".to_string(), socket([0.0, 1.0, 0.0], ROLL_X))]
                .into_iter()
                .collect(),
        ),
        (
            "models/tilted45.glb".to_string(),
            [("side".to_string(), socket([0.0, 1.0, 0.0], TILT_X))]
                .into_iter()
                .collect(),
        ),
    ]
    .into_iter()
    .collect()
}

/// A turn of 90 degrees about +X.
const ROLL_X: [f64; 4] = [
    std::f64::consts::FRAC_1_SQRT_2,
    0.0,
    0.0,
    std::f64::consts::FRAC_1_SQRT_2,
];
/// A turn of 45 degrees about +X.
const TILT_X: [f64; 4] = [0.3826834323650897, 0.0, 0.0, 0.9238795325112867];

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

/// A chain eight levels deep, the most a mount may be: the base, a link on its `rail_top`,
/// and a link on each link's `next`.
fn with_deep_chain() -> Session {
    let mut s = with_base();
    let mut top = Attachment::new("next", "models/link.glb");
    for _ in 0..6 {
        top = Attachment {
            attachments: vec![top],
            ..Attachment::new("next", "models/link.glb")
        };
    }
    let first = Attachment {
        attachments: vec![top],
        ..Attachment::new("rail_top", "models/link.glb")
    };
    let mut w = s.document().clone();
    let id = w.entities().next().unwrap().id;
    w.entity_mut(id)
        .unwrap()
        .set_component(Component::Model(Model {
            attachments: vec![first],
            ..Model::of("models/base.glb")
        }));
    s.replace_document(w);
    s
}

/// `["rail_top", "next", ...]`, eight names: the deepest attachment.
fn eight() -> Vec<String> {
    let mut path = vec!["rail_top".to_string()];
    path.extend(vec!["next".to_string(); MAX_ATTACHMENT_DEPTH - 1]);
    path
}

fn parts(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

#[test]
fn the_deepest_attachment_is_reached_and_a_ninth_level_is_refused_in_true_words() {
    let mut s = with_deep_chain();
    let deep = eight();
    assert_eq!(deep.len(), 8);
    // The tree is eight deep.
    let listed = s
        .run("attachment.list", &json!({"entity": "Base"}))
        .unwrap();
    let mut at = &tree_of(&listed, "Base")["attachments"][0];
    let mut depth = 1;
    while !at["attachments"].as_array().unwrap().is_empty() {
        at = &at["attachments"][0];
        depth += 1;
    }
    assert_eq!(depth, 8, "{listed}");
    // Set at depth 8.
    let answer = s
        .run(
            "attachment.set",
            &json!({"entity": "Base", "path": deep, "asset": "models/optic.glb", "scale": 3}),
        )
        .unwrap();
    assert_eq!(answer["path"], json!(deep));
    assert!(close(answer["scale"].as_f64().unwrap(), 3.0, 1e-12));
    assert!(!answer["world_scale"].is_null(), "{answer}");
    // A ninth level does not exist: the sentence says so, and it is true.
    refuse(
        &mut s,
        "attachment.add",
        json!({"entity": "Base", "path": deep, "socket": "next", "asset": "models/link.glb"}),
        &["level 8", "at most 8 levels deep"],
    );
    let mut nine = deep.clone();
    nine.push("next".to_string());
    refuse(
        &mut s,
        "attachment.set",
        json!({"entity": "Base", "path": nine, "asset": "models/optic.glb"}),
        &["`path`", "9", "at most 8"],
    );
    refuse(
        &mut s,
        "attachment.remove",
        json!({"entity": "Base", "path": nine}),
        &["`path`", "9", "at most 8"],
    );
    // Removed at depth 8: the leaf.
    let answer = s
        .run(
            "attachment.remove",
            &json!({"entity": "Base", "path": deep}),
        )
        .unwrap();
    assert_eq!(answer["removed"], 1);
    refuse(
        &mut s,
        "attachment.set",
        json!({"entity": "Base", "path": deep, "asset": "models/optic.glb"}),
        &["no attachment is mounted"],
    );
}

#[test]
fn add_set_and_remove_redo_after_an_undo() {
    for (command, params) in [
        (
            "attachment.add",
            json!({"entity": "Base", "socket": "rail_left", "asset": "models/optic.glb"}),
        ),
        (
            "attachment.set",
            json!({"entity": "Base", "path": ["rail_top", "top"], "asset": "models/link.glb",
                   "scale": 2}),
        ),
        (
            "attachment.remove",
            json!({"entity": "Base", "path": ["rail_top", "top"]}),
        ),
    ] {
        let mut s = with_chain();
        let before = s.document().clone();
        let list_before = s.run("attachment.list", &json!({})).unwrap();
        s.run(command, &params).unwrap();
        let after = s.document().clone();
        let list_after = s.run("attachment.list", &json!({})).unwrap();
        assert_ne!(
            &after, &before,
            "{command}: the command changes the document"
        );
        assert_ne!(list_after, list_before, "{command}");
        s.undo().unwrap();
        assert_eq!(*s.document(), before, "{command}: undo");
        assert_eq!(s.run("attachment.list", &json!({})).unwrap(), list_before);
        s.redo().unwrap();
        assert_eq!(*s.document(), after, "{command}: redo");
        assert_eq!(s.run("attachment.list", &json!({})).unwrap(), list_after);
        assert!(!s.can_redo(), "{command}: one step");
    }
}

#[test]
fn a_swap_keeps_the_children_where_the_new_model_has_their_sockets_or_refuses() {
    let mut s = with_chain();
    // Onto a model without the socket the child is mounted on: refused, naming it and the
    // way out; the world is as before.
    refuse(
        &mut s,
        "attachment.set",
        json!({"entity": "Base", "path": ["rail_top"], "asset": "models/optic.glb"}),
        &["`top`", "models/optic.glb", "attachment.remove"],
    );
    // Onto a model whose sockets were not measured: refused, saying so.
    refuse(
        &mut s,
        "attachment.set",
        json!({"entity": "Base", "path": ["rail_top"], "asset": "models/unmeasured.glb"}),
        &["not known"],
    );
    // Onto a model that has the socket: the child stays, on the new model's socket.
    s.run(
        "attachment.set",
        &json!({"entity": "Base", "path": ["rail_top"], "asset": "models/relay.glb"}),
    )
    .unwrap();
    let tree = tree_of(
        &s.run("attachment.list", &json!({"entity": "Base"}))
            .unwrap(),
        "Base",
    );
    let adapter = &tree["attachments"][0];
    assert_eq!(adapter["asset"], "models/relay.glb");
    assert_eq!(adapter["attachments"][0]["asset"], "models/optic.glb");
    assert!(
        !adapter["attachments"][0]["position"].is_null(),
        "{adapter}"
    );
    // The optic landed on the relay's socket: 0.1 up and 0.3 forward in the relay's frame,
    // at the relay's scale 2 and the base's 2, in the rolled frame of `rail_top` (the roll
    // turns the relay's up to the base's left): 0.4 to the left and 1.2 forward of the base's
    // top, 2 up, turned 30 degrees with the base.
    let at = parts(&adapter["attachments"][0]["position"]);
    let (s30, c30) = gamecraft_math::Portable::psin_cos(gamecraft_math::deg_to_rad(30.0));
    let (left, forward) = (0.4, 1.2);
    let want = [
        10.0 - left * c30 + forward * s30,
        2.0,
        20.0 + left * s30 + forward * c30,
    ];
    for k in 0..3 {
        assert!(close(at[k], want[k], 1e-9), "{adapter}: {want:?}");
    }
}

#[test]
fn the_rotation_reported_is_the_turns_under_a_scale_per_axis() {
    let mut s = Session::new(World::new("Flat", 1));
    for (name, asset) in [
        ("Post", "models/tilted.glb"),
        ("Tilt", "models/tilted45.glb"),
    ] {
        s.run(
            "entity.add",
            &json!({"name": name, "position": [10, 0, 20], "scale": [1, 1, 3],
                    "components": [{"type": "model", "asset": asset}]}),
        )
        .unwrap();
    }
    s.set_model_sockets(sockets());
    let mount = |s: &mut Session, entity: &str| {
        s.run(
            "attachment.add",
            &json!({"entity": entity, "socket": "side", "asset": "models/optic.glb"}),
        )
        .unwrap()
    };
    let post = mount(&mut s, "Post");
    let tilt = mount(&mut s, "Tilt");
    for (answer, turn) in [(&post, ROLL_X), (&tilt, TILT_X)] {
        for (found, want) in parts(&answer["rotation"]).into_iter().zip(turn) {
            assert!(close(found, want, 1e-12), "{answer}");
        }
        assert_eq!(answer["position"], json!([10.0, 1.0, 20.0]), "{answer}");
    }
    // A right angle about x puts the entity's depth on the optic's height: a scale along its
    // own axes. Half a right angle shears the frame: none is reported, rather than a wrong
    // one.
    for (found, want) in parts(&post["world_scale"]).into_iter().zip([1.0, 3.0, 1.0]) {
        assert!(close(found, want, 1e-12), "{post}");
    }
    assert!(tilt["world_scale"].is_null(), "{tilt}");
    let tree = tree_of(&s.run("attachment.list", &json!({})).unwrap(), "Tilt");
    assert!(tree["attachments"][0]["world_scale"].is_null(), "{tree}");
}
