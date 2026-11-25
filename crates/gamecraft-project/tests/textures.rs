//! The texture store (`gamecraft_project::textures`): an import takes a model's images into
//! `assets/textures/<content-hash>.<ext>`, stores an image that many models share once, rewrites
//! each model to refer to them, and leaves it reading exactly as before. Nothing here panics on
//! a broken file. The small models are built in the test.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use gamecraft_project::ModelSummary;
use gamecraft_project::model::{
    decode_uri, glb_json_len, inspect, references, resolve_uri, split_glb,
};
use gamecraft_project::textures::{
    Imported, content_hash, import, pack_glb, texture_name, texture_path,
};
use serde_json::{Value, json};

const CRATE: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/crate.glb");
const FENCE: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/fence.glb");
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

fn none(_: &str) -> Option<Vec<u8>> {
    None
}

/// The bytes of the first image embedded in the demo GLB `glb`.
fn embedded_image(glb: &[u8]) -> Vec<u8> {
    let word = |at: usize| -> usize {
        u32::from_le_bytes([glb[at], glb[at + 1], glb[at + 2], glb[at + 3]]) as usize
    };
    let json_len = word(12);
    let doc: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    let view = &doc["bufferViews"][doc["images"][0]["bufferView"].as_u64().unwrap() as usize];
    let at = view["byteOffset"].as_u64().unwrap() as usize;
    let len = view["byteLength"].as_u64().unwrap() as usize;
    let bin = 20 + json_len + 8;
    glb[bin + at..bin + at + len].to_vec()
}

/// The document and the binary chunk of a model with one textured triangle. The buffer views
/// are the positions', the indices' and the image's (the image's first with `image_first`); with
/// `shared`, an unused accessor reads the image's view as well.
fn model(image: &[u8], name: &str, image_first: bool, shared: bool) -> (Value, Vec<u8>) {
    let positions: Vec<u8> = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    let indices: Vec<u8> = [0u16, 1, 2].iter().flat_map(|i| i.to_le_bytes()).collect();
    let parts: [(&str, Vec<u8>, u32); 3] = if image_first {
        [
            ("image", image.to_vec(), 0),
            ("positions", positions, 34962),
            ("indices", indices, 34963),
        ]
    } else {
        [
            ("positions", positions, 34962),
            ("indices", indices, 34963),
            ("image", image.to_vec(), 0),
        ]
    };
    let mut bin = Vec::new();
    let mut views = Vec::new();
    let mut index_of = std::collections::BTreeMap::new();
    for (i, (what, bytes, target)) in parts.iter().enumerate() {
        let at = bin.len();
        bin.extend_from_slice(bytes);
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let mut view = json!({"buffer": 0, "byteOffset": at, "byteLength": bytes.len()});
        if *target != 0 {
            view["target"] = json!(target);
        }
        views.push(view);
        index_of.insert(*what, i);
    }
    let mut accessors = vec![
        json!({"bufferView": index_of["positions"], "componentType": 5126, "count": 3,
               "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]}),
        json!({"bufferView": index_of["indices"], "componentType": 5123, "count": 3,
               "type": "SCALAR"}),
    ];
    if shared {
        accessors.push(
            json!({"bufferView": index_of["image"], "componentType": 5121,
                              "count": 1, "type": "SCALAR"}),
        );
    }
    let mime = if image.starts_with(PNG) {
        "image/png"
    } else {
        "image/jpeg"
    };
    let doc = json!({
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": accessors,
        "images": [{"bufferView": index_of["image"], "mimeType": mime}],
        "materials": [{"name": "wall", "pbrMetallicRoughness": {
            "baseColorTexture": {"index": 0}, "metallicFactor": 0.0, "roughnessFactor": 0.9}}],
        "textures": [{"source": 0}],
        "samplers": [{"wrapS": 10497, "wrapT": 10497}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
        "nodes": [{"mesh": 0, "name": name}],
        "scenes": [{"nodes": [0]}],
        "scene": 0
    });
    (doc, bin)
}

/// A GLB with one textured triangle whose image is embedded.
fn glb(image: &[u8], name: &str) -> Vec<u8> {
    let (doc, bin) = model(image, name, false, false);
    pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap()
}

/// A `.gltf` document (text) whose image is `image` (a JSON image) and whose buffer is the
/// file `buffer(len)` names (given the buffer's length); and the buffer's bytes.
fn gltf(image: Value, buffer: impl FnOnce(usize) -> Value) -> (String, Vec<u8>) {
    let (mut doc, bin) = model(PNG, "house", false, false);
    doc["images"] = json!([image]);
    doc["buffers"] = json!([buffer(bin.len())]);
    (serde_json::to_string(&doc).unwrap(), bin)
}

fn house_bin(len: usize) -> Value {
    json!({"uri": "house.bin", "byteLength": len})
}

fn base64(data: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(A[(n >> 18 & 63) as usize] as char);
        out.push(A[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// The file extension of an image: what its first bytes say.
fn ext_of(image: &[u8]) -> &'static str {
    if image.starts_with(PNG) { "png" } else { "jpg" }
}

/// Reads a stored model of the folder `folder` back, with its textures where they are now and
/// `resource` for the files it keeps beside it.
fn read_back(
    i: &Imported,
    folder: &str,
    resource: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<ModelSummary, String> {
    inspect(&i.model, &|r: &str| {
        let inside = resolve_uri(folder, r).ok()?;
        i.textures
            .iter()
            .find(|t| t.path == inside)
            .map(|t| t.bytes.clone())
            .or_else(|| resource(r))
    })
}

/// The files a stored model keeps beside it: (URI, path inside assets/, the length of the
/// bytes the import rewrote, or `None` to copy the file as it came).
fn files_of(i: &Imported) -> Vec<(String, String, Option<usize>)> {
    i.files
        .iter()
        .map(|f| {
            (
                f.uri.clone(),
                f.path.clone(),
                f.bytes.as_ref().map(Vec::len),
            )
        })
        .collect()
}

/// The JSON document of a model file.
fn json_of(file: &[u8]) -> Value {
    serde_json::from_slice(split_glb(file).unwrap().0).unwrap()
}

/// The files a stored model refers to, as its document writes them.
fn uris_of(file: &[u8]) -> Vec<String> {
    references(split_glb(file).unwrap().0).unwrap()
}

#[test]
fn two_models_with_the_same_image_store_one_texture_and_read_as_before() {
    let wood = embedded_image(CRATE);
    let a = import(&glb(&wood, "a"), &none, "models/a.glb").unwrap();
    let b = import(&glb(&wood, "b"), &none, "models/b.glb").unwrap();
    let name = texture_name(content_hash(&wood), "png");
    let path = texture_path(&name);
    assert_eq!(a.textures.len(), 1);
    assert_eq!(a.textures, b.textures, "the same image is one texture");
    assert_eq!(a.textures[0].path, path);
    assert_eq!(a.textures[0].bytes, wood);
    // The stored models refer to it, relatively, and have nothing else beside them.
    for i in [&a, &b] {
        assert_eq!(uris_of(&i.model), [format!("../textures/{name}")]);
        assert_eq!(
            resolve_uri("models/", &format!("../textures/{name}")).unwrap(),
            path
        );
        assert!(i.files.is_empty());
        // Smaller: the binary chunk no longer holds the image.
        assert!(i.model.len() + wood.len() < glb(&wood, "x").len());
        let (_, bin) = split_glb(&i.model).unwrap();
        assert!(
            !bin.unwrap().windows(PNG.len()).any(|w| w == PNG),
            "the image is gone"
        );
        // It reads back as the original: checked by the import, and here again.
        assert_eq!(i.summary, inspect(&glb(&wood, "x"), &none).unwrap());
        assert_eq!(read_back(i, "models/", &none).unwrap(), i.summary);
    }
    // A model deeper in the pack climbs one more folder; one at the top does not climb.
    let deep = import(&glb(&wood, "deep"), &none, "models/houses/deep.glb").unwrap();
    assert_eq!(deep.textures[0].path, path);
    assert_eq!(uris_of(&deep.model), [format!("../../textures/{name}")]);
    assert_eq!(
        read_back(&deep, "models/houses/", &none).unwrap(),
        deep.summary
    );
    let top = import(&glb(&wood, "top"), &none, "top.glb").unwrap();
    assert_eq!(uris_of(&top.model), [format!("textures/{name}")]);
    assert_eq!(read_back(&top, "", &none).unwrap(), top.summary);
}

#[test]
fn a_model_with_another_image_stores_another_texture() {
    let wood = embedded_image(CRATE);
    let plank = embedded_image(FENCE);
    let name = texture_name(content_hash(&wood), "png");
    let a = import(&glb(&wood, "a"), &none, "models/a.glb").unwrap();
    let b = import(&glb(&wood, "b"), &none, "models/b.glb").unwrap();
    let c = import(&glb(&plank, "c"), &none, "models/c.glb").unwrap();
    assert_eq!(a.textures[0].path, b.textures[0].path);
    assert_ne!(a.textures[0].path, c.textures[0].path);
    let ext = if plank.starts_with(PNG) { "png" } else { "jpg" };
    assert_eq!(
        c.textures[0].path,
        texture_path(&texture_name(content_hash(&plank), ext))
    );
    // The same image twice in one model is one texture, referred to twice.
    let (mut doc, bin) = model(&wood, "twice", false, false);
    doc["images"] = json!([
        {"bufferView": 2, "mimeType": "image/png"},
        {"bufferView": 2, "mimeType": "image/png"}
    ]);
    doc["textures"] = json!([{"source": 0}, {"source": 1}]);
    let twice = import(
        &pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap(),
        &none,
        "models/twice.glb",
    )
    .unwrap();
    assert_eq!(twice.textures.len(), 1);
    assert_eq!(twice.summary.images, 2);
    assert_eq!(json_of(&twice.model)["images"].as_array().unwrap().len(), 2);
    assert_eq!(uris_of(&twice.model), [format!("../textures/{name}")]);
}

#[test]
fn the_demo_models_keep_their_materials_and_lose_their_images_from_the_binary_chunk() {
    for source in [CRATE, FENCE] {
        let image = embedded_image(source);
        let i = import(source, &none, "models/x.glb").unwrap();
        assert_eq!(i.textures.len(), 1);
        assert_eq!(i.textures[0].bytes, image);
        assert_eq!(i.summary, inspect(source, &none).unwrap());
        assert_eq!(read_back(&i, "models/", &none).unwrap(), i.summary);
        let (before, after) = (json_of(source), json_of(&i.model));
        for kept in [
            "asset",
            "accessors",
            "materials",
            "meshes",
            "nodes",
            "samplers",
            "scenes",
            "textures",
        ] {
            assert_eq!(before[kept], after[kept], "{kept}");
        }
        assert_eq!(
            after["images"],
            json!([{"uri": format!("../textures/{}", texture_name(content_hash(&image), ext_of(&image)))}])
        );
        let (_, old_bin) = split_glb(source).unwrap();
        let (_, new_bin) = split_glb(&i.model).unwrap();
        let (old_bin, new_bin) = (old_bin.unwrap(), new_bin.unwrap());
        assert!(new_bin.len() + image.len() <= old_bin.len());
        assert!(!new_bin.windows(PNG.len()).any(|w| w == PNG));
        // Every other buffer view keeps its bytes.
        let image_view = before["images"][0]["bufferView"].as_u64().unwrap() as usize;
        let views = |doc: &Value| -> Vec<(u64, u64)> {
            doc["bufferViews"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    (
                        v["byteOffset"].as_u64().unwrap(),
                        v["byteLength"].as_u64().unwrap(),
                    )
                })
                .collect()
        };
        for (k, &(bo, len)) in views(&before).iter().enumerate() {
            if k == image_view {
                continue;
            }
            let (ao, alen) = views(&after)[k];
            assert_eq!(len, alen);
            assert_eq!(
                old_bin[bo as usize..(bo + len) as usize],
                new_bin[ao as usize..(ao + alen) as usize],
                "buffer view {k}"
            );
        }
        assert_eq!(
            after["buffers"][0]["byteLength"].as_u64().unwrap(),
            new_bin.len() as u64 - new_bin.len() as u64 % 4
        );
    }
}

#[test]
fn a_view_an_accessor_reads_stays_and_the_accessors_are_renumbered() {
    let wood = embedded_image(CRATE);
    // The image's view comes first: dropping it moves the others up.
    let (d, bin) = model(&wood, "first", true, false);
    let source = pack_glb(&serde_json::to_vec(&d).unwrap(), Some(&bin)).unwrap();
    let i = import(&source, &none, "models/first.glb").unwrap();
    let doc = json_of(&i.model);
    assert_eq!(doc["bufferViews"].as_array().unwrap().len(), 2);
    assert_eq!(doc["accessors"][0]["bufferView"], 0);
    assert_eq!(doc["accessors"][1]["bufferView"], 1);
    assert_eq!(i.summary, inspect(&source, &none).unwrap());
    assert_eq!(read_back(&i, "models/", &none).unwrap(), i.summary);
    // An accessor reads the image's view: it stays, image and all.
    let (d, bin) = model(&wood, "shared", true, true);
    let source = pack_glb(&serde_json::to_vec(&d).unwrap(), Some(&bin)).unwrap();
    let i = import(&source, &none, "models/shared.glb").unwrap();
    let doc = json_of(&i.model);
    assert_eq!(doc["bufferViews"].as_array().unwrap().len(), 3);
    assert_eq!(doc["accessors"][2]["bufferView"], 0);
    assert_eq!(
        doc["images"][0]["uri"],
        format!("../textures/{}", texture_name(content_hash(&wood), "png"))
    );
    let (_, new_bin) = split_glb(&i.model).unwrap();
    assert_eq!(new_bin.unwrap(), bin, "nothing moved");
    assert_eq!(read_back(&i, "models/", &none).unwrap(), i.summary);
}

#[test]
fn a_gltf_keeps_its_buffers_beside_it_and_its_images_in_the_store() {
    let wood = embedded_image(CRATE);
    let (text, bin) = gltf(json!({"uri": "tex/wood.png"}), house_bin);
    let found = |r: &str| {
        (r == "tex/wood.png")
            .then(|| wood.clone())
            .or((r == "house.bin").then(|| bin.clone()))
    };
    let i = import(text.as_bytes(), &found, "models/houses/house.gltf").unwrap();
    let name = texture_name(content_hash(&wood), "png");
    assert_eq!(i.textures[0].path, texture_path(&name));
    let doc: Value = serde_json::from_slice(&i.model).unwrap();
    assert_eq!(
        doc["images"],
        json!([{"uri": format!("../../textures/{name}")}])
    );
    assert_eq!(doc["buffers"], json!([house_bin(bin.len())]));
    assert_eq!(
        files_of(&i),
        [(
            "house.bin".to_string(),
            "models/houses/house.bin".to_string(),
            None
        )]
    );
    assert_eq!(i.summary, inspect(text.as_bytes(), &found).unwrap());
    // An image as a data: URI is the same texture as the file; so is one with an escaped name.
    for (image, lookup) in [
        (
            json!({"uri": format!("data:image/png;base64,{}", base64(&wood))}),
            "tex/wood.png",
        ),
        (json!({"uri": "tex/wood%20data.png"}), "tex/wood data.png"),
    ] {
        let (text, bin) = gltf(image.clone(), house_bin);
        let found = |r: &str| {
            (r == lookup)
                .then(|| wood.clone())
                .or((r == "house.bin").then(|| bin.clone()))
        };
        let d = import(text.as_bytes(), &found, "models/houses/house.gltf").unwrap();
        assert_eq!(d.textures[0].path, texture_path(&name), "{image}");
        assert_eq!(d.summary.images, 1);
    }
    // A buffer file that holds the image is read too.
    let (mut doc, bin) = model(&wood, "in-buffer", false, false);
    doc["buffers"] = json!([house_bin(bin.len())]);
    let found = |r: &str| (r == "house.bin").then(|| bin.clone());
    let i = import(
        &serde_json::to_vec(&doc).unwrap(),
        &found,
        "models/house.gltf",
    )
    .unwrap();
    assert_eq!(i.textures[0].bytes, wood);
    assert_eq!(i.files.len(), 1);
    assert_eq!(read_back(&i, "models/", &found).unwrap(), i.summary);
}

#[test]
fn a_gltf_s_files_are_named_and_placed_inside_assets() {
    let wood = embedded_image(CRATE);
    // The image beside the model is missing.
    let (text, bin) = gltf(json!({"uri": "tex/wood.png"}), house_bin);
    let found = |r: &str| (r == "house.bin").then(|| bin.clone());
    let e = import(text.as_bytes(), &found, "models/house.gltf").unwrap_err();
    assert!(
        e.contains("image 0") && e.contains("tex/wood.png") && e.contains("missing"),
        "{e}"
    );
    // Buffers the pack cannot place beside the model: each is read first (the model must
    // load), then refused with why.
    for (uri, to, wants) in [
        ("../house.bin", "house.gltf", "leads outside"),
        (
            "../textures/house.bin",
            "models/house.gltf",
            "texture store",
        ),
        ("my%20house.bin", "models/house.gltf", "plain file"),
        ("../house.glb", "models/house.gltf", "model path"),
        (
            "house.bin.license.json",
            "models/house.gltf",
            "licence record",
        ),
    ] {
        let (text, bin) = gltf(
            json!({"uri": "tex/wood.png"}),
            |len| json!({"uri": uri, "byteLength": len}),
        );
        let found = |r: &str| {
            (r == "tex/wood.png")
                .then(|| wood.clone())
                .or((r == decode_uri(uri).unwrap()).then(|| bin.clone()))
        };
        let e = import(text.as_bytes(), &found, to).unwrap_err();
        assert!(e.contains(wants), "{uri} -> {to}: {e}");
    }
    // A buffer outside the model's own folder: the gate would want a record for it, so the
    // import refuses it and says to move the file beside the model.
    let (text, bin) = gltf(
        json!({"uri": "tex/wood.png"}),
        |len| json!({"uri": "../shared/house.bin", "byteLength": len}),
    );
    let found = |r: &str| {
        (r == "tex/wood.png")
            .then(|| wood.clone())
            .or((r == "../shared/house.bin").then(|| bin.clone()))
    };
    let e = import(text.as_bytes(), &found, "models/house.gltf").unwrap_err();
    assert!(
        e.contains("shared/house.bin") && e.contains("beside the model"),
        "{e}"
    );
    // In a folder of the model's own folder it is fine, and at the top of assets/ every
    // folder is the model's.
    let (text, bin) = gltf(
        json!({"uri": "tex/wood.png"}),
        |len| json!({"uri": "sub/house.bin", "byteLength": len}),
    );
    let found = |r: &str| {
        (r == "tex/wood.png")
            .then(|| wood.clone())
            .or((r == "sub/house.bin").then(|| bin.clone()))
    };
    let i = import(text.as_bytes(), &found, "models/house.gltf").unwrap();
    assert_eq!(
        files_of(&i),
        [(
            "sub/house.bin".to_string(),
            "models/sub/house.bin".to_string(),
            None
        )]
    );
    assert_eq!(read_back(&i, "models/", &found).unwrap(), i.summary);
    let (text, bin) = gltf(
        json!({"uri": "tex/wood.png"}),
        |len| json!({"uri": "shared/house.bin", "byteLength": len}),
    );
    let found = |r: &str| {
        (r == "tex/wood.png")
            .then(|| wood.clone())
            .or((r == "shared/house.bin").then(|| bin.clone()))
    };
    let i = import(text.as_bytes(), &found, "house.gltf").unwrap();
    assert_eq!(
        files_of(&i),
        [(
            "shared/house.bin".to_string(),
            "shared/house.bin".to_string(),
            None
        )]
    );
    assert_eq!(read_back(&i, "", &found).unwrap(), i.summary);
    // A model that cannot be read is refused first, with why.
    let e = import(&CRATE[..CRATE.len() / 2], &none, "models/x.glb").unwrap_err();
    assert!(e.contains("cut short"), "{e}");
}

#[test]
fn a_model_without_images_is_stored_as_it_came() {
    let wood = embedded_image(CRATE);
    let (mut doc, bin) = model(&wood, "plain", false, false);
    doc.as_object_mut().unwrap().remove("images");
    doc.as_object_mut().unwrap().remove("textures");
    doc["materials"] = json!([{"pbrMetallicRoughness": {"baseColorFactor": [1.0, 0.0, 0.0, 1.0]}}]);
    let bin = &bin[..bin.len() - wood.len() - (4 - wood.len() % 4) % 4];
    doc["buffers"] = json!([{"byteLength": bin.len()}]);
    doc["bufferViews"].as_array_mut().unwrap().truncate(2);
    // A .glb without images: the very same bytes.
    let glb = pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(bin)).unwrap();
    let i = import(&glb, &none, "models/x.glb").unwrap();
    assert_eq!(i.model, glb);
    assert!(i.textures.is_empty());
    assert!(i.files.is_empty());
    // A .gltf without images: the same text, its buffer a file to copy beside it.
    doc["buffers"] = json!([{"uri": "x.bin", "byteLength": bin.len()}]);
    let text = serde_json::to_vec(&doc).unwrap();
    let found = |r: &str| (r == "x.bin").then(|| bin.to_vec());
    let i = import(&text, &found, "models/x.gltf").unwrap();
    assert_eq!(i.model, text);
    assert!(i.textures.is_empty());
    assert_eq!(
        files_of(&i),
        [("x.bin".to_string(), "models/x.bin".to_string(), None)]
    );
    assert_eq!(read_back(&i, "models/", &found).unwrap(), i.summary);
}

#[test]
fn a_broken_model_is_an_error_never_a_panic() {
    let wood = embedded_image(CRATE);
    for source in [glb(&wood, "x"), FENCE.to_vec()] {
        for cut in 0..source.len() {
            let _ = import(&source[..cut], &none, "models/x.glb");
        }
        for at in (0..source.len()).step_by(3) {
            let mut bad = source.clone();
            bad[at] ^= 0x5A;
            let _ = import(&bad, &none, "models/x.glb");
        }
    }
    assert!(import(b"not a model", &none, "models/x.glb").is_err());
    assert!(import(b"glTF\x02\0\0\0garbage", &none, "models/x.glb").is_err());

    // Documents whose image cannot be taken into the store.
    let base = json_of(&glb(&wood, "x"));
    let image_view = |doc: &mut Value, view: Value| {
        doc["bufferViews"] = json!([view]);
        doc["images"] = json!([{"bufferView": 0, "mimeType": "image/png"}]);
    };
    let mut variants: Vec<(&str, Value)> = Vec::new();
    for (why, images) in [
        ("nor a `uri`", json!([{}])),
        (
            "bufferView 99",
            json!([{"bufferView": 99, "mimeType": "image/png"}]),
        ),
        ("not base64", json!([{"uri": "data:image/png,notbase64"}])),
        ("base64", json!([{"uri": "data:image/png;base64,!"}])),
        ("cut short", json!([{"uri": "data:image/png;base64,AAAAA"}])),
        ("/abs.png", json!([{"uri": "/abs.png"}])),
        ("outside", json!([{"uri": "../outside.png"}])),
        ("missing.png", json!([{"uri": "missing.png"}])),
        ("not a list", json!("not a list")),
        ("neither", json!([{"mimeType": "image/png"}])),
    ] {
        let mut doc = base.clone();
        doc["images"] = images;
        variants.push((why, doc));
    }
    for (why, view) in [
        (
            "buffer 1",
            json!({"buffer": 1, "byteOffset": 0, "byteLength": 10}),
        ),
        (
            "outside",
            json!({"buffer": 0, "byteOffset": 100000, "byteLength": 10}),
        ),
        ("byteLength", json!({"buffer": 0, "byteOffset": 0})),
        (
            "bufferView 0",
            json!([{"buffer": 0, "byteOffset": 0, "byteLength": 4}]),
        ),
    ] {
        let mut doc = base.clone();
        if why == "bufferView 0" {
            doc["bufferViews"] = view;
            doc["images"] = json!([{"bufferView": 5, "mimeType": "image/png"}]);
        } else {
            image_view(&mut doc, view);
        }
        variants.push((why, doc));
    }
    let mut doc = base.clone();
    doc["images"] = json!([{"bufferView": 2, "mimeType": "image/png"}]);
    doc.as_object_mut().unwrap().remove("buffers");
    variants.push(("no buffers", doc));
    for (why, doc) in variants {
        let bin = {
            let (_, bin) = model(&wood, "x", false, false);
            bin
        };
        let source = pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap();
        let e = import(&source, &none, "models/x.glb");
        assert!(e.is_err(), "{why}: {e:?}");
        let e = import(&serde_json::to_vec(&doc).unwrap(), &none, "models/x.gltf");
        assert!(e.is_err(), "{why}: {e:?}");
    }
    // An image that is neither a PNG nor a JPEG is refused, naming the image and the types.
    let (mut doc, bin) = model(b"not an image at all", "x", false, false);
    doc["images"] = json!([{"bufferView": 2, "mimeType": "image/webp"}]);
    let e = import(
        &pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap(),
        &none,
        "models/x.glb",
    )
    .unwrap_err();
    assert!(
        e.contains("image 0") && e.contains("neither a PNG nor a JPEG") && e.contains("webp"),
        "{e}"
    );
    // Its type only the MIME type tells: the file takes its extension from it.
    let mut doc = json_of(&glb(b"not an image at all", "x"));
    doc["images"] = json!([{"bufferView": 2, "mimeType": "image/jpeg"}]);
    let source = pack_glb(
        &serde_json::to_vec(&doc).unwrap(),
        Some(&{
            let (_, bin) = model(b"not an image at all", "x", false, false);
            bin
        }),
    )
    .unwrap();
    let i = import(&source, &none, "models/x.glb").unwrap();
    assert!(
        i.textures[0].path.ends_with(".jpg"),
        "{}",
        i.textures[0].path
    );
}

// ---------------------------------------------------------------- review round

/// A GLB whose geometry is in the files `buffers` name (no binary chunk), its image `image`.
fn glb_with_buffers(image: Value, buffers: Value, bin: Option<&[u8]>) -> Vec<u8> {
    let (mut doc, _) = model(PNG, "x", false, false);
    doc["images"] = json!([image]);
    doc["buffers"] = buffers;
    let views: Vec<Value> = doc["bufferViews"].as_array().unwrap().clone();
    let _ = views;
    pack_glb(&serde_json::to_vec(&doc).unwrap(), bin).unwrap()
}

#[test]
fn a_glb_without_a_binary_chunk_stays_a_glb() {
    let wood = embedded_image(CRATE);
    // The geometry in a file beside it, the image as a data: URI: a GLB with a JSON chunk only.
    let (text, bin) = gltf(
        json!({"uri": format!("data:image/png;base64,{}", base64(&wood))}),
        house_bin,
    );
    let doc: Value = serde_json::from_str(&text).unwrap();
    let source = glb_with_buffers(doc["images"][0].clone(), doc["buffers"].clone(), None);
    let found = |r: &str| (r == "house.bin").then(|| bin.clone());
    let i = import(&source, &found, "models/x.glb").unwrap();
    assert!(
        i.model.starts_with(b"glTF"),
        "still a GLB, whatever it holds"
    );
    assert_eq!(split_glb(&i.model).unwrap().1, None);
    assert_eq!(
        glb_json_len(&i.model[..20], i.model.len() as u64),
        Ok(i.model.len() - 20),
        "the host can read its JSON chunk"
    );
    assert_eq!(read_back(&i, "models/", &found).unwrap(), i.summary);
    // The image beside it instead: the same.
    let (text, bin) = gltf(json!({"uri": "tex/wood.png"}), house_bin);
    let doc: Value = serde_json::from_str(&text).unwrap();
    let source = glb_with_buffers(doc["images"][0].clone(), doc["buffers"].clone(), None);
    let found = |r: &str| {
        (r == "house.bin")
            .then(|| bin.clone())
            .or((r == "tex/wood.png").then(|| wood.clone()))
    };
    let i = import(&source, &found, "models/x.glb").unwrap();
    assert!(i.model.starts_with(b"glTF"));
    assert_eq!(split_glb(&i.model).unwrap().1, None);
    assert_eq!(read_back(&i, "models/", &found).unwrap(), i.summary);
}

#[test]
fn a_buffer_that_keeps_no_view_is_dropped_and_the_buffers_renumbered() {
    let wood = embedded_image(CRATE);
    // A GLB whose binary chunk holds only the image, its geometry in a file beside it.
    let (d, bin) = model(&wood, "x", false, false);
    let views = d["bufferViews"].as_array().unwrap();
    let (geometry, image) = (
        json!({"uri": "geo.bin", "byteLength": bin.len()}),
        json!([views[2].clone()]),
    );
    let mut doc = d.clone();
    doc["buffers"] = json!([{"byteLength": bin.len()}, geometry]);
    doc["bufferViews"] = json!([
        {"buffer": 1, "byteOffset": views[0]["byteOffset"].clone(), "byteLength": views[0]["byteLength"].clone(), "target": 34962},
        {"buffer": 1, "byteOffset": views[1]["byteOffset"].clone(), "byteLength": views[1]["byteLength"].clone(), "target": 34963},
        {"buffer": 0, "byteOffset": image[0]["byteOffset"].clone(), "byteLength": image[0]["byteLength"].clone()}
    ]);
    doc["images"] = json!([{"bufferView": 2, "mimeType": "image/png"}]);
    let source = pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap();
    let found = |r: &str| (r == "geo.bin").then(|| bin.clone());
    let i = import(&source, &found, "models/x.glb").unwrap();
    // No binary chunk, no buffer for it: the geometry's buffer is buffer 0 now.
    assert!(i.model.starts_with(b"glTF"));
    assert_eq!(split_glb(&i.model).unwrap().1, None, "no empty chunk");
    let after = json_of(&i.model);
    // The geometry's file lost no view of its own: copied as it came.
    assert_eq!(
        after["buffers"],
        json!([{"uri": "geo.bin", "byteLength": bin.len()}])
    );
    assert_eq!(after["bufferViews"].as_array().unwrap().len(), 2);
    assert_eq!(after["bufferViews"][0]["buffer"], 0);
    assert_eq!(after["bufferViews"][1]["buffer"], 0);
    assert_eq!(
        after["bufferViews"][0]["byteOffset"],
        views[0]["byteOffset"]
    );
    assert_eq!(after["accessors"][0]["bufferView"], 0);
    assert_eq!(after["accessors"][1]["bufferView"], 1);
    assert_eq!(
        files_of(&i),
        [("geo.bin".to_string(), "models/geo.bin".to_string(), None)]
    );
    assert_eq!(read_back(&i, "models/", &found).unwrap(), i.summary);
}

#[test]
fn a_document_the_engine_cannot_track_is_refused() {
    let wood = embedded_image(CRATE);
    // The real layout of EXT_meshopt_compression: the view carries the extension with
    // `buffer`, `byteOffset` and `byteLength` (no `bufferView`), a buffer carries the
    // fallback. The engine cannot read the geometry; the import says so.
    let (mut doc, bin) = model(&wood, "x", false, false);
    doc["extensionsUsed"] = json!(["EXT_meshopt_compression"]);
    doc["bufferViews"][0]["extensions"] = json!({"EXT_meshopt_compression": {
        "buffer": 0, "byteOffset": 0, "byteLength": 36, "count": 3, "byteStride": 12}});
    doc["buffers"][0]["extensions"] = json!({"EXT_meshopt_compression": {"fallback": 0}});
    let source = pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap();
    let e = import(&source, &none, "models/x.glb").unwrap_err();
    assert!(e.contains("EXT_meshopt_compression"), "{e}");
    // With its image as a file nothing would be renumbered, and still the geometry cannot
    // be read: refused too.
    let (text, bin) = gltf(
        json!({"uri": "tex/wood.png"}),
        |len| json!({"uri": "house.bin", "byteLength": len}),
    );
    let mut doc: Value = serde_json::from_str(&text).unwrap();
    doc["extensionsRequired"] = json!(["EXT_meshopt_compression"]);
    let found = |r: &str| {
        (r == "tex/wood.png")
            .then(|| wood.clone())
            .or((r == "house.bin").then(|| bin.clone()))
    };
    let e = import(&serde_json::to_vec(&doc).unwrap(), &found, "models/x.gltf").unwrap_err();
    assert!(e.contains("EXT_meshopt_compression"), "{e}");
    // Any other extension object that names a buffer is refused, naming where.
    let (mut doc, bin) = model(&wood, "x", false, false);
    doc["extensionsUsed"] = json!(["X_buffer_user"]);
    doc["meshes"][0]["primitives"][0]["extensions"] = json!({"X_buffer_user": {"buffer": 0}});
    let source = pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap();
    let e = import(&source, &none, "models/x.glb").unwrap_err();
    assert!(
        e.contains("X_buffer_user") && e.contains("meshes[0].primitives[0].extensions"),
        "{e}"
    );
    // So is a key that names a buffer view outside the accessors and the images.
    let (mut doc, bin) = model(&wood, "x", false, false);
    doc["meshes"][0]["primitives"][0]["extensions"] =
        json!({"X_view_user": {"bufferView": 2, "count": 3}});
    let source = pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap();
    let e = import(&source, &none, "models/x.glb").unwrap_err();
    assert!(e.contains("bufferView") && e.contains("meshes[0]"), "{e}");
    let (mut doc, bin) = model(&wood, "x", false, false);
    doc["buffers"][0]["extensions"] = json!({"X_fallback": {"fallbackBufferView": 2}});
    let source = pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap();
    let e = import(&source, &none, "models/x.glb").unwrap_err();
    assert!(
        e.contains("fallbackBufferView") && e.contains("buffers[0]"),
        "{e}"
    );
    // An extension that names neither (a material's emissive strength) is no problem.
    let (mut doc, bin) = model(&wood, "x", false, false);
    doc["extensionsUsed"] = json!(["KHR_materials_emissive_strength"]);
    doc["materials"][0]["extensions"] =
        json!({"KHR_materials_emissive_strength": {"emissiveStrength": 3.0}});
    let source = pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap();
    let i = import(&source, &none, "models/x.glb").unwrap();
    assert_eq!(
        json_of(&i.model)["materials"][0]["extensions"]["KHR_materials_emissive_strength"]["emissiveStrength"],
        3.0
    );
}

/// The JSON document of a stored `.gltf`.
fn json_of_text(json: &[u8]) -> Value {
    serde_json::from_slice(json).unwrap()
}

#[test]
fn a_buffer_file_that_held_images_is_rewritten_or_dropped() {
    let wood = embedded_image(CRATE);
    // One buffer file holds the geometry and the image: it is rewritten without the image.
    let (mut d, bin) = model(&wood, "house", false, false);
    d["buffers"] = json!([house_bin(bin.len())]);
    let text = serde_json::to_vec(&d).unwrap();
    let found = |r: &str| (r == "house.bin").then(|| bin.clone());
    let i = import(&text, &found, "models/house.gltf").unwrap();
    assert_eq!(i.textures[0].bytes, wood);
    assert_eq!(
        files_of(&i),
        [(
            "house.bin".to_string(),
            "models/house.bin".to_string(),
            Some(36 + 6)
        )]
    );
    let doc = json_of_text(&i.model);
    assert_eq!(doc["buffers"][0]["byteLength"], 36 + 6);
    assert_eq!(doc["bufferViews"].as_array().unwrap().len(), 2);
    assert_eq!(read_back(&i, "models/", &found).unwrap(), i.summary);
    // Two buffer files, one with the geometry and one with only the image: the second goes.
    let (mut d, bin) = model(&wood, "two", false, false);
    let geometry = bin[..bin.len() - wood.len() - (4 - wood.len() % 4) % 4].to_vec();
    let image = bin[bin.len() - wood.len() - (4 - wood.len() % 4) % 4..].to_vec();
    let views = d["bufferViews"].as_array().unwrap().clone();
    let image_len = views[2]["byteLength"].as_u64().unwrap();
    d["buffers"] = json!([
        {"uri": "geo.bin", "byteLength": geometry.len()},
        {"uri": "tex.bin", "byteLength": image.len()}
    ]);
    d["bufferViews"] = json!([
        {"buffer": 0, "byteOffset": views[0]["byteOffset"].clone(), "byteLength": views[0]["byteLength"].clone(), "target": 34962},
        {"buffer": 0, "byteOffset": views[1]["byteOffset"].clone(), "byteLength": views[1]["byteLength"].clone(), "target": 34963},
        {"buffer": 1, "byteOffset": 0, "byteLength": image_len}
    ]);
    let found = |r: &str| {
        (r == "geo.bin")
            .then(|| geometry.clone())
            .or((r == "tex.bin").then(|| image.clone()))
    };
    let i = import(&serde_json::to_vec(&d).unwrap(), &found, "models/two.gltf").unwrap();
    assert_eq!(
        files_of(&i),
        [("geo.bin".to_string(), "models/geo.bin".to_string(), None)]
    );
    let doc = json_of_text(&i.model);
    assert_eq!(
        doc["buffers"],
        json!([{"uri": "geo.bin", "byteLength": geometry.len()}])
    );
    assert_eq!(doc["bufferViews"][0]["buffer"], 0);
    assert_eq!(doc["accessors"][0]["bufferView"], 0);
    assert_eq!(read_back(&i, "models/", &found).unwrap(), i.summary);
}
