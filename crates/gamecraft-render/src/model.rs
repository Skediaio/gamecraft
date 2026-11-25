//! glTF models on the CPU: a `.glb` or `.gltf` read into one merged triangle list per material
//! with the node transforms applied, its PBR materials with the roles of their images, and the
//! images decoded to RGBA8. Uploading is the renderer's job (`renderer.rs`).
//!
//! Ported from VisionCraft's `apps/visioncraft/src/gltf_load.rs` (MIT): the walk over the
//! scene's nodes, the merge per material, the normals made when a file has none and the
//! winding flipped under a mirroring transform, the material fields. Changed here: the file is
//! opened and every accessor checked by `gamecraft_project::model` first (the `gltf` crate's
//! reader can panic on hostile files), a `.gltf` reads its buffers and images from the files
//! beside it, the occlusion texture is read (VisionCraft's library packs it into the
//! metallic-roughness image), the texture samplers' wrap modes are kept, and the walk over the
//! scene is `gamecraft_project::lod::walk`, which tells the levels of detail of every
//! primitive (the same walk as the host's `inspect`, so both read the same levels). Glass is still
//! recognised by its name only: the workspace builds `gltf` without
//! `KHR_materials_transmission`.

use gamecraft_project::lod::{self, Levels, LodSource};
use gamecraft_project::model::{check_primitive, decode_uri, open};
use glam::{Mat3, Mat4, Vec3};

/// Largest side of a texture image, pixels.
pub const MAX_TEXTURE_SIDE: u32 = 8192;
/// Most vertices of one model.
pub const MAX_VERTICES: usize = 4_000_000;

/// How a material treats its alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Alpha {
    /// Alpha is ignored.
    Opaque,
    /// Cut out below the threshold.
    Mask(f32),
    /// Blended.
    Blend,
}

/// A texture slot of a material: the image and whether it repeats outside 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TexRef {
    /// Index into [`ModelData::images`].
    pub image: usize,
    /// Repeat (both wrap modes not clamp-to-edge); otherwise clamp.
    pub repeat: bool,
}

/// A glTF PBR material, as the renderer's material takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMaterial {
    /// Linear RGBA.
    pub base_color: [f32; 4],
    /// Metallic factor, 0 to 1.
    pub metallic: f32,
    /// Roughness factor, 0 to 1.
    pub roughness: f32,
    /// Base colour image (sRGB).
    pub base_image: Option<TexRef>,
    /// Normal map (tangent space).
    pub normal_image: Option<TexRef>,
    /// The normal map's scale.
    pub normal_scale: f32,
    /// Metallic-roughness image: roughness in green, metallic in blue.
    pub metal_rough_image: Option<TexRef>,
    /// Occlusion image: occlusion in red.
    pub occlusion_image: Option<TexRef>,
    /// Linear emissive colour.
    pub emissive: [f32; 3],
    /// `KHR_materials_emissive_strength` (1 when absent).
    pub emissive_strength: f32,
    /// Alpha mode.
    pub alpha: Alpha,
    /// Draw both sides.
    pub double_sided: bool,
    /// Named like glass.
    pub glass: bool,
}

impl Default for ModelMaterial {
    /// glTF's default material.
    fn default() -> Self {
        ModelMaterial {
            base_color: [1.0; 4],
            metallic: 1.0,
            roughness: 1.0,
            base_image: None,
            normal_image: None,
            normal_scale: 1.0,
            metal_rough_image: None,
            occlusion_image: None,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            alpha: Alpha::Opaque,
            double_sided: false,
            glass: false,
        }
    }
}

/// All triangles of one material that the same levels of detail draw, in the model's own
/// axes (the node transforms applied).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prim {
    /// Index into [`ModelData::materials`].
    pub material: usize,
    /// Positions, metres.
    pub positions: Vec<[f32; 3]>,
    /// Unit normals.
    pub normals: Vec<[f32; 3]>,
    /// First texture coordinates.
    pub uvs: Vec<[f32; 2]>,
    /// Triangles, counter-clockwise from outside.
    pub indices: Vec<u32>,
}

/// A model read into plain CPU data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelData {
    /// One triangle list per material and set of levels that draw it. A model with levels of
    /// detail keeps them apart: one list for the geometry of each level, one for the geometry
    /// every level shares ([`ModelData::lods`]).
    pub prims: Vec<Prim>,
    /// The materials (the last one is glTF's default, for primitives without one).
    pub materials: Vec<ModelMaterial>,
    /// Encoded (PNG or JPEG) bytes per glTF image.
    pub images: Vec<Vec<u8>>,
    /// The levels of detail.
    pub lods: ModelLods,
}

impl ModelData {
    /// Triangles of the finest level that draws (every triangle of a model without levels).
    pub fn triangles(&self) -> usize {
        self.triangles_at(0).unwrap_or(0)
    }

    /// Triangles drawn level `level` draws (in the order of [`Levels::drawn`]); `None` when
    /// the model has no such level.
    pub fn triangles_at(&self, level: usize) -> Option<usize> {
        self.lods
            .read
            .triangles
            .get(level)
            .map(|&t| usize::try_from(t).unwrap_or(usize::MAX))
    }
}

/// The levels of detail of a model, as its file says ([`gamecraft_project::lod`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelLods {
    /// The levels that draw with their triangles, what the file says of the distances and
    /// what is broken in it.
    pub read: Levels,
    /// For each prim of [`ModelData::prims`], in the same order: bit `k` set when the prim
    /// is drawn at level `k` of the file (the levels that draw are [`Levels::drawn`]).
    pub drawn_at: Vec<u8>,
    /// The extent of the finest level that draws across its box, metres, over every vertex
    /// the file has there (strips and fans included): the size the screen-size rule follows,
    /// as the host measures it. Not a finite number when the model has no vertices.
    pub size_m: f64,
    /// How the levels were found; `None` for a model with one level.
    pub source: Option<LodSource>,
}

fn smooth_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut n = vec![Vec3::ZERO; positions.len()];
    let p = |i: u32| {
        positions
            .get(i as usize)
            .map_or(Vec3::ZERO, |v| Vec3::from(*v))
    };
    for &[a, b, c] in indices.as_chunks::<3>().0 {
        let f = (p(b) - p(a)).cross(p(c) - p(a));
        for i in [a, b, c] {
            if let Some(v) = n.get_mut(i as usize) {
                *v += f;
            }
        }
    }
    n.into_iter()
        .map(|v| v.normalize_or(Vec3::Y).to_array())
        .collect()
}

/// The texture slot of a glTF texture.
fn tex_ref(t: gltf::Texture) -> TexRef {
    use gltf::texture::WrappingMode::ClampToEdge;
    let s = t.sampler();
    TexRef {
        image: t.source().index(),
        repeat: !(s.wrap_s() == ClampToEdge && s.wrap_t() == ClampToEdge),
    }
}

/// Reads a `.glb` or `.gltf` model. `resource(path)` gives a file it refers to, by its path
/// relative to the model's folder. An error, never a panic, for anything broken.
pub fn parse(
    bytes: &[u8],
    resource: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<ModelData, String> {
    let (doc, blob) = open(bytes)?;
    let mut buffers: Vec<Option<Vec<u8>>> = Vec::new();
    for b in doc.buffers() {
        buffers.push(match b.source() {
            gltf::buffer::Source::Bin => blob.clone(),
            gltf::buffer::Source::Uri(uri) => {
                if uri.starts_with("data:") {
                    return Err("a buffer is a data: URI, which is not supported".into());
                }
                let file = decode_uri(uri)?;
                Some(
                    resource(&file)
                        .ok_or_else(|| format!("the buffer file {file:?} is missing"))?,
                )
            }
        });
    }
    let data = |b: gltf::Buffer| buffers.get(b.index()).and_then(|d| d.as_deref());

    let mut out = ModelData::default();
    for img in doc.images() {
        let bytes = match img.source() {
            gltf::image::Source::View { view, .. } => {
                let buf = data(view.buffer()).ok_or("an image has no data")?;
                let end = view
                    .offset()
                    .checked_add(view.length())
                    .ok_or("an image lies outside its buffer")?;
                buf.get(view.offset()..end)
                    .ok_or("an image lies outside its buffer")?
                    .to_vec()
            }
            gltf::image::Source::Uri { uri, .. } => {
                if uri.starts_with("data:") {
                    return Err("an image is a data: URI, which is not supported".into());
                }
                let file = decode_uri(uri)?;
                resource(&file).ok_or_else(|| format!("the image file {file:?} is missing"))?
            }
        };
        out.images.push(bytes);
    }
    for m in doc.materials() {
        let pbr = m.pbr_metallic_roughness();
        let name = m.name().unwrap_or_default().to_lowercase();
        out.materials.push(ModelMaterial {
            base_color: pbr.base_color_factor(),
            metallic: pbr.metallic_factor(),
            roughness: pbr.roughness_factor(),
            base_image: pbr.base_color_texture().map(|i| tex_ref(i.texture())),
            normal_image: m.normal_texture().map(|i| tex_ref(i.texture())),
            normal_scale: m.normal_texture().map_or(1.0, |n| n.scale()),
            metal_rough_image: pbr
                .metallic_roughness_texture()
                .map(|i| tex_ref(i.texture())),
            occlusion_image: m.occlusion_texture().map(|i| tex_ref(i.texture())),
            emissive: m.emissive_factor(),
            emissive_strength: m.emissive_strength().unwrap_or(1.0),
            alpha: match m.alpha_mode() {
                gltf::material::AlphaMode::Opaque => Alpha::Opaque,
                gltf::material::AlphaMode::Mask => Alpha::Mask(m.alpha_cutoff().unwrap_or(0.5)),
                gltf::material::AlphaMode::Blend => Alpha::Blend,
            },
            double_sided: m.double_sided(),
            glass: name.contains("glass"),
        });
    }
    // A primitive without a material uses this one.
    let default_material = out.materials.len();
    out.materials.push(ModelMaterial::default());

    // The levels of detail: which node stands for the levels of its object and which
    // primitive is drawn at which level.
    let plan = lod::plan(&doc, gamecraft_project::model::split_glb(bytes)?.0);
    let mut vertices = 0usize;
    let mut level_lo = vec![[f64::INFINITY; 3]; plan.levels];
    let mut level_hi = vec![[f64::NEG_INFINITY; 3]; plan.levels];
    let mut drawn_at: Vec<u8> = Vec::new();
    for visit in lod::walk(&doc, &plan)? {
        let p = visit.primitive;
        check_primitive(&p)?;
        let r = p.reader(data);
        let Some(pos) = r.read_positions() else {
            continue;
        };
        let pos: Vec<[f32; 3]> = pos.collect();
        vertices = vertices.saturating_add(pos.len());
        if vertices > MAX_VERTICES {
            return Err(format!("the model has more than {MAX_VERTICES} vertices"));
        }
        // The size is measured over every vertex of a level, as the host measures the model
        // (`inspect` counts strips and fans too); only triangle lists are drawn.
        for k in lod::levels_of(visit.drawn_at) {
            if let (Some(lo), Some(hi)) = (level_lo.get_mut(k), level_hi.get_mut(k)) {
                lod::extend_bounds(&visit.world, &pos, lo, hi);
            }
        }
        if p.mode() != gltf::mesh::Mode::Triangles {
            continue;
        }
        let count = u32::try_from(pos.len()).map_err(|_| "too many vertices")?;
        let mut indices: Vec<u32> = match r.read_indices() {
            Some(i) => i.into_u32().collect(),
            None => (0..count).collect(),
        };
        indices.truncate(indices.len() / 3 * 3);
        if indices.iter().any(|&i| i >= count) {
            return Err("an index points past the vertices".into());
        }
        if indices.is_empty() {
            continue;
        }
        let normals: Vec<[f32; 3]> = match r.read_normals() {
            Some(n) => n.collect(),
            None => smooth_normals(&pos, &indices),
        };
        let uvs: Vec<[f32; 2]> = match r.read_tex_coords(0) {
            Some(t) => t.into_f32().collect(),
            None => vec![[0.0; 2]; pos.len()],
        };
        if normals.len() != pos.len() || uvs.len() != pos.len() {
            return Err("vertex attributes differ in length".into());
        }
        let material = p
            .material()
            .index()
            .unwrap_or(default_material)
            .min(default_material);
        let xf = Mat4::from_cols_array(&visit.world.map(|v| v as f32));
        let normal_xf = Mat3::from_mat4(xf).inverse().transpose();
        let flip = xf.determinant() < 0.0;
        // Geometry of the same material and the same levels joins one list.
        let at =
            match out.prims.iter().enumerate().position(|(i, q)| {
                q.material == material && drawn_at.get(i) == Some(&visit.drawn_at)
            }) {
                Some(i) => i,
                None => {
                    out.prims.push(Prim {
                        material,
                        ..Default::default()
                    });
                    drawn_at.push(visit.drawn_at);
                    out.prims.len() - 1
                }
            };
        let Some(dst) = out.prims.get_mut(at) else {
            continue;
        };
        let base = u32::try_from(dst.positions.len()).map_err(|_| "too many vertices")?;
        dst.positions.extend(
            pos.iter()
                .map(|&q| xf.transform_point3(Vec3::from(q)).to_array()),
        );
        dst.normals.extend(
            normals
                .iter()
                .map(|&n| (normal_xf * Vec3::from(n)).normalize_or(Vec3::Y).to_array()),
        );
        dst.uvs.extend(uvs);
        if flip {
            for t in indices.as_chunks_mut::<3>().0 {
                t.swap(1, 2);
            }
        }
        dst.indices.extend(indices.iter().map(|i| i + base));
    }
    if out.prims.is_empty() {
        return Err("the file has no triangles".into());
    }
    let mut triangles = vec![0u64; plan.levels];
    for (prim, &mask) in out.prims.iter().zip(&drawn_at) {
        for k in lod::levels_of(mask) {
            if let Some(slot) = triangles.get_mut(k) {
                *slot = slot.saturating_add((prim.indices.len() / 3) as u64);
            }
        }
    }
    let read = plan.finish(&triangles);
    let box_of_finest = read
        .drawn
        .first()
        .and_then(|&k| Some((level_lo.get(k)?, level_hi.get(k)?)));
    let Some((min, max)) = box_of_finest else {
        return Err("the file has no triangles".into());
    };
    let size_m = lod::size_across(*min, *max);
    out.lods = ModelLods {
        read,
        drawn_at,
        size_m,
        source: plan.source,
    };
    Ok(out)
}

/// A decoded image: RGBA8, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    /// Width, pixels.
    pub width: u32,
    /// Height, pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

/// Decodes a PNG or JPEG image to RGBA8, at most [`MAX_TEXTURE_SIDE`] pixels per side.
pub fn decode_image(bytes: &[u8]) -> Result<Rgba, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("the image cannot be read: {e}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE_SIDE);
    limits.max_image_height = Some(MAX_TEXTURE_SIDE);
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|e| format!("the image cannot be decoded (PNG and JPEG are read): {e}"))?
        .to_rgba8();
    Ok(Rgba {
        width: img.width(),
        height: img.height(),
        pixels: img.into_raw(),
    })
}

/// The renderer's packed occlusion-roughness-metallic image from a glTF metallic-roughness
/// image (roughness in green, metallic in blue): the red channel, which glTF leaves to the
/// occlusion texture, becomes 1 unless the occlusion image is this same image.
pub fn orm_from_metal_rough(mut image: Rgba, occlusion_in_red: bool) -> Rgba {
    if !occlusion_in_red {
        for px in image.pixels.as_chunks_mut::<4>().0 {
            px[0] = 255;
        }
    }
    image
}
