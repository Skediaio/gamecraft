//! The plant generator side: species lookup, nearest ids and generation. The only module that
//! touches `visioncraft-flora`.
//!
//! How a generated plant becomes renderer meshes and materials follows VisionCraft's
//! application (`apps/visioncraft/src/plants.rs` and `scene.rs`, `plant_material_desc`):
//! bark opaque with its normal map; foliage, flowers and twigs alpha-cut at 0.5 and
//! double-sided, foliage tinted by the mesh's foliage tint.

use visioncraft_flora as flora;

/// What a plant part is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PartKind {
    /// Opaque bark with its normal map.
    Bark,
    /// Alpha-cut, double-sided leaves, tinted.
    Foliage,
    /// Alpha-cut, double-sided flowers.
    Flower,
    /// Alpha-cut, double-sided bare twigs.
    Twigs,
}

/// One part of a generated plant, ready to upload.
pub(crate) struct PlantPart {
    pub kind: PartKind,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub wind: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// A generated plant: its parts and the foliage tint.
pub(crate) struct GeneratedPlant {
    pub parts: Vec<PlantPart>,
    pub foliage_tint: [f32; 3],
}

/// An RGBA8 image, rows top to bottom.
pub(crate) struct PlantImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A species' textures.
pub(crate) struct SpeciesTextures {
    /// Leaf, flower and twig atlas (sRGB, clamped).
    pub atlas: PlantImage,
    /// The atlas' normal map, when made.
    pub atlas_normal: Option<PlantImage>,
    /// Tileable bark (sRGB, repeated).
    pub bark: PlantImage,
    /// Tileable bark normal map (linear, repeated).
    pub bark_normal: PlantImage,
}

/// Whether the plant library knows `species`.
pub(crate) fn known(species: &str) -> bool {
    flora::Species::by_id(species).is_some()
}

/// The library's mature height of `species`, metres.
pub(crate) fn mature_height_m(species: &str) -> Option<f64> {
    flora::Species::by_id(species).map(|s| f64::from(s.mature_height_m))
}

/// The known species id closest to `species` (fewest single-character edits; the library's
/// order breaks ties).
pub(crate) fn nearest_species(species: &str) -> Option<&'static str> {
    let wanted = species.trim().to_lowercase();
    flora::species()
        .iter()
        .enumerate()
        .min_by_key(|(order, s)| (edit_distance(&wanted, s.id), *order))
        .map(|(_, s)| s.id)
}

/// The Levenshtein distance between two strings, in characters.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = Vec::with_capacity(b.len() + 1);
        cur.push(i + 1);
        for (j, cb) in b.iter().enumerate() {
            let substitute = prev
                .get(j)
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(usize::from(ca != *cb));
            let delete = prev
                .get(j + 1)
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            let insert = cur.last().copied().unwrap_or(usize::MAX).saturating_add(1);
            cur.push(substitute.min(delete).min(insert));
        }
        prev = cur;
    }
    prev.last().copied().unwrap_or(0)
}

/// Generates `species` with `seed` at `growth` (0 to 1] at the middle level of detail, at its
/// peak display (full foliage, in flower when it flowers). `None` for an unknown species.
pub(crate) fn generate(species: &str, seed: u64, growth: f64) -> Option<GeneratedPlant> {
    let s = flora::Species::by_id(species)?;
    let mut params = s.params.clone();
    params.age = growth.clamp(f64::MIN_POSITIVE, 1.0) as f32;
    let mesh = flora::generate(&params, seed, flora::Lod::Lod1);
    let parts = mesh
        .split_parts()
        .into_iter()
        .map(|(material, part)| PlantPart {
            kind: match material {
                flora::Material::Bark => PartKind::Bark,
                flora::Material::Foliage => PartKind::Foliage,
                flora::Material::Flower => PartKind::Flower,
                flora::Material::Twigs => PartKind::Twigs,
            },
            positions: part.positions,
            normals: part.normals,
            uvs: part.uvs,
            wind: part.wind,
            indices: part.indices,
        })
        .collect();
    Some(GeneratedPlant {
        parts,
        foliage_tint: mesh.foliage_tint,
    })
}

/// The textures of `species`. `None` for an unknown species.
pub(crate) fn textures(species: &str) -> Option<SpeciesTextures> {
    let s = flora::Species::by_id(species)?;
    let t = flora::textures(&s.params);
    let image = |i: flora::Image| PlantImage {
        width: i.width,
        height: i.height,
        rgba: i.rgba,
    };
    Some(SpeciesTextures {
        atlas: image(t.atlas),
        atlas_normal: t.atlas_normal.map(image),
        bark: image(t.bark),
        bark_normal: image(t.bark_normal),
    })
}
