//! Small glTF models built in the test: a GLB (or a `.gltf` with its files beside it) with one
//! textured triangle, its image embedded or beside it.

use serde_json::{Value, json};

/// The bytes of the first image embedded in the demo GLB `glb`.
pub fn embedded_image(glb: &[u8]) -> Vec<u8> {
    let word = |at: usize| -> usize {
        u32::from_le_bytes([glb[at], glb[at + 1], glb[at + 2], glb[at + 3]]) as usize
    };
    assert_eq!(&glb[..4], b"glTF", "not a GLB");
    let json_len = word(12);
    let doc: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    let view = doc["images"][0]["bufferView"].as_u64().unwrap() as usize;
    let view = &doc["bufferViews"][view];
    let at = view["byteOffset"].as_u64().unwrap() as usize;
    let len = view["byteLength"].as_u64().unwrap() as usize;
    let bin = 20 + json_len + 8;
    glb[bin + at..bin + at + len].to_vec()
}

/// One triangle: positions (three `f32` vectors) and indices (three `u16`), as a buffer holds
/// them, each padded to four bytes; with the buffer views over them.
fn triangle() -> (Vec<u8>, Vec<Value>) {
    let mut positions = Vec::new();
    for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        positions.extend_from_slice(&f.to_le_bytes());
    }
    let mut indices = Vec::new();
    for i in [0u16, 1, 2] {
        indices.extend_from_slice(&i.to_le_bytes());
    }
    let mut bin = Vec::new();
    let mut view = |bytes: &[u8], target: u32| -> Value {
        let at = bin.len();
        bin.extend_from_slice(bytes);
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        json!({"buffer": 0, "byteOffset": at, "byteLength": bytes.len(), "target": target})
    };
    let views = vec![view(&positions, 34962), view(&indices, 34963)];
    (bin, views)
}

/// The document of a model with one material whose base colour is image 0, named `name`;
/// `views` are its buffer views.
fn document(views: Vec<Value>, name: &str) -> Value {
    json!({
        "asset": {"version": "2.0"},
        "bufferViews": views,
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
             "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
        ],
        "materials": [{"name": "wall", "pbrMetallicRoughness": {
            "baseColorTexture": {"index": 0}, "metallicFactor": 0.0, "roughnessFactor": 0.9}}],
        "textures": [{"source": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
        "nodes": [{"mesh": 0, "name": name}],
        "scenes": [{"nodes": [0]}],
        "scene": 0
    })
}

/// A GLB from its JSON chunk and its binary chunk.
pub fn glb(json: &[u8], bin: &[u8]) -> Vec<u8> {
    let pad = |n: usize| (4 - n % 4) % 4;
    let total = 12 + 8 + json.len() + pad(json.len()) + 8 + bin.len() + pad(bin.len());
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&((json.len() + pad(json.len())) as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(json);
    out.extend(std::iter::repeat_n(b' ', pad(json.len())));
    out.extend_from_slice(&((bin.len() + pad(bin.len())) as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(bin);
    out.extend(std::iter::repeat_n(0, pad(bin.len())));
    out
}

/// A GLB with one textured triangle whose image is embedded, named `name`.
pub fn glb_with_image(image: &[u8], name: &str) -> Vec<u8> {
    let (mut bin, mut views) = triangle();
    let at = bin.len();
    bin.extend_from_slice(image);
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    views.push(json!({"buffer": 0, "byteOffset": at, "byteLength": image.len()}));
    let mut doc = document(views, name);
    doc["buffers"] = json!([{"byteLength": bin.len()}]);
    doc["images"] = json!([{"bufferView": 2, "mimeType": mime_of(image)}]);
    glb(&serde_json::to_vec(&doc).unwrap(), &bin)
}

/// A `.gltf` document (pretty text) with one textured triangle whose image and buffer are the
/// files `image_uri` and `bin_uri` beside it, named `name`; and its buffer's bytes.
pub fn gltf_with_files(image_uri: &str, bin_uri: &str, name: &str) -> (String, Vec<u8>) {
    let (bin, views) = triangle();
    let mut doc = document(views, name);
    doc["buffers"] = json!([{"uri": bin_uri, "byteLength": bin.len()}]);
    doc["images"] = json!([{"uri": image_uri}]);
    (serde_json::to_string_pretty(&doc).unwrap(), bin)
}

/// The MIME type an image's first bytes say (PNG, else JPEG).
pub fn mime_of(image: &[u8]) -> &'static str {
    if image.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else {
        "image/jpeg"
    }
}
