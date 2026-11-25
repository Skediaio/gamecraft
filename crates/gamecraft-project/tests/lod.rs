//! Levels of detail as the host reads them (`gamecraft_project::lod`): the `_LOD<n>` names
//! and the `MSFT_lod` extension, the triangles of every level, what the file says of the
//! distances, the rule from the size, and every broken set giving a warning, never a panic.
//! The models are built in the test (`common/gltf.rs`): meshes of unit cubes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

mod common;

use common::gltf::{Mesh, Model, Node};
use gamecraft_project::inspect;
use gamecraft_project::lod::{
    DRAW_COVERAGE, LodDistances, LodOverride, LodSource, SWITCH_COVERAGE, lod_name, rule,
};
use serde_json::json;

const CRATE: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/crate.glb");

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

#[test]
fn names_ending_in_a_level_number_give_the_levels_and_their_triangles() {
    let s = Model::named().inspect().unwrap();
    assert_eq!(s.triangles, 36, "the finest level");
    assert_eq!(s.lod_triangles, vec![36, 24, 12]);
    assert_eq!(s.lod_source, Some(LodSource::Names));
    assert!(s.lod.is_none() && s.lod_warnings.is_empty(), "{s:?}");
    // The bounds hold every level.
    assert_eq!(s.bounds.min, [0.0, 0.0, 0.0]);
    assert_eq!(s.bounds.max, [3.0, 1.0, 1.0]);
    assert!(close(s.size_m(), 11f64.sqrt()));
    // Nothing of it is written into a model without levels.
    let c = inspect(CRATE, &|_: &str| None).unwrap();
    assert_eq!(c.triangles, 12);
    assert_eq!(c.lod_triangles, vec![12]);
    assert_eq!((c.lod_source, c.lod.as_ref()), (None, None));
    assert!(c.lod_warnings.is_empty());
}

#[test]
fn msft_lod_is_read_the_same_way() {
    let s = Model::msft().inspect().unwrap();
    assert_eq!(s.lod_triangles, vec![36, 24, 12]);
    assert_eq!(s.lod_source, Some(LodSource::MsftLod));
    assert!(s.lod_warnings.is_empty(), "{:?}", s.lod_warnings);
    // The coverage gives the distances: the first level ends where the model spans the whole
    // view height, the second at a quarter of it, and nothing is drawn below 2 percent.
    let d = s.lod.as_ref().unwrap();
    let half_tan = 0.5773502691896257;
    let size = 11f64.sqrt();
    assert!(close(
        d.switch_m.as_deref().unwrap()[0],
        size / (2.0 * half_tan * 1.0)
    ));
    assert!(close(
        d.switch_m.as_deref().unwrap()[1],
        size / (2.0 * half_tan * 0.25)
    ));
    assert!(close(d.draw_m.unwrap(), size / (2.0 * half_tan * 0.02)));
    // A client that shows every level in the scene (which draws them all on top of each
    // other) reads the same: the coarser nodes stand in for the finest one.
    let mut in_scene = Model::msft();
    in_scene.roots = vec![0, 1, 2];
    let s = in_scene.inspect().unwrap();
    assert_eq!(s.lod_triangles, vec![36, 24, 12], "{s:?}");
    assert_eq!(s.lod_source, Some(LodSource::MsftLod));
}

#[test]
fn a_node_may_hold_levels_of_its_own_and_its_transform_reaches_them() {
    // A parent 10 m up the hill with a roof of two levels and a wall of two: the model has
    // two levels, the wall of every level beside the roof of that level.
    let model = Model::new(
        &[Mesh(3), Mesh(1), Mesh(2), Mesh(1)],
        vec![
            Node::new("Building", None)
                .at(10.0, 2.0, -5.0)
                .children(&[1, 3]),
            Node::new("Roof_LOD0", Some(0)),
            Node::new("Roof_LOD1", Some(1)),
            Node::new("Wall_LOD0", Some(2)),
            Node::new("Wall_LOD1", Some(3)),
        ],
        &[0, 2, 4],
    );
    let s = model.inspect().unwrap();
    assert_eq!(s.lod_triangles, vec![36 + 24, 12 + 12], "{s:?}");
    assert_eq!(s.lod_source, Some(LodSource::Names));
    assert!(s.lod_warnings.is_empty(), "{:?}", s.lod_warnings);
    assert_eq!(s.bounds.min, [10.0, 2.0, -5.0]);
    assert_eq!(s.bounds.max, [13.0, 3.0, -4.0]);
}

#[test]
fn geometry_of_no_group_is_drawn_at_every_level() {
    let model = Model::new(
        &[Mesh(3), Mesh(1), Mesh(1)],
        vec![
            Node::new("Slab", Some(1)),
            Node::new("Thing_LOD0", Some(0)),
            Node::new("Thing_LOD1", Some(2)),
        ],
        &[0, 1, 2],
    );
    let s = model.inspect().unwrap();
    assert_eq!(s.lod_triangles, vec![36 + 12, 12 + 12], "{s:?}");
    assert_eq!(s.triangles, 48);
    // A node named with a level but alone is ordinary geometry.
    let alone = Model::new(
        &[Mesh(1), Mesh(1)],
        vec![
            Node::new("Box_LOD1", Some(0)),
            Node::new("Box_LOD1", Some(1)),
        ],
        &[0, 1],
    );
    let s = alone.inspect().unwrap();
    assert_eq!((s.triangles, s.lod_triangles.clone()), (24, vec![24]));
    assert_eq!(s.lod_source, None);
}

#[test]
fn the_file_says_its_distances_in_extras_or_by_coverage() {
    let mut model = Model::named();
    model.asset_extras =
        json!({"gamecraft": {"lod_distances_m": [40, 150.5], "draw_distance_m": 800}});
    let s = model.inspect().unwrap();
    let d = s.lod.as_ref().unwrap();
    assert_eq!(
        d,
        &LodOverride {
            switch_m: Some(vec![40.0, 150.5]),
            draw_m: Some(800.0),
        }
    );
    assert!(s.lod_warnings.is_empty(), "{:?}", s.lod_warnings);
    // Only how far it is drawn: the changes of level follow from its size.
    let mut model = Model::named();
    model.asset_extras = json!({"gamecraft": {"draw_distance_m": 120}});
    let s = model.inspect().unwrap();
    assert_eq!(s.lod.as_ref().unwrap().draw_m, Some(120.0));
    assert!(s.lod.as_ref().unwrap().switch_m.is_none());
    // The rule: three levels, two changes and a draw distance, from the size.
    let s = Model::named().inspect().unwrap();
    let (d, notes) = LodDistances::resolve(s.lod_triangles.len(), s.size_m(), s.lod.as_ref(), None);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(d, rule(s.size_m(), 3));
    let size = s.size_m();
    assert!(close(
        d.switch_m[0],
        size / (2.0 * 0.5773502691896257 * SWITCH_COVERAGE[0])
    ));
    assert!(close(
        d.draw_m,
        size / (2.0 * 0.5773502691896257 * DRAW_COVERAGE)
    ));
}

#[test]
fn a_broken_set_warns_and_is_read_all_the_same() {
    // A level with more triangles than the one before.
    let backwards = Model::new(
        &[Mesh(1), Mesh(2), Mesh(3)],
        vec![
            Node::new("Thing_LOD0", Some(0)),
            Node::new("Thing_LOD1", Some(1)),
            Node::new("Thing_LOD2", Some(2)),
        ],
        &[0, 1, 2],
    );
    let s = backwards.inspect().unwrap();
    assert_eq!(s.lod_triangles, vec![12, 24, 36]);
    assert!(
        s.lod_warnings
            .iter()
            .any(|w| { w.contains("level 1 has more triangles (24) than level 0 (12)") }),
        "{:?}",
        s.lod_warnings
    );
    // A missing level: drawn in the order of their numbers.
    let gap = Model::new(
        &[Mesh(3), Mesh(1)],
        vec![
            Node::new("Thing_LOD0", Some(0)),
            Node::new("Thing_LOD2", Some(1)),
        ],
        &[0, 1],
    );
    let s = gap.inspect().unwrap();
    assert_eq!(s.lod_triangles, vec![36, 12]);
    assert!(
        s.lod_warnings
            .iter()
            .any(|w| w.contains("has no level between _LOD0 and _LOD2")),
        "{:?}",
        s.lod_warnings
    );
    // No finest level.
    let no_top = Model::new(
        &[Mesh(3), Mesh(1)],
        vec![
            Node::new("Thing_LOD1", Some(0)),
            Node::new("Thing_LOD2", Some(1)),
        ],
        &[0, 1],
    );
    let s = no_top.inspect().unwrap();
    assert_eq!(s.lod_triangles, vec![36, 12]);
    assert!(
        s.lod_warnings.iter().any(|w| w.contains("has no _LOD0")),
        "{:?}",
        s.lod_warnings
    );
    // A level named twice under one parent: the finest is drawn alone, the rest left out.
    let twice = Model::new(
        &[Mesh(3), Mesh(1), Mesh(1)],
        vec![
            Node::new("Thing_LOD0", Some(0)),
            Node::new("Thing_LOD1", Some(1)),
            Node::new("Thing_LOD1", Some(2)),
        ],
        &[0, 1, 2],
    );
    let s = twice.inspect().unwrap();
    assert_eq!((s.triangles, s.lod_triangles.clone()), (36, vec![36]));
    assert_eq!(s.lod_source, None);
    assert!(
        s.lod_warnings.iter().any(|w| {
            w.contains("`Thing_LOD1` is named more than once") && w.contains("_LOD0 is drawn alone")
        }),
        "{:?}",
        s.lod_warnings
    );
    // A level of one name met anywhere in the file belongs to the object of that name: a
    // coarse level at the root and the finest one under a parent are one group, drawn where
    // the finest stands.
    let apart = Model::new(
        &[Mesh(3), Mesh(1)],
        vec![
            Node::new("Thing_LOD1", Some(1)).at(7.0, 0.0, 0.0),
            Node::new("Holder", None).children(&[2]),
            Node::new("Thing_LOD0", Some(0)),
        ],
        &[0, 1],
    );
    let s = apart.inspect().unwrap();
    assert_eq!(s.lod_triangles, vec![36, 12], "{s:?}");
    assert_eq!(s.lod_source, Some(LodSource::Names));
    assert!(s.lod_warnings.is_empty(), "{:?}", s.lod_warnings);
    assert_eq!(s.bounds.min, [0.0, 0.0, 0.0]);
    assert_eq!(
        s.bounds.max,
        [3.0, 1.0, 1.0],
        "the coarse level where the finest stands"
    );
    // Two objects of one name are twice the same levels: the finest is drawn alone.
    let two = Model::new(
        &[Mesh(3), Mesh(1)],
        vec![
            Node::new("A", None).children(&[1, 2]),
            Node::new("Thing_LOD0", Some(0)),
            Node::new("Thing_LOD1", Some(1)),
            Node::new("B", None).at(50.0, 0.0, 0.0).children(&[4, 5]),
            Node::new("Thing_LOD0", Some(0)),
            Node::new("Thing_LOD1", Some(1)),
        ],
        &[0, 3],
    );
    let s = two.inspect().unwrap();
    assert_eq!(
        s.lod_triangles,
        vec![72],
        "both at their finest level: {s:?}"
    );
    assert_eq!(s.lod_source, None);
    assert_eq!(s.lod_warnings.len(), 1, "{:?}", s.lod_warnings);
}

#[test]
fn broken_extras_and_extensions_are_warnings_not_panics() {
    // An id past the nodes, the node itself, one twice, and a coverage that does not fit.
    let mut model = Model::msft();
    model.nodes.push(Node::new("Spare", Some(2)));
    model.nodes[0].extensions = json!({"MSFT_lod": {"ids": [0, 1, 1, 2, 99, "2", 2.5, null]}});
    model.nodes[0].extras = json!({"MSFT_screencoverage": [1.0, 0.5]});
    let s = model.inspect().unwrap();
    assert_eq!(s.lod_triangles, vec![36, 24, 12], "{s:?}");
    let text = s.lod_warnings.join("\n");
    for part in [
        "names the node itself",
        "names node 1 twice",
        "names \"2\" where the file has 4 nodes",
        "has 2 values where the model has 3 levels",
    ] {
        assert!(text.contains(part), "{part:?} not in\n{text}");
    }
    // Extras of the wrong shape.
    let mut model = Model::named();
    model.asset_extras = json!({"gamecraft": {
        "lod_distances_m": [40, 40],
        "draw_distance_m": [1, 2],
        "lod_bias": 3
    }});
    let s = model.inspect().unwrap();
    assert!(s.lod.is_none(), "{:?}", s.lod);
    let text = s.lod_warnings.join("\n");
    for part in [
        "`asset.extras.gamecraft.lod_distances_m` (40, 40) is not a list",
        "`asset.extras.gamecraft.draw_distance_m` (1, 2) is not one distance",
        "asset.extras.gamecraft.lod_bias` is not known",
    ] {
        assert!(text.contains(part), "{part:?} not in\n{text}");
    }
    let mut model = Model::named();
    model.asset_extras = json!({"gamecraft": "far"});
    let s = model.inspect().unwrap();
    assert!(
        s.lod_warnings[0].contains("is not an object"),
        "{:?}",
        s.lod_warnings
    );
    // An extension without ids, one naming one node only, and one whose finest node is not
    // in the scene.
    let model = Model::new(
        &[Mesh(1), Mesh(1), Mesh(1)],
        vec![
            Node {
                extensions: json!({"MSFT_lod": {"ids": "no"}}),
                ..Node::new("A", Some(0))
            },
            Node::new("B", Some(1)).msft_lod(&[2], &[1.0, 0.5]),
            Node::new("C", Some(2)),
            Node::new("D", Some(0)).msft_lod(&[2], &[1.0, 0.5]),
        ],
        &[0, 1, 2],
    );
    let s = model.inspect().unwrap();
    // B's coarse level stands in for B: 12 + 12 triangles at each level.
    assert_eq!(s.lod_triangles, vec![24, 24], "{s:?}");
    let text = s.lod_warnings.join("\n");
    for part in ["has no list of ids", "is not in the scene"] {
        assert!(text.contains(part), "{part:?} not in\n{text}");
    }
    // A coarser level that holds the finest one below it would never end: the levels are
    // left as they stand in the file (12 + 24 triangles, one level).
    let looped = Model::new(
        &[Mesh(2), Mesh(1)],
        vec![
            Node::new("Top_LOD1", Some(1)).children(&[1]),
            Node::new("Top_LOD0", Some(0)),
        ],
        &[0],
    );
    let s = looped.inspect().unwrap();
    assert!(
        s.lod_warnings.iter().any(|w| w.contains("never end")),
        "{:?}",
        s.lod_warnings
    );
    assert_eq!((s.lod_triangles.clone(), s.lod_source), (vec![36], None));
    // A coarser level nested below the finest one is a level like any other.
    let nested = Model::new(
        &[Mesh(2), Mesh(1)],
        vec![
            Node::new("Top_LOD0", Some(0)).children(&[1]),
            Node::new("Top_LOD1", Some(1)),
        ],
        &[0],
    );
    let s = nested.inspect().unwrap();
    assert_eq!(s.lod_triangles, vec![24, 12], "{s:?}");
    assert!(s.lod_warnings.is_empty(), "{:?}", s.lod_warnings);
}

#[test]
fn the_finest_level_of_a_named_set_is_the_one_in_the_scene() {
    // _LOD0 left out of the scene (progressive loading): the levels show where _LOD1 stands.
    let model = Model::new(
        &[Mesh(3), Mesh(1)],
        vec![
            Node::new("Thing_LOD0", Some(0)),
            Node::new("Thing_LOD1", Some(1)).at(7.0, 0.0, 0.0),
        ],
        &[1],
    );
    let s = model.inspect().unwrap();
    assert_eq!(s.lod_triangles, vec![36, 12]);
    assert_eq!(s.bounds.min, [7.0, 0.0, 0.0]);
    assert_eq!(
        s.bounds.max,
        [10.0, 1.0, 1.0],
        "every level where the anchor stands"
    );
    assert_eq!(lod_name("Thing_LOD1"), Some(("Thing", 1)));
}

#[test]
fn every_truncation_and_mutation_is_an_error_never_a_panic() {
    let bytes = Model::msft().glb();
    for cut in 0..bytes.len() {
        let _ = inspect(&bytes[..cut], &|_: &str| None);
    }
    for at in (0..bytes.len()).step_by(3) {
        let mut bad = bytes.clone();
        bad[at] ^= 0xA5;
        let _ = inspect(&bad, &|_: &str| None);
    }
    let bytes = Model::named().glb();
    for at in (0..bytes.len()).step_by(5) {
        let mut bad = bytes.clone();
        bad[at] ^= 0x5A;
        let _ = inspect(&bad, &|_: &str| None);
    }
}

#[test]
fn the_crate_still_measures_as_before() {
    // The bounds the simulation hashes: the same walk, the same products.
    let s = inspect(CRATE, &|_: &str| None).unwrap();
    assert_eq!(s.triangles, 12);
    assert_eq!(s.meshes, 1);
    assert_eq!(
        (s.lod_triangles.clone(), s.lod_source, s.lod.is_none()),
        (vec![12], None, true)
    );
    assert!(s.bounds.is_valid());
    assert!((-0.45..=-0.449).contains(&s.bounds.min[0]) && s.bounds.max[1] > 0.899);
    // 0.9 m on a side: drawn to about 364 m, its first change at about 5.4 m.
    let d = rule(s.size_m(), 3);
    assert!(
        (364.0..366.0).contains(&d.draw_m),
        "{} {}",
        s.size_m(),
        d.draw_m
    );
    assert!((5.3..5.5).contains(&d.switch_m[0]), "{}", d.switch_m[0]);
}
