//! Assets of packs: the licence gate, the credits, and reading the demo pack's generated models
//! (triangle counts and bounds), with every truncation of them an error and never a panic. The
//! texture store: a model refers to its textures by a relative URI, and each has a record.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{pack, packs};
use gamecraft_project::textures::pack_glb;
use gamecraft_project::{
    AssetRecord, License, PackSource, Project, check_assets, credits, inspect,
    model::{references, resolve_uri, split_glb},
};

const CRATE: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/crate.glb");
const FENCE: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/fence.glb");

const CC0: &str = r#"{"license": "CC0-1.0", "author": "The GameCraft project", "source_url": "https://example.org/crate", "title": "Wooden crate"}"#;
const CCBY: &str = r#"{"license": "CC-BY-4.0", "author": "Ada", "source_url": "https://example.org/fence", "title": "Fence", "changes": "Scaled to metres"}"#;

const MANIFEST: &str = r#"{"gamecraft_pack": 1, "id": "demo", "name": "Demo", "worlds": ["main"]}"#;
const WORLD: &str = r#"{"gamecraft_world": 1}"#;

/// A pack with the given asset files (path, size) and texts (records, .gltf documents). A
/// `.glb`'s JSON chunk is the crate's, as a host hands it over.
fn with_assets(assets: &[(&str, u64)], texts: &[(&str, &str)]) -> PackSource {
    let mut files = vec![("pack.json", MANIFEST), ("worlds/main/world.json", WORLD)];
    files.extend_from_slice(texts);
    let mut p = pack("demo", &files);
    let crate_json = String::from_utf8(split_glb(CRATE).unwrap().0.to_vec()).unwrap();
    for (path, _) in assets {
        if path.ends_with(".glb") && !texts.iter().any(|(t, _)| t == path) {
            p.files.insert((*path).to_string(), crate_json.clone());
        }
    }
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
    // `..` is read and resolved against the model's folder: out of assets/ is refused there.
    json["buffers"][0]["uri"] = "../outside.bin".into();
    let refs = references(&serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(refs, ["../outside.bin"]);
    assert!(resolve_uri("", &refs[0]).is_err());
    assert_eq!(
        resolve_uri("models/", "../outside.bin").unwrap(),
        "outside.bin"
    );
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

// ------------------------------------------------------------------------ the texture store

/// The JSON chunk of the GLB `glb`, as text.
fn json_of(glb: &[u8]) -> &str {
    std::str::from_utf8(split_glb(glb).unwrap().0).unwrap()
}

/// A GLB of one textured triangle whose image is the file `uri` beside it, as a model refers
/// to the pack's texture store.
fn glb_referring(uri: &str) -> Vec<u8> {
    let doc = serde_json::json!({
        "asset": {"version": "2.0"},
        "images": [{"uri": uri}],
        "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
        "textures": [{"source": 0}]
    });
    pack_glb(&serde_json::to_vec(&doc).unwrap(), None).unwrap()
}

#[test]
fn a_model_refers_to_a_texture_of_the_store_which_has_a_record_of_its_own() {
    let glb = glb_referring("../textures/0123456789abcdef.png");
    let source = with_assets(
        &[
            ("assets/models/a.glb", glb.len() as u64),
            ("assets/textures/0123456789abcdef.png", 100),
        ],
        &[
            ("assets/models/a.glb", json_of(&glb)),
            ("assets/models/a.glb.license.json", CC0),
            ("assets/textures/0123456789abcdef.png.license.json", CCBY),
        ],
    );
    let model_size = source.assets["assets/models/a.glb"];
    let assets = check_assets(&source).unwrap();
    assert_eq!(assets.len(), 2);
    let (model, texture) = (&assets[0], &assets[1]);
    assert_eq!(model.kind(), "model");
    assert_eq!(
        model.files,
        [
            "assets/models/a.glb",
            "assets/textures/0123456789abcdef.png"
        ]
    );
    // A texture is counted once, as its own asset's bytes.
    assert_eq!(model.bytes, model_size);
    assert_eq!(texture.bytes, 100);
    assert_eq!(texture.kind(), "texture");
    assert_eq!(texture.record.license, License::CcBy);
    assert_eq!(texture.files, ["assets/textures/0123456789abcdef.png"]);
    // The texture is credited once, whoever uses it.
    assert_eq!(
        credits(&assets),
        "\"Fence\" by Ada, CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/), from \
         https://example.org/fence. Changes: Scaled to metres.\n"
    );
    // A model at the top of assets/ names it without climbing.
    let top = with_assets(
        &[
            ("assets/a.glb", 1),
            ("assets/textures/0123456789abcdef.png", 100),
        ],
        &[
            (
                "assets/a.glb",
                json_of(&glb_referring("textures/0123456789abcdef.png")),
            ),
            ("assets/a.glb.license.json", CC0),
            ("assets/textures/0123456789abcdef.png.license.json", CC0),
        ],
    );
    assert_eq!(check_assets(&top).unwrap().len(), 2);
    // The texture is missing.
    let e = check_assets(&with_assets(
        &[("assets/models/a.glb", glb.len() as u64)],
        &[
            ("assets/models/a.glb", json_of(&glb)),
            ("assets/models/a.glb.license.json", CC0),
        ],
    ))
    .unwrap_err()
    .to_string();
    assert!(
        e.contains("models/a.glb")
            && e.contains("assets/textures/0123456789abcdef.png")
            && e.contains("does not exist"),
        "{e}"
    );
    // The texture is there but has no record: a model's record does not cover it.
    let e = check_assets(&with_assets(
        &[
            ("assets/models/a.glb", glb.len() as u64),
            ("assets/textures/0123456789abcdef.png", 100),
        ],
        &[
            ("assets/models/a.glb", json_of(&glb)),
            ("assets/models/a.glb.license.json", CC0),
        ],
    ))
    .unwrap_err()
    .to_string();
    assert!(
        e.contains("textures/0123456789abcdef.png")
            && e.contains("has no licence record")
            && e.contains("texture store"),
        "{e}"
    );
    // A record for a texture that is not there.
    let e = check_assets(&with_assets(
        &[],
        &[("assets/textures/0123456789abcdef.png.license.json", CC0)],
    ))
    .unwrap_err()
    .to_string();
    assert!(e.contains("which does not exist"), "{e}");
}

#[test]
fn a_model_may_not_refer_out_of_the_pack_s_assets() {
    for uri in ["../../x.png", "/x.png", "http://x/y.png", "a//b.png"] {
        let glb = glb_referring(uri);
        let e = load(with_assets(
            &[("assets/models/a.glb", 1)],
            &[
                ("assets/models/a.glb", json_of(&glb)),
                ("assets/models/a.glb.license.json", CC0),
            ],
        ))
        .unwrap_err();
        assert!(e.contains("models/a.glb") && e.contains(uri), "{uri}: {e}");
    }
    // A .glb whose JSON chunk the host did not hand over.
    let mut source = with_assets(
        &[("assets/models/crate.glb", CRATE.len() as u64)],
        &[("assets/models/crate.glb.license.json", CC0)],
    );
    source.files.remove("assets/models/crate.glb");
    let e = check_assets(&source).unwrap_err().to_string();
    assert!(e.contains("was not read") && e.contains("crate.glb"), "{e}");
    // A .glb that is not one.
    let mut source = with_assets(
        &[("assets/models/bad.glb", 4)],
        &[("assets/models/bad.glb.license.json", CC0)],
    );
    source
        .files
        .insert("assets/models/bad.glb".into(), "not json".into());
    let e = check_assets(&source).unwrap_err().to_string();
    assert!(e.contains("bad.glb") && e.contains("glTF"), "{e}");
}

#[test]
fn a_model_covers_only_the_files_in_its_own_folder() {
    // `models/a.gltf` refers to files beside it and above it.
    let gltf = r#"{"asset": {"version": "2.0"}, "buffers": [{"uri": "../shared/a.bin", "byteLength": 4}], "images": [{"uri": "tex/wood.png"}, {"uri": "../sounds/x.ogg"}]}"#;
    let files = [
        ("assets/models/a.gltf", 10),
        ("assets/models/tex/wood.png", 100),
        ("assets/shared/a.bin", 4),
        ("assets/sounds/x.ogg", 8),
    ];
    // Neither file outside the model's folder is covered by its record.
    let e = check_assets(&with_assets(
        &files,
        &[
            ("assets/models/a.gltf", gltf),
            ("assets/models/a.gltf.license.json", CCBY),
        ],
    ))
    .unwrap_err()
    .to_string();
    assert!(e.contains("has no licence record"), "{e}");
    assert!(
        e.contains("shared/a.bin") || e.contains("sounds/x.ogg"),
        "{e}"
    );
    // With records of their own they are assets; the one in the model's folder stays covered.
    let assets = check_assets(&with_assets(
        &files,
        &[
            ("assets/models/a.gltf", gltf),
            ("assets/models/a.gltf.license.json", CCBY),
            ("assets/shared/a.bin.license.json", CC0),
            ("assets/sounds/x.ogg.license.json", CC0),
        ],
    ))
    .unwrap();
    assert_eq!(assets.len(), 3, "{assets:?}");
    assert_eq!(assets[0].path, "models/a.gltf");
    assert_eq!(
        assets[0].files,
        [
            "assets/models/a.gltf",
            "assets/shared/a.bin",
            "assets/models/tex/wood.png",
            "assets/sounds/x.ogg"
        ]
    );
    assert_eq!(assets[1].path, "shared/a.bin");
    assert_eq!(assets[2].path, "sounds/x.ogg");
    // A model at the top of assets/ has every other folder as its folder.
    let top = with_assets(
        &[("assets/a.gltf", 10), ("assets/sounds/x.ogg", 8)],
        &[
            (
                "assets/a.gltf",
                r#"{"asset": {"version": "2.0"}, "images": [{"uri": "sounds/x.ogg"}]}"#,
            ),
            ("assets/a.gltf.license.json", CC0),
        ],
    );
    assert_eq!(check_assets(&top).unwrap().len(), 1);
}
