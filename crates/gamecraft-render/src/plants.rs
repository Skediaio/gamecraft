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
    generate_level(species, seed, growth, 1)
}

/// [`generate`] at level of detail `level`: 0 close up, 1 mid distance, 2 far (few large
/// cards and a simplified trunk); a higher level is the far one.
pub(crate) fn generate_level(
    species: &str,
    seed: u64,
    growth: f64,
    level: u8,
) -> Option<GeneratedPlant> {
    let s = flora::Species::by_id(species)?;
    let mut params = s.params.clone();
    params.age = growth.clamp(f64::MIN_POSITIVE, 1.0) as f32;
    let lod = match level {
        0 => flora::Lod::Lod0,
        1 => flora::Lod::Lod1,
        _ => flora::Lod::Lod2,
    };
    let mesh = flora::generate(&params, seed, lod);
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

/// A plant at least this tall, metres, is a tree: its far level hands over to an impostor
/// (VisionCraft's `apps/visioncraft/src/plants.rs`, `TREE_M`).
pub(crate) const TREE_M: f32 = 3.0;
/// Impostors draw up to here at most, metres.
pub(crate) const IMPOSTOR_MAX_M: f32 = 2500.0;

/// The ends of a generated plant's three mesh levels and of its impostor (`None`: no
/// impostor, the far level is culled at its end), for a plant `h` metres tall. Ported from
/// VisionCraft's `apps/visioncraft/src/plants.rs` (`impostor_distances`, `lod_distances`,
/// `impostor_max`, MIT): a tree is a mesh up close and an impostor from `25 + 3 h` metres out
/// to where it is about 1.5 pixels tall; a shrub or ground plant has no impostor.
pub(crate) fn level_ends(h: f32) -> ([f32; 3], Option<f32>) {
    if h >= TREE_M {
        (
            [4.0 + h, 10.0 + 2.0 * h, 25.0 + 3.0 * h],
            Some((1540.0 * h).clamp(60.0, IMPOSTOR_MAX_M)),
        )
    } else {
        ([3.0 + 2.0 * h, 8.0 + 4.0 * h, 15.0 + 12.0 * h], None)
    }
}

/// The height of a generated plant: the top of its highest part, metres.
pub(crate) fn height_m(plant: &GeneratedPlant) -> f32 {
    plant
        .parts
        .iter()
        .flat_map(|p| p.positions.iter().map(|v| v[1]))
        .fold(0.0, f32::max)
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
