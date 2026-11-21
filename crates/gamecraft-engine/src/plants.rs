//! The plant library: VisionCraft's built-in species, behind GameCraft's own types.
//!
//! This is the only module that uses `visioncraft-flora` (AGENTS.md rule 13): no VisionCraft
//! type leaves it.

use gamecraft_math::Portable;
use visioncraft_flora::{self as flora, Category, Species, species};

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

/// A tree's trunk as a collision column, for one plant of the library.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TrunkProfile {
    /// Radius at breast height, metres, at the plant's own size (before the entity's scale).
    pub radius_m: f64,
    /// Height of the column, metres: up to where the first branches spread.
    pub height_m: f64,
}

/// Trunks thinner than this do not block, metres.
const MIN_TRUNK_RADIUS_M: f64 = 0.08;
/// The column is at least this high, metres.
const MIN_TRUNK_HEIGHT_M: f64 = 2.0;
/// The column is at most this high, metres.
const MAX_TRUNK_HEIGHT_M: f64 = 8.0;
/// Breast height, metres: the radius is measured below it.
const BREAST_HEIGHT_M: f64 = 1.3;
/// For a plant whose bark is shorter than 1.3 m over this share, the radius is measured below
/// this share of the bark's height.
const YOUNG_BAND_SHARE: f64 = 0.25;
/// Which share of the bark's distances from the axis, lowest first, is taken for the trunk's
/// radius.
const TRUNK_PERCENTILE: f64 = 0.2;
/// The most a trunk's radius is of the plant's height.
const MAX_RADIUS_SHARE: f64 = 0.04;
/// Height of the slices the trunk's straightness is scanned in, metres.
const SLICE_M: f64 = 0.25;
/// A slice belongs to the trunk while the bark spreads at most this many radii (plus
/// [`SPREAD_SLACK_M`]) from the axis.
const SPREAD_RADII: f64 = 2.0;
/// See [`SPREAD_RADII`], metres.
const SPREAD_SLACK_M: f64 = 0.1;
/// Values are rounded to this step, metres, so that a last-bit difference of the generator's
/// `f32` maths on another platform cannot change the simulation.
const QUANTUM_M: f64 = 0.01;

/// The trunk of the plant `species` grown from `seed` to `growth`: measured on the same mesh the
/// renderer draws (the middle level of detail), as a column at its base. `None` for an unknown
/// species, for plants that are not trees, conifers or palms (grass, bushes and shrubs do not
/// block), and for a trunk that is too thin.
pub(crate) fn trunk_profile(species: &str, seed: u64, growth: f64) -> Option<TrunkProfile> {
    let s = Species::by_id(species)?;
    if !matches!(
        s.category,
        Category::Tree | Category::Conifer | Category::Palm
    ) {
        return None;
    }
    let mut params = s.params.clone();
    params.age = growth.clamp(f64::MIN_POSITIVE, 1.0) as f32;
    let mesh = flora::generate(&params, seed, flora::Lod::Lod1);
    let bark = mesh
        .split_parts()
        .into_iter()
        .find(|(m, _)| *m == flora::Material::Bark)
        .map(|(_, part)| part)?;
    // (height above the base, horizontal distance from the axis) of every bark vertex.
    let points: Vec<(f64, f64)> = bark
        .positions
        .iter()
        .map(|[x, y, z]| (f64::from(*y), f64::from(*x).phypot(f64::from(*z))))
        .filter(|(y, d)| y.is_finite() && d.is_finite() && *y >= 0.0)
        .collect();
    let top = points.iter().map(|p| p.0).fold(0.0, f64::max);
    let band = BREAST_HEIGHT_M.min(top * YOUNG_BAND_SHARE);
    let mut near: Vec<f64> = points
        .iter()
        .filter(|(y, _)| *y <= band)
        .map(|p| p.1)
        .collect();
    near.sort_by(f64::total_cmp);
    // Branches and drooping twigs only ever lie outside the trunk: a low percentile of the
    // distances is the trunk's own surface.
    let radius = near
        .get(((near.len().saturating_sub(1)) as f64 * TRUNK_PERCENTILE) as usize)
        .copied()?;
    // A weeping crown can hang right down to the ground and fool the measure: no trunk is
    // thicker than a share of the plant's height.
    let radius = radius.min(MAX_RADIUS_SHARE * f64::from(params.height_m) * f64::from(params.age));
    let radius = (radius / QUANTUM_M).round() / (1.0 / QUANTUM_M);
    if radius < MIN_TRUNK_RADIUS_M {
        return None;
    }
    // Up the slices while the bark stays near the axis.
    let limit = SPREAD_RADII * radius + SPREAD_SLACK_M;
    let mut slices = 0u32;
    while f64::from(slices) * SLICE_M < MAX_TRUNK_HEIGHT_M {
        let (lo, hi) = (f64::from(slices) * SLICE_M, f64::from(slices + 1) * SLICE_M);
        let spread = points
            .iter()
            .filter(|(y, _)| *y >= lo && *y < hi)
            .map(|p| p.1)
            .fold(0.0, f64::max);
        if spread > limit {
            break;
        }
        slices += 1;
    }
    let height = (f64::from(slices) * SLICE_M).clamp(MIN_TRUNK_HEIGHT_M, MAX_TRUNK_HEIGHT_M);
    Some(TrunkProfile {
        radius_m: radius,
        height_m: height,
    })
}
