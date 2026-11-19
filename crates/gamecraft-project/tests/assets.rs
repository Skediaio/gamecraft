//! Assets of packs: the licence gate, the credits, and reading the demo pack's generated models
//! (triangle counts and bounds), with every truncation of them an error and never a panic.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{pack, packs};
use gamecraft_project::{
    AssetRecord, License, PackSource, Project, check_assets, credits, inspect, model::references,
};

const CRATE: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/crate.glb");
const FENCE: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/fence.glb");

const CC0: &str = r#"{"license": "CC0-1.0", "author": "The GameCraft project", "source_url": "https://example.org/crate", "title": "Wooden crate"}"#;
const CCBY: &str = r#"{"license": "CC-BY-4.0", "author": "Ada", "source_url": "https://example.org/fence", "title": "Fence", "changes": "Scaled to metres"}"#;

const MANIFEST: &str = r#"{"gamecraft_pack": 1, "id": "demo", "name": "Demo", "worlds": ["main"]}"#;
const WORLD: &str = r#"{"gamecraft_world": 1}"#;

/// A pack with the given asset files (path, size) and texts (records, .gltf documents).
fn with_assets(assets: &[(&str, u64)], texts: &[(&str, &str)]) -> PackSource {
    let mut files = vec![("pack.json", MANIFEST), ("worlds/main/world.json", WORLD)];
    files.extend_from_slice(texts);
    let mut p = pack("demo", &files);
    p.assets = assets
        .iter()
        .map(|(p, s)| ((*p).to_string(), *s))
        .chain(
            texts
                .iter()
                .filter(|(p, _)| p.starts_with("assets/"))
                .map(|(p, t)| ((*p).to_string(), t.len() as u64)),
        )
        .collect();
    p
}

fn load(p: PackSource) -> Result<Project, String> {
    Project::load("demo", &packs(vec![("demo", p)]), None)
        .map(|(p, _)| p)
        .map_err(|e| e.to_string())
}

#[test]
fn the_generated_models_read_with_their_triangles_and_bounds() {
    let none = |_: &str| None;
    let c = inspect(CRATE, &none).unwrap();
    assert_eq!(c.triangles, 12);
    assert_eq!((c.meshes, c.materials, c.images), (1, 1, 1));
    let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    assert!(close(c.bounds.min, [-0.45, 0.0, -0.45]), "{c:?}");
    assert!(close(c.bounds.max, [0.45, 0.9, 0.45]), "{c:?}");
    let f = inspect(FENCE, &none).unwrap();
    assert_eq!(f.triangles, 48);
    assert!(close(f.bounds.min, [-1.0, 0.0, -0.05]), "{f:?}");
    assert!(close(f.bounds.max, [1.0, 1.0, 0.05]), "{f:?}");
}

#[test]
fn a_broken_model_is_an_error_never_a_panic() {
    let none = |_: &str| None;
    for model in [CRATE, FENCE] {
        for cut in 0..model.len() {
            let _ = inspect(&model[..cut], &none);
        }
        // Flipped bytes everywhere in the header and JSON.
        for at in 0..model.len().min(2048) {
            let mut bad = model.to_vec();
            bad[at] ^= 0x5A;
            let _ = inspect(&bad, &none);
        }
    }
    assert!(inspect(&CRATE[..CRATE.len() / 2], &none).is_err());
    assert!(inspect(b"not a model", &none).is_err());
}

#[test]
fn a_gltf_reads_its_buffer_from_beside_it() {
    // The crate's JSON with its binary chunk as a file of its own.
    let json_len = u32::from_le_bytes(CRATE[12..16].try_into().unwrap()) as usize;
    let mut json: serde_json::Value = serde_json::from_slice(&CRATE[20..20 + json_len]).unwrap();
    let bin_start = 20 + json_len + 8;
    let bin = CRATE[bin_start..].to_vec();
    json["buffers"][0]["uri"] = "crate%20data.bin".into();
    let text = serde_json::to_vec(&json).unwrap();
    assert_eq!(
        references(&text).unwrap(),
        vec!["crate data.bin".to_string()]
    );
    let found = |p: &str| (p == "crate data.bin").then(|| bin.clone());
    assert_eq!(inspect(&text, &found).unwrap().triangles, 12);
    let e = inspect(&text, &|_: &str| None).unwrap_err();
    assert!(e.contains("crate data.bin") && e.contains("missing"), "{e}");
    json["buffers"][0]["uri"] = "../outside.bin".into();
    assert!(references(&serde_json::to_vec(&json).unwrap()).is_err());
}

#[test]
fn the_licence_gate() {
    let size = CRATE.len() as u64;
    // A model with its record loads, with the record.
    let p = load(with_assets(
        &[("assets/models/crate.glb", size)],
        &[("assets/models/crate.glb.license.json", CC0)],
    ))
    .unwrap();
    let a = &p.assets()["models/crate.glb"];
    assert_eq!(a.pack, "demo");
    assert_eq!(a.asset.record.license, License::Cc0);
    assert_eq!(a.asset.bytes, size);
    // No record.
    let e = load(with_assets(&[("assets/models/crate.glb", size)], &[])).unwrap_err();
    assert!(
        e.contains("assets/models/crate.glb") && e.contains("has no licence record"),
        "{e}"
    );
    // Another licence.
    let gpl = CC0.replace("CC0-1.0", "GPL-3.0");
    let e = load(with_assets(
        &[("assets/models/crate.glb", size)],
        &[("assets/models/crate.glb.license.json", &gpl)],
    ))
    .unwrap_err();
    assert!(
        e.contains("GPL-3.0") && e.contains("CC0-1.0 or CC-BY-4.0"),
        "{e}"
    );
    // A record for another file.
    let e = load(with_assets(
        &[("assets/models/crate.glb", size)],
        &[
            ("assets/models/crate.glb.license.json", CC0),
            ("assets/models/barrel.glb.license.json", CC0),
        ],
    ))
    .unwrap_err();
    assert!(
        e.contains("barrel.glb.license.json") && e.contains("does not exist"),
        "{e}"
    );
    // A record without an author.
    let e = load(with_assets(
        &[("assets/models/crate.glb", size)],
        &[(
            "assets/models/crate.glb.license.json",
            r#"{"license": "CC0-1.0", "author": " ", "source_url": "x"}"#,
        )],
    ))
    .unwrap_err();
    assert!(e.contains("author"), "{e}");
    // A stray texture without a record, and a file name that is not a plain path.
    let e = load(with_assets(&[("assets/wood.png", 10)], &[])).unwrap_err();
    assert!(
        e.contains("wood.png") && e.contains("licence record"),
        "{e}"
    );
    let e = load(with_assets(&[("assets/my crate.glb", 10)], &[])).unwrap_err();
    assert!(e.contains("not a plain asset path"), "{e}");
}

#[test]
fn a_gltf_s_files_are_covered_by_its_record() {
    let gltf = r#"{"asset": {"version": "2.0"}, "buffers": [{"uri": "crate.bin", "byteLength": 4}], "images": [{"uri": "tex/wood.png"}]}"#;
    let source = with_assets(
        &[("assets/m/crate.bin", 4), ("assets/m/tex/wood.png", 100)],
        &[
            ("assets/m/crate.gltf", gltf),
            ("assets/m/crate.gltf.license.json", CCBY),
        ],
    );
    let assets = check_assets(&source).unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(
        assets[0].files,
        [
            "assets/m/crate.gltf",
            "assets/m/crate.bin",
            "assets/m/tex/wood.png"
        ]
    );
    assert_eq!(assets[0].bytes, gltf.len() as u64 + 104);
    // The image is missing.
    let source = with_assets(
        &[("assets/m/crate.bin", 4)],
        &[
            ("assets/m/crate.gltf", gltf),
            ("assets/m/crate.gltf.license.json", CCBY),
        ],
    );
    let e = check_assets(&source).unwrap_err().to_string();
    assert!(
        e.contains("tex/wood.png") && e.contains("does not exist"),
        "{e}"
    );
}

#[test]
fn a_later_pack_s_asset_replaces_an_earlier_one_s() {
    let size = CRATE.len() as u64;
    let base = {
        let mut p = with_assets(
            &[("assets/models/crate.glb", size)],
            &[("assets/models/crate.glb.license.json", CC0)],
        );
        p.files.insert(
            "pack.json".into(),
            r#"{"gamecraft_pack": 1, "id": "base", "name": "Base", "worlds": ["main"]}"#.into(),
        );
        p.location = "base".into();
        p
    };
    let mut modded = with_assets(
        &[("assets/models/crate.glb", 7)],
        &[("assets/models/crate.glb.license.json", CCBY)],
    );
    modded.files.insert(
        "pack.json".into(),
        r#"{"gamecraft_pack": 1, "id": "demo", "name": "Demo", "depends": [{"id": "base"}]}"#
            .into(),
    );
    modded.files.remove("worlds/main/world.json");
    let (p, _) =
        Project::load("demo", &packs(vec![("base", base), ("demo", modded)]), None).unwrap();
    let a = &p.assets()["models/crate.glb"];
    assert_eq!((a.pack.as_str(), a.asset.bytes), ("demo", 7));
}

#[test]
fn credits_name_every_cc_by_asset() {
    let source = with_assets(
        &[("assets/a.glb", 1), ("assets/b.glb", 1)],
        &[
            ("assets/a.glb.license.json", CC0),
            ("assets/b.glb.license.json", CCBY),
        ],
    );
    let assets = check_assets(&source).unwrap();
    assert_eq!(
        credits(&assets),
        "\"Fence\" by Ada, CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/), from \
         https://example.org/fence. Changes: Scaled to metres.\n"
    );
    assert_eq!(credits(&assets[..1]), "");
    let r = AssetRecord::parse(CCBY, "x").unwrap();
    assert_eq!(AssetRecord::parse(&r.to_text(), "x").unwrap(), r);
}
