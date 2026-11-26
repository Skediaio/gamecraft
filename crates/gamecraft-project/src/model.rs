//! What a host needs to know of a model asset without drawing it: its triangle count, its
//! local bounding box and its collision parts (for a solid model in the simulation), and the
//! files a `.gltf` refers to.
//!
//! **Collision parts** ([`Collider`]): the boxes a model with the `mesh` collider collides as,
//! in the asset's own axes. A glTF file may carry its collision geometry as nodes named
//! `*_collider` or `*-collider` ([`crate::lod::is_collider_name`]; the letters in any case, on
//! the node or on its mesh): their meshes are the parts, one box per node, and nothing of them
//! is drawn, counted or measured in the bounds. A file without such nodes gets one box per mesh of the
//! finest level that draws, in the entity's frame (the smallest box around each mesh's
//! vertices where its node places them). More parts than [`MAX_MODEL_PARTS`] fall back to the
//! bounding box, with a sentence.
//!
//! Read with the `gltf` crate's parser (no import feature: the bytes are given). The bounding box
//! is computed in `f64` with plain multiplications and additions over the node transforms the
//! file gives, so it is the same bits on every platform: the simulation's state hash depends on
//! it.

use crate::lod::{self, colliders, levels_of};
pub use crate::lod::{COLLIDER_SUFFIXES, is_collider_name};
use gamecraft_world::{ModelBounds, ModelBox, is_asset_path};
use std::collections::BTreeMap;

/// Deepest chain of nodes followed below a scene's roots.
pub const MAX_NODE_DEPTH: usize = 64;
/// Most collision parts a model has before its bounding box is its collider instead: a
/// character or a ray tests them all.
pub const MAX_MODEL_PARTS: usize = 256;
/// Most node visits while walking a scene (a node used by many parents counts each time).
pub const MAX_NODE_VISITS: usize = 100_000;
/// Most vertex positions read from one model.
pub const MAX_MODEL_VERTICES: u64 = 20_000_000;

/// What [`inspect`] found.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelSummary {
    /// Triangles of the finest level of detail that draws (of the triangle, strip and fan
    /// primitives the scene's nodes draw there); [`ModelSummary::lod`] has the other levels.
    pub triangles: u64,
    /// The local bounding box of the vertices of the finest level that draws, node
    /// transforms applied: the box the simulation collides with and rays hit.
    pub bounds: ModelBounds,
    /// Meshes in the file.
    pub meshes: usize,
    /// Materials in the file.
    pub materials: usize,
    /// Images in the file.
    pub images: usize,
    /// The levels of detail: those that draw with their triangles, what the file says of
    /// the distances ([`crate::lod::LodOverride`]: `asset.extras.gamecraft`, else
    /// `MSFT_screencoverage`) and what is broken in them. One level for a model without any.
    pub lod: crate::lod::Levels,
    /// How the levels were found; `None` for a model with one level.
    pub lod_source: Option<crate::lod::LodSource>,
    /// The collision parts of the `mesh` collider, and where they come from.
    pub collider: Collider,
    /// The sockets of the model (its glTF nodes `socket_<name>`) with their frames in the
    /// asset's own axes, by name: where attachments mount ([`gamecraft_world::attachments`]).
    pub sockets: BTreeMap<String, gamecraft_world::Socket>,
}

/// Where a model's collision parts come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColliderSource {
    /// The file's `*_collider` (or `*-collider`) nodes, one box each.
    Colliders,
    /// One box per mesh of the finest level that draws (the file has no collider nodes).
    Meshes,
    /// The bounding box alone: the model has more meshes than [`MAX_MODEL_PARTS`].
    Box,
}

impl ColliderSource {
    /// Its name in `asset.list`.
    pub fn name(self) -> &'static str {
        match self {
            ColliderSource::Colliders => "colliders",
            ColliderSource::Meshes => "meshes",
            ColliderSource::Box => "box",
        }
    }
}

/// What a model with the `mesh` collider collides as: boxes in the asset's own axes, placed by
/// the entity as its bounding box is.
#[derive(Clone, Debug, PartialEq)]
pub struct Collider {
    /// Where the parts come from.
    pub source: ColliderSource,
    /// The parts, in the file's node order, each axis-aligned in the entity's frame (a mesh
    /// turned in the file gets the smallest box around it there). One part for a model with
    /// one mesh: its bounding box, the collider every model had before.
    pub parts: Vec<ModelBox>,
    /// Why the parts are the bounding box (too many meshes), as a sentence; `None` otherwise.
    pub note: Option<String>,
}

impl Collider {
    /// The bounding box alone.
    fn the_box(bounds: &ModelBounds, note: Option<String>) -> Collider {
        Collider {
            source: ColliderSource::Box,
            parts: vec![box_of(bounds.min, bounds.max)],
            note,
        }
    }
}

/// The box from `min` to `max`.
fn box_of(min: [f64; 3], max: [f64; 3]) -> ModelBox {
    let (mut center, mut half) = ([0.0; 3], [0.0; 3]);
    for k in 0..3 {
        let (Some(lo), Some(hi)) = (min.get(k), max.get(k)) else {
            continue;
        };
        if let (Some(c), Some(h)) = (center.get_mut(k), half.get_mut(k)) {
            *c = (lo + hi) / 2.0;
            *h = (hi - lo) / 2.0;
        }
    }
    ModelBox {
        center,
        half,
        yaw_deg: 0.0,
    }
}

impl ModelSummary {
    /// The size the screen-size rule follows: the extent of the bounds across, metres.
    pub fn size_m(&self) -> f64 {
        crate::lod::size_across(self.bounds.min, self.bounds.max)
    }
}

/// The files a glTF document refers to by relative URI (its buffers and images), in order, as
/// the document writes them: paths relative to the document's folder, which may climb with `..`
/// (a model refers to the pack's texture store that way; [`resolve_uri`] turns them into paths
/// inside `assets/`). `data:` URIs are left out. An error when the text is not a glTF document or
/// a URI is absolute, has a scheme, or is not a plain relative path.
pub fn references(gltf_json: &[u8]) -> Result<Vec<String>, String> {
    let root = gltf::json::Root::from_slice(gltf_json)
        .map_err(|e| format!("not a valid glTF document: {e}"))?;
    let mut out = Vec::new();
    let uris = root
        .buffers
        .iter()
        .filter_map(|b| b.uri.as_deref())
        .chain(root.images.iter().filter_map(|i| i.uri.as_deref()));
    for uri in uris {
        if uri.starts_with("data:") {
            continue;
        }
        let path = decode_uri(uri)?;
        if !out.contains(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

/// A relative URI as a path: percent escapes decoded, no scheme, no leading `/`, no `\`, no
/// empty part. `.` and `..` parts are kept for [`resolve_uri`] to resolve.
pub fn decode_uri(uri: &str) -> Result<String, String> {
    let bad = || {
        format!(
            "the URI {uri:?} is not a plain relative path; a model refers to files beside it \
             (\"crate.bin\", \"textures/wood.png\", \"../textures/wood.png\")"
        )
    };
    let mut bytes = Vec::with_capacity(uri.len());
    let mut it = uri.bytes();
    while let Some(b) = it.next() {
        if b == b'%' {
            let hex = [it.next().ok_or_else(bad)?, it.next().ok_or_else(bad)?];
            let text = std::str::from_utf8(&hex).map_err(|_| bad())?;
            bytes.push(u8::from_str_radix(text, 16).map_err(|_| bad())?);
        } else {
            bytes.push(b);
        }
    }
    let path = String::from_utf8(bytes).map_err(|_| bad())?;
    let plain = !path.is_empty()
        && !path.contains(':')
        && !path.starts_with('/')
        && !path.contains('\\')
        && path.split('/').all(|p| !p.is_empty());
    if plain { Ok(path) } else { Err(bad()) }
}

/// The file a glTF document in the folder `folder` of a pack's `assets/` (`models/`, empty at
/// the top) names by `uri`: the URI decoded, `.` and `..` parts resolved against the folder, as a
/// plain path inside `assets/` (`models/crate.glb` + `../textures/0abc.png` gives
/// `textures/0abc.png`). An error, never a path, for a URI that is not plain or that climbs out
/// of `assets/`.
pub fn resolve_uri(folder: &str, uri: &str) -> Result<String, String> {
    let path = decode_uri(uri)?;
    let mut parts: Vec<&str> = Vec::new();
    for part in folder.split('/').chain(path.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!(
                        "the URI {uri:?} of a model in `{folder}` leads outside the pack's assets/ \
                         folder; a model refers to files inside it"
                    ));
                }
            }
            part => parts.push(part),
        }
    }
    let inside = parts.join("/");
    if !is_asset_path(&inside) {
        return Err(format!(
            "the URI {uri:?} of a model in `{folder}` does not name a plain file inside assets/ \
             (ASCII letters, digits, `-`, `_`, `.` and `/`); rename the file"
        ));
    }
    Ok(inside)
}

/// The folder part of an asset path (`models/crate.gltf` -> `models/`), empty at the top.
pub fn folder_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => path.get(..=i).unwrap_or(""),
        None => "",
    }
}

/// Reads a `.glb` or `.gltf` model: `resource(path)` gives the bytes of a file it refers to
/// (a path relative to the model's folder, from [`references`]). Counts the triangles of each
/// level of detail ([`crate::lod`]: one for a model without any) and measures the bounding
/// box of the finest level that draws, over the default scene (else the first). An error,
/// never a panic, for a file that is
/// not glTF, a buffer that is missing or too short, an index past the vertices, a scene
/// without triangles or a model past the limits.
pub fn inspect(
    bytes: &[u8],
    resource: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<ModelSummary, String> {
    let (document, blob) = open(bytes)?;
    let doc = &document;
    let plan = crate::lod::plan(doc, split_glb(bytes)?.0);
    let sockets = crate::lod::sockets(doc)?;
    let mut buffers: Vec<Option<Vec<u8>>> = Vec::new();
    for buffer in doc.buffers() {
        let data =
            match buffer.source() {
                gltf::buffer::Source::Bin => blob.clone(),
                gltf::buffer::Source::Uri(uri) if uri.starts_with("data:") => {
                    return Err(
                        "a buffer is embedded as a data: URI, which is not supported; save the \
                     model as .glb or with its buffers as files"
                            .into(),
                    );
                }
                gltf::buffer::Source::Uri(uri) => {
                    let path = decode_uri(uri)?;
                    Some(resource(&path).ok_or_else(|| {
                        format!("the buffer file {path:?} it refers to is missing")
                    })?)
                }
            };
        if let Some(d) = &data
            && (d.len() as u64) < buffer.length() as u64
        {
            return Err(format!(
                "buffer {} holds {} bytes, but the file says {}",
                buffer.index(),
                d.len(),
                buffer.length()
            ));
        }
        buffers.push(data);
    }
    let data = |b: gltf::Buffer| buffers.get(b.index()).and_then(|d| d.as_deref());
    if doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .is_none()
    {
        return Err("the model has no scene".into());
    }
    let mut vertices: u64 = 0;
    let mut level_triangles = vec![0u64; plan.levels];
    let mut level_lo = vec![[f64::INFINITY; 3]; plan.levels];
    let mut level_hi = vec![[f64::NEG_INFINITY; 3]; plan.levels];
    // The collision parts: by node, the box of its mesh's vertices where the node places them.
    let from_colliders = !plan.colliders.is_empty();
    let mut part_lo: BTreeMap<usize, [f64; 3]> = BTreeMap::new();
    let mut part_hi: BTreeMap<usize, [f64; 3]> = BTreeMap::new();
    for visit in crate::lod::walk(doc, &plan)? {
        let p = visit.primitive;
        let count_of = |n: u64| match p.mode() {
            gltf::mesh::Mode::Triangles => n / 3,
            gltf::mesh::Mode::TriangleStrip | gltf::mesh::Mode::TriangleFan => n.saturating_sub(2),
            _ => 0,
        };
        check_primitive(&p)?;
        let r = p.reader(data);
        let Some(positions) = r.read_positions() else {
            continue;
        };
        let positions: Vec<[f32; 3]> = positions.collect();
        let n = positions.len() as u64;
        vertices = vertices.saturating_add(n);
        if vertices > MAX_MODEL_VERTICES {
            return Err(format!(
                "the model has more than {MAX_MODEL_VERTICES} vertices to read"
            ));
        }
        let drawn = match r.read_indices() {
            Some(indices) => {
                let mut count = 0u64;
                for i in indices.into_u32() {
                    if u64::from(i) >= n {
                        return Err(format!(
                            "an index {i} points past the {n} vertices of a primitive"
                        ));
                    }
                    count += 1;
                }
                count_of(count)
            }
            None => count_of(n),
        };
        if drawn == 0 {
            continue;
        }
        for k in levels_of(visit.drawn_at) {
            if let (Some(slot), Some(lo), Some(hi)) = (
                level_triangles.get_mut(k),
                level_lo.get_mut(k),
                level_hi.get_mut(k),
            ) {
                *slot = slot.saturating_add(drawn);
                crate::lod::extend_bounds(&visit.world, &positions, lo, hi);
            }
        }
        // Without collider nodes every mesh of the finest level is a part.
        if !from_colliders && visit.drawn_at & 1 == 1 {
            let lo = part_lo.entry(visit.node).or_insert([f64::INFINITY; 3]);
            let hi = part_hi.entry(visit.node).or_insert([f64::NEG_INFINITY; 3]);
            lod::extend_bounds(&visit.world, &positions, lo, hi);
        }
    }
    if from_colliders {
        for visit in colliders(doc, &plan)? {
            let p = visit.primitive;
            check_primitive(&p)?;
            let r = p.reader(data);
            let Some(positions) = r.read_positions() else {
                continue;
            };
            let positions: Vec<[f32; 3]> = positions.collect();
            let lo = part_lo.entry(visit.node).or_insert([f64::INFINITY; 3]);
            let hi = part_hi.entry(visit.node).or_insert([f64::NEG_INFINITY; 3]);
            lod::extend_bounds(&visit.world, &positions, lo, hi);
        }
    }
    let lod = plan.finish(&level_triangles);
    // The box of the finest level that draws is the model's: what most engines collide with.
    let (Some(&finest), Some(triangles)) = (lod.drawn.first(), lod.triangles.first().copied())
    else {
        return Err("the model's scene has no triangles".into());
    };
    let (Some(min), Some(max)) = (level_lo.get(finest), level_hi.get(finest)) else {
        return Err("the model's scene has no triangles".into());
    };
    let bounds = ModelBounds {
        min: *min,
        max: *max,
    };
    if !bounds.is_valid() {
        return Err("the model's vertices are not finite numbers".into());
    }
    // A node's mesh is a part when its vertices gave a usable box.
    let parts: Vec<ModelBox> = part_lo
        .iter()
        .filter_map(|(node, lo)| Some((lo, part_hi.get(node)?)))
        .filter(|(lo, hi)| {
            lo.iter()
                .zip(hi.iter())
                .all(|(l, h)| l.is_finite() && h.is_finite() && l <= h)
        })
        .map(|(lo, hi)| box_of(*lo, *hi))
        .collect();
    let collider = if parts.len() > MAX_MODEL_PARTS {
        Collider::the_box(
            &bounds,
            Some(format!(
                "the model has {} collision meshes, more than {MAX_MODEL_PARTS}; its bounding box \
                 is its collider",
                parts.len()
            )),
        )
    } else if parts.is_empty() {
        // No mesh with a position: the box is the collider, as for every model before.
        Collider::the_box(&bounds, None)
    } else {
        Collider {
            source: if from_colliders {
                ColliderSource::Colliders
            } else {
                ColliderSource::Meshes
            },
            parts,
            note: None,
        }
    };
    Ok(ModelSummary {
        triangles,
        bounds,
        collider,
        sockets,
        meshes: doc.meshes().len(),
        materials: doc.materials().len(),
        images: doc.images().len(),
        lod,
        lod_source: plan.source,
    })
}

/// The URI a model in the folder `folder` of a pack's `assets/` writes for the file `path`
/// inside `assets/`: `../` for every folder of its own, then the path (`models/` +
/// `textures/0abc.png` gives `../textures/0abc.png`). [`resolve_uri`] turns it back into `path`.
pub fn relative_uri(folder: &str, path: &str) -> String {
    let mut out = String::with_capacity(folder.len() + path.len() + 3);
    for _ in 0..folder.matches('/').count() {
        out.push_str("../");
    }
    out.push_str(path);
    out
}

/// The length of the JSON chunk of a GLB file from its first 20 bytes and the file's length:
/// a reader reads only that chunk ([`split_glb`] reads the whole file). The same checks as
/// [`split_glb`] on the header, and the chunk must lie inside the file.
pub fn glb_json_len(head: &[u8], file_len: u64) -> Result<usize, String> {
    let word = |at: usize| -> Option<u32> {
        let b = head.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes([
            *b.first()?,
            *b.get(1)?,
            *b.get(2)?,
            *b.get(3)?,
        ]))
    };
    if head.get(..4) != Some(b"glTF".as_slice()) {
        return Err("the file is not a GLB model (it does not start with glTF)".into());
    }
    if word(4) != Some(2) {
        return Err("the GLB file is not glTF version 2".into());
    }
    let total = word(8).ok_or("the GLB file is cut short")? as u64;
    if total > file_len {
        return Err("the GLB file is cut short or its length is wrong".into());
    }
    let json_len = word(12).ok_or("the GLB file is cut short")? as u64;
    if head.get(16..20) != Some(b"JSON".as_slice()) {
        return Err("the GLB file's first chunk is not JSON".into());
    }
    let end = 20u64
        .checked_add(json_len)
        .ok_or("the GLB file is cut short or its lengths are wrong")?;
    if end > total {
        return Err("the GLB file is cut short or its lengths are wrong".into());
    }
    usize::try_from(json_len)
        .map_err(|_| "the GLB file's JSON chunk is too large for this platform".into())
}

/// The JSON chunk and the binary chunk of a GLB file, or the whole of a `.gltf` document (no
/// binary chunk). Every length is checked against the bytes there are.
pub fn split_glb(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>), String> {
    if !bytes.starts_with(b"glTF") {
        return Ok((bytes, None));
    }
    let word = |at: usize| -> Option<u32> {
        let b = bytes.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes([
            *b.first()?,
            *b.get(1)?,
            *b.get(2)?,
            *b.get(3)?,
        ]))
    };
    let short = || "the GLB file is cut short or its lengths are wrong".to_string();
    if word(4) != Some(2) {
        return Err("the GLB file is not glTF version 2".into());
    }
    let total = word(8).ok_or_else(short)? as usize;
    let bytes = bytes.get(..total).ok_or_else(short)?;
    let json_len = word(12).ok_or_else(short)? as usize;
    if bytes.get(16..20) != Some(b"JSON".as_slice()) {
        return Err("the GLB file's first chunk is not JSON".into());
    }
    let json_end = 20usize.checked_add(json_len).ok_or_else(short)?;
    let json = bytes.get(20..json_end).ok_or_else(short)?;
    if json_end >= bytes.len() {
        return Ok((json, None));
    }
    let bin_len = word(json_end).ok_or_else(short)? as usize;
    let tag_end = json_end.checked_add(8).ok_or_else(short)?;
    if bytes.get(json_end + 4..tag_end) != Some(b"BIN\0".as_slice()) {
        return Err("the GLB file's second chunk is not BIN".into());
    }
    let bin_end = tag_end.checked_add(bin_len).ok_or_else(short)?;
    let bin = bytes.get(tag_end..bin_end).ok_or_else(short)?;
    Ok((json, Some(bin)))
}

/// Reads and validates a `.glb` or `.gltf` file into its document and its binary chunk.
///
/// The `gltf` crate's own reader can panic on hostile files (a GLB header shorter than its
/// length field, a `POSITION` index past the accessors during validation), so the GLB is split
/// here and those indices are checked before the crate validates the rest.
pub fn open(bytes: &[u8]) -> Result<(gltf::Document, Option<Vec<u8>>), String> {
    let (json, bin) = split_glb(bytes)?;
    let root = gltf::json::Root::from_slice(json).map_err(|e| format!("not a glTF model: {e}"))?;
    for mesh in &root.meshes {
        for p in &mesh.primitives {
            let attributes = p.attributes.values().chain(
                p.targets
                    .iter()
                    .flatten()
                    .flat_map(|t| [&t.positions, &t.normals, &t.tangents])
                    .flatten(),
            );
            for index in attributes.chain(p.indices.iter()) {
                if root.accessors.get(index.value()).is_none() {
                    return Err(format!(
                        "a primitive refers to accessor {}, but the file has {}",
                        index.value(),
                        root.accessors.len()
                    ));
                }
            }
        }
    }
    let document =
        gltf::Document::from_json(root).map_err(|e| format!("not a valid glTF model: {e}"))?;
    Ok((document, bin.map(<[u8]>::to_vec)))
}

/// Checks an accessor before the `gltf` crate's reader reads it (its reader can panic on an
/// empty accessor, a stride shorter than an element or a sparse accessor): one of the
/// `dimensions` and `types`, not sparse, at least one element, a buffer view, a stride of at
/// least an element, and every element inside the view.
pub fn check_accessor(
    accessor: &gltf::Accessor,
    what: &str,
    dimensions: &[gltf::accessor::Dimensions],
    types: &[gltf::accessor::DataType],
) -> Result<(), String> {
    let bad = |why: String| Err(format!("the {what} accessor {} {why}", accessor.index()));
    if !dimensions.contains(&accessor.dimensions()) || !types.contains(&accessor.data_type()) {
        return bad(format!(
            "is {:?} of {:?}, which is not allowed here",
            accessor.dimensions(),
            accessor.data_type()
        ));
    }
    if accessor.sparse().is_some() {
        return bad("is sparse, which is not supported".into());
    }
    if accessor.count() == 0 {
        return bad("is empty".into());
    }
    let Some(view) = accessor.view() else {
        return bad("has no buffer view".into());
    };
    let size = accessor.size();
    let stride = view.stride().unwrap_or(size);
    if size == 0 || stride < size {
        return bad(format!(
            "has a stride of {stride} for elements of {size} bytes"
        ));
    }
    let end = (accessor.count() - 1)
        .checked_mul(stride)
        .and_then(|n| n.checked_add(size))
        .and_then(|n| n.checked_add(accessor.offset()));
    if end.is_none_or(|e| e > view.length()) {
        return bad("reaches past its buffer view".into());
    }
    let view_end = view.offset().checked_add(view.length());
    if view_end.is_none_or(|e| e > view.buffer().length()) {
        return bad("lies in a view that reaches past its buffer".into());
    }
    Ok(())
}

/// Checks the accessors of a primitive that a reader of positions, normals, the first texture
/// coordinates and indices reads ([`check_accessor`]).
pub fn check_primitive(p: &gltf::Primitive) -> Result<(), String> {
    use gltf::Semantic;
    use gltf::accessor::{DataType as D, Dimensions as V};
    for (semantic, accessor) in p.attributes() {
        match semantic {
            Semantic::Positions => check_accessor(&accessor, "POSITION", &[V::Vec3], &[D::F32])?,
            Semantic::Normals => check_accessor(&accessor, "NORMAL", &[V::Vec3], &[D::F32])?,
            Semantic::TexCoords(0) => check_accessor(
                &accessor,
                "TEXCOORD_0",
                &[V::Vec2],
                &[D::F32, D::U8, D::U16],
            )?,
            _ => {}
        }
    }
    if let Some(indices) = p.indices() {
        check_accessor(&indices, "index", &[V::Scalar], &[D::U8, D::U16, D::U32])?;
    }
    Ok(())
}

/// A column-major 4 x 4 matrix, as glTF gives them.
pub(crate) const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

pub(crate) fn to_f64(m: &[[f32; 4]; 4]) -> [f64; 16] {
    let mut out = [0.0; 16];
    for (dst, src) in out.iter_mut().zip(m.iter().flatten()) {
        *dst = f64::from(*src);
    }
    out
}

/// The element at column `c`, row `r`.
fn at(m: &[f64; 16], c: usize, r: usize) -> f64 {
    m.get(c * 4 + r).copied().unwrap_or(0.0)
}

/// `a * b`, column-major, in a fixed order of operations.
pub(crate) fn mul(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut out = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            let v = at(a, 0, r) * at(b, c, 0)
                + at(a, 1, r) * at(b, c, 1)
                + at(a, 2, r) * at(b, c, 2)
                + at(a, 3, r) * at(b, c, 3);
            if let Some(slot) = out.get_mut(c * 4 + r) {
                *slot = v;
            }
        }
    }
    out
}

/// The point `p` transformed by `m` (an affine transform: the last row is ignored).
pub(crate) fn apply(m: &[f64; 16], p: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = p;
    let row = |r: usize| at(m, 0, r) * x + at(m, 1, r) * y + at(m, 2, r) * z + at(m, 3, r);
    [row(0), row(1), row(2)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_are_plain_relative_paths() {
        assert_eq!(decode_uri("a%20b.bin").unwrap(), "a b.bin");
        assert_eq!(decode_uri("tex/wood.png").unwrap(), "tex/wood.png");
        for bad in ["/x.bin", "http://x/y.bin", "a//b", "%zz", "%2", "c:\\x", ""] {
            assert!(decode_uri(bad).is_err(), "{bad}");
        }
        // `.` and `..` are kept for `resolve_uri`.
        assert_eq!(decode_uri("../x.bin").unwrap(), "../x.bin");
        assert_eq!(folder_of("models/a.gltf"), "models/");
        assert_eq!(folder_of("a.gltf"), "");
    }

    #[test]
    fn uris_resolve_against_the_model_s_folder_and_stay_inside_assets() {
        assert_eq!(
            resolve_uri("models/", "crate.bin").unwrap(),
            "models/crate.bin"
        );
        assert_eq!(
            resolve_uri("models/", "../textures/0abc.png").unwrap(),
            "textures/0abc.png"
        );
        assert_eq!(
            resolve_uri("models/houses/", "../../textures/0abc.png").unwrap(),
            "textures/0abc.png"
        );
        assert_eq!(
            resolve_uri("", "textures/0abc.png").unwrap(),
            "textures/0abc.png"
        );
        assert_eq!(
            resolve_uri("models/", "a/../b/c.bin").unwrap(),
            "models/b/c.bin"
        );
        for (folder, uri) in [
            ("", "../x.png"),
            ("models/", "../../x.png"),
            ("models/", ".."),
            ("models/", "/x.png"),
            ("models/", "my crate.bin"),
        ] {
            assert!(resolve_uri(folder, uri).is_err(), "{folder} {uri}");
        }
        // The URI a model writes for a texture of the store comes back as its path.
        for (folder, path) in [
            ("models/", "textures/0abc.png"),
            ("models/houses/", "textures/0abc.jpg"),
            ("", "textures/0abc.jpg"),
        ] {
            assert_eq!(
                resolve_uri(folder, &relative_uri(folder, path)).unwrap(),
                path
            );
        }
    }

    #[test]
    fn the_json_chunk_length_is_read_from_the_header() {
        let crate_: &[u8] = include_bytes!("../../../examples/assets-demo/assets/models/crate.glb");
        let head = &crate_[..20];
        assert_eq!(glb_json_len(head, crate_.len() as u64).unwrap(), 1240);
        assert!(glb_json_len(head, 100).is_err());
        assert!(glb_json_len(&crate_[..19], crate_.len() as u64).is_err());
        assert!(glb_json_len(b"not a glb file at all!", 30).is_err());
    }

    #[test]
    fn matrices_multiply_in_column_major_order() {
        let mut t = IDENTITY;
        t[12] = 2.0; // translate x by 2
        let mut s = IDENTITY;
        s[0] = 3.0; // scale x by 3
        // Scale first, then translate.
        assert_eq!(apply(&mul(&t, &s), [1.0, 0.0, 0.0]), [5.0, 0.0, 0.0]);
    }
}
