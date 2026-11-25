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
//! finds a texture of the store already there compares the file's bytes with the image's
//! ([`Texture::bytes`]), so a file renamed or replaced by hand is refused, not reused.
//!
//! The views that held only images leave the model: a `.glb`'s binary chunk and a rewritten
//! buffer file lose them, a buffer that keeps no view at all goes, and the accessors point at
//! the views' new indices. Anything else that names a buffer view (an extension such as
//! `EXT_meshopt_compression`) stops the import with a sentence, since the engine cannot
//! renumber what it does not know. Models with embedded images (imported before the store
//! existed, or written by hand) load as before: the renderer reads an image from a buffer view
//! or from a URI.

use crate::assets::RECORD_SUFFIX;
use crate::model::{
    ModelSummary, decode_uri, folder_of, inspect, relative_uri, resolve_uri, split_glb,
};
use gamecraft_math::StateHasher;
use gamecraft_world::is_model_path;
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

/// A file a stored model keeps beside it (a `.gltf`'s buffer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFile {
    /// The URI as the model writes it.
    pub uri: String,
    /// Where its bytes go, inside `assets/`.
    pub path: String,
    /// The bytes to write when the import changed the file (a buffer that held images keeps
    /// its geometry and loses them); `None` to copy the file the model came with as it is.
    pub bytes: Option<Vec<u8>>,
}

/// A model made ready for the store by [`import`].
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    /// The model's file as it is stored: its images refer to the texture store by relative
    /// URIs, and no buffer of it holds them. A model without images is stored as it came.
    pub model: Vec<u8>,
    /// What reading the model as it came found (triangles, bounds, ...). The stored file
    /// reads the same.
    pub summary: ModelSummary,
    /// The images it uses, each once, in the order of its `images`.
    pub textures: Vec<Texture>,
    /// The other files it refers to and that go beside it (a `.gltf`'s buffers).
    pub files: Vec<ModelFile>,
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

/// Makes the model `bytes` (a `.glb` or a `.gltf`) ready to be stored as `to` (its path inside
/// `assets/`): its images become textures of the store, keyed by their content hash, and its
/// document refers to them. `resource(path)` gives the bytes of a file the model refers to by
/// URI, relative to its own folder (its buffers, images beside a `.gltf`).
///
/// The model is read first ([`inspect`]): one that cannot be read is refused, and nothing is
/// rewritten. The views that held only images leave the model: a buffer the import can rewrite
/// (the binary chunk of a `.glb`, a buffer file) loses them and is given back as new bytes
/// ([`ModelFile::bytes`]), a buffer that keeps no view goes, and the accessors point at the
/// views' new indices; a document where anything else names a buffer view is refused. The file
/// that comes back reads the same as the one that went in (checked here), so a model draws
/// exactly as before. An error, never a panic, for anything broken.
pub fn import(
    bytes: &[u8],
    resource: &dyn Fn(&str) -> Option<Vec<u8>>,
    to: &str,
) -> Result<Imported, String> {
    let summary = inspect(bytes, resource)?;
    let glb = bytes.starts_with(b"glTF");
    let (json, chunk) = split_glb(bytes)?;
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
                let data = view_image(&doc, view, chunk, resource)
                    .map_err(|e| format!("image {i} {e}"))?;
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
        let path = store.keep(data, ext);
        let mut new_image = image.clone();
        let object = new_image
            .as_object_mut()
            .ok_or_else(|| format!("image {i} is not a JSON object"))?;
        object.remove("bufferView");
        object.remove("mimeType");
        object.insert("uri".into(), Value::String(relative_uri(folder, &path)));
        new_images.push(new_image);
    }
    let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let model = if images.is_empty() {
        bytes.to_vec()
    } else {
        object_mut(&mut doc)?.insert("images".into(), Value::Array(new_images));
        let dropped: BTreeSet<usize> = image_views.difference(&accessor_views).copied().collect();
        let mut chunk_bytes: Option<Vec<u8>> = chunk.map(<[u8]>::to_vec);
        if !dropped.is_empty() {
            refuse_foreign_view_refs(&doc)?;
            let buffers = buffer_bytes(&doc, chunk, resource);
            let rewritten = strip_views(&mut doc, &buffers, &dropped)?;
            // The binary chunk goes with its buffer, when that keeps no view.
            let chunk_stays = doc
                .get("buffers")
                .and_then(Value::as_array)
                .and_then(|b| b.first())
                .is_some_and(|b| b.get("uri").is_none());
            if !chunk_stays {
                chunk_bytes = None;
            }
            for (index, fresh) in rewritten {
                let is_chunk = doc
                    .get("buffers")
                    .and_then(Value::as_array)
                    .and_then(|b| b.get(index))
                    .is_some_and(|b| b.get("uri").is_none());
                if is_chunk {
                    chunk_bytes = Some(fresh);
                } else if let Some(uri) = doc
                    .get("buffers")
                    .and_then(Value::as_array)
                    .and_then(|b| b.get(index))
                    .and_then(|b| b.get("uri"))
                    .and_then(Value::as_str)
                {
                    written.insert(decode_uri(uri)?, fresh);
                }
            }
        }
        let json = serde_json::to_vec(&doc).map_err(write_error)?;
        if glb {
            pack_glb(&json, chunk_bytes.as_deref())?
        } else {
            json
        }
    };
    // The other files the model keeps beside it (a .gltf's buffers), placed inside assets/.
    let mut files: Vec<ModelFile> = Vec::new();
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
        if is_model_path(&inside) {
            return Err(format!(
                "the model keeps a buffer at `{inside}`, which is a model path (it ends in .glb \
                 or .gltf); rename the buffer file"
            ));
        }
        if inside.ends_with(RECORD_SUFFIX) {
            return Err(format!(
                "the model keeps a buffer at `{inside}`, which is the name of a licence record \
                 (`*{RECORD_SUFFIX}`); rename the buffer file"
            ));
        }
        if files.iter().any(|f| f.uri == decoded) {
            continue;
        }
        files.push(ModelFile {
            bytes: written.remove(&decoded),
            path: inside,
            uri: decoded,
        });
    }
    // The stored file reads back as it came, with the textures and buffers where they are now.
    let stored = |r: &str| -> Option<Vec<u8>> {
        store
            .by_hash
            .values()
            .find(|t| resolve_uri(folder, r).is_ok_and(|inside| inside == t.path))
            .map(|t| t.bytes.clone())
            .or_else(|| {
                files
                    .iter()
                    .find(|f| f.uri == r)
                    .and_then(|f| f.bytes.clone())
            })
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
        textures: store.into_textures(),
        files,
    })
}

/// The textures found so far, each once by content hash, in the order they came.
#[derive(Default)]
struct Store {
    by_hash: BTreeMap<u64, Texture>,
    order: Vec<u64>,
}

impl Store {
    /// Keeps `data` (an `ext` image) and gives the path of its texture inside `assets/`.
    fn keep(&mut self, data: Vec<u8>, ext: &str) -> String {
        let hash = content_hash(&data);
        let texture = self.by_hash.entry(hash).or_insert_with(|| {
            self.order.push(hash);
            Texture {
                path: texture_path(&texture_name(hash, ext)),
                bytes: data,
            }
        });
        texture.path.clone()
    }

    /// The textures in the order the model's images named them.
    fn into_textures(mut self) -> Vec<Texture> {
        self.order
            .into_iter()
            .filter_map(|hash| self.by_hash.remove(&hash))
            .collect()
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

/// The bytes of each buffer of `doc` the import can rewrite, by index: the file's binary chunk
/// (buffer 0 when it has no URI) and the buffer files `resource` gives. A buffer that is not
/// here is left as it is.
fn buffer_bytes(
    doc: &Value,
    chunk: Option<&[u8]>,
    resource: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Vec<Option<Vec<u8>>> {
    doc.get("buffers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, b)| match b.get("uri").and_then(Value::as_str) {
            None if i == 0 => chunk.map(<[u8]>::to_vec),
            None => None,
            Some(uri) => decode_uri(uri).ok().and_then(|path| resource(&path)),
        })
        .collect()
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

/// Refuses a document where anything but the accessors (and the images, gone by now) names a
/// buffer view: an extension object (`EXT_meshopt_compression`, `KHR_draco_mesh_compression`,
/// a buffer's fallback view) would keep the index of a view the import drops.
fn refuse_foreign_view_refs(doc: &Value) -> Result<(), String> {
    let mut found = Vec::new();
    walk_view_refs(doc, String::new(), &mut found);
    if found.is_empty() {
        return Ok(());
    }
    let used: Vec<&str> = doc
        .get("extensionsUsed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let used = if used.is_empty() {
        String::new()
    } else {
        format!(" (`extensionsUsed`: {})", used.join(", "))
    };
    Err(format!(
        "its images lie in buffer views the import would drop, but {} name{} a buffer view \
         besides the accessors{used}; the import cannot renumber those: export the model with \
         its images as files beside it, or without that extension",
        found.join(", "),
        if found.len() == 1 { "s" } else { "" }
    ))
}

/// Collects the JSON paths of the keys that name a buffer view outside the places the import
/// renumbers: the view list itself, an accessor's own view and a sparse accessor's two.
fn walk_view_refs(value: &Value, path: String, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                let name = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if key.to_ascii_lowercase().contains("bufferview")
                    && !renumbered_view_ref(&path, key)
                {
                    out.push(name.clone());
                }
                walk_view_refs(v, name, out);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                walk_view_refs(v, format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// Whether the key `key` at `path` is a buffer view reference the import renumbers itself.
fn renumbered_view_ref(path: &str, key: &str) -> bool {
    if path.is_empty() {
        return key == "bufferViews";
    }
    let accessor = path.starts_with("accessors[") && !path.contains('.');
    (accessor || path.ends_with(".sparse.indices") || path.ends_with(".sparse.values"))
        && key == "bufferView"
        && path.starts_with("accessors[")
}

/// Rewrites `doc` without the views in `dropped` (those that held only images). A buffer whose
/// bytes are in `buffers` is rebuilt from the views it keeps, one after the other (4-byte
/// aligned, and its `byteLength` follows); one of those that keeps no view goes, with the
/// binary chunk. Every other buffer stays as it is. The accessors and the kept views point at
/// the new indices. Gives the new bytes of the rewritten buffers by their new index.
fn strip_views(
    doc: &mut Value,
    buffers: &[Option<Vec<u8>>],
    dropped: &BTreeSet<usize>,
) -> Result<BTreeMap<usize, Vec<u8>>, String> {
    let views = doc
        .get("bufferViews")
        .and_then(Value::as_array)
        .cloned()
        .ok_or("has images in buffer views but no bufferViews")?;
    let mut kept: Vec<Value> = Vec::with_capacity(views.len());
    let mut remap: Vec<Option<usize>> = Vec::with_capacity(views.len());
    let buffer_of = |view: &Value| {
        view.get("buffer")
            .and_then(Value::as_u64)
            .and_then(|b| usize::try_from(b).ok())
            .unwrap_or(0)
    };
    let mut fresh: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    // The buffers that had views, and those that lose one (known before any view is kept:
    // a dropped view may come after the kept ones).
    let had_views: BTreeSet<usize> = views.iter().map(buffer_of).collect();
    let lost: BTreeSet<usize> = views
        .iter()
        .enumerate()
        .filter(|(i, _)| dropped.contains(i))
        .map(|(_, view)| buffer_of(view))
        .collect();
    let mut keeps_views: BTreeSet<usize> = BTreeSet::new();
    for (i, view) in views.iter().enumerate() {
        if dropped.contains(&i) {
            remap.push(None);
            continue;
        }
        let number = |k: &str| view.get(k).and_then(Value::as_u64);
        let b = usize_of(number("buffer").unwrap_or(0))
            .map_err(|e| format!("a buffer view names a buffer that {e}"))?;
        let mut copy = view.clone();
        // A buffer that lost a view is rebuilt from the views it keeps, when its bytes are
        // there; any other stays as it is.
        if let Some(old) = buffers
            .get(b)
            .and_then(Option::as_deref)
            .filter(|_| lost.contains(&b))
        {
            let at = usize_of(number("byteOffset").unwrap_or(0))?;
            let len = usize_of(number("byteLength").ok_or("has no byteLength")?)?;
            let end = at.checked_add(len).ok_or("reaches past its buffer")?;
            let data = old.get(at..end).ok_or("reaches past its buffer")?;
            let out = fresh.entry(b).or_default();
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
            copy.as_object_mut()
                .ok_or("is not a JSON object")?
                .insert("byteOffset".into(), Value::from(out.len()));
            out.extend_from_slice(data);
        }
        keeps_views.insert(b);
        remap.push(Some(kept.len()));
        kept.push(copy);
    }
    let object = object_mut(doc)?;
    remap_accessors(object, &remap)?;
    if kept.is_empty() {
        object.remove("bufferViews");
    } else {
        object.insert("bufferViews".into(), Value::Array(kept));
    }
    // The buffers: one that had views and keeps none goes (with the binary chunk); the others
    // stay, the rewritten ones with the length of their new bytes.
    let old_buffers = object
        .get("buffers")
        .and_then(Value::as_array)
        .cloned()
        .ok_or("has images in buffer views but no buffers")?;
    let mut new_buffers: Vec<Value> = Vec::with_capacity(old_buffers.len());
    let mut buffer_remap: Vec<Option<usize>> = Vec::with_capacity(old_buffers.len());
    for (i, mut buffer) in old_buffers.into_iter().enumerate() {
        if had_views.contains(&i) && !keeps_views.contains(&i) {
            buffer_remap.push(None);
            continue;
        }
        if let Some(bytes) = fresh.get(&i) {
            buffer
                .as_object_mut()
                .ok_or("has a buffer that is not a JSON object")?
                .insert("byteLength".into(), Value::from(bytes.len()));
        }
        buffer_remap.push(Some(new_buffers.len()));
        new_buffers.push(buffer);
    }
    for view in object
        .get_mut("bufferViews")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let Some(o) = view.as_object_mut() else {
            continue;
        };
        let Some(b) = o.get("buffer").and_then(Value::as_u64) else {
            continue;
        };
        let new = usize_of(b)
            .ok()
            .and_then(|i| buffer_remap.get(i).copied().flatten())
            .ok_or_else(|| {
                format!("a buffer view reads buffer {b}, which keeps no view and was dropped")
            })?;
        o.insert("buffer".into(), Value::from(new));
    }
    if new_buffers.is_empty() {
        object.remove("buffers");
    } else {
        object.insert("buffers".into(), Value::Array(new_buffers));
    }
    Ok(fresh
        .into_iter()
        .filter(|(b, _)| keeps_views.contains(b))
        .filter_map(|(b, bytes)| {
            buffer_remap
                .get(b)
                .copied()
                .flatten()
                .map(|new| (new, bytes))
        })
        .collect())
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
