//! The texture store of a pack: the images of its models, each stored once.
//!
//! Every imported glTF model used to embed its own copy of every image, so a pack of models
//! that share a few textures carried the same JPEGs over and over. An import now extracts the
//! images (embedded in a `.glb`, beside a `.gltf`, in a buffer file, or as `data:` URIs) into
//! the pack's texture store, `assets/textures/<content-hash>.<ext>`, and rewrites the model so
//! that each image refers to that file by a relative URI, as any glTF file refers to a file
//! beside it ([`import`]). A second model with the same image refers to the same file and
//! stores nothing new. The store's files are assets of their own with a licence record each
//! (`assets/textures/<name>.license.json`), so a texture is credited once however many models
//! use it.
//!
//! The content hash is the workspace's own `gamecraft_math::StateHasher` (FNV-1a, 64 bits):
//! deterministic, there without a new dependency, and enough to tell a pack's few hundred
//! textures apart (`docs/decisions.md`, *Shared textures are files of the pack*). A host that
//! finds a texture of the store already there checks its bytes against its name
//! ([`hash_of`]), so a file renamed or replaced by hand is refused, not reused.
//!
//! Models with embedded images (imported before the store existed, or written by hand) load
//! as before: the renderer reads an image from a buffer view or from a URI.

use crate::model::{
    ModelSummary, decode_uri, folder_of, inspect, relative_uri, resolve_uri, split_glb,
};
use gamecraft_math::StateHasher;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The folder of the texture store inside a pack's `assets/`.
pub const TEXTURES_DIR: &str = "textures";

/// A texture of the store: where it lives and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    /// Its path inside `assets/`: `textures/<content-hash>.<png | jpg>`.
    pub path: String,
    /// The encoded image (PNG or JPEG), as the model held it.
    pub bytes: Vec<u8>,
}

/// A model made ready for the store by [`import`].
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    /// The model's file as it is stored: its images refer to the texture store by relative
    /// URIs, and the binary chunk of a `.glb` no longer holds them. A model without images is
    /// stored as it came.
    pub model: Vec<u8>,
    /// What reading the model as it came found (triangles, bounds, ...). The stored file
    /// reads the same.
    pub summary: ModelSummary,
    /// The images it uses, each once, in the order of its `images`.
    pub textures: Vec<Texture>,
    /// The other files it refers to and that are copied beside it (a `.gltf`'s buffers): the
    /// URI as the model writes it, and the path inside `assets/` where the bytes go.
    pub files: Vec<(String, String)>,
}

/// The content hash of a texture's bytes: FNV-1a, 64 bits (`gamecraft_math::StateHasher`).
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut hasher = StateHasher::new();
    hasher.write_bytes(bytes);
    hasher.finish()
}

/// The file name of a texture in the store: the hash in 16 hex digits, then `.` and `ext`.
pub fn texture_name(hash: u64, ext: &str) -> String {
    format!("{hash:016x}.{ext}")
}

/// The path of the texture `name` inside `assets/`: `textures/<name>`.
pub fn texture_path(name: &str) -> String {
    format!("{TEXTURES_DIR}/{name}")
}

/// Whether `path` (inside `assets/`) lies in the texture store.
pub fn is_texture(path: &str) -> bool {
    path.strip_prefix(TEXTURES_DIR)
        .and_then(|rest| rest.strip_prefix('/'))
        .is_some_and(|rest| !rest.is_empty())
}

/// The content hash a texture of the store is named after, when its name is that of a texture
/// [`import`] stored (`<16 hex digits>.<ext>`); `None` for any other name.
pub fn hash_of(name: &str) -> Option<u64> {
    let (digits, _) = name.split_once('.')?;
    if digits.len() != 16 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, 16).ok()
}

/// Makes the model `bytes` (a `.glb` or a `.gltf`) ready to be stored as `to` (its path inside
/// `assets/`): its images become textures of the store, keyed by their content hash, and its
/// document refers to them. `resource(path)` gives the bytes of a file the model refers to by
/// URI, relative to its own folder (its buffers, images beside a `.gltf`).
///
/// The model is read first ([`inspect`]): one that cannot be read is refused, and nothing is
/// rewritten. The file that comes back reads the same as the one that went in (checked here),
/// so a model draws exactly as before. An error, never a panic, for anything broken.
pub fn import(
    bytes: &[u8],
    resource: &dyn Fn(&str) -> Option<Vec<u8>>,
    to: &str,
) -> Result<Imported, String> {
    let summary = inspect(bytes, resource)?;
    let (json, bin) = split_glb(bytes)?;
    let mut doc: Value =
        serde_json::from_slice(json).map_err(|e| format!("not a glTF document: {e}"))?;
    let folder = folder_of(to);
    let images = take_images(&mut doc)?;
    let accessor_views = accessor_views(&doc);
    let mut store = Store::default();
    let mut image_views = BTreeSet::new();
    let mut new_images = Vec::with_capacity(images.len());
    for (i, image) in images.iter().enumerate() {
        let view = image.get("bufferView").and_then(Value::as_u64);
        let (data, mime) = match view {
            Some(view) => {
                let data =
                    view_image(&doc, view, bin, resource).map_err(|e| format!("image {i} {e}"))?;
                let index = usize::try_from(view).map_err(|_| {
                    format!("image {i} refers to bufferView {view}, which is too large")
                })?;
                image_views.insert(index);
                (
                    data,
                    image
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                )
            }
            None => match image.get("uri").and_then(Value::as_str) {
                Some(uri) => uri_image(uri, resource).map_err(|e| format!("image {i} {e}"))?,
                None => return Err(format!("image {i} has neither a `bufferView` nor a `uri`")),
            },
        };
        let ext = extension_of(&data, mime.as_deref()).map_err(|e| format!("image {i} {e}"))?;
        let texture = store.keep(data, ext)?;
        let path = store
            .textures
            .get(texture)
            .map(|t| t.path.as_str())
            .ok_or_else(|| format!("image {i}: the texture store lost its entry (a bug)"))?;
        let mut new_image = image.clone();
        let object = new_image
            .as_object_mut()
            .ok_or_else(|| format!("image {i} is not a JSON object"))?;
        object.remove("bufferView");
        object.remove("mimeType");
        object.insert("uri".into(), Value::String(relative_uri(folder, path)));
        new_images.push(new_image);
    }
    let model = if images.is_empty() {
        bytes.to_vec()
    } else {
        object_mut(&mut doc)?.insert("images".into(), Value::Array(new_images));
        match bin {
            None => serde_json::to_vec(&doc)
                .map_err(|e| format!("the model's document cannot be written: {e}"))?,
            Some(old) if image_views.is_empty() => {
                pack_glb(&serde_json::to_vec(&doc).map_err(write_error)?, Some(old))?
            }
            Some(old) => strip_views(&mut doc, old, &image_views, &accessor_views)?,
        }
    };
    // The other files the model keeps beside it (a .gltf's buffers), placed inside assets/.
    let mut files: Vec<(String, String)> = Vec::new();
    for buffer in doc
        .get("buffers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(uri) = buffer.get("uri").and_then(Value::as_str) else {
            continue;
        };
        let decoded = decode_uri(uri)?;
        let inside = resolve_uri(folder, &decoded)?;
        if is_texture(&inside) {
            return Err(format!(
                "the model keeps a buffer at `{inside}`, inside the pack's texture store \
                 ({TEXTURES_DIR}/); move the file beside the model"
            ));
        }
        if !files.iter().any(|(u, _)| u == &decoded) {
            files.push((decoded, inside));
        }
    }
    // The stored file reads back as it came, with the textures where they are now.
    let stored = |r: &str| -> Option<Vec<u8>> {
        store
            .textures
            .iter()
            .find(|t| resolve_uri(folder, r).is_ok_and(|inside| inside == t.path))
            .map(|t| t.bytes.clone())
            .or_else(|| resource(r))
    };
    let after = inspect(&model, &stored).map_err(|e| {
        format!("the model rewritten for the texture store does not read back: {e}")
    })?;
    if after != summary {
        return Err(
            "the model rewritten for the texture store does not read back as it came (its \
             triangles, bounds or counts differ); nothing is stored"
                .into(),
        );
    }
    Ok(Imported {
        model,
        summary,
        textures: store.textures,
        files,
    })
}

/// The textures found so far, each once by content hash.
#[derive(Default)]
struct Store {
    textures: Vec<Texture>,
    by_hash: BTreeMap<u64, usize>,
}

impl Store {
    /// Keeps `data` (a `ext` image) and gives the index of its texture.
    fn keep(&mut self, data: Vec<u8>, ext: &str) -> Result<usize, String> {
        let hash = content_hash(&data);
        if let Some(at) = self.by_hash.get(&hash) {
            return Ok(*at);
        }
        let at = self.textures.len();
        self.textures.push(Texture {
            path: texture_path(&texture_name(hash, ext)),
            bytes: data,
        });
        self.by_hash.insert(hash, at);
        Ok(at)
    }
}

fn write_error(e: serde_json::Error) -> String {
    format!("the model's document cannot be written: {e}")
}

/// The images of `doc`, taken out of it (it has none left).
fn take_images(doc: &mut Value) -> Result<Vec<Value>, String> {
    match doc.get_mut("images") {
        None => Ok(Vec::new()),
        Some(Value::Array(list)) => Ok(std::mem::take(list)),
        Some(_) => Err("the file's `images` is not a list".into()),
    }
}

fn object_mut(doc: &mut Value) -> Result<&mut Map<String, Value>, String> {
    doc.as_object_mut()
        .ok_or_else(|| "the glTF document is not a JSON object".to_string())
}

fn usize_of(v: u64) -> Result<usize, String> {
    usize::try_from(v).map_err(|_| format!("{v} is too large for this platform"))
}

/// The indices of the buffer views that accessors read (their view, and the views of a sparse
/// accessor's indices and values).
fn accessor_views(doc: &Value) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    let accessors = doc.get("accessors").and_then(Value::as_array);
    for accessor in accessors.into_iter().flatten() {
        let view = |v: &Value| v.get("bufferView").and_then(Value::as_u64);
        let mut add = |v: Option<u64>| {
            if let Some(i) = v.and_then(|v| usize::try_from(v).ok()) {
                out.insert(i);
            }
        };
        add(accessor.get("bufferView").and_then(Value::as_u64));
        for part in ["indices", "values"] {
            add(accessor
                .get("sparse")
                .and_then(|s| s.get(part))
                .and_then(view));
        }
    }
    out
}

/// The bytes of the image in buffer view `bv` of `doc`: in the file's binary chunk, or in a
/// buffer the model refers to as a file.
fn view_image(
    doc: &Value,
    bv: u64,
    bin: Option<&[u8]>,
    resource: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let views = doc
        .get("bufferViews")
        .and_then(Value::as_array)
        .ok_or("lies in a buffer view, but the file has no bufferViews")?;
    let view = usize_of(bv)
        .ok()
        .and_then(|i| views.get(i))
        .ok_or_else(|| {
            format!(
                "refers to bufferView {bv}, but the file has {}",
                views.len()
            )
        })?;
    let number = |k: &str| view.get(k).and_then(Value::as_u64);
    let at = usize_of(number("byteOffset").unwrap_or(0))?;
    let len = usize_of(number("byteLength").ok_or("lies in a buffer view without a byteLength")?)?;
    let end = at.checked_add(len).ok_or("lies outside its buffer")?;
    let buffer = number("buffer").unwrap_or(0);
    let buffers = doc
        .get("buffers")
        .and_then(Value::as_array)
        .ok_or("lies in a buffer view, but the file has no buffers")?;
    let b = usize_of(buffer)
        .ok()
        .and_then(|i| buffers.get(i))
        .ok_or_else(|| {
            format!(
                "lies in buffer {buffer}, but the file has {}",
                buffers.len()
            )
        })?;
    match b.get("uri").and_then(Value::as_str) {
        None => {
            let bin = bin.ok_or(
                "lies in the file's binary chunk, but the file has none (a .gltf keeps its \
                 buffers in files, named by uri)",
            )?;
            Ok(bin
                .get(at..end)
                .ok_or("lies outside the file's binary chunk")?
                .to_vec())
        }
        Some(uri) if uri.starts_with("data:") => {
            Err("lies in a buffer embedded as a data: URI, which is not supported".into())
        }
        Some(uri) => {
            let path = decode_uri(uri)?;
            let bytes = resource(&path)
                .ok_or_else(|| format!("lies in the buffer file {path:?}, which is missing"))?;
            Ok(bytes
                .get(at..end)
                .ok_or("lies outside its buffer file")?
                .to_vec())
        }
    }
}

/// The bytes of an image given by its URI: a `data:` URI (base64), or a file beside the model.
fn uri_image(
    uri: &str,
    resource: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<(Vec<u8>, Option<String>), String> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (meta, payload) = rest
            .split_once(',')
            .ok_or("is a data: URI without a comma before its data")?;
        let mut parts = meta.split(';');
        let mime = parts
            .next()
            .filter(|m| m.starts_with("image/"))
            .map(str::to_string);
        if !parts.any(|p| p == "base64") {
            return Err("is a data: URI that is not base64".into());
        }
        return Ok((decode_base64(payload)?, mime));
    }
    let path = decode_uri(uri)?;
    let data = resource(&path)
        .ok_or_else(|| format!("is the file {path:?}, which is missing beside the model"))?;
    let mime = match path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()) {
        Some(e) if e == "png" => Some("image/png".into()),
        Some(e) if e == "jpg" || e == "jpeg" => Some("image/jpeg".into()),
        _ => None,
    };
    Ok((data, mime))
}

/// The file extension of an image: what its first bytes say (PNG, JPEG), else its MIME type.
fn extension_of(data: &[u8], mime: Option<&str>) -> Result<&'static str, String> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok("png");
    }
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok("jpg");
    }
    match mime {
        Some("image/png") => Ok("png"),
        Some("image/jpeg") => Ok("jpg"),
        other => Err(format!(
            "is neither a PNG nor a JPEG (its MIME type is {}); a glTF image is one of the two",
            other.unwrap_or("not given")
        )),
    }
}

/// Decodes base64 (the standard alphabet, `=` padding optional). An error for any other
/// character or a cut short group.
fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    let value = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let bad = "is a data: URI whose base64 text is not base64".to_string();
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits, mut padded) = (0u32, 0u32, false);
    for c in text.bytes() {
        if c == b'=' {
            padded = true;
            continue;
        }
        if padded {
            return Err(bad);
        }
        acc = (acc << 6) | value(c).ok_or_else(|| bad.clone())?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xFF).unwrap_or(0));
        }
    }
    if bits >= 6 {
        return Err("is a data: URI whose base64 text is cut short".into());
    }
    Ok(out)
}

/// Rewrites the GLB `doc` and its binary chunk `old` without the views that only held images:
/// every other view of the binary chunk is copied into a new chunk, one after the other, and
/// the accessors point at the views' new indices. Gives the new GLB file.
fn strip_views(
    doc: &mut Value,
    old: &[u8],
    image_views: &BTreeSet<usize>,
    accessor_views: &BTreeSet<usize>,
) -> Result<Vec<u8>, String> {
    let views = doc
        .get("bufferViews")
        .and_then(Value::as_array)
        .ok_or("has images in buffer views but no bufferViews")?;
    let chunk_buffer = doc
        .get("buffers")
        .and_then(Value::as_array)
        .and_then(|b| b.first())
        .is_some_and(|b| b.get("uri").is_none());
    let mut kept: Vec<Value> = Vec::with_capacity(views.len());
    let mut remap: Vec<Option<usize>> = Vec::with_capacity(views.len());
    let mut bin: Vec<u8> = Vec::new();
    for (i, view) in views.iter().enumerate() {
        if image_views.contains(&i) && !accessor_views.contains(&i) {
            remap.push(None);
            continue;
        }
        let number = |k: &str| view.get(k).and_then(Value::as_u64);
        let mut copy = view.clone();
        if chunk_buffer && number("buffer").unwrap_or(0) == 0 {
            let at = usize_of(number("byteOffset").unwrap_or(0))?;
            let len = usize_of(number("byteLength").ok_or("has no byteLength")?)?;
            let end = at.checked_add(len).ok_or("reaches past the binary chunk")?;
            let data = old.get(at..end).ok_or("reaches past the binary chunk")?;
            while !bin.len().is_multiple_of(4) {
                bin.push(0);
            }
            copy.as_object_mut()
                .ok_or("is not a JSON object")?
                .insert("byteOffset".into(), Value::from(bin.len()));
            bin.extend_from_slice(data);
        }
        remap.push(Some(kept.len()));
        kept.push(copy);
    }
    let object = object_mut(doc)?;
    if kept.is_empty() {
        object.remove("bufferViews");
        object.remove("buffers");
        let json = serde_json::to_vec(doc).map_err(write_error)?;
        return pack_glb(&json, None);
    }
    remap_accessors(object, &remap)?;
    object.insert("bufferViews".into(), Value::Array(kept));
    let first = object
        .get_mut("buffers")
        .and_then(Value::as_array_mut)
        .and_then(|b| b.first_mut())
        .ok_or("has images in buffer views but no buffers")?
        .as_object_mut()
        .ok_or("has a buffer that is not a JSON object")?;
    first.insert("byteLength".into(), Value::from(bin.len()));
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let json = serde_json::to_vec(doc).map_err(write_error)?;
    pack_glb(&json, Some(&bin))
}

/// Points every accessor of `doc` at the new index of its buffer view (`remap`: old index to
/// new).
fn remap_accessors(doc: &mut Map<String, Value>, remap: &[Option<usize>]) -> Result<(), String> {
    let Some(accessors) = doc.get_mut("accessors").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    let new = |old: u64| -> Result<usize, String> {
        usize_of(old)
            .ok()
            .and_then(|i| remap.get(i).copied().flatten())
            .ok_or_else(|| format!("an accessor reads bufferView {old}, which held only an image"))
    };
    for accessor in accessors {
        let Some(object) = accessor.as_object_mut() else {
            continue;
        };
        let view = object
            .get("bufferView")
            .and_then(Value::as_u64)
            .map(new)
            .transpose()?;
        if let Some(view) = view {
            object.insert("bufferView".into(), Value::from(view));
        }
        for part in ["indices", "values"] {
            let sparse = object
                .get_mut("sparse")
                .and_then(Value::as_object_mut)
                .and_then(|s| s.get_mut(part))
                .and_then(Value::as_object_mut);
            let Some(sparse) = sparse else { continue };
            let view = sparse
                .get("bufferView")
                .and_then(Value::as_u64)
                .map(new)
                .transpose()?;
            if let Some(view) = view {
                sparse.insert("bufferView".into(), Value::from(view));
            }
        }
    }
    Ok(())
}

/// A GLB file from its JSON chunk and its binary chunk (`None` when the model's buffers are
/// files or it has none): each chunk's declared length includes its padding, as the format asks.
pub fn pack_glb(json: &[u8], bin: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let pad = |n: usize| (4 - n % 4) % 4;
    let too_large = || "the model is too large for a GLB file (4 GB)".to_string();
    let chunk_len =
        |n: usize| -> Result<usize, String> { n.checked_add(pad(n)).ok_or_else(too_large) };
    let json_len = chunk_len(json.len())?;
    let bin_len = match bin {
        None => 0,
        Some(b) => 8 + chunk_len(b.len())?,
    };
    let total = 12 + 8 + json_len + bin_len;
    let word = |n: usize| {
        u32::try_from(n)
            .map_err(|_| too_large())
            .map(|w| w.to_le_bytes())
    };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&word(total)?);
    out.extend_from_slice(&word(json_len)?);
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(json);
    out.extend(std::iter::repeat_n(b' ', pad(json.len())));
    if let Some(bin) = bin {
        out.extend_from_slice(&word(chunk_len(bin.len())?)?);
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(bin);
        out.extend(std::iter::repeat_n(0, pad(bin.len())));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_names_come_from_the_hash_and_back() {
        let hash = content_hash(b"not an image");
        let name = texture_name(hash, "png");
        assert_eq!(name.len(), 20);
        assert_eq!(hash_of(&name), Some(hash));
        assert_eq!(hash_of("wood.png"), None);
        assert_eq!(texture_path(&name), format!("textures/{name}"));
        assert!(is_texture("textures/wood.png"));
        assert!(!is_texture("models/wood.png"));
        assert!(!is_texture("textures"));
    }

    #[test]
    fn base64_decodes_and_refuses_what_is_not() {
        assert_eq!(decode_base64("").unwrap(), b"");
        assert_eq!(decode_base64("AQID").unwrap(), b"\x01\x02\x03");
        assert_eq!(decode_base64("AQI=").unwrap(), b"\x01\x02");
        assert_eq!(decode_base64("AQ==").unwrap(), b"\x01");
        assert_eq!(decode_base64("/w").unwrap(), b"\xff");
        assert!(decode_base64("A").is_err());
        assert!(decode_base64("A===").is_err());
        assert!(decode_base64("AQ=D").is_err());
        assert!(decode_base64("A Q").is_err());
        assert!(decode_base64("-_").is_err());
    }

    #[test]
    fn extensions_follow_the_bytes_then_the_mime_type() {
        assert_eq!(extension_of(b"\x89PNG\r\n\x1a\nxx", None).unwrap(), "png");
        assert_eq!(extension_of(b"\xFF\xD8\xFFxx", None).unwrap(), "jpg");
        assert_eq!(extension_of(b"????", Some("image/png")).unwrap(), "png");
        assert_eq!(extension_of(b"????", Some("image/jpeg")).unwrap(), "jpg");
        assert!(extension_of(b"????", Some("image/webp")).is_err());
        assert!(extension_of(b"????", None).is_err());
    }

    #[test]
    fn a_glb_is_packed_as_the_format_asks() {
        let file = pack_glb(b"{}", Some(&[1, 2, 3])).unwrap();
        assert_eq!(&file[..4], b"glTF");
        assert_eq!(&file[20..22], b"{}");
        assert_eq!(&file[22..24], b"  ");
        assert_eq!(&file[24..28], &4u32.to_le_bytes());
        assert_eq!(&file[28..32], b"BIN\0");
        assert_eq!(&file[32..36], &[1, 2, 3, 0]);
        let (json, bin) = split_glb(&file).unwrap();
        assert_eq!(json, b"{}  ");
        assert_eq!(bin, Some([1u8, 2, 3, 0].as_slice()));
        let file = pack_glb(b"{}", None).unwrap();
        assert_eq!(file.len(), 24);
        assert_eq!(split_glb(&file).unwrap().1, None);
    }
}
