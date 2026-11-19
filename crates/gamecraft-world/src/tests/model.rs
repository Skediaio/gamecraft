//! The `model` component: its JSON form, its checks, the derived bounds and the collider box.

use crate::{
    Component, EntityDraft, Model, ModelBounds, ModelBox, ModelCollider, World, is_asset_path,
    is_model_path,
};
use std::collections::BTreeMap;

fn model(asset: &str) -> Model {
    Model {
        asset: asset.into(),
        scale: 1.0,
        solid: true,
        collider: ModelCollider::Box,
    }
}

fn world_with(m: Model, yaw: f64, scale: f64) -> World {
    let mut w = World::new("m", 1);
    w.add_entity(EntityDraft {
        name: "Crate".into(),
        position: [10.0, 2.0, -4.0],
        rotation_y_deg: yaw,
        scale,
        components: vec![Component::Model(m)],
        layer: None,
    })
    .unwrap();
    w
}

#[test]
fn a_model_reads_with_its_defaults_and_writes_back() {
    let c: Component =
        serde_json::from_str(r#"{"type": "model", "asset": "models/crate.glb"}"#).unwrap();
    assert_eq!(c.kind(), "model");
    let Component::Model(m) = &c else { panic!() };
    assert_eq!(m.scale, 1.0);
    assert!(!m.solid);
    assert_eq!(m.collider, ModelCollider::Box);
    let text = serde_json::to_string(&c).unwrap();
    assert_eq!(
        text,
        r#"{"type":"model","asset":"models/crate.glb","scale":1.0,"solid":false,"collider":"box"}"#
    );
    assert!(
        serde_json::from_str::<Component>(r#"{"type": "model", "asset": "a.glb", "colider": "x"}"#)
            .is_err()
    );
    assert!(Component::KINDS.contains(&"model"));
}

#[test]
fn asset_paths_are_plain_paths_inside_the_assets_folder() {
    for good in ["models/crate.glb", "crate.gltf", "a/b-c/d_e.1.glb"] {
        assert!(is_model_path(good), "{good}");
    }
    for bad in [
        "",
        "/models/crate.glb",
        "models//crate.glb",
        "../crate.glb",
        "models/./crate.glb",
        "models/crate.obj",
        "models/crate",
        "models\\crate.glb",
        "mödel.glb",
        "models/crate.glb/",
    ] {
        assert!(!is_model_path(bad), "{bad:?}");
    }
    assert!(is_asset_path("textures/wood.png"));
    assert!(!is_asset_path(&"a".repeat(201)));
}

#[test]
fn a_bad_model_names_the_field_and_what_is_allowed() {
    let bad = |change: fn(&mut Model)| {
        let mut w = world_with(model("crate.glb"), 0.0, 1.0);
        let id = w.entities().next().unwrap().id;
        let mut m = w.entity(id).unwrap().model().unwrap().clone();
        change(&mut m);
        w.entity_mut(id).unwrap().set_component(Component::Model(m));
        w.validate().unwrap_err().to_string()
    };
    let e = bad(|m| m.asset = "../escape.glb".into());
    assert!(
        e.contains("model.asset") && e.contains("models/crate.glb"),
        "{e}"
    );
    let e = bad(|m| m.scale = f64::NAN);
    assert!(e.contains("model.scale"), "{e}");
}

#[test]
fn bounds_are_derived_and_never_written() {
    let mut w = world_with(model("crate.glb"), 0.0, 1.0);
    let text = w.to_json().unwrap();
    let bounds = ModelBounds {
        min: [-0.5, 0.0, -0.5],
        max: [0.5, 1.0, 0.5],
    };
    w.set_model_bounds(BTreeMap::from([("crate.glb".to_string(), bounds)]));
    assert_eq!(w.model_bounds("crate.glb"), Some(&bounds));
    assert_eq!(w.to_json().unwrap(), text);
    // Invalid bounds are left out.
    w.set_model_bounds(BTreeMap::from([(
        "crate.glb".to_string(),
        ModelBounds {
            min: [0.0, f64::NAN, 0.0],
            max: [1.0; 3],
        },
    )]));
    assert_eq!(w.model_bounds("crate.glb"), None);
    // A clone carries them (a running world is a clone of the document).
    w.set_model_bounds(BTreeMap::from([("crate.glb".to_string(), bounds)]));
    assert_eq!(w.clone().model_bounds("crate.glb"), Some(&bounds));
}

#[test]
fn the_collider_box_is_the_bounds_scaled_and_turned_about_the_base() {
    let bounds = ModelBounds {
        min: [0.0, 0.0, -1.0],
        max: [2.0, 1.0, 0.0],
    };
    let mut m = model("crate.glb");
    m.scale = 2.0;
    let w = world_with(m, 90.0, 0.5);
    let e = w.entities().next().unwrap();
    let b = ModelBox::of(e, &bounds).unwrap();
    // Local centre (1, 0.5, -0.5) at scale 1; turned 90 degrees counter-clockwise from above,
    // +X goes to -Z and -Z goes to -X.
    let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
    assert!(close(b.center[0], 10.0 - 0.5), "{b:?}");
    assert!(close(b.center[1], 2.5), "{b:?}");
    assert!(close(b.center[2], -4.0 - 1.0), "{b:?}");
    assert_eq!(b.half, [1.0, 0.5, 0.5]);
    assert_eq!(b.yaw_deg, 90.0);
    let mut none = model("crate.glb");
    none.collider = ModelCollider::None;
    let w = world_with(none, 0.0, 1.0);
    assert_eq!(ModelBox::of(w.entities().next().unwrap(), &bounds), None);
}
