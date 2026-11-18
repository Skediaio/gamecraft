//! The plant library: VisionCraft's built-in species, behind GameCraft's own types.
//!
//! This is the only module that uses `visioncraft-flora` (AGENTS.md rule 13): no VisionCraft
//! type leaves it.

use visioncraft_flora::{Category, Species, species};

/// One species of the plant library.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesInfo {
    /// The id a `plant` component names (`quercus-robur`).
    pub id: String,
    /// The English common name (`English oak`).
    pub common_name: String,
    /// The botanical name (`Quercus robur`).
    pub botanical_name: String,
    /// `tree`, `conifer`, `palm`, `shrub`, `hedge`, `perennial`, `fern`, `grass`,
    /// `ground_cover` or `climber`.
    pub category: String,
    /// Height of a mature plant, metres.
    pub mature_height_m: f64,
    /// Width of a mature plant, metres.
    pub mature_width_m: f64,
    /// Whether it keeps its leaves in winter.
    pub evergreen: bool,
}

fn category_name(c: Category) -> &'static str {
    match c {
        Category::Tree => "tree",
        Category::Conifer => "conifer",
        Category::Palm => "palm",
        Category::Shrub => "shrub",
        Category::Hedge => "hedge",
        Category::Perennial => "perennial",
        Category::Fern => "fern",
        Category::Grass => "grass",
        Category::GroundCover => "ground_cover",
        Category::Climber => "climber",
    }
}

fn info(s: &Species) -> SpeciesInfo {
    SpeciesInfo {
        id: s.id.to_string(),
        common_name: s.common_name.to_string(),
        botanical_name: s.botanical_name.to_string(),
        category: category_name(s.category).to_string(),
        mature_height_m: f64::from(s.mature_height_m),
        mature_width_m: f64::from(s.mature_width_m),
        evergreen: s.evergreen,
    }
}

/// Every species of the library, in the library's order.
pub fn species_list() -> Vec<SpeciesInfo> {
    species().iter().map(info).collect()
}

/// Whether `id` is a species of the library (exact match).
pub fn species_exists(id: &str) -> bool {
    Species::by_id(id).is_some()
}

/// The species id with the smallest edit distance to `id` (ignoring case); `None` only when
/// the library is empty.
pub fn nearest_species(id: &str) -> Option<String> {
    let wanted: String = id.to_lowercase().chars().take(100).collect();
    species()
        .iter()
        .map(|s| (crate::spec::edit_distance(&wanted, s.id), s.id))
        .min_by_key(|(d, _)| *d)
        .map(|(_, id)| id.to_string())
}
